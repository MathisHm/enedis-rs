use crate::error::EnedisError;
use crate::models::Measurement;
use secrecy::{ExposeSecret, SecretString};
use std::fmt;
use std::fmt::Write as FmtWrite;
use std::fs::File;
use std::io::Write as IoWrite;
use std::path::Path;

/// Configuration du connecteur InfluxDB
#[derive(Clone)]
pub struct InfluxDbConfig {
    /// URL de base InfluxDB (ex: "http://localhost:8086")
    pub endpoint_url: String,
    /// Organisation InfluxDB v2 (optionnel pour InfluxDB v1)
    pub org: Option<String>,
    /// Bucket InfluxDB v2 ou Database v1 (ex: "enedis_energy")
    pub bucket: String,
    /// Jeton d'authentification API Token (v2) ou mot de passe
    pub token: Option<SecretString>,
    /// Nom de la mesure InfluxDB (défaut: "enedis_measurements")
    pub measurement_name: String,
    /// Taille maximale des lots pour les envois HTTP (défaut: 1000 lignes)
    pub batch_size: usize,
}

impl fmt::Debug for InfluxDbConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InfluxDbConfig")
            .field("endpoint_url", &self.endpoint_url)
            .field("org", &self.org)
            .field("bucket", &self.bucket)
            .field("token", &self.token.as_ref().map(|_| "[REDACTED]"))
            .field("measurement_name", &self.measurement_name)
            .field("batch_size", &self.batch_size)
            .finish()
    }
}

impl Default for InfluxDbConfig {
    fn default() -> Self {
        Self {
            endpoint_url: "http://localhost:8086".to_string(),
            org: None,
            bucket: "enedis".to_string(),
            token: None,
            measurement_name: "enedis_measurements".to_string(),
            batch_size: 1000,
        }
    }
}

impl InfluxDbConfig {
    pub fn new(endpoint_url: impl Into<String>, bucket: impl Into<String>) -> Self {
        Self {
            endpoint_url: endpoint_url.into(),
            bucket: bucket.into(),
            ..Default::default()
        }
    }

    pub fn with_org(mut self, org: impl Into<String>) -> Self {
        self.org = Some(org.into());
        self
    }

    pub fn with_token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(SecretString::new(token.into()));
        self
    }

    pub fn with_measurement_name(mut self, name: impl Into<String>) -> Self {
        self.measurement_name = name.into();
        self
    }
}

/// Convertit une liste de mesures Enedis au format InfluxDB Line Protocol
/// Format: `<measurement>,<tags> <fields> <timestamp_nanos>`
pub fn measurements_to_line_protocol(
    measurements: &[Measurement],
    measurement_name: Option<&str>,
) -> String {
    let name = measurement_name.unwrap_or("enedis_measurements");
    let mut buffer = String::with_capacity(measurements.len() * 128);

    for m in measurements {
        // 1. Tags (prm, direction, unit, quality)
        let prm = escape_tag_value(m.point_id.as_str());
        let direction = escape_tag_value(m.direction.as_str());
        let unit = escape_tag_value(m.unit.as_str());
        let quality = escape_tag_value(m.quality.to_string().as_str());

        // 2. Champs numériques (value, interval_seconds, energy_kwh, power_w)
        let energy_kwh = m.energy_kwh();
        let power_w = m.power_w();

        // 3. Timestamp Unix en nanosecondes
        let nanos = match m.timestamp.timestamp_nanos_opt() {
            Some(n) => n,
            None => m.timestamp.timestamp() * 1_000_000_000,
        };

        // Syntaxe: measurement,tag1=val1,tag2=val2 field1=val1,field2=val2 timestamp
        let _ = writeln!(
            buffer,
            "{},prm={},direction={},unit={},quality={} value={},interval_seconds={}i,energy_kwh={},power_w={} {}",
            escape_measurement_name(name),
            prm,
            direction,
            unit,
            quality,
            m.value,
            m.interval_seconds,
            energy_kwh,
            power_w,
            nanos
        );
    }

    buffer
}

/// Échappe les caractères réservés dans le nom de mesure InfluxDB (virgules et espaces)
fn escape_measurement_name(s: &str) -> String {
    s.replace(',', "\\,").replace(' ', "\\ ")
}

/// Échappe les caractères réservés dans les clés et valeurs de tags (virgules, espaces, signes égal)
fn escape_tag_value(s: &str) -> String {
    s.replace(',', "\\,")
        .replace(' ', "\\ ")
        .replace('=', "\\=")
}

/// Connecteur d'exportation vers InfluxDB
pub struct InfluxDbExporter {
    config: InfluxDbConfig,
}

impl InfluxDbExporter {
    pub fn new(config: InfluxDbConfig) -> Self {
        Self { config }
    }

    /// Exporte les mesures dans un fichier Line Protocol (.lp) sur disque
    pub fn export_to_file<P: AsRef<Path>>(
        &self,
        path: P,
        measurements: &[Measurement],
    ) -> Result<usize, EnedisError> {
        let payload =
            measurements_to_line_protocol(measurements, Some(&self.config.measurement_name));
        let mut file = File::create(path.as_ref()).map_err(|e| {
            EnedisError::Configuration(format!(
                "Impossible de créer le fichier Line Protocol '{:?}': {}",
                path.as_ref(),
                e
            ))
        })?;

        file.write_all(payload.as_bytes()).map_err(|e| {
            EnedisError::Configuration(format!(
                "Erreur d'écriture dans le fichier '{:?}': {}",
                path.as_ref(),
                e
            ))
        })?;

        Ok(measurements.len())
    }

    /// Envoie les mesures directement à l'API HTTP InfluxDB en flux ou par lots
    #[cfg(feature = "client")]
    pub async fn export_http(&self, measurements: &[Measurement]) -> Result<usize, EnedisError> {
        if measurements.is_empty() {
            return Ok(0);
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| {
                EnedisError::Configuration(format!("Erreur client HTTP InfluxDB: {}", e))
            })?;

        // Détection InfluxDB v2 ou v1
        let url = if let Some(ref org) = self.config.org {
            format!(
                "{}/api/v2/write?org={}&bucket={}&precision=ns",
                self.config.endpoint_url.trim_end_matches('/'),
                urlencoding_simple(org),
                urlencoding_simple(&self.config.bucket)
            )
        } else {
            format!(
                "{}/write?db={}&precision=ns",
                self.config.endpoint_url.trim_end_matches('/'),
                urlencoding_simple(&self.config.bucket)
            )
        };

        let batch_size = self.config.batch_size.max(1);
        let mut sent = 0;

        for chunk in measurements.chunks(batch_size) {
            let body = measurements_to_line_protocol(chunk, Some(&self.config.measurement_name));

            let mut req = client.post(&url).body(body);

            if let Some(ref token) = self.config.token {
                if self.config.org.is_some() {
                    req = req.header("Authorization", format!("Token {}", token.expose_secret()));
                } else {
                    req = req.header("Authorization", format!("Bearer {}", token.expose_secret()));
                }
            }

            let resp = req.send().await.map_err(|e| {
                EnedisError::Transport(crate::error::TransportError::Network(format!(
                    "Échec envoi InfluxDB: {}",
                    e
                )))
            })?;

            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let text = resp.text().await.unwrap_or_default();
                return Err(EnedisError::Http {
                    status,
                    body: format!("Erreur rejet InfluxDB ({}): {}", status, text),
                });
            }

            sent += chunk.len();
        }

        Ok(sent)
    }
}

fn urlencoding_simple(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{:02X}", b);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FlowDirection, MeasurementQuality, PointId, Unit};
    use chrono::TimeZone;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    #[test]
    fn test_measurements_to_line_protocol_formatting() {
        let prm = PointId::new("01234567890123").unwrap();
        let ts = chrono::Utc
            .with_ymd_and_hms(2026, 9, 28, 14, 30, 0)
            .unwrap();
        let m = Measurement {
            point_id: prm,
            timestamp: ts,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str("1.5000").unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        };

        let lp = measurements_to_line_protocol(&[m], None);
        assert!(lp.starts_with("enedis_measurements,prm=01234567890123,direction=CONSUMPTION,unit=kWh,quality=VALIDATED"));
        assert!(lp.contains("value=1.5000"));
        assert!(lp.contains("interval_seconds=1800i"));
        assert!(lp.contains("energy_kwh=1.5000"));
        assert!(
            lp.contains("power_w=3000.0000000000000000000000000") || lp.contains("power_w=3000")
        );
        assert!(lp.contains(&ts.timestamp_nanos_opt().unwrap().to_string()));
    }
}
