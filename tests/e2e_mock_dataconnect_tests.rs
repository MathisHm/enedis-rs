#![cfg(all(feature = "mock-sge", feature = "client"))]

use chrono::{Duration, Utc};
use enedis_rs::client::{DataConnectClient, DataConnectConfig};
use enedis_rs::error::{EnedisError, SgeBusinessError};
use enedis_rs::mock::{MockDataConnectServer, MockScenario};
use enedis_rs::models::{FlowDirection, PointId, Unit};
use secrecy::SecretString;

fn build_dc_config(mock: &MockDataConnectServer) -> DataConnectConfig {
    DataConnectConfig {
        base_url: mock.data_connect_url(),
        token_url: mock.data_connect_token_url(),
        authorize_url: Some(mock.authorize_url()),
        client_id: Some("mock-client-id".to_string()),
        client_secret: Some(SecretString::new("mock-client-secret".to_string())),
        direct_token: None,
        connect_timeout: std::time::Duration::from_secs(5),
        request_timeout: std::time::Duration::from_secs(5),
        user_agent: "enedis-test/1.0".to_string(),
    }
}

#[tokio::test]
async fn test_mock_dataconnect_nominal_operations() {
    let mock = MockDataConnectServer::start(MockScenario::Success).await;
    let config = build_dc_config(&mock);
    let client = DataConnectClient::new(config).expect("Client DC valide");

    let prm = PointId::new("01234567890123").unwrap();
    let to = Utc::now();
    let from = to - Duration::days(1);

    // 1. Courbe de charge Consommation
    let cons = client
        .fetch_measurements(prm, from, to, FlowDirection::Consumption)
        .await
        .expect("Collecte conso nominale");
    assert_eq!(cons.len(), 2);
    assert_eq!(cons[0].direction, FlowDirection::Consumption);

    // 2. Courbe de charge Production
    let prod = client
        .fetch_measurements(prm, from, to, FlowDirection::Production)
        .await
        .expect("Collecte prod nominale");
    assert_eq!(prod.len(), 2);
    assert_eq!(prod[0].direction, FlowDirection::Production);

    // 3. Pointes maximales de puissance
    let powers = client
        .fetch_daily_max_power(prm, from, to)
        .await
        .expect("Collecte puissance max");
    assert_eq!(powers.len(), 2);
    assert_eq!(powers[0].unit, Unit::VoltAmpere);

    // 4. Données contractuelles
    let contract = client
        .fetch_contract_data(prm)
        .await
        .expect("Données contractuelles");
    assert_eq!(contract.subscribed_power_kva, 9);
    assert!(contract.calendar.is_some());
    assert!(contract.meter.is_some());

    // 5. Consentement
    let consent = client
        .fetch_consent_status(prm)
        .await
        .expect("Consentement");
    assert_eq!(consent.status, enedis_rs::models::ConsentStatus::Active);

    mock.stop();
}

#[tokio::test]
async fn test_mock_dataconnect_consent_expired_scenario() {
    let mock = MockDataConnectServer::start(MockScenario::BusinessConsentExpired).await;
    let config = build_dc_config(&mock);
    let client = DataConnectClient::new(config).expect("Client DC valide");

    let prm = PointId::new("01234567890123").unwrap();
    let to = Utc::now();
    let from = to - Duration::days(1);

    let err = client
        .fetch_measurements(prm, from, to, FlowDirection::Consumption)
        .await
        .unwrap_err();

    match err {
        EnedisError::Business(SgeBusinessError::ConsentMissingOrExpired { .. }) => {}
        other => panic!("Erreur inattendue: {:?}", other),
    }

    mock.stop();
}

#[tokio::test]
async fn test_mock_dataconnect_point_not_found_scenario() {
    let mock = MockDataConnectServer::start(MockScenario::BusinessPointNotFound).await;
    let config = build_dc_config(&mock);
    let client = DataConnectClient::new(config).expect("Client DC valide");

    let prm = PointId::new("01234567890123").unwrap();
    let to = Utc::now();
    let from = to - Duration::days(1);

    let err = client
        .fetch_measurements(prm, from, to, FlowDirection::Consumption)
        .await
        .unwrap_err();

    match err {
        EnedisError::Business(SgeBusinessError::PointNotFound { .. }) => {}
        other => panic!("Erreur inattendue: {:?}", other),
    }

    mock.stop();
}

#[tokio::test]
async fn test_mock_dataconnect_quota_exceeded_scenario() {
    let mock = MockDataConnectServer::start(MockScenario::QuotaExceeded).await;
    let config = build_dc_config(&mock);
    let client = DataConnectClient::new(config).expect("Client DC valide");

    let prm = PointId::new("01234567890123").unwrap();
    let to = Utc::now();
    let from = to - Duration::days(1);

    let err = client
        .fetch_measurements(prm, from, to, FlowDirection::Consumption)
        .await
        .unwrap_err();

    match err {
        EnedisError::Business(SgeBusinessError::QuotaExceeded { .. }) => {}
        other => panic!("Erreur inattendue: {:?}", other),
    }

    mock.stop();
}
