#![cfg(feature = "mqtt")]

use enedis_rs::models::{FlowDirection, PointId};
use enedis_rs::mqtt::{
    diagnostic_sensors_discovery, energy_sensor_discovery, EnedisPrmState, MqttPublisher,
    MqttPublisherConfig,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

/// Test unitaire validant la stricte conformité du JSON Home Assistant Discovery généré
#[test]
fn test_home_assistant_discovery_json_compliance() {
    let prm = PointId::new("01234567890123").unwrap();

    // 1. Capteur de Consommation
    let (topic_cons, config_cons) =
        energy_sensor_discovery(prm, FlowDirection::Consumption, "enedis", "homeassistant");

    assert_eq!(
        topic_cons,
        "homeassistant/sensor/enedis_01234567890123_consumption/config"
    );
    assert_eq!(config_cons.name, "Enedis 01234567890123 Consommation");
    assert_eq!(config_cons.state_topic, "enedis/01234567890123/state");
    assert_eq!(
        config_cons.value_template,
        "{{ value_json.consumption_kwh }}"
    );
    assert_eq!(config_cons.device_class.as_deref(), Some("energy"));
    assert_eq!(config_cons.state_class.as_deref(), Some("total_increasing"));
    assert_eq!(config_cons.unit_of_measurement.as_deref(), Some("kWh"));
    assert_eq!(config_cons.unique_id, "enedis_01234567890123_consumption");
    assert_eq!(
        config_cons.device.identifiers,
        vec!["enedis_01234567890123"]
    );
    assert_eq!(config_cons.device.manufacturer, "Enedis");
    assert_eq!(config_cons.device.model, "Compteur Linky");
    assert_eq!(config_cons.device.name, "Compteur 01234567890123");

    // Vérification de la charge utile JSON brute
    let json_cons = config_cons.to_json().expect("Sérialisation JSON valide");
    let val_cons: serde_json::Value =
        serde_json::from_str(&json_cons).expect("Parsing JSON valide");

    assert_eq!(val_cons["name"], "Enedis 01234567890123 Consommation");
    assert_eq!(val_cons["state_topic"], "enedis/01234567890123/state");
    assert_eq!(
        val_cons["value_template"],
        "{{ value_json.consumption_kwh }}"
    );
    assert_eq!(val_cons["device_class"], "energy");
    assert_eq!(val_cons["state_class"], "total_increasing");
    assert_eq!(val_cons["unit_of_measurement"], "kWh");
    assert_eq!(val_cons["unique_id"], "enedis_01234567890123_consumption");
    assert_eq!(
        val_cons["device"]["identifiers"][0],
        "enedis_01234567890123"
    );
    assert_eq!(val_cons["device"]["manufacturer"], "Enedis");
    assert_eq!(val_cons["device"]["model"], "Compteur Linky");
    assert_eq!(val_cons["device"]["name"], "Compteur 01234567890123");

    // 2. Capteur de Production
    let (topic_prod, config_prod) =
        energy_sensor_discovery(prm, FlowDirection::Production, "enedis", "homeassistant");

    assert_eq!(
        topic_prod,
        "homeassistant/sensor/enedis_01234567890123_production/config"
    );
    assert_eq!(config_prod.name, "Enedis 01234567890123 Production");
    assert_eq!(config_prod.state_topic, "enedis/01234567890123/state");
    assert_eq!(
        config_prod.value_template,
        "{{ value_json.production_kwh }}"
    );
    assert_eq!(config_prod.device_class.as_deref(), Some("energy"));
    assert_eq!(config_prod.state_class.as_deref(), Some("total_increasing"));
    assert_eq!(config_prod.unit_of_measurement.as_deref(), Some("kWh"));
    assert_eq!(config_prod.unique_id, "enedis_01234567890123_production");

    // 3. Capteurs de Diagnostic
    let diagnostics = diagnostic_sensors_discovery(prm, "enedis", "homeassistant");
    assert_eq!(diagnostics.len(), 3);

    // Qualité de la dernière mesure
    let (topic_qual, config_qual) = &diagnostics[0];
    assert_eq!(
        topic_qual,
        "homeassistant/sensor/enedis_01234567890123_quality/config"
    );
    assert_eq!(config_qual.value_template, "{{ value_json.quality }}");
    assert_eq!(config_qual.entity_category.as_deref(), Some("diagnostic"));

    // Date du dernier relevé
    let (topic_date, config_date) = &diagnostics[1];
    assert_eq!(
        topic_date,
        "homeassistant/sensor/enedis_01234567890123_last_reading/config"
    );
    assert_eq!(config_date.value_template, "{{ value_json.last_reading }}");
    assert_eq!(config_date.device_class.as_deref(), Some("timestamp"));
    assert_eq!(config_date.entity_category.as_deref(), Some("diagnostic"));

    // Statut de synchronisation
    let (topic_sync, config_sync) = &diagnostics[2];
    assert_eq!(
        topic_sync,
        "homeassistant/sensor/enedis_01234567890123_sync_status/config"
    );
    assert_eq!(config_sync.value_template, "{{ value_json.sync_status }}");
    assert_eq!(config_sync.entity_category.as_deref(), Some("diagnostic"));

    // 4. Capteurs de Puissance
    let (topic_pwr, config_pwr) =
        enedis_rs::mqtt::power_sensor_discovery(prm, "enedis", "homeassistant");
    assert_eq!(
        topic_pwr,
        "homeassistant/sensor/enedis_01234567890123_power/config"
    );
    assert_eq!(config_pwr.device_class.as_deref(), Some("power"));
    assert_eq!(config_pwr.unit_of_measurement.as_deref(), Some("W"));
    assert_eq!(
        config_pwr.availability_topic.as_deref(),
        Some("enedis/status")
    );

    let (topic_max_pwr, config_max_pwr) =
        enedis_rs::mqtt::max_power_sensor_discovery(prm, "enedis", "homeassistant");
    assert_eq!(
        topic_max_pwr,
        "homeassistant/sensor/enedis_01234567890123_max_power/config"
    );
    assert_eq!(
        config_max_pwr.device_class.as_deref(),
        Some("apparent_power")
    );
    assert_eq!(config_max_pwr.unit_of_measurement.as_deref(), Some("kVA"));

    let (topic_sub, config_sub) =
        enedis_rs::mqtt::subscribed_power_sensor_discovery(prm, "enedis", "homeassistant");
    assert_eq!(
        topic_sub,
        "homeassistant/sensor/enedis_01234567890123_subscribed_power/config"
    );
    assert_eq!(config_sub.entity_category.as_deref(), Some("diagnostic"));

    // 5. Ensemble complet (12 capteurs au total avec solaire, spot et baseload)
    let all = enedis_rs::mqtt::all_discovery_configs(prm, "enedis", "homeassistant");
    assert_eq!(all.len(), 12);
    assert!(config_cons.device.sw_version.is_some());
    assert_eq!(
        config_cons.availability_topic.as_deref(),
        Some("enedis/status")
    );
}

/// Message MQTT reçu par le mock broker
#[derive(Debug, Clone)]
struct ReceivedMessage {
    topic: String,
    payload: String,
}

/// Serveur Mock MQTT asynchrone pour simuler le broker et les déconnexions/reconnexions
struct MockMqttBroker {
    port: u16,
    received_messages: Arc<Mutex<Vec<ReceivedMessage>>>,
    connection_count: Arc<AtomicUsize>,
}

impl MockMqttBroker {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let received_messages = Arc::new(Mutex::new(Vec::new()));
        let connection_count = Arc::new(AtomicUsize::new(0));

        let msgs = Arc::clone(&received_messages);
        let conns = Arc::clone(&connection_count);

        tokio::spawn(async move {
            loop {
                let (socket, _) = match listener.accept().await {
                    Ok(res) => res,
                    Err(_) => break,
                };

                let conn_idx = conns.fetch_add(1, Ordering::SeqCst);
                let msgs_clone = Arc::clone(&msgs);

                tokio::spawn(async move {
                    Self::handle_connection(socket, conn_idx, msgs_clone).await;
                });
            }
        });

        Self {
            port,
            received_messages,
            connection_count,
        }
    }

    async fn handle_connection(
        mut socket: TcpStream,
        conn_idx: usize,
        messages: Arc<Mutex<Vec<ReceivedMessage>>>,
    ) {
        loop {
            let packet = match Self::read_packet(&mut socket).await {
                Ok(Some(p)) => p,
                _ => break,
            };

            let packet_type = packet.0;
            let payload = packet.1;

            if packet_type == 0x10 {
                // CONNECT -> Répondre CONNACK
                let _ = socket.write_all(&[0x20, 0x02, 0x00, 0x00]).await;
            } else if (packet_type & 0xF0) == 0x30 {
                // PUBLISH
                if payload.len() >= 2 {
                    let topic_len = u16::from_be_bytes([payload[0], payload[1]]) as usize;
                    if payload.len() >= 2 + topic_len {
                        let topic = String::from_utf8_lossy(&payload[2..2 + topic_len]).to_string();
                        let qos = (packet_type & 0x06) >> 1;
                        let mut offset = 2 + topic_len;

                        if qos > 0 && payload.len() >= offset + 2 {
                            let pid = [payload[offset], payload[offset + 1]];
                            offset += 2;
                            // Envoi PUBACK pour QoS 1
                            let _ = socket.write_all(&[0x40, 0x02, pid[0], pid[1]]).await;
                        }

                        let body = String::from_utf8_lossy(&payload[offset..]).to_string();
                        messages.lock().await.push(ReceivedMessage {
                            topic,
                            payload: body,
                        });

                        // Si c'est la première connexion et qu'on a reçu un message, on coupe la connexion pour tester la reconnexion
                        if conn_idx == 0 {
                            break;
                        }
                    }
                }
            } else if packet_type == 0xC0 {
                // PINGREQ -> Répondre PINGRESP
                let _ = socket.write_all(&[0xD0, 0x00]).await;
            }
        }
    }

    async fn read_packet(socket: &mut TcpStream) -> std::io::Result<Option<(u8, Vec<u8>)>> {
        let mut header = [0u8; 1];
        match socket.read_exact(&mut header).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }

        let packet_type = header[0];
        let mut rem_len: usize = 0;
        let mut multiplier: usize = 1;

        loop {
            let mut b = [0u8; 1];
            socket.read_exact(&mut b).await?;
            let byte = b[0];
            rem_len += ((byte & 0x7F) as usize) * multiplier;
            multiplier *= 128;
            if (byte & 0x80) == 0 {
                break;
            }
        }

        let mut payload = vec![0u8; rem_len];
        if rem_len > 0 {
            socket.read_exact(&mut payload).await?;
        }

        Ok(Some((packet_type, payload)))
    }
}

/// Test d'intégration simulant la publication et vérifiant la gestion des reconnexions MQTT
#[tokio::test]
async fn test_mqtt_publication_and_reconnection_recovery() {
    let broker = MockMqttBroker::start().await;
    let prm = PointId::new("01234567890123").unwrap();

    let config = MqttPublisherConfig {
        broker_url: format!("mqtt://127.0.0.1:{}", broker.port),
        topic_prefix: "enedis".to_string(),
        discovery_prefix: "homeassistant".to_string(),
        username: None,
        password: None,
        client_id: "test-reconnect-client".to_string(),
        keep_alive_secs: 5,
    };

    let (publisher, _bg) = MqttPublisher::start(config).expect("Démarrage publisher");

    // 1. Première publication (déclenche la connexion initiale)
    let state_1 = EnedisPrmState {
        prm: prm.to_string(),
        consumption_kwh: Some(10.5),
        production_kwh: None,
        last_power_w: Some(1250.0),
        max_power_kva: Some(4.8),
        subscribed_power_kva: Some(6),
        quality: Some("VALIDATED".to_string()),
        last_reading: Some("2026-09-28T10:00:00Z".to_string()),
        sync_status: "OK".to_string(),
        baseload_w: None,
        solar_autoconsumption_percent: None,
        solar_autoproduction_percent: None,
        spot_price_eur_mwh: None,
        flex_shed_power_w: None,
        flex_active_order: None,
    };

    publisher
        .publish_state(prm, &state_1)
        .await
        .expect("Publication 1 réussie");

    // 2. Attendre que le mock broker reçoive le 1er message et coupe la première connexion
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Vérifier que le 1er message est bien arrivé
    {
        let msgs = broker.received_messages.lock().await;
        assert!(
            !msgs.is_empty(),
            "Au moins un message doit avoir été reçu avant la coupure"
        );
        assert_eq!(msgs[0].topic, "enedis/01234567890123/state");
    }

    // 3. Deuxième publication après la coupure de connexion
    // Le client rumqttc doit détecter la coupure, se reconnecter au broker et publier le message
    let state_2 = EnedisPrmState {
        prm: prm.to_string(),
        consumption_kwh: Some(15.2),
        production_kwh: Some(2.1),
        last_power_w: Some(2200.0),
        max_power_kva: Some(5.2),
        subscribed_power_kva: Some(6),
        quality: Some("VALIDATED".to_string()),
        last_reading: Some("2026-09-28T11:00:00Z".to_string()),
        sync_status: "OK".to_string(),
        baseload_w: None,
        solar_autoconsumption_percent: None,
        solar_autoproduction_percent: None,
        spot_price_eur_mwh: None,
        flex_shed_power_w: None,
        flex_active_order: None,
    };

    // Publier après la déconnexion
    publisher
        .publish_state(prm, &state_2)
        .await
        .expect("Publication 2 acceptée par le client");

    // Laisser le temps à la reconnexion automatique rumqttc d'aboutir
    let mut reconnected = false;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if broker.connection_count.load(Ordering::SeqCst) >= 2 {
            let msgs = broker.received_messages.lock().await;
            if msgs.iter().any(|m| m.payload.contains("15.2")) {
                reconnected = true;
                break;
            }
        }
    }

    assert!(
        reconnected,
        "Le client MQTT doit s'être reconnecté automatiquement après coupure et avoir délivré le message 2"
    );
    assert!(
        broker.connection_count.load(Ordering::SeqCst) >= 2,
        "Le broker doit avoir enregistré au moins 2 connexions distinctes (initiale + reconnexion)"
    );
}

/// Test d'intégration complet de l'agent de collecte avec persistance SQLite et publication MQTT Home Assistant
#[cfg(all(feature = "agent", feature = "mock-sge", feature = "storage-sqlite"))]
#[tokio::test]
async fn test_agent_sync_triggers_mqtt_publication() {
    use chrono::{Duration as ChronoDuration, Utc};
    use enedis_rs::agent::{CollectorConfig, CollectorDaemon, SgeRateLimiter, ShutdownSignal};
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use enedis_rs::storage::{SqliteStorage, StorageBackend, SyncState};
    use enedis_rs::{SgeClient, SgeClientConfig};

    // 1. Initialisation mock SGE
    let sge_mock = MockSgeServer::start(MockScenario::Success).await;

    // 2. Initialisation mock MQTT broker
    let mqtt_broker = MockMqttBroker::start().await;

    // 3. Base SQLite en mémoire
    let storage: Arc<dyn StorageBackend> =
        Arc::new(SqliteStorage::connect("sqlite::memory:").await.unwrap());
    let prm = PointId::new("12345678901234").unwrap();

    let initial_ts = Utc::now() - ChronoDuration::days(2);
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

    // 4. Client SGE
    let client_config = SgeClientConfig {
        endpoint_url: sge_mock.endpoint_url(),
        ..Default::default()
    };
    let client = SgeClient::new(client_config).unwrap();

    // 5. Publisher MQTT
    let mqtt_config = MqttPublisherConfig {
        broker_url: format!("mqtt://127.0.0.1:{}", mqtt_broker.port),
        topic_prefix: "enedis".to_string(),
        discovery_prefix: "homeassistant".to_string(),
        username: None,
        password: None,
        client_id: "agent-test-client".to_string(),
        keep_alive_secs: 10,
    };
    let (publisher, _bg) = MqttPublisher::start(mqtt_config).unwrap();

    // 6. Collector Daemon configuré avec MQTT
    let daemon = CollectorDaemon::new(
        client,
        Arc::clone(&storage),
        SgeRateLimiter::new(100, Duration::from_millis(1)),
        CollectorConfig::default(),
        ShutdownSignal::new(),
    )
    .with_mqtt_publisher(Arc::new(publisher));

    // 7. Exécution de la synchronisation du point
    daemon.sync_point(prm).await.expect("Sync point réussi");

    // 8. Attendre que la tâche de publication MQTT non-bloquante s'exécute
    let mut discovery_received = false;
    let mut state_received = false;

    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let msgs = mqtt_broker.received_messages.lock().await;

        for m in msgs.iter() {
            if m.topic
                .starts_with("homeassistant/sensor/enedis_12345678901234_consumption/config")
            {
                discovery_received = true;
            }
            if m.topic == "enedis/12345678901234/state" {
                state_received = true;
            }
        }

        if discovery_received && state_received {
            break;
        }
    }

    assert!(
        discovery_received,
        "Home Assistant Discovery doit avoir été publié"
    );
    assert!(
        state_received,
        "L'état consolidé enedis/{prm}/state doit avoir été publié"
    );

    sge_mock.stop();
}
