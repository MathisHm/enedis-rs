#![cfg(all(feature = "agent", feature = "mock-sge", feature = "storage-sqlite"))]
#[tokio::test]
async fn test_agent_sync_cycle_nominal() {
    use chrono::{Duration as ChronoDuration, Utc};
    use enedis_rs::agent::{CollectorConfig, CollectorDaemon, SgeRateLimiter, ShutdownSignal};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::storage::{SqliteStorage, StorageBackend, SyncState};
    use enedis_rs::{FlowDirection, PointId, SgeClient, SgeClientConfig};
    use std::sync::Arc;
    use std::time::Duration;

    // 1. Initialisation du mock SGE en succès
    let mock = MockSgeServer::start(MockScenario::Success).await;

    // 2. Base SQLite
    let storage: Arc<dyn StorageBackend> =
        Arc::new(SqliteStorage::connect("sqlite::memory:").await.unwrap());
    let prm = PointId::new("12345678901234").unwrap();

    // 3. État initial : dernière synchro il y a 2 jours
    let two_days_ago = Utc::now() - ChronoDuration::days(2);
    storage
        .update_sync_state(&SyncState {
            point_id: prm,
            direction: FlowDirection::Consumption,
            last_synced_timestamp: two_days_ago,
            last_sync_attempt: two_days_ago,
            sync_status: "OK".to_string(),
        })
        .await
        .unwrap();

    // 4. Instanciation client et daemon
    let client_config = SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(client_config).unwrap();

    let rate_limiter = SgeRateLimiter::new(100, Duration::from_millis(1));
    let collector_config = CollectorConfig {
        chunk_size_days: 7,
        initial_lookback_days: 7,
        cycle_interval: Duration::from_secs(3600),
        max_retries: 2,
        concurrency: 2,
        ..CollectorConfig::default()
    };
    let signal = ShutdownSignal::new();

    let daemon = CollectorDaemon::new(
        client,
        Arc::clone(&storage),
        rate_limiter,
        collector_config,
        signal,
    );

    // 5. Exécution d'une passe de synchronisation
    daemon
        .sync_point(prm)
        .await
        .expect("Sync point nominal réussi");

    // 6. Vérifications
    let state = storage
        .get_sync_state(prm, FlowDirection::Consumption)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(state.sync_status, "OK");
    assert!(
        state.last_synced_timestamp > two_days_ago,
        "L'horodatage de synchro doit avoir progressé"
    );

    // Vérification des mesures insérées
    let measurements = storage
        .get_measurements(
            prm,
            two_days_ago,
            Utc::now(),
            Some(FlowDirection::Consumption),
        )
        .await
        .unwrap();
    assert!(
        !measurements.is_empty(),
        "Les mesures doivent être enregistrées en base"
    );

    mock.stop();
}

#[cfg(all(feature = "agent", feature = "mock-sge", feature = "storage-sqlite"))]
#[tokio::test]
async fn test_agent_quarantine_on_consent_expiration() {
    use chrono::{Duration as ChronoDuration, Utc};
    use enedis_rs::agent::{CollectorConfig, CollectorDaemon, SgeRateLimiter, ShutdownSignal};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::storage::{SqliteStorage, StorageBackend, SyncState};
    use enedis_rs::{FlowDirection, PointId, SgeClient, SgeClientConfig};
    use std::sync::Arc;
    use std::time::Duration;

    // Mock renvoyant une erreur de consentement SGE
    let mock = MockSgeServer::start(MockScenario::BusinessConsentExpired).await;

    let storage: Arc<dyn StorageBackend> =
        Arc::new(SqliteStorage::connect("sqlite::memory:").await.unwrap());
    let prm = PointId::new("12345678901234").unwrap();

    let initial_ts = Utc::now() - ChronoDuration::days(1);
    storage
        .update_sync_state(&SyncState {
            point_id: prm,
            direction: FlowDirection::Consumption,
            last_synced_timestamp: initial_ts,
            last_sync_attempt: initial_ts,
            sync_status: "OK".to_string(),
        })
        .await
        .unwrap();

    let client_config = SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(client_config).unwrap();

    let daemon = CollectorDaemon::new(
        client,
        Arc::clone(&storage),
        SgeRateLimiter::new(100, Duration::from_millis(1)),
        CollectorConfig::default(),
        ShutdownSignal::new(),
    );

    // La synchronisation ne doit PAS paniquer ni faire crasher le daemon
    let res = daemon.sync_point(prm).await;
    assert!(
        res.is_ok(),
        "Le collecteur doit absorber l'erreur métier sans crasher"
    );

    // L'état en base doit passer en quarantaine ERROR_CONSENT_EXPIRED
    let state = storage
        .get_sync_state(prm, FlowDirection::Consumption)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(state.sync_status, "ERROR_CONSENT_EXPIRED");

    mock.stop();
}

#[cfg(all(feature = "agent", feature = "mock-sge", feature = "storage-sqlite"))]
#[tokio::test]
async fn test_agent_concurrent_sync_multi_points() {
    use chrono::{Duration as ChronoDuration, Utc};
    use enedis_rs::agent::{CollectorConfig, CollectorDaemon, SgeRateLimiter, ShutdownSignal};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::storage::{SqliteStorage, StorageBackend, SyncState};
    use enedis_rs::{FlowDirection, PointId, SgeClient, SgeClientConfig};
    use std::sync::Arc;
    use std::time::Duration;

    let mock = MockSgeServer::start(MockScenario::Success).await;

    let storage: Arc<dyn StorageBackend> =
        Arc::new(SqliteStorage::connect("sqlite::memory:").await.unwrap());
    let prms = vec![
        PointId::new("11111111111111").unwrap(),
        PointId::new("22222222222222").unwrap(),
        PointId::new("33333333333333").unwrap(),
    ];

    let past_ts = Utc::now() - ChronoDuration::days(2);
    for prm in &prms {
        storage
            .update_sync_state(&SyncState {
                point_id: *prm,
                direction: FlowDirection::Consumption,
                last_synced_timestamp: past_ts,
                last_sync_attempt: past_ts,
                sync_status: "OK".to_string(),
            })
            .await
            .unwrap();
    }

    let client_config = SgeClientConfig {
        endpoint_url: mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(client_config).unwrap();

    let rate_limiter = SgeRateLimiter::new(100, Duration::from_millis(1));
    let collector_config = CollectorConfig {
        chunk_size_days: 7,
        initial_lookback_days: 7,
        cycle_interval: Duration::from_millis(100),
        max_retries: 2,
        concurrency: 3,
        ..CollectorConfig::default()
    };
    let signal = ShutdownSignal::new();
    let signal_clone = signal.clone();

    let daemon = CollectorDaemon::new(
        client,
        Arc::clone(&storage),
        rate_limiter,
        collector_config,
        signal,
    );

    let daemon_handle = tokio::spawn(async move { daemon.run().await });

    let mut all_synced = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut count = 0;
        for prm in &prms {
            if let Ok(Some(s)) = storage
                .get_sync_state(*prm, FlowDirection::Consumption)
                .await
            {
                if s.last_synced_timestamp > past_ts {
                    count += 1;
                }
            }
        }
        if count == prms.len() {
            all_synced = true;
            break;
        }
    }

    signal_clone.cancel();
    let _ = daemon_handle.await;

    assert!(
        all_synced,
        "Tous les PRMs doivent être synchronisés en mode concurrent"
    );
    mock.stop();
}

#[cfg(all(feature = "mock-sge", feature = "agent", feature = "storage-sqlite"))]
#[tokio::test]
async fn test_daemon_with_dataconnect_provider() {
    use enedis_rs::agent::{CollectorConfig, CollectorDaemon, SgeRateLimiter, ShutdownSignal};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::storage::{SqliteStorage, StorageBackend, SyncState};
    use enedis_rs::{DataConnectClient, DataConnectConfig, FlowDirection, PointId};
    use secrecy::SecretString;
    use std::sync::Arc;
    use std::time::Duration;

    let mock = MockSgeServer::start(MockScenario::Success).await;

    let storage: Arc<dyn StorageBackend> =
        Arc::new(SqliteStorage::connect("sqlite::memory:").await.unwrap());
    let prm = PointId::new("01234567890123").unwrap();

    let past_ts = chrono::Utc::now() - chrono::Duration::days(10);
    storage
        .update_sync_state(&SyncState {
            point_id: prm,
            direction: FlowDirection::Consumption,
            last_synced_timestamp: past_ts,
            last_sync_attempt: past_ts,
            sync_status: "OK".to_string(),
        })
        .await
        .unwrap();

    let dc_config = DataConnectConfig {
        base_url: mock.base_url(),
        token_url: mock.token_url(),
        direct_token: Some(SecretString::new("mock-direct-token".to_string())),
        ..Default::default()
    };
    let client = DataConnectClient::new(dc_config).unwrap();

    let rate_limiter = SgeRateLimiter::new(100, Duration::from_millis(1));
    let collector_config = CollectorConfig {
        chunk_size_days: 7,
        initial_lookback_days: 7,
        cycle_interval: Duration::from_secs(3600),
        max_retries: 2,
        concurrency: 1,
        ..CollectorConfig::default()
    };
    let signal = ShutdownSignal::new();

    // Vérifie que CollectorDaemon accepte directement DataConnectClient grâce au trait polymorphe IntoProvider
    let daemon = CollectorDaemon::new(
        client,
        Arc::clone(&storage),
        rate_limiter,
        collector_config,
        signal,
    );

    daemon.sync_point(prm).await.unwrap();
    let measurements = storage
        .get_measurements(prm, past_ts, chrono::Utc::now(), None)
        .await
        .unwrap();
    assert!(
        !measurements.is_empty(),
        "DataConnect doit alimenter le daemon avec succès"
    );

    mock.stop();
}
