#![cfg(all(feature = "mock-sge", feature = "client"))]
#[tokio::test]
async fn test_data_connect_oauth2_and_operations() {
    use chrono::{TimeZone, Utc};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::{
        ConsentStatus, DataConnectClient, DataConnectConfig, EnedisProvider, FlowDirection,
        MeterType, PhaseCount, PointId, TariffOption, Unit,
    };
    use rust_decimal::Decimal;

    let mock = MockSgeServer::start(MockScenario::Success).await;

    // Configuration Data Connect ciblant le serveur de mock
    let config = DataConnectConfig {
        base_url: mock.base_url(),
        token_url: mock.token_url(),
        client_id: Some("mock-client-id".to_string()),
        client_secret: Some(secrecy::SecretString::new("mock-secret".to_string())),
        direct_token: None,
        ..Default::default()
    };

    let client = DataConnectClient::new(config).expect("Client DataConnect instancié");
    let prm = PointId::new("01234567890123").unwrap();
    let from = Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 9, 29, 0, 0, 0).unwrap();

    // 1. Obtention automatique du Bearer token via OAuth2 client_credentials
    let token = client.get_access_token().await.unwrap();
    assert_eq!(token, "mock-dataconnect-bearer-token");

    // 2. Récupération des mesures (courbe de charge conso)
    let measurements = client
        .fetch_measurements(prm, from, to, FlowDirection::Consumption)
        .await
        .unwrap();
    assert_eq!(measurements.len(), 2);
    assert_eq!(measurements[0].value, Decimal::from(1250));
    assert_eq!(measurements[0].unit, Unit::Watt);
    assert_eq!(measurements[1].value, Decimal::from(1410));

    // 3. Récupération de la courbe de charge production
    let prod_measurements = client
        .fetch_measurements(prm, from, to, FlowDirection::Production)
        .await
        .unwrap();
    assert_eq!(prod_measurements.len(), 2);
    assert_eq!(prod_measurements[0].direction, FlowDirection::Production);
    assert_eq!(prod_measurements[0].value, Decimal::from(2400));

    // 4. Données contractuelles
    let contract = client.fetch_contract_data(prm).await.unwrap();
    assert_eq!(contract.point_id, prm);
    assert_eq!(contract.subscribed_power_kva, 9);
    assert_eq!(contract.tariff_option, TariffOption::HeuresPleinesCreuses);
    assert_eq!(contract.status.as_deref(), Some("ACTIF"));

    let meter = contract.meter.unwrap();
    assert_eq!(meter.serial_number.as_deref(), Some("211975001234"));
    assert_eq!(meter.meter_type, MeterType::Linky);
    assert_eq!(meter.phase_count, PhaseCount::SinglePhase);

    // 5. Puissance maximale quotidienne
    let max_power = client.fetch_daily_max_power(prm, from, to).await.unwrap();
    assert_eq!(max_power.len(), 2);
    assert_eq!(max_power[0].value, Decimal::from(4820));
    assert_eq!(max_power[0].unit, Unit::VoltAmpere);

    // 6. Vérification du consentement
    let consent = client.fetch_consent_status(prm).await.unwrap();
    assert_eq!(consent.status, ConsentStatus::Active);
    assert!(consent.valid_from.is_some());
    assert!(consent.valid_to.is_some());

    // 7. Validation de l'interchangeabilité via le trait EnedisProvider
    let provider: &dyn EnedisProvider = &client;
    assert_eq!(provider.provider_name(), "DataConnect-REST-v5");
    let c = provider.fetch_contract_data(prm).await.unwrap();
    assert_eq!(c.subscribed_power_kva, 9);

    mock.stop();
}

#[cfg(all(feature = "mock-sge", feature = "client"))]
#[tokio::test]
async fn test_provider_polymorphism() {
    use chrono::{TimeZone, Utc};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::{
        DataConnectClient, DataConnectConfig, EnedisProvider, FlowDirection, PointId, SgeClient,
        SgeClientConfig,
    };
    use std::sync::Arc;

    let mock = MockSgeServer::start(MockScenario::Success).await;

    let sge_client = SgeClient::new(SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    })
    .unwrap();

    let dc_client = DataConnectClient::new(DataConnectConfig {
        base_url: mock.base_url(),
        token_url: mock.token_url(),
        direct_token: Some(secrecy::SecretString::new("direct-bearer-tok".to_string())),
        ..Default::default()
    })
    .unwrap();

    let prm = PointId::new("01234567890123").unwrap();
    let from = Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 9, 29, 0, 0, 0).unwrap();

    // Vecteur de fournisseurs polymorphes
    let providers: Vec<Arc<dyn EnedisProvider>> = vec![Arc::new(sge_client), Arc::new(dc_client)];

    for provider in &providers {
        // Collecte unifiée
        let measurements = provider
            .fetch_measurements(prm, from, to, FlowDirection::Consumption)
            .await
            .unwrap();
        assert!(
            !measurements.is_empty(),
            "Mesures non vides pour {}",
            provider.provider_name()
        );

        let contract = provider.fetch_contract_data(prm).await.unwrap();
        assert_eq!(contract.subscribed_power_kva, 9);

        let max_power = provider.fetch_daily_max_power(prm, from, to).await.unwrap();
        assert_eq!(max_power.len(), 2);

        let consent = provider.fetch_consent_status(prm).await.unwrap();
        assert!(consent.status.is_active());
    }

    mock.stop();
}
