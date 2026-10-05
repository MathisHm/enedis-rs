#[cfg(all(feature = "api", feature = "storage-sqlite"))]
#[tokio::test]
async fn test_tariff_and_signals_complete_suite() {
    use chrono::{NaiveDate, TimeZone, Utc};
    use enedis_rs::api::{ApiServer, AppState};
    use enedis_rs::metrics::MetricsRegistry;
    use enedis_rs::signal::ShutdownSignal;
    use enedis_rs::storage::{SqliteStorage, StorageBackend};
    use enedis_rs::{
        calculate_energy_costs, correlate_measurements_with_grid, BaseTariff, CostCalculation,
        DynamicTariff, EcoWattLevel, EcoWattSignal, FlowDirection, GridCorrelationReport,
        HpHcTariff, Measurement, MeasurementQuality, PointId, TariffConfig, TempoColor,
        TempoDayRecord, TempoTariff, TimeSlot, Unit,
    };
    use rust_decimal::Decimal;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    let storage = Arc::new(SqliteStorage::connect("sqlite::memory:").await.unwrap());
    let prm = PointId::new("01234567890123").unwrap();

    // 1. Insertion de mesures de consommation réparties sur une journée (2026-01-15 - Jour d'hiver UTC+1)
    // 02:00 UTC = 03:00 local (HC Tempo du 14 janvier, HC standard)
    let m1 = Measurement {
        point_id: prm,
        timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 2, 0, 0).unwrap(),
        interval_seconds: 3600,
        direction: FlowDirection::Consumption,
        value: Decimal::from(10), // 10 kWh
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };

    // 12:00 UTC = 13:00 local (HP Tempo du 15 janvier, HP standard)
    let m2 = Measurement {
        point_id: prm,
        timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap(),
        interval_seconds: 3600,
        direction: FlowDirection::Consumption,
        value: Decimal::from(5), // 5 kWh
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };

    // 18:00 UTC = 19:00 local (Pointe de soirée, HP Tempo Rouge du 15 janvier, alerte EcoWatt Rouge)
    let m3 = Measurement {
        point_id: prm,
        timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 18, 0, 0).unwrap(),
        interval_seconds: 3600,
        direction: FlowDirection::Consumption,
        value: Decimal::from(2), // 2 kWh (effacé)
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };

    storage
        .upsert_measurements(&[m1.clone(), m2.clone(), m3.clone()])
        .await
        .unwrap();

    // 2. Insertion des signaux réseau dans le stockage SQLite
    let tempo_14 = TempoDayRecord {
        date: NaiveDate::from_ymd_opt(2026, 1, 14).unwrap(),
        color: TempoColor::White,
        updated_at: Utc::now(),
    };
    let tempo_15 = TempoDayRecord {
        date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
        color: TempoColor::Red,
        updated_at: Utc::now(),
    };
    let tempo_16 = TempoDayRecord {
        date: NaiveDate::from_ymd_opt(2026, 1, 16).unwrap(),
        color: TempoColor::Blue,
        updated_at: Utc::now(),
    };

    storage
        .upsert_tempo_days(&[tempo_14.clone(), tempo_15.clone(), tempo_16.clone()])
        .await
        .unwrap();

    // Vérification de la persistance Tempo
    let retrieved_tempo = storage
        .get_tempo_days(
            NaiveDate::from_ymd_opt(2026, 1, 14).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 16).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retrieved_tempo.len(), 3);
    assert_eq!(retrieved_tempo[1].color, TempoColor::Red);

    // Insertion des signaux EcoWatt
    let eco_green = EcoWattSignal {
        timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 2, 0, 0).unwrap(),
        level: EcoWattLevel::Green,
        message: None,
    };
    let eco_orange = EcoWattSignal {
        timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap(),
        level: EcoWattLevel::Orange,
        message: Some("Tension modérée".to_string()),
    };
    let eco_red = EcoWattSignal {
        timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 18, 0, 0).unwrap(),
        level: EcoWattLevel::Red,
        message: Some("Alerte rouge réseau".to_string()),
    };

    storage
        .upsert_ecowatt_signals(&[eco_green.clone(), eco_orange.clone(), eco_red.clone()])
        .await
        .unwrap();

    let retrieved_eco = storage
        .get_ecowatt_signals(
            Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 1, 15, 23, 59, 59).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retrieved_eco.len(), 3);

    // 3. Test unitaire direct du moteur tarifaire
    let from = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 1, 15, 23, 59, 59).unwrap();
    let measures = vec![m1.clone(), m2.clone(), m3.clone()];

    // A. Tarif Base
    let base_cost = calculate_energy_costs(
        prm,
        from,
        to,
        &measures,
        &TariffConfig::Base(BaseTariff::default()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(base_cost.total_energy_kwh, Decimal::from(17));
    assert_eq!(base_cost.breakdown.len(), 1);
    assert_eq!(base_cost.breakdown[0].bucket_name, "Base");

    // B. Tarif HP/HC
    let hphc_cost = calculate_energy_costs(
        prm,
        from,
        to,
        &measures,
        &TariffConfig::HpHc(HpHcTariff::default()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(hphc_cost.total_energy_kwh, Decimal::from(17));
    let hp_b = hphc_cost
        .breakdown
        .iter()
        .find(|b| b.bucket_name == "Heures Pleines")
        .unwrap();
    let hc_b = hphc_cost
        .breakdown
        .iter()
        .find(|b| b.bucket_name == "Heures Creuses")
        .unwrap();
    assert_eq!(hp_b.energy_kwh, Decimal::from(7)); // 5 kWh (12h) + 2 kWh (18h)
    assert_eq!(hc_b.energy_kwh, Decimal::from(10)); // 10 kWh (02h)
    assert!(hphc_cost.comparison_with_base.is_some());

    // C. Tarif Tempo avec calendrier
    let tempo_cost = calculate_energy_costs(
        prm,
        from,
        to,
        &measures,
        &TariffConfig::Tempo(TempoTariff::default()),
        None,
        Some(&retrieved_tempo),
    )
    .unwrap();
    assert_eq!(tempo_cost.total_energy_kwh, Decimal::from(17));
    // La mesure de 2h00 appartient à la journée Tempo du 14 janvier (BLANC HC)
    let white_hc = tempo_cost
        .breakdown
        .iter()
        .find(|b| b.bucket_name == "Tempo Blanc HC")
        .unwrap();
    assert_eq!(white_hc.energy_kwh, Decimal::from(10));
    // Les mesures de 12h et 18h appartiennent à la journée Tempo du 15 janvier (ROUGE HP)
    let red_hp = tempo_cost
        .breakdown
        .iter()
        .find(|b| b.bucket_name == "Tempo Rouge HP")
        .unwrap();
    assert_eq!(red_hp.energy_kwh, Decimal::from(7));

    // D. Tarif Dynamique avec prix spots
    let mut dyn_prices = HashMap::new();
    dyn_prices.insert(
        Utc.with_ymd_and_hms(2026, 1, 15, 2, 0, 0).unwrap(),
        Decimal::from_str_exact("0.0800").unwrap(),
    );
    dyn_prices.insert(
        Utc.with_ymd_and_hms(2026, 1, 15, 18, 0, 0).unwrap(),
        Decimal::from_str_exact("0.4500").unwrap(),
    );
    let dyn_tariff = DynamicTariff {
        fallback_price_per_kwh: Decimal::from_str_exact("0.1800").unwrap(),
        fixed_margin_per_kwh: Decimal::from_str_exact("0.0200").unwrap(),
        monthly_subscription: Decimal::from_str_exact("14.00").unwrap(),
        hourly_prices: dyn_prices,
    };
    let dyn_cost = calculate_energy_costs(
        prm,
        from,
        to,
        &measures,
        &TariffConfig::Dynamic(dyn_tariff),
        None,
        None,
    )
    .unwrap();
    assert_eq!(dyn_cost.total_energy_kwh, Decimal::from(17));
    assert!(dyn_cost.total_cost_ttc > Decimal::ZERO);

    // 4. Test unitaire du moteur de corrélation réseau
    let report = correlate_measurements_with_grid(
        prm,
        from,
        to,
        &measures,
        &retrieved_eco,
        Some(&retrieved_tempo),
    );
    assert_eq!(report.total_consumption_kwh, Decimal::from(17));
    assert_eq!(report.ecowatt.green_samples_count, 1);
    assert_eq!(report.ecowatt.orange_samples_count, 1);
    assert_eq!(report.ecowatt.red_samples_count, 1);
    // Puissance en W : 10 kWh/1h = 10000 W (vert), 2 kWh/1h = 2000 W (rouge)
    // Flexibilité : (10000 - 2000) / 10000 = +80%
    assert_eq!(
        report.ecowatt.red_flexibility_score_percentage,
        Decimal::from(80)
    );
    assert!(report.tempo.is_some());
    let t_corr = report.tempo.unwrap();
    assert_eq!(t_corr.white_hc_kwh, Decimal::from(10));
    assert_eq!(t_corr.red_hp_kwh, Decimal::from(7));

    // 5. Démarrage du serveur API Axum et validation des endpoints HTTP
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let metrics = MetricsRegistry::new();
    let state = AppState::new(storage.clone(), None, metrics);
    let router = ApiServer::router(state);
    let shutdown = ShutdownSignal::new();
    let shutdown_rx = shutdown.clone();

    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                shutdown_rx.cancelled().await;
            })
            .await
            .unwrap();
    });

    let http_client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // Test GET /api/v1/points/:prm/costs (Option Base par défaut)
    let res = http_client
        .get(format!(
            "{}/api/v1/points/{}/costs?from=2026-01-15T00:00:00Z&to=2026-01-15T23:59:59Z",
            base_url, prm
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let calc: CostCalculation = res.json().await.unwrap();
    assert_eq!(calc.point_id, prm);
    assert_eq!(calc.tariff_type, "BASE");
    assert_eq!(calc.total_energy_kwh, Decimal::from(17));

    // Test GET /api/v1/points/:prm/costs?tariff_type=tempo
    let res = http_client
        .get(format!(
            "{}/api/v1/points/{}/costs?tariff_type=tempo&from=2026-01-15T00:00:00Z&to=2026-01-15T23:59:59Z",
            base_url, prm
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let tempo_calc: CostCalculation = res.json().await.unwrap();
    assert_eq!(tempo_calc.tariff_type, "TEMPO");
    assert!(tempo_calc.comparison_with_base.is_some());

    // Test POST /api/v1/points/:prm/costs avec configuration JSON complète
    let custom_hphc = TariffConfig::HpHc(HpHcTariff {
        hp_price_per_kwh: Decimal::from_str_exact("0.3000").unwrap(),
        hc_price_per_kwh: Decimal::from_str_exact("0.1800").unwrap(),
        monthly_subscription: Decimal::from_str_exact("15.00").unwrap(),
        off_peak_slots: vec![TimeSlot::new(
            chrono::NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
            chrono::NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
        )],
    });
    let res = http_client
        .post(format!(
            "{}/api/v1/points/{}/costs?from=2026-01-15T00:00:00Z&to=2026-01-15T23:59:59Z",
            base_url, prm
        ))
        .json(&custom_hphc)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let post_calc: CostCalculation = res.json().await.unwrap();
    assert_eq!(post_calc.tariff_type, "HEURES_PLEINES_HEURES_CREUSES");

    // Test GET /api/v1/signals/tempo
    let res = http_client
        .get(format!("{}/api/v1/signals/tempo", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let tempo_signals: serde_json::Value = res.json().await.unwrap();
    assert!(tempo_signals["history"].is_array());

    // Test GET /api/v1/signals/ecowatt
    let res = http_client
        .get(format!("{}/api/v1/signals/ecowatt", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let eco_list: Vec<EcoWattSignal> = res.json().await.unwrap();
    assert!(!eco_list.is_empty());

    // Test GET /api/v1/points/:prm/grid-correlation
    let res = http_client
        .get(format!(
            "{}/api/v1/points/{}/grid-correlation?from=2026-01-15T00:00:00Z&to=2026-01-15T23:59:59Z",
            base_url, prm
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let corr_report: GridCorrelationReport = res.json().await.unwrap();
    assert_eq!(corr_report.total_consumption_kwh, Decimal::from(17));
    assert_eq!(
        corr_report.ecowatt.red_flexibility_score_percentage,
        Decimal::from(80)
    );

    // Arrêt gracieux
    shutdown.cancel();
}
