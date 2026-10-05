use crate::error::EnedisError;
use crate::models::{
    AggregatedMeasurement, AggregationInterval, FlowDirection, Measurement, MeasurementQuality,
    PointId, Unit,
};
use crate::storage::models::{SyncState, UpsertStats};
use crate::storage::retention::{RetentionPolicy, RollupStats};
use crate::storage::traits::{StorageBackend, StorageFuture};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::{Row, SqlitePool};
use std::str::FromStr;

/// Implémentation du stockage temporel pour SQLite (fichier local ou mémoire)
#[derive(Clone)]
pub struct SqliteStorage {
    pool: SqlitePool,
}

impl SqliteStorage {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Ouvre ou crée une base SQLite à partir d'une URL de connexion (ex: `sqlite://enedis.db` ou `sqlite::memory:`)
    pub async fn connect(url: &str) -> Result<Self, EnedisError> {
        let pool = SqlitePool::connect(url)
            .await
            .map_err(EnedisError::Database)?;
        let storage = Self::new(pool);
        storage.init_schema().await?;
        Ok(storage)
    }
}

impl StorageBackend for SqliteStorage {
    fn init_schema(&self) -> StorageFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS measurements (
                    point_id TEXT NOT NULL,
                    timestamp TEXT NOT NULL,
                    direction TEXT NOT NULL,
                    interval_seconds INTEGER NOT NULL,
                    value TEXT NOT NULL,
                    unit TEXT NOT NULL,
                    quality INTEGER NOT NULL,
                    inserted_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                    PRIMARY KEY (point_id, timestamp, direction)
                );

                CREATE INDEX IF NOT EXISTS idx_measurements_point_range 
                ON measurements (point_id, direction, timestamp DESC);

                CREATE TABLE IF NOT EXISTS sync_state (
                    point_id TEXT NOT NULL,
                    direction TEXT NOT NULL,
                    last_synced_timestamp TEXT NOT NULL,
                    last_sync_attempt TEXT NOT NULL DEFAULT (datetime('now')),
                    sync_status TEXT NOT NULL DEFAULT 'OK',
                    PRIMARY KEY (point_id, direction)
                );

                CREATE TABLE IF NOT EXISTS tempo_days (
                    date TEXT PRIMARY KEY,
                    color TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS ecowatt_signals (
                    timestamp TEXT PRIMARY KEY,
                    level INTEGER NOT NULL,
                    message TEXT
                );

                CREATE TABLE IF NOT EXISTS spot_prices (
                    timestamp TEXT PRIMARY KEY,
                    price_eur_per_mwh TEXT NOT NULL,
                    price_eur_per_kwh TEXT NOT NULL,
                    is_negative INTEGER NOT NULL DEFAULT 0,
                    source TEXT NOT NULL DEFAULT 'EPEX_SPOT_FR'
                );
                "#,
            )
            .execute(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            Ok(())
        })
    }

    fn upsert_measurements<'a>(
        &'a self,
        measurements: &'a [Measurement],
    ) -> StorageFuture<'a, UpsertStats> {
        Box::pin(async move {
            if measurements.is_empty() {
                return Ok(UpsertStats::default());
            }

            let mut tx = self.pool.begin().await.map_err(EnedisError::Database)?;
            let mut affected = 0;

            // Lot d'insertion de 100 mesures max (100 x 7 paramètres = 700 < limite SQLite 999)
            const BATCH_SIZE: usize = 100;

            for chunk in measurements.chunks(BATCH_SIZE) {
                let mut builder: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
                    "INSERT INTO measurements (point_id, timestamp, direction, interval_seconds, value, unit, quality) ",
                );

                builder.push_values(chunk, |mut b, m| {
                    b.push_bind(m.point_id.as_str())
                        .push_bind(m.timestamp.to_rfc3339())
                        .push_bind(m.direction.as_str())
                        .push_bind(m.interval_seconds as i64)
                        .push_bind(m.value.to_string())
                        .push_bind(m.unit.as_str())
                        .push_bind(m.quality.as_u8() as i64);
                });

                builder.push(
                    " ON CONFLICT(point_id, timestamp, direction) DO UPDATE SET \
                     value = excluded.value, \
                     interval_seconds = excluded.interval_seconds, \
                     unit = excluded.unit, \
                     quality = excluded.quality, \
                     updated_at = datetime('now') \
                     WHERE excluded.quality >= measurements.quality",
                );

                let res = builder
                    .build()
                    .execute(&mut *tx)
                    .await
                    .map_err(EnedisError::Database)?;

                affected += res.rows_affected() as usize;
            }

            tx.commit().await.map_err(EnedisError::Database)?;

            Ok(UpsertStats {
                processed: measurements.len(),
                affected,
            })
        })
    }

    fn get_measurements<'a>(
        &'a self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        direction: Option<FlowDirection>,
    ) -> StorageFuture<'a, Vec<Measurement>> {
        self.get_measurements_limited(point_id, from, to, direction, None)
    }

    fn get_measurements_limited<'a>(
        &'a self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        direction: Option<FlowDirection>,
        limit: Option<usize>,
    ) -> StorageFuture<'a, Vec<Measurement>> {
        Box::pin(async move {
            let from_str = from.to_rfc3339();
            let to_str = to.to_rfc3339();
            let prm_str = point_id.as_str();

            let rows = match (direction, limit) {
                (Some(dir), Some(lim)) => {
                    sqlx::query(
                        r#"
                        SELECT point_id, timestamp, direction, interval_seconds, value, unit, quality
                        FROM measurements
                        WHERE point_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3 AND direction = ?4
                        ORDER BY timestamp ASC
                        LIMIT ?5
                        "#,
                    )
                    .bind(prm_str)
                    .bind(from_str)
                    .bind(to_str)
                    .bind(dir.as_str())
                    .bind(lim as i64)
                    .fetch_all(&self.pool)
                    .await
                }
                (Some(dir), None) => {
                    sqlx::query(
                        r#"
                        SELECT point_id, timestamp, direction, interval_seconds, value, unit, quality
                        FROM measurements
                        WHERE point_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3 AND direction = ?4
                        ORDER BY timestamp ASC
                        "#,
                    )
                    .bind(prm_str)
                    .bind(from_str)
                    .bind(to_str)
                    .bind(dir.as_str())
                    .fetch_all(&self.pool)
                    .await
                }
                (None, Some(lim)) => {
                    sqlx::query(
                        r#"
                        SELECT point_id, timestamp, direction, interval_seconds, value, unit, quality
                        FROM measurements
                        WHERE point_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3
                        ORDER BY timestamp ASC
                        LIMIT ?4
                        "#,
                    )
                    .bind(prm_str)
                    .bind(from_str)
                    .bind(to_str)
                    .bind(lim as i64)
                    .fetch_all(&self.pool)
                    .await
                }
                (None, None) => {
                    sqlx::query(
                        r#"
                        SELECT point_id, timestamp, direction, interval_seconds, value, unit, quality
                        FROM measurements
                        WHERE point_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3
                        ORDER BY timestamp ASC
                        "#,
                    )
                    .bind(prm_str)
                    .bind(from_str)
                    .bind(to_str)
                    .fetch_all(&self.pool)
                    .await
                }
            }
            .map_err(EnedisError::Database)?;

            let mut results = Vec::with_capacity(rows.len());
            for row in rows {
                let pid_str: String = row.get("point_id");
                let ts_str: String = row.get("timestamp");
                let dir_str: String = row.get("direction");
                let interval: i64 = row.get("interval_seconds");
                let val_str: String = row.get("value");
                let unit_str: String = row.get("unit");
                let quality_val: i64 = row.get("quality");

                let pid = PointId::new(&pid_str).map_err(|e| {
                    EnedisError::StorageDecode(format!("PRM corrompu en base: {}", e))
                })?;
                let ts = DateTime::parse_from_rfc3339(&ts_str)
                    .map_err(|e| {
                        EnedisError::StorageDecode(format!("Timestamp invalide en base: {}", e))
                    })?
                    .with_timezone(&Utc);
                let val = Decimal::from_str(&val_str).map_err(|e| {
                    EnedisError::StorageDecode(format!("Valeur numérique invalide en base: {}", e))
                })?;

                results.push(Measurement {
                    point_id: pid,
                    timestamp: ts,
                    interval_seconds: interval as u32,
                    direction: FlowDirection::from_sge_code(&dir_str),
                    value: val,
                    unit: Unit::from_sge_code(&unit_str),
                    quality: MeasurementQuality::from_u8(quality_val as u8)
                        .unwrap_or(MeasurementQuality::Estimated),
                });
            }

            Ok(results)
        })
    }

    fn get_aggregated_measurements<'a>(
        &'a self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: AggregationInterval,
        direction: Option<FlowDirection>,
    ) -> StorageFuture<'a, Vec<AggregatedMeasurement>> {
        Box::pin(async move {
            let from_str = from.to_rfc3339();
            let to_str = to.to_rfc3339();
            let prm_str = point_id.as_str();

            let strftime_format = match interval {
                AggregationInterval::Hourly => "%Y-%m-%dT%H:00:00Z",
                AggregationInterval::Daily => "%Y-%m-%dT00:00:00Z",
                AggregationInterval::Monthly => "%Y-%m-01T00:00:00Z",
                AggregationInterval::Yearly => "%Y-01-01T00:00:00Z",
            };

            let sql = format!(
                r#"
                SELECT
                    point_id,
                    direction,
                    strftime('{fmt}', timestamp) AS bucket_start,
                    SUM(
                        CASE 
                            WHEN UPPER(unit) = 'KWH' THEN CAST(value AS REAL)
                            WHEN UPPER(unit) = 'WH' THEN CAST(value AS REAL) / 1000.0
                            WHEN UPPER(unit) = 'KW' OR UPPER(unit) = 'KVA' THEN (CAST(value AS REAL) * interval_seconds) / 3600.0
                            WHEN UPPER(unit) = 'W' OR UPPER(unit) = 'VA' THEN (CAST(value AS REAL) * interval_seconds) / 3600000.0
                            ELSE CAST(value AS REAL)
                        END
                    ) AS total_energy_kwh,
                    MAX(
                        CASE
                            WHEN UPPER(unit) = 'W' OR UPPER(unit) = 'VA' THEN CAST(value AS REAL)
                            WHEN UPPER(unit) = 'KW' OR UPPER(unit) = 'KVA' THEN CAST(value AS REAL) * 1000.0
                            WHEN UPPER(unit) = 'WH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (CAST(value AS REAL) * 3600.0) / interval_seconds ELSE CAST(value AS REAL) END
                            WHEN UPPER(unit) = 'KWH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (CAST(value AS REAL) * 3600000.0) / interval_seconds ELSE CAST(value AS REAL) * 1000.0 END
                            ELSE CAST(value AS REAL)
                        END
                    ) AS max_power_w,
                    MIN(
                        CASE
                            WHEN UPPER(unit) = 'W' OR UPPER(unit) = 'VA' THEN CAST(value AS REAL)
                            WHEN UPPER(unit) = 'KW' OR UPPER(unit) = 'KVA' THEN CAST(value AS REAL) * 1000.0
                            WHEN UPPER(unit) = 'WH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (CAST(value AS REAL) * 3600.0) / interval_seconds ELSE CAST(value AS REAL) END
                            WHEN UPPER(unit) = 'KWH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (CAST(value AS REAL) * 3600000.0) / interval_seconds ELSE CAST(value AS REAL) * 1000.0 END
                            ELSE CAST(value AS REAL)
                        END
                    ) AS min_power_w,
                    AVG(
                        CASE
                            WHEN UPPER(unit) = 'W' OR UPPER(unit) = 'VA' THEN CAST(value AS REAL)
                            WHEN UPPER(unit) = 'KW' OR UPPER(unit) = 'KVA' THEN CAST(value AS REAL) * 1000.0
                            WHEN UPPER(unit) = 'WH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (CAST(value AS REAL) * 3600.0) / interval_seconds ELSE CAST(value AS REAL) END
                            WHEN UPPER(unit) = 'KWH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (CAST(value AS REAL) * 3600000.0) / interval_seconds ELSE CAST(value AS REAL) * 1000.0 END
                            ELSE CAST(value AS REAL)
                        END
                    ) AS avg_power_w,
                    COUNT(*) AS sample_count
                FROM measurements
                WHERE point_id = ?1 AND timestamp >= ?2 AND timestamp <= ?3 {dir_clause}
                GROUP BY point_id, direction, strftime('{fmt}', timestamp)
                ORDER BY bucket_start ASC, direction ASC
                "#,
                fmt = strftime_format,
                dir_clause = if direction.is_some() {
                    "AND direction = ?4"
                } else {
                    ""
                }
            );

            let rows = match direction {
                Some(dir) => {
                    sqlx::query(&sql)
                        .bind(prm_str)
                        .bind(from_str)
                        .bind(to_str)
                        .bind(dir.as_str())
                        .fetch_all(&self.pool)
                        .await
                }
                None => {
                    sqlx::query(&sql)
                        .bind(prm_str)
                        .bind(from_str)
                        .bind(to_str)
                        .fetch_all(&self.pool)
                        .await
                }
            }
            .map_err(EnedisError::Database)?;

            let mut results = Vec::with_capacity(rows.len());
            for row in rows {
                let pid_str: String = row.get("point_id");
                let dir_str: String = row.get("direction");
                let b_start_str: String = row.get("bucket_start");
                let total_energy: Option<f64> = row.try_get("total_energy_kwh").ok();
                let max_p: Option<f64> = row.try_get("max_power_w").ok();
                let min_p: Option<f64> = row.try_get("min_power_w").ok();
                let avg_p: Option<f64> = row.try_get("avg_power_w").ok();
                let count: i64 = row.get("sample_count");

                let pid = PointId::new(&pid_str).map_err(|e| {
                    EnedisError::StorageDecode(format!("PRM corrompu en base: {}", e))
                })?;
                let bucket_start = DateTime::parse_from_rfc3339(&b_start_str)
                    .map_err(|e| {
                        EnedisError::StorageDecode(format!(
                            "Date d'agrégation invalide en base: {}",
                            e
                        ))
                    })?
                    .with_timezone(&Utc);
                let bucket_end = interval.bucket_end(bucket_start);

                let total_energy_kwh = total_energy
                    .and_then(|v| Decimal::from_f64_retain(v).map(|d| d.round_dp(4)))
                    .unwrap_or(Decimal::ZERO);
                let max_power_w =
                    max_p.and_then(|v| Decimal::from_f64_retain(v).map(|d| d.round_dp(4)));
                let min_power_w =
                    min_p.and_then(|v| Decimal::from_f64_retain(v).map(|d| d.round_dp(4)));
                let avg_power_w =
                    avg_p.and_then(|v| Decimal::from_f64_retain(v).map(|d| d.round_dp(4)));

                results.push(AggregatedMeasurement {
                    point_id: pid,
                    bucket_start,
                    bucket_end,
                    direction: FlowDirection::from_sge_code(&dir_str),
                    total_energy_kwh,
                    max_power_w,
                    min_power_w,
                    avg_power_w,
                    sample_count: count as u32,
                });
            }

            Ok(results)
        })
    }

    fn get_sync_state(
        &self,
        point_id: PointId,
        direction: FlowDirection,
    ) -> StorageFuture<'_, Option<SyncState>> {
        Box::pin(async move {
            let row = sqlx::query(
                r#"
                SELECT point_id, direction, last_synced_timestamp, last_sync_attempt, sync_status
                FROM sync_state
                WHERE point_id = ?1 AND direction = ?2
                "#,
            )
            .bind(point_id.as_str())
            .bind(direction.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            if let Some(r) = row {
                let pid_str: String = r.get("point_id");
                let dir_str: String = r.get("direction");
                let last_sync_str: String = r.get("last_synced_timestamp");
                let attempt_str: String = r.get("last_sync_attempt");
                let status: String = r.get("sync_status");

                let last_sync = DateTime::parse_from_rfc3339(&last_sync_str)
                    .map_err(|e| EnedisError::StorageDecode(e.to_string()))?
                    .with_timezone(&Utc);
                let attempt = DateTime::parse_from_rfc3339(&attempt_str)
                    .map_err(|e| EnedisError::StorageDecode(e.to_string()))?
                    .with_timezone(&Utc);

                let pid = PointId::new(&pid_str).map_err(|e| {
                    EnedisError::StorageDecode(format!("PRM corrompu en base: {}", e))
                })?;

                Ok(Some(SyncState {
                    point_id: pid,
                    direction: FlowDirection::from_sge_code(&dir_str),
                    last_synced_timestamp: last_sync,
                    last_sync_attempt: attempt,
                    sync_status: status,
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn update_sync_state<'a>(&'a self, state: &'a SyncState) -> StorageFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query(
                r#"
                INSERT INTO sync_state (
                    point_id, direction, last_synced_timestamp, last_sync_attempt, sync_status
                ) VALUES (
                    ?1, ?2, ?3, ?4, ?5
                )
                ON CONFLICT(point_id, direction) DO UPDATE SET
                    last_synced_timestamp = excluded.last_synced_timestamp,
                    last_sync_attempt = excluded.last_sync_attempt,
                    sync_status = excluded.sync_status;
                "#,
            )
            .bind(state.point_id.as_str())
            .bind(state.direction.as_str())
            .bind(state.last_synced_timestamp.to_rfc3339())
            .bind(state.last_sync_attempt.to_rfc3339())
            .bind(&state.sync_status)
            .execute(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            Ok(())
        })
    }

    fn list_sync_points(&self) -> StorageFuture<'_, Vec<PointId>> {
        Box::pin(async move {
            let rows =
                sqlx::query("SELECT DISTINCT point_id FROM sync_state ORDER BY point_id ASC")
                    .fetch_all(&self.pool)
                    .await
                    .map_err(EnedisError::Database)?;

            let mut points = Vec::with_capacity(rows.len());
            for row in rows {
                let pid_str: String = row.get("point_id");
                if let Ok(pid) = PointId::new(&pid_str) {
                    points.push(pid);
                }
            }
            Ok(points)
        })
    }

    fn get_total_energy_kwh<'a>(
        &'a self,
        point_id: PointId,
        direction: FlowDirection,
    ) -> StorageFuture<'a, Option<Decimal>> {
        Box::pin(async move {
            let row = sqlx::query(
                r#"
                SELECT SUM(
                    CASE 
                        WHEN unit = 'Wh' OR unit = 'WH' THEN CAST(value AS REAL) / 1000.0
                        WHEN unit = 'W' THEN (CAST(value AS REAL) / 1000.0) * (interval_seconds / 3600.0)
                        WHEN unit = 'kW' OR unit = 'KW' THEN CAST(value AS REAL) * (interval_seconds / 3600.0)
                        ELSE CAST(value AS REAL)
                    END
                ) as total_kwh
                FROM measurements
                WHERE point_id = ?1 AND direction = ?2
                "#,
            )
            .bind(point_id.as_str())
            .bind(direction.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            let total: Option<f64> = row.try_get("total_kwh").ok();
            match total {
                Some(val) => {
                    let dec = Decimal::from_f64_retain(val).unwrap_or(Decimal::ZERO);
                    Ok(Some(dec))
                }
                None => Ok(None),
            }
        })
    }

    fn get_latest_measurement<'a>(
        &'a self,
        point_id: PointId,
        direction: FlowDirection,
    ) -> StorageFuture<'a, Option<Measurement>> {
        Box::pin(async move {
            let row = sqlx::query(
                r#"
                SELECT point_id, timestamp, direction, interval_seconds, value, unit, quality
                FROM measurements
                WHERE point_id = ?1 AND direction = ?2
                ORDER BY timestamp DESC
                LIMIT 1
                "#,
            )
            .bind(point_id.as_str())
            .bind(direction.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            if let Some(r) = row {
                let pid_str: String = r.get("point_id");
                let ts_str: String = r.get("timestamp");
                let dir_str: String = r.get("direction");
                let interval: i64 = r.get("interval_seconds");
                let val_str: String = r.get("value");
                let unit_str: String = r.get("unit");
                let quality_val: i64 = r.get("quality");

                let pid = PointId::new(&pid_str).map_err(|e| {
                    EnedisError::StorageDecode(format!("PRM corrompu en base: {}", e))
                })?;
                let ts = DateTime::parse_from_rfc3339(&ts_str)
                    .map_err(|e| {
                        EnedisError::StorageDecode(format!("Timestamp invalide en base: {}", e))
                    })?
                    .with_timezone(&Utc);
                let val = Decimal::from_str(&val_str).map_err(|e| {
                    EnedisError::StorageDecode(format!("Valeur numérique invalide en base: {}", e))
                })?;

                Ok(Some(Measurement {
                    point_id: pid,
                    timestamp: ts,
                    interval_seconds: interval as u32,
                    direction: FlowDirection::from_sge_code(&dir_str),
                    value: val,
                    unit: Unit::from_sge_code(&unit_str),
                    quality: MeasurementQuality::from_u8(quality_val as u8)
                        .unwrap_or(MeasurementQuality::Estimated),
                }))
            } else {
                Ok(None)
            }
        })
    }

    fn upsert_tempo_days<'a>(
        &'a self,
        records: &'a [crate::models::TempoDayRecord],
    ) -> StorageFuture<'a, ()> {
        Box::pin(async move {
            if records.is_empty() {
                return Ok(());
            }
            let mut tx = self.pool.begin().await.map_err(EnedisError::Database)?;
            for chunk in records.chunks(100) {
                let mut builder: sqlx::QueryBuilder<sqlx::Sqlite> =
                    sqlx::QueryBuilder::new("INSERT INTO tempo_days (date, color, updated_at) ");
                builder.push_values(chunk, |mut b, r| {
                    b.push_bind(r.date.to_string())
                        .push_bind(r.color.as_str())
                        .push_bind(r.updated_at.to_rfc3339());
                });
                builder.push(
                    " ON CONFLICT(date) DO UPDATE SET color = excluded.color, updated_at = excluded.updated_at",
                );
                builder
                    .build()
                    .execute(&mut *tx)
                    .await
                    .map_err(EnedisError::Database)?;
            }
            tx.commit().await.map_err(EnedisError::Database)?;
            Ok(())
        })
    }

    fn get_tempo_days<'a>(
        &'a self,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> StorageFuture<'a, Vec<crate::models::TempoDayRecord>> {
        Box::pin(async move {
            let from_str = from.to_string();
            let to_str = to.to_string();
            let rows = sqlx::query(
                "SELECT date, color, updated_at FROM tempo_days WHERE date >= ?1 AND date <= ?2 ORDER BY date ASC",
            )
            .bind(from_str)
            .bind(to_str)
            .fetch_all(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            let mut results = Vec::with_capacity(rows.len());
            for row in rows {
                let d_str: String = row.get("date");
                let c_str: String = row.get("color");
                let u_str: String = row.get("updated_at");

                let d = chrono::NaiveDate::parse_from_str(&d_str, "%Y-%m-%d")
                    .map_err(|e| EnedisError::StorageDecode(e.to_string()))?;
                let u = DateTime::parse_from_rfc3339(&u_str)
                    .map_err(|e| EnedisError::StorageDecode(e.to_string()))?
                    .with_timezone(&Utc);

                results.push(crate::models::TempoDayRecord {
                    date: d,
                    color: crate::models::TempoColor::from_str_code(&c_str),
                    updated_at: u,
                });
            }
            Ok(results)
        })
    }

    fn upsert_ecowatt_signals<'a>(
        &'a self,
        signals: &'a [crate::models::EcoWattSignal],
    ) -> StorageFuture<'a, ()> {
        Box::pin(async move {
            if signals.is_empty() {
                return Ok(());
            }
            let mut tx = self.pool.begin().await.map_err(EnedisError::Database)?;
            for chunk in signals.chunks(100) {
                let mut builder: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
                    "INSERT INTO ecowatt_signals (timestamp, level, message) ",
                );
                builder.push_values(chunk, |mut b, s| {
                    b.push_bind(s.timestamp.to_rfc3339())
                        .push_bind(s.level.as_u8() as i64)
                        .push_bind(s.message.as_deref());
                });
                builder.push(
                    " ON CONFLICT(timestamp) DO UPDATE SET level = excluded.level, message = excluded.message",
                );
                builder
                    .build()
                    .execute(&mut *tx)
                    .await
                    .map_err(EnedisError::Database)?;
            }
            tx.commit().await.map_err(EnedisError::Database)?;
            Ok(())
        })
    }

    fn get_ecowatt_signals<'a>(
        &'a self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> StorageFuture<'a, Vec<crate::models::EcoWattSignal>> {
        Box::pin(async move {
            let from_str = from.to_rfc3339();
            let to_str = to.to_rfc3339();
            let rows = sqlx::query(
                "SELECT timestamp, level, message FROM ecowatt_signals WHERE timestamp >= ?1 AND timestamp <= ?2 ORDER BY timestamp ASC",
            )
            .bind(from_str)
            .bind(to_str)
            .fetch_all(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            let mut results = Vec::with_capacity(rows.len());
            for row in rows {
                let t_str: String = row.get("timestamp");
                let lvl_val: i64 = row.get("level");
                let msg: Option<String> = row.get("message");

                let t = DateTime::parse_from_rfc3339(&t_str)
                    .map_err(|e| EnedisError::StorageDecode(e.to_string()))?
                    .with_timezone(&Utc);

                results.push(crate::models::EcoWattSignal {
                    timestamp: t,
                    level: crate::models::EcoWattLevel::from_u8(lvl_val as u8)
                        .unwrap_or(crate::models::EcoWattLevel::Green),
                    message: msg,
                });
            }
            Ok(results)
        })
    }

    fn upsert_spot_prices<'a>(
        &'a self,
        prices: &'a [crate::models::SpotPriceRecord],
    ) -> StorageFuture<'a, ()> {
        Box::pin(async move {
            if prices.is_empty() {
                return Ok(());
            }
            let mut tx = self.pool.begin().await.map_err(EnedisError::Database)?;
            for chunk in prices.chunks(100) {
                let mut builder: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
                    "INSERT INTO spot_prices (timestamp, price_eur_per_mwh, price_eur_per_kwh, is_negative, source) ",
                );
                builder.push_values(chunk, |mut b, p| {
                    b.push_bind(p.timestamp.to_rfc3339())
                        .push_bind(p.price_eur_per_mwh.to_string())
                        .push_bind(p.price_eur_per_kwh.to_string())
                        .push_bind(if p.is_negative { 1i64 } else { 0i64 })
                        .push_bind(&p.source);
                });
                builder.push(
                    " ON CONFLICT(timestamp) DO UPDATE SET price_eur_per_mwh = excluded.price_eur_per_mwh, price_eur_per_kwh = excluded.price_eur_per_kwh, is_negative = excluded.is_negative, source = excluded.source",
                );
                builder
                    .build()
                    .execute(&mut *tx)
                    .await
                    .map_err(EnedisError::Database)?;
            }
            tx.commit().await.map_err(EnedisError::Database)?;
            Ok(())
        })
    }

    fn get_spot_prices<'a>(
        &'a self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> StorageFuture<'a, Vec<crate::models::SpotPriceRecord>> {
        Box::pin(async move {
            let from_str = from.to_rfc3339();
            let to_str = to.to_rfc3339();
            let rows = sqlx::query(
                "SELECT timestamp, price_eur_per_mwh, price_eur_per_kwh, is_negative, source FROM spot_prices WHERE timestamp >= ?1 AND timestamp <= ?2 ORDER BY timestamp ASC",
            )
            .bind(from_str)
            .bind(to_str)
            .fetch_all(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            let mut results = Vec::with_capacity(rows.len());
            for row in rows {
                let t_str: String = row.get("timestamp");
                let mwh_str: String = row.get("price_eur_per_mwh");
                let kwh_str: String = row.get("price_eur_per_kwh");
                let is_neg: i64 = row.get("is_negative");
                let src: String = row.get("source");

                let t = DateTime::parse_from_rfc3339(&t_str)
                    .map_err(|e| EnedisError::StorageDecode(e.to_string()))?
                    .with_timezone(&Utc);

                let mwh = rust_decimal::Decimal::from_str_exact(&mwh_str)
                    .unwrap_or(rust_decimal::Decimal::ZERO);
                let kwh = rust_decimal::Decimal::from_str_exact(&kwh_str)
                    .unwrap_or(rust_decimal::Decimal::ZERO);

                results.push(crate::models::SpotPriceRecord {
                    timestamp: t,
                    price_eur_per_mwh: mwh,
                    price_eur_per_kwh: kwh,
                    is_negative: is_neg != 0,
                    source: src,
                });
            }
            Ok(results)
        })
    }

    fn detect_missing_ranges<'a>(
        &'a self,
        point_id: PointId,
        direction: FlowDirection,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> StorageFuture<'a, Vec<(DateTime<Utc>, DateTime<Utc>)>> {
        Box::pin(async move {
            if from >= to {
                return Ok(Vec::new());
            }

            let from_str = from.to_rfc3339();
            let to_str = to.to_rfc3339();
            let prm_str = point_id.as_str();

            let rows = sqlx::query(
                r#"
                SELECT DISTINCT substr(timestamp, 1, 10) as day
                FROM measurements
                WHERE point_id = ?1 AND direction = ?2 AND timestamp >= ?3 AND timestamp < ?4
                ORDER BY day ASC
                "#,
            )
            .bind(prm_str)
            .bind(direction.as_str())
            .bind(from_str)
            .bind(to_str)
            .fetch_all(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            let mut existing_days = std::collections::HashSet::new();
            for r in rows {
                let day_str: String = r.get("day");
                if let Ok(d) = chrono::NaiveDate::parse_from_str(&day_str, "%Y-%m-%d") {
                    existing_days.insert(d);
                }
            }

            let start_date = from.date_naive();
            let end_date = (to - chrono::Duration::seconds(1)).date_naive();

            if start_date > end_date {
                return Ok(Vec::new());
            }

            let mut missing_ranges = Vec::new();
            let mut current_gap_start: Option<chrono::NaiveDate> = None;
            let mut current_gap_end: Option<chrono::NaiveDate> = None;

            let mut cur = start_date;
            while cur <= end_date {
                if !existing_days.contains(&cur) {
                    if current_gap_start.is_none() {
                        current_gap_start = Some(cur);
                    }
                    current_gap_end = Some(cur);
                } else if let (Some(g_start), Some(g_end)) =
                    (current_gap_start.take(), current_gap_end.take())
                {
                    let range_start = g_start.and_hms_opt(0, 0, 0).unwrap().and_utc().max(from);
                    let range_end = (g_end + chrono::Duration::days(1))
                        .and_hms_opt(0, 0, 0)
                        .unwrap()
                        .and_utc()
                        .min(to);
                    missing_ranges.push((range_start, range_end));
                }

                if let Some(next) = cur.succ_opt() {
                    cur = next;
                } else {
                    break;
                }
            }

            if let (Some(g_start), Some(g_end)) = (current_gap_start, current_gap_end) {
                let range_start = g_start.and_hms_opt(0, 0, 0).unwrap().and_utc().max(from);
                let range_end = (g_end + chrono::Duration::days(1))
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc()
                    .min(to);
                missing_ranges.push((range_start, range_end));
            }

            Ok(missing_ranges)
        })
    }

    fn apply_retention_policy<'a>(
        &'a self,
        point_id: Option<PointId>,
        policy: &'a RetentionPolicy,
    ) -> StorageFuture<'a, RollupStats> {
        Box::pin(async move {
            let now = Utc::now();
            let cutoff = now - chrono::Duration::days(policy.raw_data_retention_days as i64);
            let cutoff_str = cutoff.to_rfc3339();

            let (target_interval_seconds, strftime_format) = match policy.rollup_interval {
                AggregationInterval::Hourly => (3600u32, "%Y-%m-%dT%H:00:00Z"),
                AggregationInterval::Daily
                | AggregationInterval::Monthly
                | AggregationInterval::Yearly => (86400u32, "%Y-%m-%dT00:00:00Z"),
            };

            let mut tx = self.pool.begin().await.map_err(EnedisError::Database)?;

            let sql_select = format!(
                r#"
                SELECT
                    point_id,
                    direction,
                    strftime('{fmt}', timestamp) AS bucket_start,
                    SUM(
                        CASE 
                            WHEN UPPER(unit) = 'KWH' THEN CAST(value AS REAL)
                            WHEN UPPER(unit) = 'WH' THEN CAST(value AS REAL) / 1000.0
                            WHEN UPPER(unit) = 'KW' OR UPPER(unit) = 'KVA' THEN (CAST(value AS REAL) * interval_seconds) / 3600.0
                            WHEN UPPER(unit) = 'W' OR UPPER(unit) = 'VA' THEN (CAST(value AS REAL) * interval_seconds) / 3600000.0
                            ELSE CAST(value AS REAL)
                        END
                    ) AS total_energy_kwh,
                    COUNT(*) AS raw_count
                FROM measurements
                WHERE timestamp < ?1 AND interval_seconds < ?2 {prm_filter}
                GROUP BY point_id, direction, strftime('{fmt}', timestamp)
                "#,
                fmt = strftime_format,
                prm_filter = if point_id.is_some() {
                    "AND point_id = ?3"
                } else {
                    ""
                }
            );

            let rows = match point_id {
                Some(pid) => {
                    sqlx::query(&sql_select)
                        .bind(&cutoff_str)
                        .bind(target_interval_seconds as i64)
                        .bind(pid.as_str())
                        .fetch_all(&mut *tx)
                        .await
                }
                None => {
                    sqlx::query(&sql_select)
                        .bind(&cutoff_str)
                        .bind(target_interval_seconds as i64)
                        .fetch_all(&mut *tx)
                        .await
                }
            }
            .map_err(EnedisError::Database)?;

            if rows.is_empty() {
                tx.rollback().await.map_err(EnedisError::Database)?;
                return Ok(RollupStats::default());
            }

            let mut total_raw_count: usize = 0;
            let mut rollups_to_insert = Vec::with_capacity(rows.len());

            for r in rows {
                let pid_str: String = r.get("point_id");
                let dir_str: String = r.get("direction");
                let bucket_str: String = r.get("bucket_start");
                let total_energy: Option<f64> = r.try_get("total_energy_kwh").ok();
                let count: i64 = r.get("raw_count");

                total_raw_count += count as usize;

                let energy_val = total_energy
                    .and_then(|v| Decimal::from_f64_retain(v).map(|d| d.round_dp(4)))
                    .unwrap_or(Decimal::ZERO);

                rollups_to_insert.push((
                    pid_str,
                    bucket_str,
                    dir_str,
                    target_interval_seconds,
                    energy_val.to_string(),
                ));
            }

            let sql_delete = format!(
                "DELETE FROM measurements WHERE timestamp < ?1 AND interval_seconds < ?2 {}",
                if point_id.is_some() {
                    "AND point_id = ?3"
                } else {
                    ""
                }
            );
            let delete_res = match point_id {
                Some(pid) => {
                    sqlx::query(&sql_delete)
                        .bind(&cutoff_str)
                        .bind(target_interval_seconds as i64)
                        .bind(pid.as_str())
                        .execute(&mut *tx)
                        .await
                }
                None => {
                    sqlx::query(&sql_delete)
                        .bind(&cutoff_str)
                        .bind(target_interval_seconds as i64)
                        .execute(&mut *tx)
                        .await
                }
            }
            .map_err(EnedisError::Database)?;

            let raw_deleted = delete_res.rows_affected() as usize;

            for chunk in rollups_to_insert.chunks(100) {
                let mut builder: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
                    "INSERT INTO measurements (point_id, timestamp, direction, interval_seconds, value, unit, quality) ",
                );
                builder.push_values(chunk, |mut b, (pid, ts, dir, interval, val)| {
                    b.push_bind(pid)
                        .push_bind(ts)
                        .push_bind(dir)
                        .push_bind(*interval as i64)
                        .push_bind(val)
                        .push_bind("kWh")
                        .push_bind(crate::models::MeasurementQuality::Validated.as_u8() as i64);
                });
                builder.push(
                    " ON CONFLICT(point_id, timestamp, direction) DO UPDATE SET \
                     value = excluded.value, \
                     interval_seconds = excluded.interval_seconds, \
                     unit = excluded.unit, \
                     quality = excluded.quality, \
                     updated_at = datetime('now')",
                );
                builder
                    .build()
                    .execute(&mut *tx)
                    .await
                    .map_err(EnedisError::Database)?;
            }

            if let Some(max_days) = policy.max_retention_days {
                let max_cutoff = now - chrono::Duration::days(max_days as i64);
                let max_sql = format!(
                    "DELETE FROM measurements WHERE timestamp < ?1 {}",
                    if point_id.is_some() {
                        "AND point_id = ?2"
                    } else {
                        ""
                    }
                );
                match point_id {
                    Some(pid) => {
                        sqlx::query(&max_sql)
                            .bind(max_cutoff.to_rfc3339())
                            .bind(pid.as_str())
                            .execute(&mut *tx)
                            .await
                    }
                    None => {
                        sqlx::query(&max_sql)
                            .bind(max_cutoff.to_rfc3339())
                            .execute(&mut *tx)
                            .await
                    }
                }
                .map_err(EnedisError::Database)?;
            }

            tx.commit().await.map_err(EnedisError::Database)?;

            let mut vacuum_done = false;
            if policy.auto_vacuum {
                if let Err(e) = sqlx::query("VACUUM").execute(&self.pool).await {
                    tracing::warn!("Échec du VACUUM SQLite après rollup: {}", e);
                } else {
                    vacuum_done = true;
                }
            }

            Ok(RollupStats {
                raw_measurements_processed: total_raw_count,
                rollups_created: rollups_to_insert.len(),
                raw_measurements_deleted: raw_deleted,
                vacuum_executed: vacuum_done,
            })
        })
    }

    fn vacuum<'a>(&'a self) -> StorageFuture<'a, ()> {
        Box::pin(async move {
            sqlx::query("VACUUM")
                .execute(&self.pool)
                .await
                .map_err(EnedisError::Database)?;
            Ok(())
        })
    }
}
