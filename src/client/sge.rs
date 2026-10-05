use crate::client::config::{ClientIdentitySource, SgeClientConfig};
use crate::client::provider::{EnedisProvider, ProviderFuture};
use crate::error::{EnedisError, TransportError};
use crate::models::{
    ConsentInfo, ContractData, FlowDirection, MaxPowerRecord, Measurement, PointId,
};
use crate::xml::{
    build_soap_envelope, parse_soap_response, validate_xml_security, SgeResponseParser,
    XmlSecurityLimits,
};
use chrono::{DateTime, Utc};
use reqwest::{header, Certificate, Client, Identity, StatusCode};
use secrecy::ExposeSecret;
use std::fmt;
use std::fs;
use tracing::{debug, info, instrument};

/// Client asynchrone pour les Web Services Enedis SGE
#[derive(Clone)]
pub struct SgeClient {
    client: Client,
    endpoint: String,
    xml_limits: std::sync::Arc<XmlSecurityLimits>,
}

impl fmt::Debug for SgeClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SgeClient")
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

impl SgeClient {
    /// Initialise un nouveau client SGE avec authentification mTLS et timeouts configurés
    pub fn new(config: SgeClientConfig) -> Result<Self, EnedisError> {
        let mut builder = Client::builder()
            .timeout(config.request_timeout)
            .connect_timeout(config.connect_timeout)
            .user_agent(&config.user_agent)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .pool_max_idle_per_host(10);

        // 1. Configuration mTLS si l'identité client est fournie
        if let Some(ident_source) = &config.identity {
            let identity = match ident_source {
                ClientIdentitySource::Pkcs12File { path, password } => {
                    let der = fs::read(path).map_err(|e| {
                        EnedisError::Configuration(format!(
                            "Échec de lecture du fichier PKCS#12 '{:?}': {}",
                            path, e
                        ))
                    })?;
                    Identity::from_pkcs12_der(&der, password.expose_secret()).map_err(|e| {
                        EnedisError::Tls(format!(
                            "Échec du déchiffrement du certificat PKCS#12 client: {}",
                            e
                        ))
                    })?
                }
                ClientIdentitySource::Pkcs12Der {
                    der_bytes,
                    password,
                } => {
                    Identity::from_pkcs12_der(der_bytes, password.expose_secret()).map_err(|e| {
                        EnedisError::Tls(format!(
                            "Échec du déchiffrement du conteneur PKCS#12 en mémoire: {}",
                            e
                        ))
                    })?
                }
                ClientIdentitySource::PemFile {
                    cert_path,
                    key_path,
                } => {
                    let mut pem_data = fs::read(cert_path).map_err(|e| {
                        EnedisError::Configuration(format!(
                            "Échec de lecture du certificat PEM '{:?}': {}",
                            cert_path, e
                        ))
                    })?;
                    let key_data = fs::read(key_path).map_err(|e| {
                        EnedisError::Configuration(format!(
                            "Échec de lecture de la clé privée PEM '{:?}': {}",
                            key_path, e
                        ))
                    })?;
                    pem_data.push(b'\n');
                    pem_data.extend_from_slice(&key_data);

                    Identity::from_pem(&pem_data).map_err(|e| {
                        EnedisError::Tls(format!("Paire PEM invalide pour mTLS: {}", e))
                    })?
                }
                ClientIdentitySource::PemBytes { cert_pem, key_pem } => {
                    let mut combined = cert_pem.clone();
                    combined.push(b'\n');
                    combined.extend_from_slice(key_pem);

                    Identity::from_pem(&combined).map_err(|e| {
                        EnedisError::Tls(format!("Paires PEM en mémoire invalides: {}", e))
                    })?
                }
            };
            builder = builder.identity(identity);
        }

        // 2. Ajout de la CA personnalisée si nécessaire
        if let Some(ca_bytes) = &config.custom_ca_pem {
            let ca_cert = Certificate::from_pem(ca_bytes).map_err(|e| {
                EnedisError::Tls(format!("CA racine personnalisée invalide: {}", e))
            })?;
            builder = builder.add_root_certificate(ca_cert);
        }

        let client = builder
            .build()
            .map_err(|e| EnedisError::Transport(TransportError::from(e)))?;

        Ok(Self {
            client,
            endpoint: config.endpoint_url,
            xml_limits: std::sync::Arc::new(XmlSecurityLimits::default()),
        })
    }

    /// Envoie une requête SOAP brute avec gestion des en-têtes et sécurité XML
    #[instrument(skip(self, body_payload), fields(endpoint = %self.endpoint, action = %soap_action))]
    pub async fn send_soap_request(
        &self,
        soap_action: &str,
        body_payload: &str,
    ) -> Result<String, EnedisError> {
        let envelope = build_soap_envelope(body_payload, None);
        debug!("Envoi de requête SOAP (taille: {} octets)", envelope.len());

        let res = self
            .client
            .post(&self.endpoint)
            .header(header::CONTENT_TYPE, "text/xml; charset=utf-8")
            .header("SOAPAction", format!("\"{}\"", soap_action))
            .body(envelope)
            .send()
            .await
            .map_err(TransportError::from)?;

        let status = res.status();
        if let Some(cl) = res.content_length() {
            if cl as usize > self.xml_limits.max_size_bytes {
                return Err(EnedisError::XmlSecurity(format!(
                    "Réponse SOAP annoncée trop volumineuse (Content-Length: {} octets, limite: {})",
                    cl, self.xml_limits.max_size_bytes
                )));
            }
        }
        let bytes = res.bytes().await.map_err(TransportError::from)?;

        // 1. Validation de sécurité XML (Billion Laughs / DTD / Taille)
        validate_xml_security(&bytes, &self.xml_limits)?;

        // 2. Gestion spécifique des SOAP Faults (qui arrivent généralement en HTTP 500)
        let parsed_soap = parse_soap_response(&bytes);

        if status.is_success() {
            parsed_soap
        } else if status == StatusCode::INTERNAL_SERVER_ERROR {
            // Si c'est un SOAP Fault bien formé, parse_soap_response retourne déjà EnedisError::Soap
            match parsed_soap {
                Err(EnedisError::Soap(fault)) => Err(EnedisError::Soap(fault)),
                _ => Err(EnedisError::Http {
                    status: status.as_u16(),
                    body: String::from_utf8_lossy(&bytes).into_owned(),
                }),
            }
        } else {
            Err(EnedisError::Http {
                status: status.as_u16(),
                body: String::from_utf8_lossy(&bytes).into_owned(),
            })
        }
    }

    /// Récupère la courbe de charge ou les index pour un PRM donné sur une plage de dates
    #[instrument(skip(self), fields(prm = %point_id, direction = %direction))]
    pub async fn fetch_measurements(
        &self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        direction: FlowDirection,
    ) -> Result<Vec<Measurement>, EnedisError> {
        let request_payload = format!(
            r#"<consulterMesures xmlns="http://www.enedis.fr/sge/ws/v1">
                <pointDeLivraison>{}</pointDeLivraison>
                <sens>{}</sens>
                <dateDebut>{}</dateDebut>
                <dateFin>{}</dateFin>
            </consulterMesures>"#,
            point_id.as_str(),
            direction.as_str(),
            from.to_rfc3339(),
            to.to_rfc3339()
        );

        let soap_action = "http://www.enedis.fr/sge/ws/v1/consulterMesures";
        let response_body = self
            .send_soap_request(soap_action, &request_payload)
            .await?;

        let measurements = SgeResponseParser::parse_measurements(response_body.as_bytes())?;
        info!(
            "Collecte réussie pour PRM {} : {} mesures récupérées",
            point_id,
            measurements.len()
        );

        Ok(measurements)
    }

    /// Récupère les données contractuelles (puissance souscrite, option tarifaire, caractéristiques compteur)
    #[instrument(skip(self), fields(prm = %point_id))]
    pub async fn fetch_contract_data(
        &self,
        point_id: PointId,
    ) -> Result<ContractData, EnedisError> {
        let request_payload = format!(
            r#"<consulterDonneesContractuelles xmlns="http://www.enedis.fr/sge/ws/v1">
                <pointDeLivraison>{}</pointDeLivraison>
            </consulterDonneesContractuelles>"#,
            point_id.as_str()
        );

        let soap_action = "http://www.enedis.fr/sge/ws/v1/consulterDonneesContractuelles";
        let response_body = self
            .send_soap_request(soap_action, &request_payload)
            .await?;

        let contract = SgeResponseParser::parse_contract_data(response_body.as_bytes())?;
        info!(
            "Données contractuelles récupérées pour PRM {} (Puissance: {} kVA, Option: {:?})",
            point_id, contract.subscribed_power_kva, contract.tariff_option
        );

        Ok(contract)
    }

    /// Récupère l'historique des pointes maximales quotidiennes de puissance atteinte (W / kVA)
    #[instrument(skip(self), fields(prm = %point_id))]
    pub async fn fetch_daily_max_power(
        &self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<MaxPowerRecord>, EnedisError> {
        let request_payload = format!(
            r#"<consulterPuissanceMax xmlns="http://www.enedis.fr/sge/ws/v1">
                <pointDeLivraison>{}</pointDeLivraison>
                <dateDebut>{}</dateDebut>
                <dateFin>{}</dateFin>
            </consulterPuissanceMax>"#,
            point_id.as_str(),
            from.to_rfc3339(),
            to.to_rfc3339()
        );

        let soap_action = "http://www.enedis.fr/sge/ws/v1/consulterPuissanceMax";
        let response_body = self
            .send_soap_request(soap_action, &request_payload)
            .await?;

        let records = SgeResponseParser::parse_max_power(response_body.as_bytes())?;
        info!(
            "Puissance maximale atteinte pour PRM {} : {} pointes relevées",
            point_id,
            records.len()
        );

        Ok(records)
    }

    /// Vérifie et récupère l'état et le cycle de vie du consentement client
    #[instrument(skip(self), fields(prm = %point_id))]
    pub async fn fetch_consent_status(
        &self,
        point_id: PointId,
    ) -> Result<ConsentInfo, EnedisError> {
        let request_payload = format!(
            r#"<consulterConsentement xmlns="http://www.enedis.fr/sge/ws/v1">
                <pointDeLivraison>{}</pointDeLivraison>
            </consulterConsentement>"#,
            point_id.as_str()
        );

        let soap_action = "http://www.enedis.fr/sge/ws/v1/consulterConsentement";
        let response_body = self
            .send_soap_request(soap_action, &request_payload)
            .await?;

        let consent = SgeResponseParser::parse_consent_status(response_body.as_bytes())?;
        info!(
            "Statut du consentement pour PRM {} : {:?} (Échéance: {:?})",
            point_id, consent.status, consent.valid_to
        );

        Ok(consent)
    }
}

impl EnedisProvider for SgeClient {
    fn provider_name(&self) -> &'static str {
        "SGE-SOAP-mTLS"
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
