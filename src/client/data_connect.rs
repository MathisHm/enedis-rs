use crate::client::config::DataConnectConfig;
use crate::client::provider::{EnedisProvider, ProviderFuture};
use crate::error::{EnedisError, SgeBusinessError, TransportError};
use crate::models::{
    from_french_local_time, to_french_local_time, CalendarSchedule, ConsentInfo, ConsentStatus,
    ContractData, FlowDirection, MaxPowerRecord, Measurement, MeasurementQuality,
    MeterCharacteristics, MeterType, PhaseCount, PointId, TariffOption, Unit,
};
use chrono::{DateTime, NaiveDateTime, Utc};
use reqwest::{header, Client, StatusCode};
use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, instrument, warn};

#[derive(Clone, Debug)]
struct CachedToken {
    token: String,
    expires_at: Option<DateTime<Utc>>,
}

/// Réponse retournée lors de l'échange de code d'autorisation OAuth2 Enedis
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: Option<i64>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub usage_point_id: Option<String>,
}

/// Client pour l'API Enedis Data Connect (REST v5 / OAuth2 Bearer Tokens)
///
/// Contrairement aux flux SGE (SOAP sur TLS mutuel avec certificats matériels ou PKCS#12),
/// Data Connect expose une API web JSON moderne sécurisée par jetons OAuth2 (Bearer).
#[derive(Clone)]
pub struct DataConnectClient {
    client: Client,
    config: DataConnectConfig,
    cached_token: Arc<RwLock<Option<CachedToken>>>,
}

impl fmt::Debug for DataConnectClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataConnectClient")
            .field("base_url", &self.config.base_url)
            .field("client_id", &self.config.client_id)
            .finish()
    }
}

impl DataConnectClient {
    /// Initialise un nouveau client Enedis Data Connect
    pub fn new(config: DataConnectConfig) -> Result<Self, EnedisError> {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .connect_timeout(config.connect_timeout)
            .user_agent(&config.user_agent)
            .build()
            .map_err(|e| EnedisError::Transport(TransportError::from(e)))?;

        let initial_token = config.direct_token.as_ref().map(|t| CachedToken {
            token: t.expose_secret().to_string(),
            expires_at: None,
        });

        Ok(Self {
            client,
            config,
            cached_token: Arc::new(RwLock::new(initial_token)),
        })
    }

    /// Référence vers la configuration active du client
    pub fn config(&self) -> &DataConnectConfig {
        &self.config
    }

    /// Génère l'URL d'autorisation Enedis pour le recueil de consentement client OAuth2
    pub fn get_authorize_url(
        &self,
        redirect_uri: &str,
        state: &str,
        duration: Option<&str>,
    ) -> Result<String, EnedisError> {
        let client_id = self.config.client_id.as_ref().ok_or_else(|| {
            EnedisError::Configuration(
                "client_id manquant dans DataConnectConfig pour initier le consentement"
                    .to_string(),
            )
        })?;

        let duration = duration.unwrap_or("P3Y");
        let base_auth_url = self.config.get_authorize_url();

        let mut url = reqwest::Url::parse(base_auth_url).map_err(|e| {
            EnedisError::Configuration(format!("URL d'autorisation invalide: {}", e))
        })?;

        url.query_pairs_mut()
            .append_pair("client_id", client_id)
            .append_pair("response_type", "code")
            .append_pair("state", state)
            .append_pair("duration", duration)
            .append_pair("redirect_uri", redirect_uri);

        Ok(url.to_string())
    }

    /// Échange un code d'autorisation OAuth2 contre un jeton d'accès Bearer et l'identifiant PRM
    pub async fn exchange_authorization_code(
        &self,
        code: &str,
        redirect_uri: &str,
    ) -> Result<OAuthTokenResponse, EnedisError> {
        let client_id = self.config.client_id.as_ref().ok_or_else(|| {
            EnedisError::Configuration(
                "client_id absent de la configuration Data Connect pour l'échange de code"
                    .to_string(),
            )
        })?;

        let client_secret = self.config.client_secret.as_ref().ok_or_else(|| {
            EnedisError::Configuration(
                "client_secret absent de la configuration Data Connect pour l'échange de code"
                    .to_string(),
            )
        })?;

        let params = [
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.expose_secret()),
            ("redirect_uri", redirect_uri),
        ];

        let res = self
            .client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(TransportError::from)?;

        let status = res.status();
        let bytes = res.bytes().await.map_err(TransportError::from)?;

        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes).to_string();
            warn!(
                "Échec de l'échange OAuth2 authorization_code (HTTP {}): {}",
                status, body
            );
            return Err(EnedisError::Http {
                status: status.as_u16(),
                body,
            });
        }

        let resp: OAuthTokenResponse = serde_json::from_slice(&bytes).map_err(|e| {
            EnedisError::Configuration(format!(
                "Réponse OAuth2 token invalide lors de l'échange de code: {}",
                e
            ))
        })?;

        // Mise en cache immédiate du token reçu
        let expires_at = resp
            .expires_in
            .map(|exp| Utc::now() + chrono::Duration::seconds(exp));
        let mut cache = self.cached_token.write().await;
        *cache = Some(CachedToken {
            token: resp.access_token.clone(),
            expires_at,
        });

        info!(
            "Code OAuth2 échangé avec succès. Jeton Bearer mis en cache (validité: {:?}s)",
            resp.expires_in
        );
        Ok(resp)
    }

    /// Récupère un jeton d'accès valide (soit le jeton en cache, soit par échange OAuth2 `client_credentials`)
    pub async fn get_access_token(&self) -> Result<String, EnedisError> {
        // 1. Vérification du jeton actuellement en mémoire cache (avec marge de 60s avant expiration)
        {
            let cache = self.cached_token.read().await;
            if let Some(cached) = cache.as_ref() {
                let is_valid = match cached.expires_at {
                    Some(exp) => exp > Utc::now() + chrono::Duration::seconds(60),
                    None => true, // direct token sans expiration connue
                };
                if is_valid && !cached.token.is_empty() {
                    return Ok(cached.token.clone());
                }
            }
        }

        // 2. Si pas de cache mais des identifiants client, demande d'un nouveau jeton via le serveur OAuth2
        let client_id = self.config.client_id.as_ref().ok_or_else(|| {
            EnedisError::Configuration(
                "Jeton OAuth2 absent et aucun client_id configuré pour Data Connect".to_string(),
            )
        })?;

        let client_secret = self.config.client_secret.as_ref().ok_or_else(|| {
            EnedisError::Configuration(
                "Jeton OAuth2 absent et aucun client_secret configuré pour Data Connect"
                    .to_string(),
            )
        })?;

        debug!(
            "Demande d'un nouveau jeton OAuth2 auprès de {}",
            self.config.token_url
        );

        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.expose_secret()),
        ];

        let res = self
            .client
            .post(&self.config.token_url)
            .form(&params)
            .send()
            .await
            .map_err(TransportError::from)?;

        let status = res.status();
        let bytes = res.bytes().await.map_err(TransportError::from)?;

        if !status.is_success() {
            let body = String::from_utf8_lossy(&bytes).to_string();
            warn!(
                "Échec d'obtention du jeton OAuth2 (HTTP {}): {}",
                status, body
            );
            return Err(EnedisError::Http {
                status: status.as_u16(),
                body,
            });
        }

        let json: Value = serde_json::from_slice(&bytes).map_err(|e| {
            EnedisError::Configuration(format!("Réponse OAuth2 JSON invalide: {}", e))
        })?;

        let token = json
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                EnedisError::Configuration(
                    "Propriété 'access_token' manquante dans la réponse OAuth2".to_string(),
                )
            })?
            .to_string();

        let expires_in = json
            .get("expires_in")
            .and_then(|v| v.as_i64())
            .unwrap_or(3600);
        let expires_at = Some(Utc::now() + chrono::Duration::seconds(expires_in));

        let mut cache = self.cached_token.write().await;
        *cache = Some(CachedToken {
            token: token.clone(),
            expires_at,
        });

        info!(
            "Jeton d'accès OAuth2 Data Connect obtenu et mis en cache avec succès (validité: {}s)",
            expires_in
        );
        Ok(token)
    }

    /// Envoie une requête GET autorisée avec Bearer token et gestion des erreurs de l'API REST v5
    async fn send_authorized_get(
        &self,
        path: &str,
        query: &[(&str, &str)],
        point_id: Option<PointId>,
    ) -> Result<Value, EnedisError> {
        let token = self.get_access_token().await?;
        let url = format!("{}{}", self.config.base_url, path);

        let res = self
            .client
            .get(&url)
            .query(query)
            .header(header::AUTHORIZATION, format!("Bearer {}", token))
            .header(header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(TransportError::from)?;

        let status = res.status();

        // Gestion de l'invalidation / expiration du jeton (HTTP 401)
        if status == StatusCode::UNAUTHORIZED {
            warn!("Jeton d'accès révoqué ou expiré (HTTP 401). Renouvellement en cours...");
            {
                let mut cache = self.cached_token.write().await;
                *cache = None;
            }

            // Tentative unique avec un nouveau token
            let fresh_token = self.get_access_token().await?;
            let retry_res = self
                .client
                .get(&url)
                .query(query)
                .header(header::AUTHORIZATION, format!("Bearer {}", fresh_token))
                .header(header::ACCEPT, "application/json")
                .send()
                .await
                .map_err(TransportError::from)?;

            let retry_status = retry_res.status();
            let retry_bytes = retry_res.bytes().await.map_err(TransportError::from)?;
            return self.process_rest_response(retry_status, &retry_bytes, point_id);
        }

        let bytes = res.bytes().await.map_err(TransportError::from)?;
        self.process_rest_response(status, &bytes, point_id)
    }

    /// Traite et traduit le retour JSON de l'API REST Enedis en erreurs ou en objet Value
    fn process_rest_response(
        &self,
        status: StatusCode,
        bytes: &[u8],
        point_id: Option<PointId>,
    ) -> Result<Value, EnedisError> {
        let body_str = String::from_utf8_lossy(bytes).into_owned();

        if status.is_success() {
            let json: Value = serde_json::from_str(&body_str).map_err(|e| {
                EnedisError::Configuration(format!("Erreur décodage JSON Data Connect: {}", e))
            })?;
            Ok(json)
        } else {
            // Tentative d'extraction d'erreur structurée Enedis
            if let Ok(json) = serde_json::from_str::<Value>(&body_str) {
                let tag = json
                    .get("tag")
                    .or_else(|| json.get("error"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let desc = json
                    .get("description")
                    .or_else(|| json.get("error_description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                let tag_upper = tag.to_uppercase();
                let desc_upper = desc.to_uppercase();

                if tag_upper.contains("CONSENT")
                    || desc_upper.contains("CONSENT")
                    || tag_upper == "ADSP_403"
                {
                    return Err(EnedisError::Business(
                        SgeBusinessError::ConsentMissingOrExpired {
                            point_id: point_id.unwrap_or(PointId::DUMMY),
                        },
                    ));
                } else if tag_upper.contains("NOT_FOUND") || status == StatusCode::NOT_FOUND {
                    return Err(EnedisError::Business(SgeBusinessError::PointNotFound {
                        point_id: point_id.unwrap_or(PointId::DUMMY),
                    }));
                } else if status == StatusCode::TOO_MANY_REQUESTS
                    || tag_upper.contains("LIMIT")
                    || tag_upper.contains("QUOTA")
                {
                    return Err(EnedisError::Business(SgeBusinessError::QuotaExceeded {
                        reset_at: None,
                    }));
                }
            }

            Err(EnedisError::Http {
                status: status.as_u16(),
                body: body_str,
            })
        }
    }

    /// Récupère les mesures de courbe de charge ou consommations journalières pour un PRM
    #[instrument(skip(self), fields(prm = %point_id, direction = %direction))]
    pub async fn fetch_measurements(
        &self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        direction: FlowDirection,
    ) -> Result<Vec<Measurement>, EnedisError> {
        let path = match direction {
            FlowDirection::Consumption => "/metering_data_dc/v5/consumption_load_curve",
            FlowDirection::Production => "/metering_data_dc/v5/production_load_curve",
        };

        let start_str = to_french_local_time(from).format("%Y-%m-%d").to_string();
        let end_str = to_french_local_time(to).format("%Y-%m-%d").to_string();
        let prm_str = point_id.as_str();

        let query = [
            ("usage_point_id", prm_str),
            ("start", start_str.as_str()),
            ("end", end_str.as_str()),
        ];

        let json = self
            .send_authorized_get(path, &query, Some(point_id))
            .await?;
        let mut measurements = Vec::new();

        if let Some(meter_reading) = json.get("meter_reading") {
            let unit_str = meter_reading
                .get("reading_type")
                .and_then(|rt| rt.get("unit"))
                .and_then(|u| u.as_str())
                .unwrap_or("W");
            let unit = Unit::from_sge_code(unit_str);

            let interval_seconds = meter_reading
                .get("reading_type")
                .and_then(|rt| rt.get("interval_length"))
                .and_then(|il| il.as_str())
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(1800);

            if let Some(readings) = meter_reading
                .get("interval_reading")
                .and_then(|r| r.as_array())
            {
                for item in readings {
                    let date_str = item.get("date").and_then(|d| d.as_str()).unwrap_or("");
                    let val_str = item.get("value").and_then(|v| v.as_str()).unwrap_or("");

                    let timestamp = if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
                        dt.with_timezone(&Utc)
                    } else if let Ok(naive) =
                        NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M:%S")
                    {
                        from_french_local_time(naive)
                    } else {
                        continue;
                    };

                    let value = Decimal::from_str(val_str).unwrap_or(Decimal::ZERO);

                    measurements.push(Measurement {
                        point_id,
                        timestamp,
                        interval_seconds,
                        direction,
                        value,
                        unit,
                        quality: MeasurementQuality::Validated,
                    });
                }
            }
        }

        info!(
            "Data Connect : {} mesures récupérées pour le PRM {}",
            measurements.len(),
            point_id
        );
        Ok(measurements)
    }

    /// Récupère les données contractuelles pour un point d'usage
    #[instrument(skip(self), fields(prm = %point_id))]
    pub async fn fetch_contract_data(
        &self,
        point_id: PointId,
    ) -> Result<ContractData, EnedisError> {
        let path = format!("/customers_v5/v5/usage_points/{}/contract", point_id);
        let json = self.send_authorized_get(&path, &[], Some(point_id)).await?;

        let mut subscribed_power_kva = 6;
        let mut tariff_option = TariffOption::Base;
        let mut status = None;
        let mut schedule_name = None;
        let mut off_peak_ranges = Vec::new();
        let mut meter_serial = None;
        let mut meter_type = MeterType::Linky;
        let mut phase_count = PhaseCount::SinglePhase;

        // Extraction tolérante selon la structure Enedis Data Connect v5
        let contract_node = json
            .pointer("/customer/usage_points/0/usage_point/contract")
            .or_else(|| json.pointer("/customer/usage_points/0/contract"))
            .or_else(|| json.get("contract"));

        if let Some(contract) = contract_node {
            if let Some(p) = contract.get("subscribed_power").and_then(|v| v.as_str()) {
                let clean: String = p
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                if let Ok(num) = clean.parse::<f64>() {
                    subscribed_power_kva = num.round() as u32;
                }
            }
            if let Some(opt) = contract.get("pricing_system").and_then(|v| v.as_str()) {
                tariff_option = TariffOption::from_sge_code(opt);
            }
            if let Some(st) = contract.get("contract_status").and_then(|v| v.as_str()) {
                status = Some(st.to_string());
            }
            if let Some(hc) = contract.get("offpeak_hours").and_then(|v| v.as_str()) {
                off_peak_ranges.push(hc.to_string());
                schedule_name = Some("Plages Heures Creuses".to_string());
            }
        }

        let meter_node = json
            .pointer("/customer/usage_points/0/usage_point/meter")
            .or_else(|| json.pointer("/customer/usage_points/0/meter"))
            .or_else(|| json.get("meter"));

        if let Some(meter) = meter_node {
            if let Some(sn) = meter.get("serial_number").and_then(|v| v.as_str()) {
                meter_serial = Some(sn.to_string());
            }
            if let Some(mt) = meter.get("meter_type").and_then(|v| v.as_str()) {
                meter_type = MeterType::from_sge_code(mt);
            }
            if let Some(pc) = meter.get("phase_count").and_then(|v| v.as_str()) {
                phase_count = PhaseCount::from_sge_code(pc);
            }
        }

        let calendar = if schedule_name.is_some() || !off_peak_ranges.is_empty() {
            Some(CalendarSchedule {
                schedule_name,
                off_peak_ranges,
                tempo_color: None,
                seasonal_mode: None,
            })
        } else {
            None
        };

        let meter = Some(MeterCharacteristics {
            serial_number: meter_serial,
            meter_type,
            phase_count,
            circuit_breaker_amperes: None,
        });

        Ok(ContractData {
            point_id,
            subscribed_power_kva,
            tariff_option,
            calendar,
            meter,
            status,
        })
    }

    /// Récupère l'historique des puissances maximales quotidiennes
    #[instrument(skip(self), fields(prm = %point_id))]
    pub async fn fetch_daily_max_power(
        &self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<MaxPowerRecord>, EnedisError> {
        let path = "/metering_data_dc/v5/daily_consumption_max_power";
        let start_str = to_french_local_time(from).format("%Y-%m-%d").to_string();
        let end_str = to_french_local_time(to).format("%Y-%m-%d").to_string();
        let prm_str = point_id.as_str();

        let query = [
            ("usage_point_id", prm_str),
            ("start", start_str.as_str()),
            ("end", end_str.as_str()),
        ];

        let json = self
            .send_authorized_get(path, &query, Some(point_id))
            .await?;
        let mut records = Vec::new();

        if let Some(meter_reading) = json.get("meter_reading") {
            let unit_str = meter_reading
                .get("reading_type")
                .and_then(|rt| rt.get("unit"))
                .and_then(|u| u.as_str())
                .unwrap_or("VA");
            let unit = Unit::from_sge_code(unit_str);

            if let Some(readings) = meter_reading
                .get("interval_reading")
                .and_then(|r| r.as_array())
            {
                for item in readings {
                    let date_str = item.get("date").and_then(|d| d.as_str()).unwrap_or("");
                    let val_str = item.get("value").and_then(|v| v.as_str()).unwrap_or("");

                    let timestamp = if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
                        dt.with_timezone(&Utc)
                    } else if let Ok(naive) =
                        NaiveDateTime::parse_from_str(date_str, "%Y-%m-%d %H:%M:%S")
                    {
                        from_french_local_time(naive)
                    } else {
                        continue;
                    };

                    let value = Decimal::from_str(val_str).unwrap_or(Decimal::ZERO);

                    records.push(MaxPowerRecord {
                        point_id,
                        timestamp,
                        value,
                        unit,
                    });
                }
            }
        }

        info!(
            "Data Connect : {} pointes de puissance max récupérées pour PRM {}",
            records.len(),
            point_id
        );
        Ok(records)
    }

    /// Récupère le statut et l'échéance du consentement client via l'API Data Connect
    #[instrument(skip(self), fields(prm = %point_id))]
    pub async fn fetch_consent_status(
        &self,
        point_id: PointId,
    ) -> Result<ConsentInfo, EnedisError> {
        let path = format!("/customers_v5/v5/usage_points/{}/consents", point_id);
        let json = self.send_authorized_get(&path, &[], Some(point_id)).await?;

        let mut status = ConsentStatus::Active;
        let mut valid_from = None;
        let mut valid_to = None;
        let mut authorized_usages = Vec::new();

        if let Some(st) = json.get("consent_status").and_then(|v| v.as_str()) {
            status = ConsentStatus::from_sge_code(st);
        } else if let Some(st) = json.pointer("/consent/status").and_then(|v| v.as_str()) {
            status = ConsentStatus::from_sge_code(st);
        }

        if let Some(s) = json.get("start_date").and_then(|v| v.as_str()) {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                valid_from = Some(dt.with_timezone(&Utc));
            }
        }
        if let Some(e) = json.get("end_date").and_then(|v| v.as_str()) {
            if let Ok(dt) = DateTime::parse_from_rfc3339(e) {
                valid_to = Some(dt.with_timezone(&Utc));
            }
        }

        if let Some(usages) = json.get("authorized_usages").and_then(|v| v.as_array()) {
            for u in usages {
                if let Some(s) = u.as_str() {
                    authorized_usages.push(s.to_string());
                }
            }
        }

        Ok(ConsentInfo {
            point_id,
            status,
            valid_from,
            valid_to,
            authorized_usages,
        })
    }
}

impl EnedisProvider for DataConnectClient {
    fn provider_name(&self) -> &'static str {
        "DataConnect-REST-v5"
    }

    fn fetch_measurements<'a>(
        &'a self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        direction: FlowDirection,
    ) -> ProviderFuture<'a, Vec<Measurement>> {
        Box::pin(async move { self.fetch_measurements(point_id, from, to, direction).await })
    }

    fn fetch_contract_data<'a>(&'a self, point_id: PointId) -> ProviderFuture<'a, ContractData> {
        Box::pin(async move { self.fetch_contract_data(point_id).await })
    }

    fn fetch_daily_max_power<'a>(
        &'a self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> ProviderFuture<'a, Vec<MaxPowerRecord>> {
        Box::pin(async move { self.fetch_daily_max_power(point_id, from, to).await })
    }

    fn fetch_consent_status<'a>(&'a self, point_id: PointId) -> ProviderFuture<'a, ConsentInfo> {
        Box::pin(async move { self.fetch_consent_status(point_id).await })
    }
}
