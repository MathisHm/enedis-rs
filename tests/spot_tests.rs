use chrono::{DateTime, Duration, TimeZone, Timelike, Utc};
use enedis_rs::models::{
    analyze_spot_consumption, generate_synthetic_spot_profile, FlowDirection, Measurement,
    MeasurementQuality, PointId, SpotPriceRecord, TaxConfig, Unit,
};
use rust_decimal::Decimal;

fn make_measurement(
    prm: PointId,
    timestamp: DateTime<Utc>,
    w: u32,
    direction: FlowDirection,
) -> Measurement {
    Measurement {
        point_id: prm,
        timestamp,
        interval_seconds: 1800,
        direction,
        value: Decimal::from(w),
        unit: Unit::Watt,
        quality: MeasurementQuality::Validated,
    }
}

#[test]
fn test_spot_price_record_unit_conversions_and_negative_detection() {
    let ts = Utc.with_ymd_and_hms(2026, 9, 30, 14, 0, 0).unwrap();
    let tax = TaxConfig::default();
    let margin = Decimal::from_str_exact("0.0150").unwrap(); // 1.5 c€/kWh

    // 1. Prix spot positif classique (65.40 €/MWh)
    let sp_pos = SpotPriceRecord::new(ts, Decimal::from_str_exact("65.40").unwrap(), None);
    assert_eq!(
        sp_pos.price_eur_per_kwh,
        Decimal::from_str_exact("0.0654").unwrap()
    );
    assert!(!sp_pos.is_negative);

    // Calcul consommateur TTC : (0.0654 + 0.0150 + 0.0210 TICFE) * 1.20 = 0.1014 * 1.20 = 0.1217 €/kWh
    let ttc_pos = sp_pos.consumer_price_ttc(margin, &tax);
    assert_eq!(ttc_pos, Decimal::from_str_exact("0.1217").unwrap());

    // 2. Prix spot négatif (-20.00 €/MWh)
    let sp_neg = SpotPriceRecord::new(
        ts,
        Decimal::from_str_exact("-20.00").unwrap(),
        Some("EPEX_SPOT_FR"),
    );
    assert_eq!(
        sp_neg.price_eur_per_kwh,
        Decimal::from_str_exact("-0.0200").unwrap()
    );
    assert!(sp_neg.is_negative);

    // Calcul consommateur TTC : (-0.0200 + 0.0150 + 0.0210) * 1.20 = 0.0160 * 1.20 = 0.0192 €/kWh
    let ttc_neg = sp_neg.consumer_price_ttc(margin, &tax);
    assert_eq!(ttc_neg, Decimal::from_str_exact("0.0192").unwrap());
    assert!(ttc_neg < ttc_pos);
}

#[test]
fn test_generate_synthetic_spot_profile_shapes_and_negative_hours() {
    let from = Utc.with_ymd_and_hms(2026, 6, 20, 0, 0, 0).unwrap(); // Samedi d'été
    let to = from + Duration::days(2);

    let profile = generate_synthetic_spot_profile(from, to);
    assert_eq!(profile.len(), 48); // 48 heures

    // Au moins 1 créneau doit être négatif le samedi/dimanche après-midi en été (midi solaire)
    let has_negative = profile.iter().any(|p| p.is_negative);
    assert!(
        has_negative,
        "Un week-end d'été doit comporter des prix spot négatifs"
    );

    // Les prix de pointe du soir (vers 19h-20h) doivent être supérieurs aux prix de nuit (02h-04h)
    let night_price = profile.iter().find(|p| p.timestamp.hour() == 3).unwrap();
    let evening_price = profile.iter().find(|p| p.timestamp.hour() == 19).unwrap();
    assert!(evening_price.price_eur_per_mwh > night_price.price_eur_per_mwh);
}

#[test]
fn test_analyze_spot_consumption_profiling_and_arbitrage() {
    let prm = PointId::new("01234567890123").unwrap();
    let start = Utc.with_ymd_and_hms(2026, 6, 20, 0, 0, 0).unwrap();
    let end = start + Duration::days(7);

    let spot_prices = generate_synthetic_spot_profile(start, end);

    // Profil 1 : Consommateur concentré en soirée (19h-21h, heures de pointe de prix)
    let mut peak_measurements = Vec::new();
    for day in 0..7 {
        let day_start = start + Duration::days(day);
        for half_hour in 0..48 {
            let ts = day_start + Duration::minutes(half_hour * 30);
            let hour = ts.hour();
            let w = if (19..=21).contains(&hour) { 4000 } else { 200 };
            peak_measurements.push(make_measurement(prm, ts, w, FlowDirection::Consumption));
        }
    }

    let analysis_peak = analyze_spot_consumption(prm, &peak_measurements, &spot_prices, None, None)
        .expect("Analyse du profil en pointe");

    // Profiler coefficient > 1.0 (consomme plus cher que la moyenne du marché)
    assert!(analysis_peak.profiling_coefficient > Decimal::ONE);
    assert!(
        analysis_peak.weighted_average_spot_price_mwh > analysis_peak.market_average_spot_price_mwh
    );
    assert!(analysis_peak.arbitrage.annual_arbitrage_savings_euro > Decimal::ZERO);

    // Profil 2 : Consommateur vertueux effacé (recharge en milieu de journée solaire et nuit creuse)
    let mut offpeak_measurements = Vec::new();
    for day in 0..7 {
        let day_start = start + Duration::days(day);
        for half_hour in 0..48 {
            let ts = day_start + Duration::minutes(half_hour * 30);
            let hour = ts.hour();
            let w = if (13..=15).contains(&hour) || (2..=4).contains(&hour) {
                3000
            } else {
                150
            };
            offpeak_measurements.push(make_measurement(prm, ts, w, FlowDirection::Consumption));
        }
    }

    let analysis_offpeak =
        analyze_spot_consumption(prm, &offpeak_measurements, &spot_prices, None, None)
            .expect("Analyse du profil effacé");

    // Le profil effacé doit payer un prix moyen pondéré inférieur au profil en pointe
    assert!(
        analysis_offpeak.weighted_average_spot_price_mwh
            < analysis_peak.weighted_average_spot_price_mwh
    );
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test]
async fn test_sqlite_spot_prices_upsert_and_query() {
    use enedis_rs::storage::{SqliteStorage, StorageBackend};

    let storage = SqliteStorage::connect("sqlite::memory:").await.unwrap();
    storage.init_schema().await.unwrap();

    let t1 = Utc.with_ymd_and_hms(2026, 9, 30, 10, 0, 0).unwrap();
    let t2 = Utc.with_ymd_and_hms(2026, 9, 30, 11, 0, 0).unwrap();

    let records = vec![
        SpotPriceRecord::new(
            t1,
            Decimal::from_str_exact("72.50").unwrap(),
            Some("EPEX_SPOT_FR"),
        ),
        SpotPriceRecord::new(
            t2,
            Decimal::from_str_exact("-15.00").unwrap(),
            Some("EPEX_SPOT_FR"),
        ),
    ];

    // Insertion
    storage.upsert_spot_prices(&records).await.unwrap();

    // Récupération
    let retrieved = storage
        .get_spot_prices(t1 - Duration::hours(1), t2 + Duration::hours(1))
        .await
        .unwrap();

    assert_eq!(retrieved.len(), 2);
    assert_eq!(
        retrieved[0].price_eur_per_mwh,
        Decimal::from_str_exact("72.50").unwrap()
    );
    assert!(!retrieved[0].is_negative);
    assert_eq!(
        retrieved[1].price_eur_per_mwh,
        Decimal::from_str_exact("-15.00").unwrap()
    );
    assert!(retrieved[1].is_negative);
}
