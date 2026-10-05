use crate::error::EnedisError;
use crate::models::{
    aggregate_measurements, AggregatedMeasurement, AggregationInterval, FlowDirection, Measurement,
    PointId,
};
use crate::storage::models::{SyncState, UpsertStats};
use crate::storage::retention::{RetentionPolicy, RollupStats};
use chrono::{DateTime, Utc};
use std::future::Future;
use std::pin::Pin;

/// Type alias pour les futures retournées par les méthodes de StorageBackend (préserve l'object-safety)
pub type StorageFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, EnedisError>> + Send + 'a>>;

/// Trait unifié d'accès aux bases de données temporelles (PostgreSQL, SQLite)
pub trait StorageBackend: Send + Sync {
    /// Initialise les schémas de tables et index s'ils n'existent pas
    fn init_schema(&self) -> StorageFuture<'_, ()>;

    /// Insère ou met à jour de manière conditionnelle un lot de mesures.
    /// Ne modifie une mesure existante que si `new_quality >= existing_quality`.
    fn upsert_measurements<'a>(
        &'a self,
        measurements: &'a [Measurement],
    ) -> StorageFuture<'a, UpsertStats>;

    /// Récupère les mesures d'un PRM sur une plage temporelle donnée
    fn get_measurements<'a>(
        &'a self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        direction: Option<FlowDirection>,
    ) -> StorageFuture<'a, Vec<Measurement>>;

    /// Récupère les mesures d'un PRM avec une limite maximale de lignes appliquée au niveau de la requête SQL
    fn get_measurements_limited<'a>(
        &'a self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        direction: Option<FlowDirection>,
        limit: Option<usize>,
    ) -> StorageFuture<'a, Vec<Measurement>> {
        Box::pin(async move {
            let mut measurements = self.get_measurements(point_id, from, to, direction).await?;
            if let Some(lim) = limit {
                measurements.truncate(lim);
            }
            Ok(measurements)
        })
    }

    /// Récupère les mesures agrégées d'un PRM selon un intervalle temporel donné
    fn get_aggregated_measurements<'a>(
        &'a self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        interval: AggregationInterval,
        direction: Option<FlowDirection>,
    ) -> StorageFuture<'a, Vec<AggregatedMeasurement>> {
        Box::pin(async move {
            let measurements = self.get_measurements(point_id, from, to, direction).await?;
            Ok(aggregate_measurements(&measurements, interval))
        })
    }

    /// Récupère l'état de synchronisation d'un point
    fn get_sync_state(
        &self,
        point_id: PointId,
        direction: FlowDirection,
    ) -> StorageFuture<'_, Option<SyncState>>;

    /// Met à jour ou insère l'état de synchronisation pour l'Agent de collecte
    fn update_sync_state<'a>(&'a self, state: &'a SyncState) -> StorageFuture<'a, ()>;

    /// Liste l'ensemble des PRM enregistrés pour synchronisation
    fn list_sync_points(&self) -> StorageFuture<'_, Vec<PointId>>;

    /// Calcule l'énergie cumulée totale en kWh pour un PRM et un sens de flux donné
    fn get_total_energy_kwh<'a>(
        &'a self,
        point_id: PointId,
        direction: FlowDirection,
    ) -> StorageFuture<'a, Option<rust_decimal::Decimal>> {
        Box::pin(async move {
            let now = Utc::now();
            let from = now - chrono::Duration::days(365 * 10);
            let measurements = self
                .get_measurements(
                    point_id,
                    from,
                    now + chrono::Duration::days(1),
                    Some(direction),
                )
                .await?;
            if measurements.is_empty() {
                return Ok(None);
            }
            let mut total = rust_decimal::Decimal::ZERO;
            for m in measurements {
                let kwh = match m.unit {
                    crate::models::Unit::KiloWattHour => m.value,
                    crate::models::Unit::WattHour => m.value / rust_decimal::Decimal::from(1000),
                    crate::models::Unit::KiloWatt => {
                        m.value
                            * (rust_decimal::Decimal::from(m.interval_seconds)
                                / rust_decimal::Decimal::from(3600))
                    }
                    crate::models::Unit::Watt => {
                        (m.value / rust_decimal::Decimal::from(1000))
                            * (rust_decimal::Decimal::from(m.interval_seconds)
                                / rust_decimal::Decimal::from(3600))
                    }
                    _ => m.value,
                };
                total += kwh;
            }
            Ok(Some(total))
        })
    }

    /// Récupère la mesure la plus récente pour un PRM et un sens de flux donné
    fn get_latest_measurement<'a>(
        &'a self,
        point_id: PointId,
        direction: FlowDirection,
    ) -> StorageFuture<'a, Option<Measurement>> {
        Box::pin(async move {
            let now = Utc::now();
            let from = now - chrono::Duration::days(365 * 10);
            let measurements = self
                .get_measurements(
                    point_id,
                    from,
                    now + chrono::Duration::days(1),
                    Some(direction),
                )
                .await?;
            Ok(measurements.into_iter().max_by_key(|m| m.timestamp))
        })
    }

    /// Enregistre ou met à jour les couleurs Tempo pour les jours spécifiés (J / J+1)
    fn upsert_tempo_days<'a>(
        &'a self,
        records: &'a [crate::models::TempoDayRecord],
    ) -> StorageFuture<'a, ()>;

    /// Récupère l'historique des couleurs Tempo sur une période donnée
    fn get_tempo_days<'a>(
        &'a self,
        from: chrono::NaiveDate,
        to: chrono::NaiveDate,
    ) -> StorageFuture<'a, Vec<crate::models::TempoDayRecord>>;

    /// Enregistre ou met à jour les signaux de tension RTE EcoWatt
    fn upsert_ecowatt_signals<'a>(
        &'a self,
        signals: &'a [crate::models::EcoWattSignal],
    ) -> StorageFuture<'a, ()>;

    /// Récupère les signaux RTE EcoWatt sur une période temporelle
    fn get_ecowatt_signals<'a>(
        &'a self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> StorageFuture<'a, Vec<crate::models::EcoWattSignal>>;

    /// Enregistre ou met à jour les prix du marché spot Day-Ahead (EPEX SPOT)
    fn upsert_spot_prices<'a>(
        &'a self,
        _prices: &'a [crate::models::SpotPriceRecord],
    ) -> StorageFuture<'a, ()> {
        Box::pin(async move { Ok(()) })
    }

    /// Récupère l'historique des prix du marché spot sur une plage temporelle
    fn get_spot_prices<'a>(
        &'a self,
        _from: DateTime<Utc>,
        _to: DateTime<Utc>,
    ) -> StorageFuture<'a, Vec<crate::models::SpotPriceRecord>> {
        Box::pin(async move { Ok(Vec::new()) })
    }

    /// Détecte les plages temporelles de jours manquants pour un PRM et un sens de flux donnés entre `from` et `to`.
    /// Regroupe les jours consécutifs manquants en plages continues `(debut, fin)`.
    fn detect_missing_ranges<'a>(
        &'a self,
        point_id: PointId,
        direction: FlowDirection,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> StorageFuture<'a, Vec<(DateTime<Utc>, DateTime<Utc>)>>;

    /// Applique une politique de rétention et compression (downsampling / rollup).
    /// Si `point_id` est `None`, la politique s'applique à l'ensemble des PRM enregistrés en base.
    fn apply_retention_policy<'a>(
        &'a self,
        point_id: Option<PointId>,
        policy: &'a RetentionPolicy,
    ) -> StorageFuture<'a, RollupStats>;

    /// Exécute une opération de défragmentation physique et libération de l'espace disque (VACUUM)
    fn vacuum<'a>(&'a self) -> StorageFuture<'a, ()>;
}
