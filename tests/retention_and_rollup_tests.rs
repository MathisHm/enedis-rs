#![cfg(feature = "storage-sqlite")]

use chrono::{Duration, Utc};
use enedis_rs::models::{
    AggregationInterval, FlowDirection, Measurement, MeasurementQuality, PointId, Unit,
};
use enedis_rs::storage::{RetentionPolicy, SqliteStorage, StorageBackend};
use rust_decimal::Decimal;
use std::str::FromStr;

#[tokio::test]
async fn test_sqlite_retention_rollup_downsampling_accuracy() {
    let storage = SqliteStorage::connect("sqlite::memory:").await.unwrap();
    let point_id = PointId::new("01234567890123").unwrap();

    let now = Utc::now();
    // Jour ancien : il y a 800 jours (> seuil de 730 jours / 2 ans)
    let old_day_base = (now - Duration::days(800))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();

    let mut measurements = Vec::new();

    // 48 mesures au pas de 30 minutes (1 journée complète de courbe de charge)
    // Chaque mesure = 0.5 kWh (total = 24.0 kWh)
    for i in 0..48 {
        measurements.push(Measurement {
            point_id,
            timestamp: old_day_base + Duration::minutes(i * 30),
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str("0.5000").unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        });
    }

    // Mesures récentes (hier) : ne doivent PAS être compactées
    let recent_day_base = (now - Duration::days(1))
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();

    for i in 0..48 {
        measurements.push(Measurement {
            point_id,
            timestamp: recent_day_base + Duration::minutes(i * 30),
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str("1.0000").unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        });
    }

    storage.upsert_measurements(&measurements).await.unwrap();

    // Vérifier l'énergie cumulée initiale
    let total_before = storage
        .get_total_energy_kwh(point_id, FlowDirection::Consumption)
        .await
        .unwrap()
        .unwrap();

    // 24 kWh (ancien) + 48 kWh (récent) = 72 kWh
    assert_eq!(total_before, Decimal::from_str("72.0000").unwrap());

    // Application de la politique de rétention (rollup journalier sur les données > 730 jours)
    let policy = RetentionPolicy::new(730, AggregationInterval::Daily).with_auto_vacuum(true);
    let stats = storage
        .apply_retention_policy(Some(point_id), &policy)
        .await
        .unwrap();

    assert_eq!(stats.raw_measurements_processed, 48);
    assert_eq!(stats.rollups_created, 1);
    assert_eq!(stats.raw_measurements_deleted, 48);
    assert!(stats.vacuum_executed);

    // Vérifier après compactage :
    // 1. Les mesures de l'ancien jour sont désormais réduites à 1 seule mesure compactée au pas journalier (86400s)
    let old_measurements = storage
        .get_measurements(
            point_id,
            old_day_base,
            old_day_base + Duration::days(1),
            Some(FlowDirection::Consumption),
        )
        .await
        .unwrap();

    assert_eq!(old_measurements.len(), 1);
    assert_eq!(old_measurements[0].interval_seconds, 86400);
    assert_eq!(
        old_measurements[0].value,
        Decimal::from_str("24.0000").unwrap()
    );

    // 2. Les mesures récentes n'ont pas été altérées (toujours 48 pas de 30m)
    let recent_measurements = storage
        .get_measurements(
            point_id,
            recent_day_base,
            recent_day_base + Duration::days(1),
            Some(FlowDirection::Consumption),
        )
        .await
        .unwrap();

    assert_eq!(recent_measurements.len(), 48);

    // 3. L'énergie totale calculée reste strictement identique : 72 kWh (zéro perte d'information financière ou énergétique)
    let total_after = storage
        .get_total_energy_kwh(point_id, FlowDirection::Consumption)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(total_after, total_before);
}
