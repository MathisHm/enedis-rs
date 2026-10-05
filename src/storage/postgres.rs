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
use sqlx::{PgPool, Row};

/// Implémentation du stockage temporel pour PostgreSQL
#[derive(Clone)]
pub struct PostgresStorage {
    pool: PgPool,
    use_timescaledb: bool,
}

impl PostgresStorage {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            use_timescaledb: false,
        }
    }

    /// Active explicitement l'utilisation et la configuration de TimescaleDB
    pub fn with_timescaledb(mut self, enabled: bool) -> Self {
        self.use_timescaledb = enabled;
        self
    }

    /// Ouvre un pool de connexions vers PostgreSQL et initialise les schémas
    pub async fn connect(url: &str) -> Result<Self, EnedisError> {
        let pool = PgPool::connect(url).await.map_err(EnedisError::Database)?;
        let storage = Self::new(pool);
        storage.init_schema().await?;
        Ok(storage)
    }

    /// Active et convertit la table `measurements` en hypertable TimescaleDB
    pub async fn enable_timescaledb(&self) -> Result<(), EnedisError> {
        let _ = sqlx::query("CREATE EXTENSION IF NOT EXISTS timescaledb CASCADE")
            .execute(&self.pool)
            .await;

        sqlx::query(
            "SELECT create_hypertable('measurements', 'timestamp', if_not_exists => TRUE, chunk_time_interval => INTERVAL '1 month')",
        )
        .execute(&self.pool)
        .await
        .map_err(EnedisError::Database)?;

        Ok(())
    }

    /// Initialise le schéma PostgreSQL avec partitionnement déclaratif natif par plage temporelle (RANGE)
    pub async fn init_native_partitioned_schema(&self) -> Result<(), EnedisError> {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS measurements (
                point_id VARCHAR(14) NOT NULL,
                timestamp TIMESTAMPTZ NOT NULL,
                direction VARCHAR(16) NOT NULL,
                interval_seconds INTEGER NOT NULL,
                value NUMERIC(18, 4) NOT NULL,
                unit VARCHAR(16) NOT NULL,
                quality SMALLINT NOT NULL,
                inserted_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (point_id, timestamp, direction)
            ) PARTITION BY RANGE (timestamp);

            -- Partition par défaut garantissant qu'aucune insertion n'échoue
            CREATE TABLE IF NOT EXISTS measurements_default 
            PARTITION OF measurements DEFAULT;

            CREATE INDEX IF NOT EXISTS idx_measurements_point_range 
            ON measurements (point_id, timestamp DESC);

            CREATE TABLE IF NOT EXISTS sync_state (
                point_id VARCHAR(14) NOT NULL,
                direction VARCHAR(16) NOT NULL,
                last_synced_timestamp TIMESTAMPTZ NOT NULL,
                last_sync_attempt TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                sync_status VARCHAR(32) NOT NULL DEFAULT 'OK',
                PRIMARY KEY (point_id, direction)
            );
            "#,
        )
        .execute(&self.pool)
        .await
        .map_err(EnedisError::Database)?;

        Ok(())
    }

    /// S'assure de l'existence d'une partition mensuelle spécifique (ex: `measurements_y2026m09`)
    pub async fn ensure_monthly_partition(&self, year: i32, month: u32) -> Result<(), EnedisError> {
        let (next_year, next_month) = if month == 12 {
            (year + 1, 1)
        } else {
            (year, month + 1)
        };

        let table_name = format!("measurements_y{:04}m{:02}", year, month);
        let start_date = format!("{:04}-{:02}-01 00:00:00+00", year, month);
        let end_date = format!("{:04}-{:02}-01 00:00:00+00", next_year, next_month);

        let query = format!(
            "CREATE TABLE IF NOT EXISTS {} PARTITION OF measurements FOR VALUES FROM ('{}') TO ('{}')",
            table_name, start_date, end_date
        );

        sqlx::query(&query)
            .execute(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

        Ok(())
    }
}

impl StorageBackend for PostgresStorage {
    fn init_schema(&self) -> StorageFuture<'_, ()> {
        Box::pin(async move {
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS measurements (
                    point_id VARCHAR(14) NOT NULL,
                    timestamp TIMESTAMPTZ NOT NULL,
                    direction VARCHAR(16) NOT NULL,
                    interval_seconds INTEGER NOT NULL,
                    value NUMERIC(18, 4) NOT NULL,
                    unit VARCHAR(16) NOT NULL,
                    quality SMALLINT NOT NULL,
                    inserted_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    updated_at TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    PRIMARY KEY (point_id, timestamp, direction)
                );

                CREATE INDEX IF NOT EXISTS idx_measurements_point_range 
                ON measurements (point_id, timestamp DESC);

                CREATE TABLE IF NOT EXISTS sync_state (
                    point_id VARCHAR(14) NOT NULL,
                    direction VARCHAR(16) NOT NULL,
                    last_synced_timestamp TIMESTAMPTZ NOT NULL,
                    last_sync_attempt TIMESTAMPTZ NOT NULL DEFAULT CURRENT_TIMESTAMP,
                    sync_status VARCHAR(32) NOT NULL DEFAULT 'OK',
                    PRIMARY KEY (point_id, direction)
                );

                CREATE TABLE IF NOT EXISTS tempo_days (
                    date DATE PRIMARY KEY,
                    color VARCHAR(16) NOT NULL,
                    updated_at TIMESTAMPTZ NOT NULL
                );

                CREATE TABLE IF NOT EXISTS ecowatt_signals (
                    timestamp TIMESTAMPTZ PRIMARY KEY,
                    level SMALLINT NOT NULL,
                    message TEXT
                );
                "#,
            )
            .execute(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            // Détection automatique ou configuration explicite de TimescaleDB
            let has_timescale = if self.use_timescaledb {
                true
            } else {
                sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM pg_extension WHERE extname = 'timescaledb')",
                )
                .fetch_one(&self.pool)
                .await
                .unwrap_or(false)
            };

            if has_timescale {
                let _ = sqlx::query(
                    "SELECT create_hypertable('measurements', 'timestamp', if_not_exists => TRUE, chunk_time_interval => INTERVAL '1 month')",
                )
                .execute(&self.pool)
                .await;
            }

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

            // Insertion groupée par lots de 500 mesures (500 x 7 paramètres = 3500 << limite Postgres 65535)
            const BATCH_SIZE: usize = 500;

            for chunk in measurements.chunks(BATCH_SIZE) {
                let mut builder: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
                    "INSERT INTO measurements (point_id, timestamp, direction, interval_seconds, value, unit, quality) ",
                );

                builder.push_values(chunk, |mut b, m| {
                    b.push_bind(m.point_id.as_str())
                        .push_bind(m.timestamp)
                        .push_bind(m.direction.as_str())
                        .push_bind(m.interval_seconds as i32)
                        .push_bind(m.value)
                        .push_bind(m.unit.as_str())
                        .push_bind(m.quality.as_u8() as i16);
                });

                builder.push(
                    " ON CONFLICT (point_id, timestamp, direction) DO UPDATE SET \
                     value = EXCLUDED.value, \
                     interval_seconds = EXCLUDED.interval_seconds, \
                     unit = EXCLUDED.unit, \
                     quality = EXCLUDED.quality, \
                     updated_at = CURRENT_TIMESTAMP \
                     WHERE EXCLUDED.quality >= measurements.quality",
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
            let prm_str = point_id.as_str();

            let rows = match (direction, limit) {
                (Some(dir), Some(lim)) => {
                    sqlx::query(
                        r#"
                        SELECT point_id, timestamp, direction, interval_seconds, value, unit, quality
                        FROM measurements
                        WHERE point_id = $1 AND timestamp >= $2 AND timestamp <= $3 AND direction = $4
                        ORDER BY timestamp ASC
                        LIMIT $5
                        "#,
                    )
                    .bind(prm_str)
                    .bind(from)
                    .bind(to)
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
                        WHERE point_id = $1 AND timestamp >= $2 AND timestamp <= $3 AND direction = $4
                        ORDER BY timestamp ASC
                        "#,
                    )
                    .bind(prm_str)
                    .bind(from)
                    .bind(to)
                    .bind(dir.as_str())
                    .fetch_all(&self.pool)
                    .await
                }
                (None, Some(lim)) => {
                    sqlx::query(
                        r#"
                        SELECT point_id, timestamp, direction, interval_seconds, value, unit, quality
                        FROM measurements
                        WHERE point_id = $1 AND timestamp >= $2 AND timestamp <= $3
                        ORDER BY timestamp ASC
                        LIMIT $4
                        "#,
                    )
                    .bind(prm_str)
                    .bind(from)
                    .bind(to)
                    .bind(lim as i64)
                    .fetch_all(&self.pool)
                    .await
                }
                (None, None) => {
                    sqlx::query(
                        r#"
                        SELECT point_id, timestamp, direction, interval_seconds, value, unit, quality
                        FROM measurements
                        WHERE point_id = $1 AND timestamp >= $2 AND timestamp <= $3
                        ORDER BY timestamp ASC
                        "#,
                    )
                    .bind(prm_str)
                    .bind(from)
                    .bind(to)
                    .fetch_all(&self.pool)
                    .await
                }
            }
            .map_err(EnedisError::Database)?;

            let mut results = Vec::with_capacity(rows.len());
            for row in rows {
                let pid_str: String = row.get("point_id");
                let ts: DateTime<Utc> = row.get("timestamp");
                let dir_str: String = row.get("direction");
                let interval: i32 = row.get("interval_seconds");
                let val: Decimal = row.get("value");
                let unit_str: String = row.get("unit");
                let quality_val: i16 = row.get("quality");

                let pid = PointId::new(&pid_str).map_err(|e| {
                    EnedisError::StorageDecode(format!("PRM corrompu en base: {}", e))
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
            let prm_str = point_id.as_str();

            let bucket_expr = if self.use_timescaledb {
                match interval {
                    AggregationInterval::Hourly => "time_bucket('1 hour', timestamp)",
                    AggregationInterval::Daily => "time_bucket('1 day', timestamp)",
                    AggregationInterval::Monthly => "date_trunc('month', timestamp)",
                    AggregationInterval::Yearly => "date_trunc('year', timestamp)",
                }
            } else {
                match interval {
                    AggregationInterval::Hourly => "date_trunc('hour', timestamp)",
                    AggregationInterval::Daily => "date_trunc('day', timestamp)",
                    AggregationInterval::Monthly => "date_trunc('month', timestamp)",
                    AggregationInterval::Yearly => "date_trunc('year', timestamp)",
                }
            };

            let sql = format!(
                r#"
                SELECT
                    point_id,
                    direction,
                    {bucket_expr} AS bucket_start,
                    SUM(
                        CASE 
                            WHEN UPPER(unit) = 'KWH' THEN value
                            WHEN UPPER(unit) = 'WH' THEN (value / 1000.0)
                            WHEN UPPER(unit) = 'KW' OR UPPER(unit) = 'KVA' THEN (value * interval_seconds) / 3600.0
                            WHEN UPPER(unit) = 'W' OR UPPER(unit) = 'VA' THEN (value * interval_seconds) / 3600000.0
                            ELSE value
                        END
                    )::NUMERIC AS total_energy_kwh,
                    MAX(
                        CASE
                            WHEN UPPER(unit) = 'W' OR UPPER(unit) = 'VA' THEN value
                            WHEN UPPER(unit) = 'KW' OR UPPER(unit) = 'KVA' THEN value * 1000.0
                            WHEN UPPER(unit) = 'WH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (value * 3600.0) / interval_seconds ELSE value END
                            WHEN UPPER(unit) = 'KWH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (value * 3600000.0) / interval_seconds ELSE value * 1000.0 END
                            ELSE value
                        END
                    )::NUMERIC AS max_power_w,
                    MIN(
                        CASE
                            WHEN UPPER(unit) = 'W' OR UPPER(unit) = 'VA' THEN value
                            WHEN UPPER(unit) = 'KW' OR UPPER(unit) = 'KVA' THEN value * 1000.0
                            WHEN UPPER(unit) = 'WH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (value * 3600.0) / interval_seconds ELSE value END
                            WHEN UPPER(unit) = 'KWH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (value * 3600000.0) / interval_seconds ELSE value * 1000.0 END
                            ELSE value
                        END
                    )::NUMERIC AS min_power_w,
                    AVG(
                        CASE
                            WHEN UPPER(unit) = 'W' OR UPPER(unit) = 'VA' THEN value
                            WHEN UPPER(unit) = 'KW' OR UPPER(unit) = 'KVA' THEN value * 1000.0
                            WHEN UPPER(unit) = 'WH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (value * 3600.0) / interval_seconds ELSE value END
                            WHEN UPPER(unit) = 'KWH' THEN 
                                CASE WHEN interval_seconds > 0 THEN (value * 3600000.0) / interval_seconds ELSE value * 1000.0 END
                            ELSE value
                        END
                    )::NUMERIC AS avg_power_w,
                    COUNT(*)::BIGINT AS sample_count
                FROM measurements
                WHERE point_id = $1 AND timestamp >= $2 AND timestamp <= $3 {dir_clause}
                GROUP BY point_id, direction, {bucket_expr}
                ORDER BY bucket_start ASC, direction ASC
                "#,
                bucket_expr = bucket_expr,
                dir_clause = if direction.is_some() {
                    "AND direction = $4"
                } else {
                    ""
                }
            );

            let rows = match direction {
                Some(dir) => {
                    sqlx::query(&sql)
                        .bind(prm_str)
                        .bind(from)
                        .bind(to)
                        .bind(dir.as_str())
                        .fetch_all(&self.pool)
                        .await
                }
                None => {
                    sqlx::query(&sql)
                        .bind(prm_str)
                        .bind(from)
                        .bind(to)
                        .fetch_all(&self.pool)
                        .await
                }
            }
            .map_err(EnedisError::Database)?;

            let mut results = Vec::with_capacity(rows.len());
            for row in rows {
                let pid_str: String = row.get("point_id");
                let dir_str: String = row.get("direction");
                let bucket_start: DateTime<Utc> = row.get("bucket_start");
                let total_energy: Option<Decimal> = row.try_get("total_energy_kwh").ok();
                let max_p: Option<Decimal> = row.try_get("max_power_w").ok();
                let min_p: Option<Decimal> = row.try_get("min_power_w").ok();
                let avg_p: Option<Decimal> = row.try_get("avg_power_w").ok();
                let count: i64 = row.get("sample_count");

                let pid = PointId::new(&pid_str).map_err(|e| {
                    EnedisError::StorageDecode(format!("PRM corrompu en base: {}", e))
                })?;
                let bucket_end = interval.bucket_end(bucket_start);

                let total_energy_kwh = total_energy.map(|d| d.round_dp(4)).unwrap_or(Decimal::ZERO);
                let max_power_w = max_p.map(|d| d.round_dp(4));
                let min_power_w = min_p.map(|d| d.round_dp(4));
                let avg_power_w = avg_p.map(|d| d.round_dp(4));

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
                WHERE point_id = $1 AND direction = $2
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
                let last_sync: DateTime<Utc> = r.get("last_synced_timestamp");
                let attempt: DateTime<Utc> = r.get("last_sync_attempt");
                let status: String = r.get("sync_status");

                let pid = PointId::new(&pid_str).map_err(|e| {
                    EnedisError::Configuration(format!("PRM corrompu en base: {}", e))
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
                    $1, $2, $3, $4, $5
                )
                ON CONFLICT (point_id, direction) DO UPDATE SET
                    last_synced_timestamp = EXCLUDED.last_synced_timestamp,
                    last_sync_attempt = EXCLUDED.last_sync_attempt,
                    sync_status = EXCLUDED.sync_status;
                "#,
            )
            .bind(state.point_id.as_str())
            .bind(state.direction.as_str())
            .bind(state.last_synced_timestamp)
            .bind(state.last_sync_attempt)
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
                        WHEN UPPER(unit) = 'WH' THEN (value / 1000.0)
                        WHEN UPPER(unit) = 'W' THEN (value / 1000.0) * (interval_seconds / 3600.0)
                        WHEN UPPER(unit) = 'KW' THEN value * (interval_seconds / 3600.0)
                        ELSE value
                    END
                ) as total_kwh
                FROM measurements
                WHERE point_id = $1 AND direction = $2
                "#,
            )
            .bind(point_id.as_str())
            .bind(direction.as_str())
            .fetch_one(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            let total: Option<Decimal> = row.try_get("total_kwh").ok();
            Ok(total)
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
                WHERE point_id = $1 AND direction = $2
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
                let ts: DateTime<Utc> = r.get("timestamp");
                let dir_str: String = r.get("direction");
                let interval: i32 = r.get("interval_seconds");
                let val: Decimal = r.get("value");
                let unit_str: String = r.get("unit");
                let quality_val: i16 = r.get("quality");

                let pid = PointId::new(&pid_str).map_err(|e| {
                    EnedisError::StorageDecode(format!("PRM corrompu en base: {}", e))
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
                let mut builder: sqlx::QueryBuilder<sqlx::Postgres> =
                    sqlx::QueryBuilder::new("INSERT INTO tempo_days (date, color, updated_at) ");
                builder.push_values(chunk, |mut b, r| {
                    b.push_bind(r.date)
                        .push_bind(r.color.as_str())
                        .push_bind(r.updated_at);
                });
                builder.push(
                    " ON CONFLICT(date) DO UPDATE SET color = EXCLUDED.color, updated_at = EXCLUDED.updated_at",
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
            let rows = sqlx::query(
                "SELECT date, color, updated_at FROM tempo_days WHERE date >= $1 AND date <= $2 ORDER BY date ASC",
            )
            .bind(from)
            .bind(to)
            .fetch_all(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            let mut results = Vec::with_capacity(rows.len());
            for row in rows {
                let d: chrono::NaiveDate = row.get("date");
                let c_str: String = row.get("color");
                let u: DateTime<Utc> = row.get("updated_at");

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
                let mut builder: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
                    "INSERT INTO ecowatt_signals (timestamp, level, message) ",
                );
                builder.push_values(chunk, |mut b, s| {
                    b.push_bind(s.timestamp)
                        .push_bind(s.level.as_u8() as i16)
                        .push_bind(s.message.as_deref());
                });
                builder.push(
                    " ON CONFLICT(timestamp) DO UPDATE SET level = EXCLUDED.level, message = EXCLUDED.message",
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
            let rows = sqlx::query(
                "SELECT timestamp, level, message FROM ecowatt_signals WHERE timestamp >= $1 AND timestamp <= $2 ORDER BY timestamp ASC",
            )
            .bind(from)
            .bind(to)
            .fetch_all(&self.pool)
            .await
            .map_err(EnedisError::Database)?;

            let mut results = Vec::with_capacity(rows.len());
            for row in rows {
                let t: DateTime<Utc> = row.get("timestamp");
                let lvl_val: i16 = row.get("level");
                let msg: Option<String> = row.get("message");

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

            let prm_str = point_id.as_str();

            let rows = sqlx::query(
                r#"
                SELECT DISTINCT TO_CHAR(timestamp, 'YYYY-MM-DD') as day
                FROM measurements
                WHERE point_id = $1 AND direction = $2 AND timestamp >= $3 AND timestamp < $4
                ORDER BY day ASC
                "#,
            )
            .bind(prm_str)
            .bind(direction.as_str())
            .bind(from)
            .bind(to)
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

            let (target_interval_seconds, trunc_unit) = match policy.rollup_interval {
                AggregationInterval::Hourly => (3600u32, "hour"),
                AggregationInterval::Daily
                | AggregationInterval::Monthly
                | AggregationInterval::Yearly => (86400u32, "day"),
            };

            let mut tx = self.pool.begin().await.map_err(EnedisError::Database)?;

            let sql_select = format!(
                r#"
                SELECT
                    point_id,
                    direction,
                    date_trunc('{trunc}', timestamp) AS bucket_start,
                    SUM(
                        CASE 
                            WHEN UPPER(unit) = 'KWH' THEN value
                            WHEN UPPER(unit) = 'WH' THEN value / 1000.0
                            WHEN UPPER(unit) = 'KW' OR UPPER(unit) = 'KVA' THEN (value * interval_seconds) / 3600.0
                            WHEN UPPER(unit) = 'W' OR UPPER(unit) = 'VA' THEN (value * interval_seconds) / 3600000.0
                            ELSE value
                        END
                    ) AS total_energy_kwh,
                    COUNT(*) AS raw_count
                FROM measurements
                WHERE timestamp < $1 AND interval_seconds < $2 {prm_filter}
                GROUP BY point_id, direction, date_trunc('{trunc}', timestamp)
                "#,
                trunc = trunc_unit,
                prm_filter = if point_id.is_some() {
                    "AND point_id = $3"
                } else {
                    ""
                }
            );

            let rows = match point_id {
                Some(pid) => {
                    sqlx::query(&sql_select)
                        .bind(cutoff)
                        .bind(target_interval_seconds as i32)
                        .bind(pid.as_str())
                        .fetch_all(&mut *tx)
                        .await
                }
                None => {
                    sqlx::query(&sql_select)
                        .bind(cutoff)
                        .bind(target_interval_seconds as i32)
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
                let bucket_dt: DateTime<Utc> = r.get("bucket_start");
                let total_energy: Option<Decimal> = r.try_get("total_energy_kwh").ok();
                let count: i64 = r.get("raw_count");

                total_raw_count += count as usize;

                let energy_val = total_energy.unwrap_or(Decimal::ZERO).round_dp(4);

                rollups_to_insert.push((
                    pid_str,
                    bucket_dt,
                    dir_str,
                    target_interval_seconds,
                    energy_val,
                ));
            }

            let sql_delete = format!(
                "DELETE FROM measurements WHERE timestamp < $1 AND interval_seconds < $2 {}",
                if point_id.is_some() {
                    "AND point_id = $3"
                } else {
                    ""
                }
            );
            let delete_res = match point_id {
                Some(pid) => {
                    sqlx::query(&sql_delete)
                        .bind(cutoff)
                        .bind(target_interval_seconds as i32)
                        .bind(pid.as_str())
                        .execute(&mut *tx)
                        .await
                }
                None => {
                    sqlx::query(&sql_delete)
                        .bind(cutoff)
                        .bind(target_interval_seconds as i32)
                        .execute(&mut *tx)
                        .await
                }
            }
            .map_err(EnedisError::Database)?;

            let raw_deleted = delete_res.rows_affected() as usize;

            for chunk in rollups_to_insert.chunks(100) {
                let mut builder: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(
                    "INSERT INTO measurements (point_id, timestamp, direction, interval_seconds, value, unit, quality) ",
                );
                builder.push_values(chunk, |mut b, (pid, ts, dir, interval, val)| {
                    b.push_bind(pid)
                        .push_bind(ts)
                        .push_bind(dir)
                        .push_bind(*interval as i32)
                        .push_bind(val)
                        .push_bind("kWh")
                        .push_bind(crate::models::MeasurementQuality::Validated.as_u8() as i16);
                });
                builder.push(
                    " ON CONFLICT(point_id, timestamp, direction) DO UPDATE SET \
                     value = EXCLUDED.value, \
                     interval_seconds = EXCLUDED.interval_seconds, \
                     unit = EXCLUDED.unit, \
                     quality = EXCLUDED.quality, \
                     updated_at = CURRENT_TIMESTAMP",
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
                    "DELETE FROM measurements WHERE timestamp < $1 {}",
                    if point_id.is_some() {
                        "AND point_id = $2"
                    } else {
                        ""
                    }
                );
                match point_id {
                    Some(pid) => {
                        sqlx::query(&max_sql)
                            .bind(max_cutoff)
                            .bind(pid.as_str())
                            .execute(&mut *tx)
                            .await
                    }
                    None => {
                        sqlx::query(&max_sql)
                            .bind(max_cutoff)
                            .execute(&mut *tx)
                            .await
                    }
                }
                .map_err(EnedisError::Database)?;
            }

            tx.commit().await.map_err(EnedisError::Database)?;

            let mut vacuum_done = false;
            if policy.auto_vacuum {
                if let Err(e) = sqlx::query("VACUUM ANALYZE measurements")
                    .execute(&self.pool)
                    .await
                {
                    tracing::warn!("Échec du VACUUM PostgreSQL après rollup: {}", e);
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
            sqlx::query("VACUUM ANALYZE measurements")
                .execute(&self.pool)
                .await
                .map_err(EnedisError::Database)?;
            Ok(())
        })
    }
}
