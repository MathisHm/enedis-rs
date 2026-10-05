#![cfg(all(feature = "agent", feature = "mock-sge", feature = "storage-sqlite"))]

use chrono::{TimeZone, Utc};
use enedis_rs::agent::{CollectorConfig, CollectorDaemon, SgeRateLimiter, ShutdownSignal};
use enedis_rs::client::{SgeClient, SgeClientConfig};
use enedis_rs::mock::{MockScenario, MockSgeServer};
use enedis_rs::models::{FlowDirection, Measurement, MeasurementQuality, PointId, Unit};
use enedis_rs::storage::{SqliteStorage, StorageBackend};
use rust_decimal::Decimal;
use std::str::FromStr;
use std::sync::Arc;

#[tokio::test]
async fn test_storage_detect_missing_ranges() {
    let storage = SqliteStorage::connect("sqlite::memory:").await.unwrap();
    let point_id = PointId::new("01234567890123").unwrap();

    let day1 = Utc.with_ymd_and_hms(2026, 9, 1, 10, 0, 0).unwrap();
    let day2 = Utc.with_ymd_and_hms(2026, 9, 2, 10, 0, 0).unwrap();
    // day 3, 4, 5 are missing
    let day6 = Utc.with_ymd_and_hms(2026, 9, 6, 10, 0, 0).unwrap();

    let measurements = vec![
        Measurement {
            point_id,
            timestamp: day1,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str("1.2").unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        },
        Measurement {
            point_id,
            timestamp: day2,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str("1.5").unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        },
        Measurement {
            point_id,
            timestamp: day6,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str("2.0").unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        },
    ];

    storage.upsert_measurements(&measurements).await.unwrap();

    let check_from = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let check_to = Utc.with_ymd_and_hms(2026, 9, 7, 0, 0, 0).unwrap();

    let missing = storage
        .detect_missing_ranges(point_id, FlowDirection::Consumption, check_from, check_to)
        .await
        .unwrap();

    // Days 3, 4, 5 are contiguous missing days -> one merged range [2026-09-03, 2026-09-06]
    assert_eq!(missing.len(), 1);
    let (gap_start, gap_end) = missing[0];
    assert_eq!(
        gap_start,
        Utc.with_ymd_and_hms(2026, 9, 3, 0, 0, 0).unwrap()
    );
    assert_eq!(gap_end, Utc.with_ymd_and_hms(2026, 9, 6, 0, 0, 0).unwrap());
}

#[tokio::test]
async fn test_storage_detect_missing_ranges_multiple_gaps() {
    let storage = SqliteStorage::connect("sqlite::memory:").await.unwrap();
    let point_id = PointId::new("01234567890123").unwrap();

    // Day 1 present, Day 2 missing, Day 3 present, Day 4 missing
    let day1 = Utc.with_ymd_and_hms(2026, 9, 1, 12, 0, 0).unwrap();
    let day3 = Utc.with_ymd_and_hms(2026, 9, 3, 12, 0, 0).unwrap();

    let measurements = vec![
        Measurement {
            point_id,
            timestamp: day1,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str("1.0").unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        },
        Measurement {
            point_id,
            timestamp: day3,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str("1.0").unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        },
    ];

    storage.upsert_measurements(&measurements).await.unwrap();

    let check_from = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let check_to = Utc.with_ymd_and_hms(2026, 9, 5, 0, 0, 0).unwrap();

    let missing = storage
        .detect_missing_ranges(point_id, FlowDirection::Consumption, check_from, check_to)
        .await
        .unwrap();

    // Deux trous distincts : Day 2 et Day 4
    assert_eq!(missing.len(), 2);
    assert_eq!(
        missing[0],
        (
            Utc.with_ymd_and_hms(2026, 9, 2, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 3, 0, 0, 0).unwrap()
        )
    );
    assert_eq!(
        missing[1],
        (
            Utc.with_ymd_and_hms(2026, 9, 4, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 5, 0, 0, 0).unwrap()
        )
    );
}

#[tokio::test]
async fn test_agent_surgical_backfill_integration() {
    let mock = MockSgeServer::start(MockScenario::Success).await;
    let storage = Arc::new(SqliteStorage::connect("sqlite::memory:").await.unwrap());
    let prm = PointId::new("01234567890123").unwrap();

    let client_config = SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(client_config).unwrap();

    let rate_limiter = SgeRateLimiter::new(100, std::time::Duration::from_millis(1));
    let config = CollectorConfig::default();
    let daemon = CollectorDaemon::new(
        client,
        storage.clone(),
        rate_limiter,
        config,
        ShutdownSignal::new(),
    );

    // Initialement la base est vide sur [2026-09-28 .. 2026-09-29]
    let from = Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 9, 29, 0, 0, 0).unwrap();

    let gaps = daemon.detect_missing_ranges(prm, from, to).await.unwrap();
    assert_eq!(gaps.len(), 1);
    assert_eq!(gaps[0], (from, to));

    // Exécution du rattrapage chirurgical
    let stats = daemon
        .backfill_missing_ranges(prm, FlowDirection::Consumption, from, to)
        .await
        .unwrap();

    assert_eq!(stats.ranges_detected, 1);
    assert_eq!(stats.requests_made, 1);
    assert!(stats.measurements_recovered > 0);

    // Après le rattrapage, il n'y a plus aucun trou
    let after_gaps = daemon.detect_missing_ranges(prm, from, to).await.unwrap();
    assert!(after_gaps.is_empty());
}
