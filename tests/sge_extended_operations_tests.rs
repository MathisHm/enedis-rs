#![cfg(all(feature = "mock-sge", feature = "client"))]
#[tokio::test]
async fn test_sge_contract_data_operation() {
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::{
        EnedisProvider, MeterType, PhaseCount, PointId, SgeClient, SgeClientConfig, TariffOption,
    };

    let mock = MockSgeServer::start(MockScenario::Success).await;
    let config = SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(config).unwrap();
    let prm = PointId::new("01234567890123").unwrap();

    // 1. Appel direct via méthode inhérente de SgeClient
    let contract = client.fetch_contract_data(prm).await.unwrap();
    assert_eq!(contract.point_id, prm);
    assert_eq!(contract.subscribed_power_kva, 9);
    assert_eq!(contract.tariff_option, TariffOption::HeuresPleinesCreuses);
    assert_eq!(contract.status.as_deref(), Some("ACTIF"));

    let cal = contract.calendar.unwrap();
    assert_eq!(cal.schedule_name.as_deref(), Some("Option Creuse SGE"));
    assert_eq!(cal.off_peak_ranges, vec!["22h00-06h00"]);

    let meter = contract.meter.unwrap();
    assert_eq!(meter.serial_number.as_deref(), Some("211975001234"));
    assert_eq!(meter.meter_type, MeterType::Linky);
    assert_eq!(meter.phase_count, PhaseCount::SinglePhase);
    assert_eq!(meter.circuit_breaker_amperes, Some(45));

    // 2. Appel via le trait EnedisProvider
    let provider: &dyn EnedisProvider = &client;
    assert_eq!(provider.provider_name(), "SGE-SOAP-mTLS");
    let contract_from_trait = provider.fetch_contract_data(prm).await.unwrap();
    assert_eq!(contract_from_trait.subscribed_power_kva, 9);

    mock.stop();
}

#[cfg(all(feature = "mock-sge", feature = "client"))]
#[tokio::test]
async fn test_sge_max_power_operation_and_audit() {
    use chrono::{TimeZone, Utc};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::{
        audit_subscription_sizing, PointId, SgeClient, SgeClientConfig, SizingStatus, Unit,
    };
    use rust_decimal::Decimal;

    let mock = MockSgeServer::start(MockScenario::Success).await;
    let config = SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(config).unwrap();
    let prm = PointId::new("01234567890123").unwrap();
    let from = Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 9, 30, 0, 0, 0).unwrap();

    let records = client.fetch_daily_max_power(prm, from, to).await.unwrap();
    assert_eq!(records.len(), 2);

    assert_eq!(records[0].value, Decimal::from(4820));
    assert_eq!(records[0].unit, Unit::Watt);
    assert_eq!(
        records[0].value_kva(),
        Decimal::from_str_exact("4.82").unwrap()
    );

    assert_eq!(records[1].value, Decimal::from(5200));
    assert_eq!(records[1].unit, Unit::VoltAmpere);
    assert_eq!(
        records[1].value_kva(),
        Decimal::from_str_exact("5.2").unwrap()
    );

    // Audit d'adéquation de l'abonnement
    // 1. Pour un abonnement de 6 kVA (pointe de 5.2 kVA + marge 10% = 5.72 kVA) -> Optimal (ou 6 kVA)
    let audit_6kva = audit_subscription_sizing(prm, 6, &records);
    assert_eq!(audit_6kva.recommended_power_kva, 6);

    // 2. Pour un abonnement de 12 kVA -> Oversized (pourrait passer à 6 kVA)
    let audit_12kva = audit_subscription_sizing(prm, 12, &records);
    assert_eq!(audit_12kva.sizing_status, SizingStatus::Oversized);
    assert_eq!(audit_12kva.recommended_power_kva, 6);

    mock.stop();
}

#[cfg(all(feature = "mock-sge", feature = "client"))]
#[tokio::test]
async fn test_sge_proactive_consent_verification() {
    use chrono::{TimeZone, Utc};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::{ConsentAlertSeverity, ConsentStatus, PointId, SgeClient, SgeClientConfig};

    let mock = MockSgeServer::start(MockScenario::Success).await;
    let config = SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(config).unwrap();
    let prm = PointId::new("01234567890123").unwrap();

    let consent = client.fetch_consent_status(prm).await.unwrap();
    assert_eq!(consent.status, ConsentStatus::Active);
    assert!(consent.valid_from.is_some());
    assert!(consent.valid_to.is_some());
    assert!(consent
        .authorized_usages
        .contains(&"COURBE_DE_CHARGE".to_string()));

    // Test de l'alerte proactive
    // Date de référence : 2026-12-25 (expiration le 2027-01-01 -> 7 jours restants)
    let ref_date = Utc.with_ymd_and_hms(2026, 12, 25, 0, 0, 0).unwrap();
    let alert = consent.check_alert(30, ref_date);
    assert_eq!(alert.severity, ConsentAlertSeverity::Critical);
    assert_eq!(alert.days_remaining, 7);

    // Date de référence : 2026-12-10 (22 jours restants)
    let ref_date_warning = Utc.with_ymd_and_hms(2026, 12, 10, 0, 0, 0).unwrap();
    let alert_warning = consent.check_alert(30, ref_date_warning);
    assert_eq!(alert_warning.severity, ConsentAlertSeverity::Warning);
    assert_eq!(alert_warning.days_remaining, 22);

    mock.stop();
}
