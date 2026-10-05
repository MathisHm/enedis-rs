#![cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn test_conditional_upsert_quality_hierarchy() {
    use chrono::{TimeZone, Utc};
    use enedis_rs::storage::{SqliteStorage, StorageBackend};
    use enedis_rs::{FlowDirection, Measurement, MeasurementQuality, PointId, Unit};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    let storage = SqliteStorage::connect("sqlite::memory:")
        .await
        .expect("Connexion SQLite mémoire impossible");

    let prm = PointId::new("01234567890123").unwrap();
    let ts = Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap();

    // 1. Insertion initiale d'une valeur ESTIMÉE (qualité = 1)
    let m_estimated = Measurement {
        point_id: prm,
        timestamp: ts,
        interval_seconds: 1800,
        direction: FlowDirection::Consumption,
        value: Decimal::from_str("10.0000").unwrap(),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Estimated,
    };

    let stats = storage
        .upsert_measurements(&[m_estimated])
        .await
        .expect("Upsert initial échoué");
    assert_eq!(stats.affected, 1);

    let current = storage
        .get_measurements(prm, ts, ts, Some(FlowDirection::Consumption))
        .await
        .unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].value, Decimal::from_str("10.0000").unwrap());
    assert_eq!(current[0].quality, MeasurementQuality::Estimated);

    // 2. Arrivée ultérieure d'une valeur VALIDÉE / MESURÉE (qualité = 3)
    // Règle : Validated (3) >= Estimated (1) -> La valeur DOIT être mise à jour
    let m_validated = Measurement {
        point_id: prm,
        timestamp: ts,
        interval_seconds: 1800,
        direction: FlowDirection::Consumption,
        value: Decimal::from_str("12.5000").unwrap(),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };

    let stats = storage
        .upsert_measurements(&[m_validated])
        .await
        .expect("Upsert validé échoué");
    assert_eq!(stats.affected, 1);

    let current = storage
        .get_measurements(prm, ts, ts, Some(FlowDirection::Consumption))
        .await
        .unwrap();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].value, Decimal::from_str("12.5000").unwrap());
    assert_eq!(current[0].quality, MeasurementQuality::Validated);

    // 3. Arrivée ultérieure tardive d'une valeur ESTIMÉE ou REDRESSÉE (qualité = 1 ou 2)
    // Règle stricte Enedis : Une mesure validée ne doit JAMAIS être écrasée par une estimation
    let m_late_estimated = Measurement {
        point_id: prm,
        timestamp: ts,
        interval_seconds: 1800,
        direction: FlowDirection::Consumption,
        value: Decimal::from_str("999.9999").unwrap(),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Estimated,
    };

    let stats = storage
        .upsert_measurements(&[m_late_estimated])
        .await
        .expect("Upsert écrasement refusé");
    // L'UPSERT conditionnel WHERE EXCLUDED.quality >= measurements.quality doit ignorer l'update (0 lignes affectées)
    assert_eq!(stats.affected, 0);

    let current = storage
        .get_measurements(prm, ts, ts, Some(FlowDirection::Consumption))
        .await
        .unwrap();
    assert_eq!(current.len(), 1);
    // La valeur validée originale est intacte !
    assert_eq!(current[0].value, Decimal::from_str("12.5000").unwrap());
    assert_eq!(current[0].quality, MeasurementQuality::Validated);
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn test_sync_state_tracking() {
    use chrono::{TimeZone, Utc};
    use enedis_rs::storage::{SqliteStorage, StorageBackend, SyncState};
    use enedis_rs::{FlowDirection, PointId};

    let storage = SqliteStorage::connect("sqlite::memory:").await.unwrap();
    let prm = PointId::new("01234567890123").unwrap();

    let initial_state = SyncState {
        point_id: prm,
        direction: FlowDirection::Consumption,
        last_synced_timestamp: Utc.with_ymd_and_hms(2026, 9, 27, 0, 0, 0).unwrap(),
        last_sync_attempt: Utc.with_ymd_and_hms(2026, 9, 28, 6, 0, 0).unwrap(),
        sync_status: "OK".to_string(),
    };

    storage.update_sync_state(&initial_state).await.unwrap();

    let fetched = storage
        .get_sync_state(prm, FlowDirection::Consumption)
        .await
        .unwrap()
        .expect("L'état de synchronisation doit être trouvé");

    assert_eq!(fetched.point_id, prm);
    assert_eq!(fetched.sync_status, "OK");
    assert_eq!(
        fetched.last_synced_timestamp,
        Utc.with_ymd_and_hms(2026, 9, 27, 0, 0, 0).unwrap()
    );

    // Mise à jour de l'état
    let updated_state = SyncState {
        point_id: prm,
        direction: FlowDirection::Consumption,
        last_synced_timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap(),
        last_sync_attempt: Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap(),
        sync_status: "OK".to_string(),
    };

    storage.update_sync_state(&updated_state).await.unwrap();

    let fetched_updated = storage
        .get_sync_state(prm, FlowDirection::Consumption)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        fetched_updated.last_synced_timestamp,
        Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap()
    );
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn test_batch_upsert_chunking_large_payload() {
    use chrono::{Duration as ChronoDuration, Utc};
    use enedis_rs::storage::{SqliteStorage, StorageBackend};
    use enedis_rs::{FlowDirection, Measurement, MeasurementQuality, PointId, Unit};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    let storage = SqliteStorage::connect("sqlite::memory:").await.unwrap();
    let prm = PointId::new("01234567890123").unwrap();
    let base_ts = Utc::now();

    // 250 mesures (dépasse BATCH_SIZE = 100 pour valider le découpage multi-lots)
    let mut measurements = Vec::with_capacity(250);
    for i in 0..250 {
        measurements.push(Measurement {
            point_id: prm,
            timestamp: base_ts + ChronoDuration::minutes(30 * i as i64),
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str(&format!("{}.5000", i)).unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        });
    }

    let stats = storage.upsert_measurements(&measurements).await.unwrap();
    assert_eq!(stats.processed, 250);
    assert_eq!(stats.affected, 250);

    let retrieved = storage
        .get_measurements(
            prm,
            base_ts,
            base_ts + ChronoDuration::minutes(30 * 249),
            Some(FlowDirection::Consumption),
        )
        .await
        .unwrap();
    assert_eq!(retrieved.len(), 250);
}
