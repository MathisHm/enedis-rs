use crate::error::EnedisError;
use crate::models::{FlowDirection, PointId};
use crate::mqtt::discovery::all_discovery_configs;
use crate::storage::StorageBackend;
use rumqttc::{AsyncClient, MqttOptions, QoS};
use rust_decimal::prelude::ToPrimitive;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// État consolidé d'un PRM publié sur le topic `enedis/{prm}/state`
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnedisPrmState {
    /// Identifiant du PRM (14 chiffres)
    pub prm: String,
    /// Consommation cumulée totale en kWh pour Home Assistant Energy Dashboard
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumption_kwh: Option<f64>,
    /// Production cumulée totale en kWh pour Home Assistant Energy Dashboard
    #[serde(skip_serializing_if = "Option::is_none")]
    pub production_kwh: Option<f64>,
    /// Puissance active mesurée ou calculée au dernier pas en Watts (W)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_power_w: Option<f64>,
    /// Pointe maximale quotidienne de puissance relevée en kVA
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_power_kva: Option<f64>,
    /// Puissance souscrite contractuelle en kVA
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscribed_power_kva: Option<u32>,
    /// Qualité de la dernière mesure relevée (VALIDATED, CORRECTED, ESTIMATED)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    /// Horodatage de la dernière mesure reçue (format ISO 8601 / RFC 3339)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_reading: Option<String>,
    /// Statut de synchronisation de la collecte (OK, ERROR_CONSENT, etc.)
    pub sync_status: String,
    /// Puissance talon de veille résiduelle permanente en Watts (W)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseload_w: Option<f64>,
    /// Taux d'autoconsommation photovoltaïque en %
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solar_autoconsumption_percent: Option<f64>,
    /// Taux d'autoproduction photovoltaïque en %
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub solar_autoproduction_percent: Option<f64>,
    /// Prix du marché spot Day-Ahead (EPEX SPOT) en €/MWh
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spot_price_eur_mwh: Option<f64>,
    /// Puissance effacée/délestée active en Watts (W)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flex_shed_power_w: Option<f64>,
    /// Résumé du dernier ordre de flexibilité actif
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flex_active_order: Option<String>,
}

impl EnedisPrmState {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

#[derive(Clone)]
pub struct MqttPublisherConfig {
    /// URL du broker (ex: `mqtt://localhost:1883` ou `192.168.1.100:1883`)
    pub broker_url: String,
    /// Préfixe racine des topics d'état (défaut: `enedis`)
    pub topic_prefix: String,
    /// Préfixe du protocole Home Assistant Discovery (défaut: `homeassistant`)
    pub discovery_prefix: String,
    /// Nom d'utilisateur MQTT optionnel
    pub username: Option<String>,
    /// Mot de passe MQTT optionnel
    pub password: Option<SecretString>,
    /// Identifiant client MQTT unique
    pub client_id: String,
    /// Durée de keep-alive en secondes (défaut: 30s)
    pub keep_alive_secs: u64,
}

impl fmt::Debug for MqttPublisherConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MqttPublisherConfig")
            .field("broker_url", &self.broker_url)
            .field("topic_prefix", &self.topic_prefix)
            .field("discovery_prefix", &self.discovery_prefix)
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("client_id", &self.client_id)
            .field("keep_alive_secs", &self.keep_alive_secs)
            .finish()
    }
}

impl Default for MqttPublisherConfig {
    fn default() -> Self {
        Self {
            broker_url: "mqtt://localhost:1883".to_string(),
            topic_prefix: "enedis".to_string(),
            discovery_prefix: "homeassistant".to_string(),
            username: None,
            password: None,
            client_id: format!("enedis-rs-{}", std::process::id()),
            keep_alive_secs: 30,
        }
    }
}

/// Client de publication MQTT compatible Home Assistant Auto-Discovery
#[derive(Clone)]
pub struct MqttPublisher {
    client: AsyncClient,
    config: MqttPublisherConfig,
}

/// Informations de connexion extraites de l'URL du broker MQTT
#[derive(Debug, Clone)]
pub struct ParsedBrokerUrl {
    pub host: String,
    pub port: u16,
    pub is_tls: bool,
    pub username: Option<String>,
    pub password: Option<SecretString>,
}

impl MqttPublisher {
    /// Crée un publisher à partir d'un client `rumqttc::AsyncClient` existant
    pub fn from_client(client: AsyncClient, config: MqttPublisherConfig) -> Self {
        Self { client, config }
    }

    /// Analyse l'URL du broker MQTT pour en extraire l'hôte, le port, le mode TLS et d'éventuels identifiants
    pub fn parse_broker_url_details(url: &str) -> Result<ParsedBrokerUrl, EnedisError> {
        let (is_tls, stripped) = if let Some(s) = url.strip_prefix("mqtts://") {
            (true, s)
        } else if let Some(s) = url.strip_prefix("ssl://") {
            (true, s)
        } else if let Some(s) = url.strip_prefix("tls://") {
            (true, s)
        } else {
            (false, url.strip_prefix("mqtt://").unwrap_or(url))
        };

        let (credentials, host_port) = match stripped.split_once('@') {
            Some((user_pass, hp)) => {
                let (user, pass) = match user_pass.split_once(':') {
                    Some((u, p)) => (Some(u.to_string()), Some(SecretString::new(p.to_string()))),
                    None => (Some(user_pass.to_string()), None),
                };
                ((user, pass), hp)
            }
            None => ((None, None), stripped),
        };

        let default_port = if is_tls { 8883 } else { 1883 };
        let (host, port) = match host_port.split_once(':') {
            Some((h, p)) => {
                let port_num = p.parse::<u16>().map_err(|e| {
                    EnedisError::Configuration(format!("Port MQTT invalide '{}': {}", p, e))
                })?;
                (h.to_string(), port_num)
            }
            None => (host_port.to_string(), default_port),
        };

        if host.is_empty() {
            return Err(EnedisError::Configuration(
                "Hôte du broker MQTT vide".to_string(),
            ));
        }

        Ok(ParsedBrokerUrl {
            host,
            port,
            is_tls,
            username: credentials.0,
            password: credentials.1,
        })
    }

    /// Analyse l'URL du broker MQTT pour en extraire l'hôte, le port et d'éventuels identifiants
    pub fn parse_broker_url(
        url: &str,
    ) -> Result<(String, u16, Option<String>, Option<SecretString>), EnedisError> {
        let parsed = Self::parse_broker_url_details(url)?;
        Ok((parsed.host, parsed.port, parsed.username, parsed.password))
    }

    /// Démarre le client MQTT asynchrone et lance la tâche de fond de gestion des événements et reconnexions
    pub fn start(
        config: MqttPublisherConfig,
    ) -> Result<(Self, tokio::task::JoinHandle<()>), EnedisError> {
        let parsed = Self::parse_broker_url_details(&config.broker_url)?;

        let mut mqttoptions = MqttOptions::new(&config.client_id, parsed.host, parsed.port);
        if parsed.is_tls {
            mqttoptions
                .set_transport(rumqttc::Transport::Tls(rumqttc::TlsConfiguration::default()));
        }
        mqttoptions.set_keep_alive(Duration::from_secs(config.keep_alive_secs));

        let will_topic = format!("{}/status", config.topic_prefix.trim_end_matches('/'));
        let last_will =
            rumqttc::LastWill::new(will_topic, "offline".as_bytes(), QoS::AtLeastOnce, true);
        mqttoptions.set_last_will(last_will);

        let user = config.username.clone().or(parsed.username);
        let pass = config.password.clone().or(parsed.password);
        if let (Some(u), Some(p)) = (user, pass) {
            mqttoptions.set_credentials(u, p.expose_secret());
        }

        let (client, mut eventloop) = AsyncClient::new(mqttoptions, 100);

        let bg_handle = tokio::spawn(async move {
            loop {
                match eventloop.poll().await {
                    Ok(notification) => {
                        debug!("MQTT notification reçue: {:?}", notification);
                    }
                    Err(rumqttc::ConnectionError::RequestsDone) => {
                        debug!("Client MQTT fermé, arrêt propre de la boucle d'événements.");
                        break;
                    }
                    Err(e) => {
                        warn!("Déconnexion ou erreur MQTT ({:?}), reconnexion automatique en cours...", e);
                        tokio::time::sleep(Duration::from_millis(1000)).await;
                    }
                }
            }
        });

        let publisher = Self { client, config };
        Ok((publisher, bg_handle))
    }

    /// Publie le statut de disponibilité ("online" sur {topic_prefix}/status avec retain: true)
    pub async fn publish_birth(&self) -> Result<(), EnedisError> {
        let topic = format!("{}/status", self.config.topic_prefix.trim_end_matches('/'));
        debug!("Publication statut de disponibilité 'online' sur {}", topic);
        self.client
            .publish(topic, QoS::AtLeastOnce, true, "online".as_bytes())
            .await
            .map_err(|e| {
                EnedisError::Transport(crate::error::TransportError::Network(format!(
                    "Erreur publication MQTT birth message: {}",
                    e
                )))
            })?;
        Ok(())
    }

    /// Publie les configurations Home Assistant Auto-Discovery pour un PRM donné (retained: true)
    pub async fn publish_discovery(&self, point_id: PointId) -> Result<(), EnedisError> {
        // Envoi préalable du message de disponibilité
        let _ = self.publish_birth().await;

        let configs = all_discovery_configs(
            point_id,
            &self.config.topic_prefix,
            &self.config.discovery_prefix,
        );

        for (topic, config) in configs {
            let payload = config.to_json().map_err(|e| {
                EnedisError::Configuration(format!("Erreur de sérialisation JSON: {}", e))
            })?;
            debug!("Publication Home Assistant Discovery sur {}", topic);
            self.client
                .publish(topic, QoS::AtLeastOnce, true, payload.into_bytes())
                .await
                .map_err(|e| {
                    EnedisError::Transport(crate::error::TransportError::Network(format!(
                        "Erreur publication MQTT: {}",
                        e
                    )))
                })?;
        }

        info!(
            "Configurations Home Assistant Discovery publiées pour le PRM {}",
            point_id
        );
        Ok(())
    }

    /// Publie la charge utile d'état consolidé sur `enedis/{prm}/state` (retained: true)
    pub async fn publish_state(
        &self,
        point_id: PointId,
        state: &EnedisPrmState,
    ) -> Result<(), EnedisError> {
        let topic = format!(
            "{}/{}/state",
            self.config.topic_prefix.trim_end_matches('/'),
            point_id
        );
        let payload = state.to_json().map_err(|e| {
            EnedisError::Configuration(format!("Erreur de sérialisation JSON: {}", e))
        })?;

        info!(
            "Publication de l'état consolidé pour {} sur {}",
            point_id, topic
        );
        self.client
            .publish(topic, QoS::AtLeastOnce, true, payload.into_bytes())
            .await
            .map_err(|e| {
                EnedisError::Transport(crate::error::TransportError::Network(format!(
                    "Erreur publication MQTT: {}",
                    e
                )))
            })?;

        Ok(())
    }

    /// Construit l'état consolidé d'un PRM à partir des données présentes en base
    pub async fn build_prm_state(
        storage: &dyn StorageBackend,
        point_id: PointId,
    ) -> Result<EnedisPrmState, EnedisError> {
        let cons_state = storage
            .get_sync_state(point_id, FlowDirection::Consumption)
            .await?;
        let prod_state = storage
            .get_sync_state(point_id, FlowDirection::Production)
            .await?;

        let cons_total = storage
            .get_total_energy_kwh(point_id, FlowDirection::Consumption)
            .await?;
        let prod_total = storage
            .get_total_energy_kwh(point_id, FlowDirection::Production)
            .await?;

        let cons_latest = storage
            .get_latest_measurement(point_id, FlowDirection::Consumption)
            .await?;
        let prod_latest = storage
            .get_latest_measurement(point_id, FlowDirection::Production)
            .await?;

        let quality = cons_latest
            .as_ref()
            .map(|m| m.quality.to_string())
            .or_else(|| prod_latest.as_ref().map(|m| m.quality.to_string()));

        let last_reading = cons_latest
            .as_ref()
            .map(|m| m.timestamp.to_rfc3339())
            .or_else(|| {
                cons_state
                    .as_ref()
                    .map(|s| s.last_synced_timestamp.to_rfc3339())
            })
            .or_else(|| prod_latest.as_ref().map(|m| m.timestamp.to_rfc3339()));

        let sync_status = cons_state
            .as_ref()
            .map(|s| s.sync_status.clone())
            .or_else(|| prod_state.as_ref().map(|s| s.sync_status.clone()))
            .unwrap_or_else(|| "OK".to_string());

        let consumption_kwh = cons_total
            .and_then(|d| d.to_f64())
            .map(|v| (v * 1000.0).round() / 1000.0);

        let production_kwh = prod_total
            .and_then(|d| d.to_f64())
            .map(|v| (v * 1000.0).round() / 1000.0);

        let last_power_w = cons_latest
            .as_ref()
            .and_then(|m| match m.unit {
                crate::models::Unit::Watt => m.value.to_f64(),
                crate::models::Unit::KiloWatt => m.value.to_f64().map(|kw| kw * 1000.0),
                crate::models::Unit::KiloWattHour if m.interval_seconds > 0 => m
                    .value
                    .to_f64()
                    .map(|kwh| (kwh * 3600.0 / m.interval_seconds as f64) * 1000.0),
                crate::models::Unit::WattHour if m.interval_seconds > 0 => m
                    .value
                    .to_f64()
                    .map(|wh| wh * 3600.0 / m.interval_seconds as f64),
                _ => None,
            })
            .map(|w| (w * 10.0).round() / 10.0);

        let baseload_w: Option<f64> = None;
        let (solar_autoconsumption_percent, solar_autoproduction_percent): (
            Option<f64>,
            Option<f64>,
        ) = (None, None);

        let now = chrono::Utc::now();
        let week_ago = now - chrono::Duration::days(7);
        let spot_prices = storage
            .get_spot_prices(week_ago, now)
            .await
            .unwrap_or_default();
        let spot_price_eur_mwh = spot_prices
            .last()
            .and_then(|s| s.price_eur_per_mwh.to_f64());

        Ok(EnedisPrmState {
            prm: point_id.to_string(),
            consumption_kwh,
            production_kwh,
            last_power_w,
            max_power_kva: None,
            subscribed_power_kva: None,
            quality,
            last_reading,
            sync_status,
            baseload_w,
            solar_autoconsumption_percent,
            solar_autoproduction_percent,
            spot_price_eur_mwh,
            flex_shed_power_w: None,
            flex_active_order: None,
        })
    }

    /// Déclenche la publication complète Home Assistant (Discovery + État) pour un PRM
    pub async fn publish_prm_update(
        &self,
        storage: &dyn StorageBackend,
        point_id: PointId,
    ) -> Result<(), EnedisError> {
        // 1. Publication systématique ou mise à jour de la configuration de découverte
        if let Err(err) = self.publish_discovery(point_id).await {
            error!("Erreur publication Discovery pour {}: {}", point_id, err);
        }

        // 2. Récupération des données consolidées
        let state = Self::build_prm_state(storage, point_id).await?;

        // 3. Publication de l'état
        self.publish_state(point_id, &state).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_broker_url_variations() {
        // Standard mqtt://host:port
        let (host, port, user, pass) =
            MqttPublisher::parse_broker_url("mqtt://192.168.1.10:1883").unwrap();
        assert_eq!(host, "192.168.1.10");
        assert_eq!(port, 1883);
        assert_eq!(user, None);
        assert!(pass.is_none());

        // Sans préfixe de protocole
        let (host, port, _, _) = MqttPublisher::parse_broker_url("localhost:1884").unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 1884);

        // Sans port spécifié (défaut 1883)
        let (host, port, _, _) =
            MqttPublisher::parse_broker_url("mqtt://broker.hivemq.com").unwrap();
        assert_eq!(host, "broker.hivemq.com");
        assert_eq!(port, 1883);

        // Avec identifiants
        let (host, port, user, pass) =
            MqttPublisher::parse_broker_url("mqtt://homeassistant:my_secret@10.0.0.1:1883")
                .unwrap();
        assert_eq!(host, "10.0.0.1");
        assert_eq!(port, 1883);
        assert_eq!(user.as_deref(), Some("homeassistant"));
        use secrecy::ExposeSecret;
        assert_eq!(
            pass.as_ref().map(|s| s.expose_secret().as_str()),
            Some("my_secret")
        );

        // Avec TLS mqtts:// (port par défaut 8883)
        let parsed =
            MqttPublisher::parse_broker_url_details("mqtts://secure-broker.hivemq.cloud").unwrap();
        assert_eq!(parsed.host, "secure-broker.hivemq.cloud");
        assert_eq!(parsed.port, 8883);
        assert!(parsed.is_tls);
    }

    #[test]
    fn test_prm_state_serialization() {
        let state = EnedisPrmState {
            prm: "01234567890123".to_string(),
            consumption_kwh: Some(1234.567),
            production_kwh: Some(89.123),
            last_power_w: Some(2500.0),
            max_power_kva: Some(6.2),
            subscribed_power_kva: Some(9),
            quality: Some("VALIDATED".to_string()),
            last_reading: Some("2026-09-28T10:00:00Z".to_string()),
            sync_status: "OK".to_string(),
            baseload_w: Some(180.0),
            solar_autoconsumption_percent: Some(72.5),
            solar_autoproduction_percent: Some(45.0),
            spot_price_eur_mwh: Some(68.50),
            flex_shed_power_w: Some(2400.0),
            flex_active_order: Some("Délestage ECS actif (Tempo Rouge HP)".to_string()),
        };

        let json = state.to_json().unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["prm"], "01234567890123");
        assert_eq!(val["consumption_kwh"], 1234.567);
        assert_eq!(val["production_kwh"], 89.123);
        assert_eq!(val["last_power_w"], 2500.0);
        assert_eq!(val["max_power_kva"], 6.2);
        assert_eq!(val["subscribed_power_kva"], 9);
        assert_eq!(val["quality"], "VALIDATED");
        assert_eq!(val["last_reading"], "2026-09-28T10:00:00Z");
        assert_eq!(val["sync_status"], "OK");
        assert_eq!(val["baseload_w"], 180.0);
        assert_eq!(val["solar_autoconsumption_percent"], 72.5);
        assert_eq!(val["solar_autoproduction_percent"], 45.0);
        assert_eq!(val["spot_price_eur_mwh"], 68.50);
    }
}
