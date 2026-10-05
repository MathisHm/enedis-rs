#![cfg(feature = "storage-sqlite")]

use chrono::{TimeZone, Utc};
use enedis_rs::models::{
    aggregate_measurements, AggregationInterval, FlowDirection, Measurement, MeasurementQuality,
    PointId, Unit,
};
use enedis_rs::storage::{SqliteStorage, StorageBackend};
use rust_decimal::Decimal;
use std::str::FromStr;

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn test_sqlite_aggregation_variable_intervals_and_units() {
    let storage = SqliteStorage::connect("sqlite::memory:")
        .await
        .expect("Connexion SQLite mémoire impossible");

    let prm = PointId::new("01234567890123").unwrap();

    // Insertion de mesures avec différents pas et différentes unités pour la même journée (2026-09-28)
    // 1. 10:00 UTC : 10 min (600s), 1200 W (Watt) -> 0.2000 kWh, 1200 W
    // 2. 10:10 UTC : 15 min (900s), 2 kW (KiloWatt) -> 2000 W, 0.5000 kWh
    // 3. 10:25 UTC : 30 min (1800s), 1000 Wh (WattHour) -> 2000 W, 1.0000 kWh
    // 4. 11:00 UTC : 30 min (1800s), 1.5 kWh (KiloWattHour) -> 3000 W, 1.5000 kWh
    let m1 = Measurement {
        point_id: prm,
        timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 10, 0, 0).unwrap(),
        interval_seconds: 600,
        direction: FlowDirection::Consumption,
        value: Decimal::from(1200),
        unit: Unit::Watt,
        quality: MeasurementQuality::Validated,
    };
    let m2 = Measurement {
        point_id: prm,
        timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 10, 10, 0).unwrap(),
        interval_seconds: 900,
        direction: FlowDirection::Consumption,
        value: Decimal::from(2),
        unit: Unit::KiloWatt,
        quality: MeasurementQuality::Validated,
    };
    let m3 = Measurement {
        point_id: prm,
        timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 10, 25, 0).unwrap(),
        interval_seconds: 1800,
        direction: FlowDirection::Consumption,
        value: Decimal::from(1000),
        unit: Unit::WattHour,
        quality: MeasurementQuality::Validated,
    };
    let m4 = Measurement {
        point_id: prm,
        timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 11, 0, 0).unwrap(),
        interval_seconds: 1800,
        direction: FlowDirection::Consumption,
        value: Decimal::from_str("1.5").unwrap(),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };

    let all_measurements = vec![m1.clone(), m2.clone(), m3.clone(), m4.clone()];
    storage
        .upsert_measurements(&all_measurements)
        .await
        .unwrap();

    let from = Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 9, 28, 23, 59, 59).unwrap();

    // 1. Agrégation horaire (Hourly)
    let hourly_aggs = storage
        .get_aggregated_measurements(
            prm,
            from,
            to,
            AggregationInterval::Hourly,
            Some(FlowDirection::Consumption),
        )
        .await
        .unwrap();

    assert_eq!(hourly_aggs.len(), 2);

    // Heure 10:00 - 11:00 (m1, m2, m3)
    let h10 = &hourly_aggs[0];
    assert_eq!(
        h10.bucket_start,
        Utc.with_ymd_and_hms(2026, 9, 28, 10, 0, 0).unwrap()
    );
    assert_eq!(
        h10.bucket_end,
        Utc.with_ymd_and_hms(2026, 9, 28, 11, 0, 0).unwrap()
    );
    assert_eq!(h10.sample_count, 3);
    // Somme: 0.2 + 0.5 + 1.0 = 1.7 kWh
    assert_eq!(h10.total_energy_kwh, Decimal::from_str("1.7000").unwrap());
    // Puissances: 1200 W, 2000 W, 2000 W
    assert_eq!(
        h10.min_power_w,
        Some(Decimal::from_str("1200.0000").unwrap())
    );
    assert_eq!(
        h10.max_power_w,
        Some(Decimal::from_str("2000.0000").unwrap())
    );
    assert_eq!(
        h10.avg_power_w,
        Some(Decimal::from_str("1733.3333").unwrap())
    );

    // Heure 11:00 - 12:00 (m4)
    let h11 = &hourly_aggs[1];
    assert_eq!(
        h11.bucket_start,
        Utc.with_ymd_and_hms(2026, 9, 28, 11, 0, 0).unwrap()
    );
    assert_eq!(
        h11.bucket_end,
        Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap()
    );
    assert_eq!(h11.sample_count, 1);
    assert_eq!(h11.total_energy_kwh, Decimal::from_str("1.5000").unwrap());
    assert_eq!(
        h11.min_power_w,
        Some(Decimal::from_str("3000.0000").unwrap())
    );
    assert_eq!(
        h11.max_power_w,
        Some(Decimal::from_str("3000.0000").unwrap())
    );
    assert_eq!(
        h11.avg_power_w,
        Some(Decimal::from_str("3000.0000").unwrap())
    );

    // 2. Agrégation quotidienne (Daily)
    let daily_aggs = storage
        .get_aggregated_measurements(
            prm,
            from,
            to,
            AggregationInterval::Daily,
            Some(FlowDirection::Consumption),
        )
        .await
        .unwrap();

    assert_eq!(daily_aggs.len(), 1);
    let d = &daily_aggs[0];
    assert_eq!(
        d.bucket_start,
        Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap()
    );
    assert_eq!(
        d.bucket_end,
        Utc.with_ymd_and_hms(2026, 9, 29, 0, 0, 0).unwrap()
    );
    assert_eq!(d.sample_count, 4);
    // Somme totale: 1.7 + 1.5 = 3.2 kWh
    assert_eq!(d.total_energy_kwh, Decimal::from_str("3.2000").unwrap());
    // Puissances: min = 1200 W, max = 3000 W, avg = (1200 + 2000 + 2000 + 3000) / 4 = 2050 W
    assert_eq!(d.min_power_w, Some(Decimal::from_str("1200.0000").unwrap()));
    assert_eq!(d.max_power_w, Some(Decimal::from_str("3000.0000").unwrap()));
    assert_eq!(d.avg_power_w, Some(Decimal::from_str("2050.0000").unwrap()));

    // Comparaison stricte avec la fonction de référence aggregate_measurements
    let ref_daily = aggregate_measurements(&all_measurements, AggregationInterval::Daily);
    assert_eq!(daily_aggs, ref_daily);
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn test_sqlite_aggregation_direction_filtering_and_monthly() {
    let storage = SqliteStorage::connect("sqlite::memory:")
        .await
        .expect("Connexion SQLite mémoire impossible");

    let prm = PointId::new("01234567890123").unwrap();

    let ts1 = Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap();
    let ts2 = Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap();
    let ts3 = Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap();

    // 1. Conso: 10 kWh
    let m_cons = Measurement {
        point_id: prm,
        timestamp: ts1,
        interval_seconds: 3600,
        direction: FlowDirection::Consumption,
        value: Decimal::from(10),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };

    // 2. Prod: 4 kWh
    let m_prod = Measurement {
        point_id: prm,
        timestamp: ts2,
        interval_seconds: 3600,
        direction: FlowDirection::Production,
        value: Decimal::from(4),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };

    // 3. Conso mois suivant (octobre): 5 kWh
    let m_cons_oct = Measurement {
        point_id: prm,
        timestamp: ts3,
        interval_seconds: 3600,
        direction: FlowDirection::Consumption,
        value: Decimal::from(5),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };

    storage
        .upsert_measurements(&[m_cons, m_prod, m_cons_oct])
        .await
        .unwrap();

    let from = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 10, 31, 23, 59, 59).unwrap();

    // Filtre sur Production uniquement
    let prod_monthly = storage
        .get_aggregated_measurements(
            prm,
            from,
            to,
            AggregationInterval::Monthly,
            Some(FlowDirection::Production),
        )
        .await
        .unwrap();
    assert_eq!(prod_monthly.len(), 1);
    assert_eq!(prod_monthly[0].direction, FlowDirection::Production);
    assert_eq!(
        prod_monthly[0].total_energy_kwh,
        Decimal::from_str("4.0000").unwrap()
    );
    assert_eq!(
        prod_monthly[0].bucket_start,
        Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap()
    );
    assert_eq!(
        prod_monthly[0].bucket_end,
        Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap()
    );

    // Pas de filtre sur direction (doit retourner consommation et production séparées)
    let all_monthly = storage
        .get_aggregated_measurements(prm, from, to, AggregationInterval::Monthly, None)
        .await
        .unwrap();
    // Mois 9: Conso (10 kWh) et Prod (4 kWh), Mois 10: Conso (5 kWh) -> 3 agrégats
    assert_eq!(all_monthly.len(), 3);

    // Vérification de l'agrégation annuelle (Yearly)
    let yearly = storage
        .get_aggregated_measurements(
            prm,
            from,
            to,
            AggregationInterval::Yearly,
            Some(FlowDirection::Consumption),
        )
        .await
        .unwrap();
    assert_eq!(yearly.len(), 1);
    assert_eq!(
        yearly[0].bucket_start,
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    );
    assert_eq!(
        yearly[0].bucket_end,
        Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()
    );
    assert_eq!(
        yearly[0].total_energy_kwh,
        Decimal::from_str("15.0000").unwrap()
    );
    assert_eq!(yearly[0].sample_count, 2);
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn test_sqlite_aggregation_empty_range() {
    let storage = SqliteStorage::connect("sqlite::memory:").await.unwrap();
    let prm = PointId::new("01234567890123").unwrap();
    let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();

    let res = storage
        .get_aggregated_measurements(prm, from, to, AggregationInterval::Daily, None)
        .await
        .unwrap();
    assert!(res.is_empty());
}

#[cfg(feature = "storage-postgres")]
#[tokio::test]
async fn test_postgres_aggregation_live_if_configured() {
    use enedis_rs::storage::PostgresStorage;

    let url = std::env::var("POSTGRES_URL").or_else(|_| std::env::var("DATABASE_URL"));
    let url = match url {
        Ok(u) if u.starts_with("postgres://") || u.starts_with("postgresql://") => u,
        _ => {
            eprintln!("Test PostgreSQL ignoré: URL non configurée");
            return;
        }
    };

    let storage = match PostgresStorage::connect(&url).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Connexion PostgreSQL impossible: {}", e);
            return;
        }
    };

    let prm = PointId::new("01234567890123").unwrap();
    let ts = Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap();

    let m = Measurement {
        point_id: prm,
        timestamp: ts,
        interval_seconds: 1800,
        direction: FlowDirection::Consumption,
        value: Decimal::from_str("2.5000").unwrap(),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };

    storage.upsert_measurements(&[m]).await.unwrap();

    let from = Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 9, 28, 23, 59, 59).unwrap();

    let aggs = storage
        .get_aggregated_measurements(
            prm,
            from,
            to,
            AggregationInterval::Daily,
            Some(FlowDirection::Consumption),
        )
        .await
        .unwrap();

    assert!(!aggs.is_empty());
    assert_eq!(aggs[0].point_id, prm);
    assert_eq!(
        aggs[0].total_energy_kwh,
        Decimal::from_str("2.5000").unwrap()
    );
}
