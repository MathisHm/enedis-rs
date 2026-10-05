use crate::models::{FlowDirection, PointId};
use serde::{Deserialize, Serialize};

/// Représentation d'un équipement (Device) selon le protocole Home Assistant MQTT Discovery
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HaDevice {
    /// Identifiants uniques de l'équipement dans Home Assistant
    pub identifiers: Vec<String>,
    /// Fabricant (ex: Enedis)
    pub manufacturer: String,
    /// Modèle de l'appareil (ex: Compteur Linky)
    pub model: String,
    /// Nom affiché dans Home Assistant
    pub name: String,
    /// Version logicielle de l'équipement
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sw_version: Option<String>,
    /// URL de configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration_url: Option<String>,
}

impl HaDevice {
    /// Crée la description du compteur Linky pour un PRM donné
    pub fn for_prm(prm: &str) -> Self {
        Self {
            identifiers: vec![format!("enedis_{}", prm)],
            manufacturer: "Enedis".to_string(),
            model: "Compteur Linky".to_string(),
            name: format!("Compteur {}", prm),
            sw_version: Some(format!("enedis-rs v{}", env!("CARGO_PKG_VERSION"))),
            configuration_url: None,
        }
    }
}

/// Configuration d'une entité Sensor selon le protocole Home Assistant MQTT Discovery
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HaSensorConfig {
    /// Nom complet de l'entité
    pub name: String,
    /// Topic MQTT sur lequel sont publiés les états
    pub state_topic: String,
    /// Modèle Jinja2 pour extraire la valeur depuis le payload JSON
    pub value_template: String,
    /// Classe de l'appareil (ex: energy, power, apparent_power, timestamp)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_class: Option<String>,
    /// Type de compteur (ex: total_increasing pour Energy Dashboard, measurement pour instantané)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_class: Option<String>,
    /// Unité de mesure (ex: kWh, W, kVA)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit_of_measurement: Option<String>,
    /// Identifiant unique global dans Home Assistant
    pub unique_id: String,
    /// Catégorie d'entité (ex: diagnostic)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_category: Option<String>,
    /// Topic de disponibilité Home Assistant (LWT)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_topic: Option<String>,
    /// Icône Material Design (ex: mdi:solar-power, mdi:power-sleep)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Équipement auquel est rattaché le capteur
    pub device: HaDevice,
}

impl HaSensorConfig {
    /// Sérialise la configuration en chaîne JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Sérialise la configuration en chaîne JSON indentée
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Configuration d'une entité Select selon le protocole Home Assistant MQTT Discovery
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HaSelectConfig {
    pub name: String,
    pub command_topic: String,
    pub state_topic: String,
    pub options: Vec<String>,
    pub unique_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub device: HaDevice,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_topic: Option<String>,
}

impl HaSelectConfig {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// Configuration d'une entité Switch selon le protocole Home Assistant MQTT Discovery
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HaSwitchConfig {
    pub name: String,
    pub command_topic: String,
    pub state_topic: String,
    pub payload_on: String,
    pub payload_off: String,
    pub unique_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub device: HaDevice,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_topic: Option<String>,
}

impl HaSwitchConfig {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }
}

/// Génère le topic MQTT Discovery pour un capteur d'énergie (Consommation ou Production)
pub fn energy_sensor_discovery_topic(prm: &str, direction: &str, discovery_prefix: &str) -> String {
    format!(
        "{}/sensor/enedis_{}_{}/config",
        discovery_prefix.trim_end_matches('/'),
        prm,
        direction.to_ascii_lowercase()
    )
}

/// Génère la configuration Discovery pour un capteur d'énergie (Consommation ou Production)
pub fn energy_sensor_discovery(
    point_id: PointId,
    direction: FlowDirection,
    topic_prefix: &str,
    discovery_prefix: &str,
) -> (String, HaSensorConfig) {
    let prm = point_id.as_str();
    let (dir_str, dir_name, val_template) = match direction {
        FlowDirection::Consumption => (
            "consumption",
            "Consommation",
            "{{ value_json.consumption_kwh }}",
        ),
        FlowDirection::Production => (
            "production",
            "Production",
            "{{ value_json.production_kwh }}",
        ),
    };

    let topic = energy_sensor_discovery_topic(prm, dir_str, discovery_prefix);
    let state_topic = format!("{}/{}/state", topic_prefix.trim_end_matches('/'), prm);
    let avail_topic = format!("{}/status", topic_prefix.trim_end_matches('/'));

    let config = HaSensorConfig {
        name: format!("Enedis {} {}", prm, dir_name),
        state_topic,
        value_template: val_template.to_string(),
        device_class: Some("energy".to_string()),
        state_class: Some("total_increasing".to_string()),
        unit_of_measurement: Some("kWh".to_string()),
        unique_id: format!("enedis_{}_{}", prm, dir_str),
        entity_category: None,
        availability_topic: Some(avail_topic),
        icon: None,
        device: HaDevice::for_prm(prm),
    };

    (topic, config)
}

/// Génère la configuration Discovery pour le capteur de puissance active instantanée
pub fn power_sensor_discovery(
    point_id: PointId,
    topic_prefix: &str,
    discovery_prefix: &str,
) -> (String, HaSensorConfig) {
    let prm = point_id.as_str();
    let topic = format!(
        "{}/sensor/enedis_{}_power/config",
        discovery_prefix.trim_end_matches('/'),
        prm
    );
    let state_topic = format!("{}/{}/state", topic_prefix.trim_end_matches('/'), prm);
    let avail_topic = format!("{}/status", topic_prefix.trim_end_matches('/'));

    let config = HaSensorConfig {
        name: format!("Enedis {} Puissance active", prm),
        state_topic,
        value_template: "{{ value_json.last_power_w }}".to_string(),
        device_class: Some("power".to_string()),
        state_class: Some("measurement".to_string()),
        unit_of_measurement: Some("W".to_string()),
        unique_id: format!("enedis_{}_power", prm),
        entity_category: None,
        availability_topic: Some(avail_topic),
        icon: None,
        device: HaDevice::for_prm(prm),
    };

    (topic, config)
}

/// Génère la configuration Discovery pour le capteur de pointe maximale quotidienne
pub fn max_power_sensor_discovery(
    point_id: PointId,
    topic_prefix: &str,
    discovery_prefix: &str,
) -> (String, HaSensorConfig) {
    let prm = point_id.as_str();
    let topic = format!(
        "{}/sensor/enedis_{}_max_power/config",
        discovery_prefix.trim_end_matches('/'),
        prm
    );
    let state_topic = format!("{}/{}/state", topic_prefix.trim_end_matches('/'), prm);
    let avail_topic = format!("{}/status", topic_prefix.trim_end_matches('/'));

    let config = HaSensorConfig {
        name: format!("Enedis {} Puissance max", prm),
        state_topic,
        value_template: "{{ value_json.max_power_kva }}".to_string(),
        device_class: Some("apparent_power".to_string()),
        state_class: Some("measurement".to_string()),
        unit_of_measurement: Some("kVA".to_string()),
        unique_id: format!("enedis_{}_max_power", prm),
        entity_category: None,
        availability_topic: Some(avail_topic),
        icon: None,
        device: HaDevice::for_prm(prm),
    };

    (topic, config)
}

/// Génère la configuration Discovery pour le capteur de puissance souscrite contractuelle
pub fn subscribed_power_sensor_discovery(
    point_id: PointId,
    topic_prefix: &str,
    discovery_prefix: &str,
) -> (String, HaSensorConfig) {
    let prm = point_id.as_str();
    let topic = format!(
        "{}/sensor/enedis_{}_subscribed_power/config",
        discovery_prefix.trim_end_matches('/'),
        prm
    );
    let state_topic = format!("{}/{}/state", topic_prefix.trim_end_matches('/'), prm);
    let avail_topic = format!("{}/status", topic_prefix.trim_end_matches('/'));

    let config = HaSensorConfig {
        name: format!("Enedis {} Puissance souscrite", prm),
        state_topic,
        value_template: "{{ value_json.subscribed_power_kva }}".to_string(),
        device_class: Some("apparent_power".to_string()),
        state_class: None,
        unit_of_measurement: Some("kVA".to_string()),
        unique_id: format!("enedis_{}_subscribed_power", prm),
        entity_category: Some("diagnostic".to_string()),
        availability_topic: Some(avail_topic),
        icon: None,
        device: HaDevice::for_prm(prm),
    };

    (topic, config)
}

/// Génère les configurations Discovery pour les capteurs de diagnostic Home Assistant
pub fn diagnostic_sensors_discovery(
    point_id: PointId,
    topic_prefix: &str,
    discovery_prefix: &str,
) -> Vec<(String, HaSensorConfig)> {
    let prm = point_id.as_str();
    let state_topic = format!("{}/{}/state", topic_prefix.trim_end_matches('/'), prm);
    let disc_prefix = discovery_prefix.trim_end_matches('/');
    let avail_topic = format!("{}/status", topic_prefix.trim_end_matches('/'));

    let quality_topic = format!("{}/sensor/enedis_{}_quality/config", disc_prefix, prm);
    let quality_config = HaSensorConfig {
        name: format!("Enedis {} Qualité", prm),
        state_topic: state_topic.clone(),
        value_template: "{{ value_json.quality }}".to_string(),
        device_class: None,
        state_class: None,
        unit_of_measurement: None,
        unique_id: format!("enedis_{}_quality", prm),
        entity_category: Some("diagnostic".to_string()),
        availability_topic: Some(avail_topic.clone()),
        icon: Some("mdi:check-decagram".to_string()),
        device: HaDevice::for_prm(prm),
    };

    let last_reading_topic = format!("{}/sensor/enedis_{}_last_reading/config", disc_prefix, prm);
    let last_reading_config = HaSensorConfig {
        name: format!("Enedis {} Dernier relevé", prm),
        state_topic: state_topic.clone(),
        value_template: "{{ value_json.last_reading }}".to_string(),
        device_class: Some("timestamp".to_string()),
        state_class: None,
        unit_of_measurement: None,
        unique_id: format!("enedis_{}_last_reading", prm),
        entity_category: Some("diagnostic".to_string()),
        availability_topic: Some(avail_topic.clone()),
        icon: Some("mdi:clock-check-outline".to_string()),
        device: HaDevice::for_prm(prm),
    };

    let sync_status_topic = format!("{}/sensor/enedis_{}_sync_status/config", disc_prefix, prm);
    let sync_status_config = HaSensorConfig {
        name: format!("Enedis {} Statut synchronisation", prm),
        state_topic,
        value_template: "{{ value_json.sync_status }}".to_string(),
        device_class: None,
        state_class: None,
        unit_of_measurement: None,
        unique_id: format!("enedis_{}_sync_status", prm),
        entity_category: Some("diagnostic".to_string()),
        availability_topic: Some(avail_topic),
        icon: Some("mdi:sync".to_string()),
        device: HaDevice::for_prm(prm),
    };

    vec![
        (quality_topic, quality_config),
        (last_reading_topic, last_reading_config),
        (sync_status_topic, sync_status_config),
    ]
}

/// Génère la configuration Discovery pour le capteur de talon de veille (puissance permanente)
pub fn baseload_sensor_discovery(
    point_id: PointId,
    topic_prefix: &str,
    discovery_prefix: &str,
) -> (String, HaSensorConfig) {
    let prm = point_id.as_str();
    let topic = format!(
        "{}/sensor/enedis_{}_baseload/config",
        discovery_prefix.trim_end_matches('/'),
        prm
    );
    let state_topic = format!("{}/{}/state", topic_prefix.trim_end_matches('/'), prm);
    let avail_topic = format!("{}/status", topic_prefix.trim_end_matches('/'));

    let config = HaSensorConfig {
        name: format!("Enedis {} Talon de veille", prm),
        state_topic,
        value_template: "{{ value_json.baseload_w }}".to_string(),
        device_class: Some("power".to_string()),
        state_class: Some("measurement".to_string()),
        unit_of_measurement: Some("W".to_string()),
        unique_id: format!("enedis_{}_baseload", prm),
        entity_category: None,
        availability_topic: Some(avail_topic),
        icon: Some("mdi:power-sleep".to_string()),
        device: HaDevice::for_prm(prm),
    };

    (topic, config)
}

/// Génère la configuration Discovery pour le capteur de taux d'autoconsommation solaire
pub fn solar_autoconsumption_sensor_discovery(
    point_id: PointId,
    topic_prefix: &str,
    discovery_prefix: &str,
) -> (String, HaSensorConfig) {
    let prm = point_id.as_str();
    let topic = format!(
        "{}/sensor/enedis_{}_autoconsumption/config",
        discovery_prefix.trim_end_matches('/'),
        prm
    );
    let state_topic = format!("{}/{}/state", topic_prefix.trim_end_matches('/'), prm);
    let avail_topic = format!("{}/status", topic_prefix.trim_end_matches('/'));

    let config = HaSensorConfig {
        name: format!("Enedis {} Autoconsommation", prm),
        state_topic,
        value_template: "{{ value_json.solar_autoconsumption_percent }}".to_string(),
        device_class: None,
        state_class: Some("measurement".to_string()),
        unit_of_measurement: Some("%".to_string()),
        unique_id: format!("enedis_{}_autoconsumption", prm),
        entity_category: None,
        availability_topic: Some(avail_topic),
        icon: Some("mdi:solar-power-variant".to_string()),
        device: HaDevice::for_prm(prm),
    };

    (topic, config)
}

/// Génère la configuration Discovery pour le capteur de taux d'autoproduction solaire
pub fn solar_autoproduction_sensor_discovery(
    point_id: PointId,
    topic_prefix: &str,
    discovery_prefix: &str,
) -> (String, HaSensorConfig) {
    let prm = point_id.as_str();
    let topic = format!(
        "{}/sensor/enedis_{}_autoproduction/config",
        discovery_prefix.trim_end_matches('/'),
        prm
    );
    let state_topic = format!("{}/{}/state", topic_prefix.trim_end_matches('/'), prm);
    let avail_topic = format!("{}/status", topic_prefix.trim_end_matches('/'));

    let config = HaSensorConfig {
        name: format!("Enedis {} Autoproduction", prm),
        state_topic,
        value_template: "{{ value_json.solar_autoproduction_percent }}".to_string(),
        device_class: None,
        state_class: Some("measurement".to_string()),
        unit_of_measurement: Some("%".to_string()),
        unique_id: format!("enedis_{}_autoproduction", prm),
        entity_category: None,
        availability_topic: Some(avail_topic),
        icon: Some("mdi:solar-power".to_string()),
        device: HaDevice::for_prm(prm),
    };

    (topic, config)
}

/// Génère la configuration Discovery pour le capteur de prix spot Day-Ahead (EPEX SPOT)
pub fn spot_price_sensor_discovery(
    point_id: PointId,
    topic_prefix: &str,
    discovery_prefix: &str,
) -> (String, HaSensorConfig) {
    let prm = point_id.as_str();
    let topic = format!(
        "{}/sensor/enedis_{}_spot_price/config",
        discovery_prefix.trim_end_matches('/'),
        prm
    );
    let state_topic = format!("{}/{}/state", topic_prefix.trim_end_matches('/'), prm);
    let avail_topic = format!("{}/status", topic_prefix.trim_end_matches('/'));

    let config = HaSensorConfig {
        name: format!("Enedis {} Prix Spot Day-Ahead", prm),
        state_topic,
        value_template: "{{ value_json.spot_price_eur_mwh }}".to_string(),
        device_class: None,
        state_class: Some("measurement".to_string()),
        unit_of_measurement: Some("€/MWh".to_string()),
        unique_id: format!("enedis_{}_spot_price", prm),
        entity_category: None,
        availability_topic: Some(avail_topic),
        icon: Some("mdi:chart-timeline-variant-shimmer".to_string()),
        device: HaDevice::for_prm(prm),
    };

    (topic, config)
}

/// Génère l'ensemble des configurations Discovery (Consommation, Production, Puissances, Solaire, Spot, Diagnostic) pour un PRM
pub fn all_discovery_configs(
    point_id: PointId,
    topic_prefix: &str,
    discovery_prefix: &str,
) -> Vec<(String, HaSensorConfig)> {
    let mut configs = Vec::with_capacity(12);
    configs.push(energy_sensor_discovery(
        point_id,
        FlowDirection::Consumption,
        topic_prefix,
        discovery_prefix,
    ));
    configs.push(energy_sensor_discovery(
        point_id,
        FlowDirection::Production,
        topic_prefix,
        discovery_prefix,
    ));
    configs.push(power_sensor_discovery(
        point_id,
        topic_prefix,
        discovery_prefix,
    ));
    configs.push(max_power_sensor_discovery(
        point_id,
        topic_prefix,
        discovery_prefix,
    ));
    configs.push(subscribed_power_sensor_discovery(
        point_id,
        topic_prefix,
        discovery_prefix,
    ));
    configs.push(baseload_sensor_discovery(
        point_id,
        topic_prefix,
        discovery_prefix,
    ));
    configs.push(solar_autoconsumption_sensor_discovery(
        point_id,
        topic_prefix,
        discovery_prefix,
    ));
    configs.push(solar_autoproduction_sensor_discovery(
        point_id,
        topic_prefix,
        discovery_prefix,
    ));
    configs.push(spot_price_sensor_discovery(
        point_id,
        topic_prefix,
        discovery_prefix,
    ));
    configs.extend(diagnostic_sensors_discovery(
        point_id,
        topic_prefix,
        discovery_prefix,
    ));
    configs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_energy_sensor_discovery_consumption() {
        let prm = PointId::new("01234567890123").unwrap();
        let (topic, config) =
            energy_sensor_discovery(prm, FlowDirection::Consumption, "enedis", "homeassistant");

        assert_eq!(
            topic,
            "homeassistant/sensor/enedis_01234567890123_consumption/config"
        );
        assert_eq!(config.name, "Enedis 01234567890123 Consommation");
        assert_eq!(config.state_topic, "enedis/01234567890123/state");
        assert_eq!(config.value_template, "{{ value_json.consumption_kwh }}");
        assert_eq!(config.device_class.as_deref(), Some("energy"));
        assert_eq!(config.state_class.as_deref(), Some("total_increasing"));
        assert_eq!(config.unit_of_measurement.as_deref(), Some("kWh"));
        assert_eq!(config.unique_id, "enedis_01234567890123_consumption");
        assert_eq!(config.device.identifiers, vec!["enedis_01234567890123"]);
        assert_eq!(config.device.manufacturer, "Enedis");
        assert_eq!(config.device.model, "Compteur Linky");
        assert_eq!(config.device.name, "Compteur 01234567890123");

        let json = config.to_json().unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["name"], "Enedis 01234567890123 Consommation");
        assert_eq!(v["device_class"], "energy");
        assert_eq!(v["state_class"], "total_increasing");
        assert_eq!(v["unit_of_measurement"], "kWh");
        assert_eq!(v["device"]["identifiers"][0], "enedis_01234567890123");
    }

    #[test]
    fn test_energy_sensor_discovery_production() {
        let prm = PointId::new("01234567890123").unwrap();
        let (topic, config) =
            energy_sensor_discovery(prm, FlowDirection::Production, "enedis", "homeassistant");

        assert_eq!(
            topic,
            "homeassistant/sensor/enedis_01234567890123_production/config"
        );
        assert_eq!(config.name, "Enedis 01234567890123 Production");
        assert_eq!(config.state_topic, "enedis/01234567890123/state");
        assert_eq!(config.value_template, "{{ value_json.production_kwh }}");
        assert_eq!(config.device_class.as_deref(), Some("energy"));
        assert_eq!(config.state_class.as_deref(), Some("total_increasing"));
        assert_eq!(config.unit_of_measurement.as_deref(), Some("kWh"));
        assert_eq!(config.unique_id, "enedis_01234567890123_production");
    }

    #[test]
    fn test_diagnostic_sensors_discovery() {
        let prm = PointId::new("01234567890123").unwrap();
        let diagnostics = diagnostic_sensors_discovery(prm, "enedis", "homeassistant");

        assert_eq!(diagnostics.len(), 3);

        let (quality_topic, quality) = &diagnostics[0];
        assert_eq!(
            quality_topic,
            "homeassistant/sensor/enedis_01234567890123_quality/config"
        );
        assert_eq!(quality.value_template, "{{ value_json.quality }}");
        assert_eq!(quality.entity_category.as_deref(), Some("diagnostic"));

        let (date_topic, date_sensor) = &diagnostics[1];
        assert_eq!(
            date_topic,
            "homeassistant/sensor/enedis_01234567890123_last_reading/config"
        );
        assert_eq!(date_sensor.value_template, "{{ value_json.last_reading }}");
        assert_eq!(date_sensor.device_class.as_deref(), Some("timestamp"));
        assert_eq!(date_sensor.entity_category.as_deref(), Some("diagnostic"));

        let (sync_topic, sync_sensor) = &diagnostics[2];
        assert_eq!(
            sync_topic,
            "homeassistant/sensor/enedis_01234567890123_sync_status/config"
        );
        assert_eq!(sync_sensor.value_template, "{{ value_json.sync_status }}");
        assert_eq!(sync_sensor.entity_category.as_deref(), Some("diagnostic"));
    }

    #[test]
    fn test_custom_prefixes() {
        let prm = PointId::new("99999999999999").unwrap();
        let (topic, config) = energy_sensor_discovery(
            prm,
            FlowDirection::Consumption,
            "custom/enedis",
            "ha_discovery",
        );

        assert_eq!(
            topic,
            "ha_discovery/sensor/enedis_99999999999999_consumption/config"
        );
        assert_eq!(config.state_topic, "custom/enedis/99999999999999/state");
    }

    #[test]
    fn test_advanced_sensors_discovery() {
        let prm = PointId::new("01234567890123").unwrap();
        let (baseload_topic, baseload_cfg) =
            baseload_sensor_discovery(prm, "enedis", "homeassistant");
        assert_eq!(
            baseload_topic,
            "homeassistant/sensor/enedis_01234567890123_baseload/config"
        );
        assert_eq!(baseload_cfg.unit_of_measurement.as_deref(), Some("W"));
        assert_eq!(baseload_cfg.device_class.as_deref(), Some("power"));
        assert_eq!(baseload_cfg.icon.as_deref(), Some("mdi:power-sleep"));

        let (solar_topic, solar_cfg) =
            solar_autoconsumption_sensor_discovery(prm, "enedis", "homeassistant");
        assert_eq!(
            solar_topic,
            "homeassistant/sensor/enedis_01234567890123_autoconsumption/config"
        );
        assert_eq!(solar_cfg.unit_of_measurement.as_deref(), Some("%"));

        let (spot_topic, spot_cfg) = spot_price_sensor_discovery(prm, "enedis", "homeassistant");
        assert_eq!(
            spot_topic,
            "homeassistant/sensor/enedis_01234567890123_spot_price/config"
        );
        assert_eq!(spot_cfg.unit_of_measurement.as_deref(), Some("€/MWh"));

        let all = all_discovery_configs(prm, "enedis", "homeassistant");
        assert_eq!(all.len(), 12);
    }
}
