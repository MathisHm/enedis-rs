#![cfg(all(feature = "mock-sge", feature = "client", feature = "storage-sqlite"))]
#[tokio::test]
async fn test_e2e_fetch_and_store_measurements() {
    use chrono::{TimeZone, Utc};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::storage::{SqliteStorage, StorageBackend};
    use enedis_rs::{FlowDirection, MeasurementQuality, PointId, SgeClient, SgeClientConfig};
    use rust_decimal::Decimal;
    use std::str::FromStr;

    // 1. Démarrage du serveur Mock SGE en mode succès nominal
    let mock = MockSgeServer::start(MockScenario::Success).await;

    // 2. Initialisation du stockage SQLite en mémoire
    let storage = SqliteStorage::connect("sqlite::memory:").await.unwrap();

    // 3. Configuration et instanciation du client SGE ciblant le mock
    let config = SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(config).expect("Client SGE initialisé avec succès");

    let prm = PointId::new("12345678901234").unwrap();
    let from = Utc.with_ymd_and_hms(2026, 9, 28, 8, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 9, 28, 9, 0, 0).unwrap();

    // 4. Collecte depuis le mock SGE
    let measurements = client
        .fetch_measurements(prm, from, to, FlowDirection::Consumption)
        .await
        .expect("Collecte SOAP réussie");

    assert_eq!(measurements.len(), 2);
    assert_eq!(mock.total_requests(), 1);

    // 5. Persistance en base avec UPSERT conditionnel
    let stats = storage
        .upsert_measurements(&measurements)
        .await
        .expect("Persistance SQL réussie");
    assert_eq!(stats.affected, 2);

    // 6. Vérification de la lecture depuis la base
    let stored = storage
        .get_measurements(prm, from, to, Some(FlowDirection::Consumption))
        .await
        .unwrap();

    assert_eq!(stored.len(), 2);
    assert_eq!(stored[0].value, Decimal::from_str("1.2500").unwrap());
    assert_eq!(stored[0].quality, MeasurementQuality::Estimated);
    assert_eq!(stored[1].value, Decimal::from_str("1.4100").unwrap());
    assert_eq!(stored[1].quality, MeasurementQuality::Validated);

    mock.stop();
}

#[cfg(all(feature = "mock-sge", feature = "client"))]
#[tokio::test]
async fn test_e2e_soap_fault_handling() {
    use chrono::{TimeZone, Utc};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::{EnedisError, FlowDirection, PointId, SgeClient, SgeClientConfig};

    let mock = MockSgeServer::start(MockScenario::SoapFault {
        code: "soapenv:Server".to_string(),
        message: "Serveur SGE en maintenance planifiee".to_string(),
    })
    .await;

    let config = SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(config).unwrap();

    let prm = PointId::new("12345678901234").unwrap();
    let from = Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 9, 28, 1, 0, 0).unwrap();

    let res = client
        .fetch_measurements(prm, from, to, FlowDirection::Consumption)
        .await;

    assert!(res.is_err());
    match res.unwrap_err() {
        EnedisError::Soap(fault) => {
            assert_eq!(fault.code, "soapenv:Server");
            assert_eq!(fault.message, "Serveur SGE en maintenance planifiee");
        }
        other => panic!("Erreur inattendue reçue: {:?}", other),
    }

    mock.stop();
}

#[cfg(all(feature = "mock-sge", feature = "client"))]
#[tokio::test]
async fn test_e2e_retry_with_transient_failure_recovery() {
    use chrono::{TimeZone, Utc};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::{FlowDirection, PointId, ResilienceAction, SgeClient, SgeClientConfig};
    use std::time::Duration;

    // Échoue 2 fois avec 503, puis réussit à la 3ème tentative
    let mock =
        MockSgeServer::start(MockScenario::TransientFailureThenSuccess { fail_count: 2 }).await;

    let config = SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(config).unwrap();

    let prm = PointId::new("12345678901234").unwrap();
    let from = Utc.with_ymd_and_hms(2026, 9, 28, 8, 0, 0).unwrap();
    let to = Utc.with_ymd_and_hms(2026, 9, 28, 9, 0, 0).unwrap();

    // Boucle de retry supervisée par `classify()`
    let mut attempts = 0;
    let max_attempts = 5;
    let mut final_measurements = None;

    while attempts < max_attempts {
        attempts += 1;
        match client
            .fetch_measurements(prm, from, to, FlowDirection::Consumption)
            .await
        {
            Ok(m) => {
                final_measurements = Some(m);
                break;
            }
            Err(err) => {
                let action = err.classify();
                match action {
                    ResilienceAction::RetryAfter(_) => {
                        // Dans un test, on utilise un délai minimal pour ne pas ralentir la CI
                        tokio::time::sleep(Duration::from_millis(10)).await;
                    }
                    _ => panic!(
                        "Action inattendue pour une erreur transitoire: {:?}",
                        action
                    ),
                }
            }
        }
    }

    assert_eq!(
        attempts, 3,
        "Doit avoir réussi précisément à la 3ème tentative"
    );
    assert!(final_measurements.is_some());
    assert_eq!(final_measurements.unwrap().len(), 1);

    mock.stop();
}
