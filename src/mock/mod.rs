use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Scénario de réponse du simulateur SGE / Data Connect
#[derive(Clone, Debug)]
pub enum MockScenario {
    /// Succès nominal avec des mesures réalistes
    Success,
    /// Erreur transitoire (503 Service Unavailable) pendant `fail_count` requêtes, puis succès
    TransientFailureThenSuccess { fail_count: usize },
    /// Erreur SOAP Fault (HTTP 500 avec XML de faute)
    SoapFault { code: String, message: String },
    /// Erreur fonctionnelle SGE (HTTP 200 contenant un bloc <erreurFonctionnelle>)
    BusinessConsentExpired,
    /// Erreur PRM inconnu
    BusinessPointNotFound,
    /// Quota dépassé (HTTP 429)
    QuotaExceeded,
}

#[derive(Clone)]
struct MockState {
    scenario: MockScenario,
    request_counter: Arc<AtomicUsize>,
}

/// Serveur de mock HTTP pour les Web Services SGE et l'API Data Connect d'Enedis
pub struct MockSgeServer {
    addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    request_counter: Arc<AtomicUsize>,
}

impl MockSgeServer {
    /// Démarre le serveur mock sur un port éphémère disponible
    pub async fn start(scenario: MockScenario) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Impossible d'allouer un port éphémère pour mock-sge");
        let addr = listener
            .local_addr()
            .expect("Impossible d'obtenir l'adresse locale du mock-sge");

        let request_counter = Arc::new(AtomicUsize::new(0));
        let state = MockState {
            scenario,
            request_counter: Arc::clone(&request_counter),
        };

        let app = Router::new()
            // Endpoints Web Services SOAP SGE
            .route("/", post(handle_soap_request))
            .route("/services/", post(handle_soap_request))
            // Endpoints REST Enedis Data Connect v5
            .route("/oauth2/v3/authorize", get(handle_mock_authorize))
            .route("/oauth2/v3/token", post(handle_oauth2_token))
            .route(
                "/metering_data_dc/v5/consumption_load_curve",
                get(handle_dc_consumption),
            )
            .route(
                "/metering_data_dc/v5/production_load_curve",
                get(handle_dc_production),
            )
            .route(
                "/metering_data_dc/v5/daily_consumption_max_power",
                get(handle_dc_max_power),
            )
            .route(
                "/customers_v5/v5/usage_points/:prm/contract",
                get(handle_dc_contract),
            )
            .route(
                "/customers_v5/v5/usage_points/:prm/consents",
                get(handle_dc_consents),
            )
            .with_state(state);

        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await
                .ok();
        });

        Self {
            addr,
            shutdown_tx: Some(shutdown_tx),
            request_counter,
        }
    }

    /// URL racine à fournir au `SgeClientConfig`
    pub fn endpoint_url(&self) -> String {
        format!("http://{}/services/", self.addr)
    }

    /// URL de base pour `DataConnectConfig`
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// URL OAuth2 token pour `DataConnectConfig`
    pub fn token_url(&self) -> String {
        format!("http://{}/oauth2/v3/token", self.addr)
    }

    /// URL OAuth2 authorize pour `DataConnectConfig`
    pub fn authorize_url(&self) -> String {
        format!("http://{}/oauth2/v3/authorize", self.addr)
    }

    /// Nombre de requêtes reçues par le mock
    pub fn total_requests(&self) -> usize {
        self.request_counter.load(Ordering::SeqCst)
    }

    /// Arrête le serveur mock
    pub fn stop(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }

    /// Démarre le serveur mock sur une adresse spécifique et attend l'arrêt gracieux
    pub async fn run_server(
        addr: SocketAddr,
        scenario: MockScenario,
        shutdown: oneshot::Receiver<()>,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(addr).await?;
        let request_counter = Arc::new(AtomicUsize::new(0));
        let state = MockState {
            scenario,
            request_counter,
        };

        let app = Router::new()
            .route("/", post(handle_soap_request))
            .route("/services/", post(handle_soap_request))
            .route("/oauth2/v3/authorize", get(handle_mock_authorize))
            .route("/oauth2/v3/token", post(handle_oauth2_token))
            .route(
                "/metering_data_dc/v5/consumption_load_curve",
                get(handle_dc_consumption),
            )
            .route(
                "/metering_data_dc/v5/production_load_curve",
                get(handle_dc_production),
            )
            .route(
                "/metering_data_dc/v5/daily_consumption_max_power",
                get(handle_dc_max_power),
            )
            .route(
                "/customers_v5/v5/usage_points/:prm/contract",
                get(handle_dc_contract),
            )
            .route(
                "/customers_v5/v5/usage_points/:prm/consents",
                get(handle_dc_consents),
            )
            .with_state(state);

        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown.await;
            })
            .await?;

        Ok(())
    }
}

impl std::str::FromStr for MockScenario {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace('_', "-").as_str() {
            "success" | "nominal" | "ok" => Ok(MockScenario::Success),
            "consent-expired" | "consent" => Ok(MockScenario::BusinessConsentExpired),
            "point-not-found" | "not-found" => Ok(MockScenario::BusinessPointNotFound),
            "quota-exceeded" | "quota" | "rate-limit" => Ok(MockScenario::QuotaExceeded),
            other => Err(format!(
                "Scénario inconnu: '{}'. Valeurs valides: success, consent-expired, point-not-found, quota-exceeded",
                other
            )),
        }
    }
}

impl Drop for MockSgeServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

async fn handle_soap_request(
    State(state): State<MockState>,
    _headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    let count = state.request_counter.fetch_add(1, Ordering::SeqCst);

    // Extraction basique du PRM présent dans le corps de requête
    let prm = extract_prm_from_payload(&body).unwrap_or("01234567890123".to_string());

    match &state.scenario {
        MockScenario::Success => {
            let xml = if body.contains("consulterDonneesContractuelles") {
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
                    <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
                        <soapenv:Header/>
                        <soapenv:Body>
                            <consulterDonneesContractuellesResponse xmlns="http://www.enedis.fr/sge/ws/v1">
                                <pointDeLivraison>{}</pointDeLivraison>
                                <donneesContractuelles>
                                    <puissanceSouscrite>9</puissanceSouscrite>
                                    <optionTarifaire>HEURES_PLEINES_HEURES_CREUSES</optionTarifaire>
                                    <statutContrat>ACTIF</statutContrat>
                                    <calendrier>
                                        <libelle>Option Creuse SGE</libelle>
                                        <plagesHeuresCreuses>22h00-06h00</plagesHeuresCreuses>
                                    </calendrier>
                                    <caracteristiquesCompteur>
                                        <numeroSerie>211975001234</numeroSerie>
                                        <typeCompteur>LINKY</typeCompteur>
                                        <nombrePhases>MONOPHASE</nombrePhases>
                                        <calibreDisjoncteur>45</calibreDisjoncteur>
                                    </caracteristiquesCompteur>
                                </donneesContractuelles>
                            </consulterDonneesContractuellesResponse>
                        </soapenv:Body>
                    </soapenv:Envelope>"#,
                    prm
                )
            } else if body.contains("consulterPuissanceMax") {
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
                    <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
                        <soapenv:Header/>
                        <soapenv:Body>
                            <consulterPuissanceMaxResponse xmlns="http://www.enedis.fr/sge/ws/v1">
                                <pointDeLivraison>{}</pointDeLivraison>
                                <pointesPuissance>
                                    <pointe>
                                        <date>2026-09-28T19:30:00Z</date>
                                        <valeur>4820</valeur>
                                        <unite>W</unite>
                                    </pointe>
                                    <pointe>
                                        <date>2026-09-29T20:15:00Z</date>
                                        <valeur>5200</valeur>
                                        <unite>VA</unite>
                                    </pointe>
                                </pointesPuissance>
                            </consulterPuissanceMaxResponse>
                        </soapenv:Body>
                    </soapenv:Envelope>"#,
                    prm
                )
            } else if body.contains("consulterConsentement") {
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
                    <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
                        <soapenv:Header/>
                        <soapenv:Body>
                            <consulterConsentementResponse xmlns="http://www.enedis.fr/sge/ws/v1">
                                <pointDeLivraison>{}</pointDeLivraison>
                                <consentement>
                                    <etat>ACTIF</etat>
                                    <dateDebut>2026-01-01T00:00:00Z</dateDebut>
                                    <dateFin>2027-01-01T00:00:00Z</dateFin>
                                    <usage>COURBE_DE_CHARGE</usage>
                                    <usage>PUISSANCE_MAX</usage>
                                    <usage>DONNEES_CONTRACTUELLES</usage>
                                </consentement>
                            </consulterConsentementResponse>
                        </soapenv:Body>
                    </soapenv:Envelope>"#,
                    prm
                )
            } else {
                let (ts1, ts2) = if let Some(dt) = extract_date_debut_from_payload(&body) {
                    let end_ts = if let Some(end_dt) = extract_date_fin_from_payload(&body) {
                        if end_dt > dt + chrono::Duration::minutes(30) {
                            end_dt - chrono::Duration::minutes(30)
                        } else {
                            dt + chrono::Duration::minutes(30)
                        }
                    } else {
                        dt + chrono::Duration::minutes(30)
                    };
                    (dt.to_rfc3339(), end_ts.to_rfc3339())
                } else {
                    (
                        "2026-09-28T08:00:00Z".to_string(),
                        "2026-09-28T08:30:00Z".to_string(),
                    )
                };
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
                    <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
                        <soapenv:Header/>
                        <soapenv:Body>
                            <consulterMesuresResponse xmlns="http://www.enedis.fr/sge/ws/v1">
                                <prm>{}</prm>
                                <sens>Consommation</sens>
                                <pas>1800</pas>
                                <courbeDeCharge>
                                    <mesure>
                                        <timestamp>{}</timestamp>
                                        <valeur>1.2500</valeur>
                                        <unite>kWh</unite>
                                        <qualite>ESTIME</qualite>
                                    </mesure>
                                    <mesure>
                                        <timestamp>{}</timestamp>
                                        <valeur>1.4100</valeur>
                                        <unite>kWh</unite>
                                        <qualite>MESURE</qualite>
                                    </mesure>
                                </courbeDeCharge>
                            </consulterMesuresResponse>
                        </soapenv:Body>
                    </soapenv:Envelope>"#,
                    prm, ts1, ts2
                )
            };
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/xml; charset=utf-8")],
                xml,
            )
        }
        MockScenario::TransientFailureThenSuccess { fail_count } => {
            if count < *fail_count {
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    [(header::CONTENT_TYPE, "text/plain")],
                    "Service SGE Enedis temporairement indisponible".to_string(),
                )
            } else {
                let ts = if let Some(dt) = extract_date_debut_from_payload(&body) {
                    (dt + chrono::Duration::minutes(30)).to_rfc3339()
                } else {
                    "2026-09-28T09:00:00Z".to_string()
                };
                let xml = format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
                    <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
                        <soapenv:Header/>
                        <soapenv:Body>
                            <consulterMesuresResponse xmlns="http://www.enedis.fr/sge/ws/v1">
                                <prm>{}</prm>
                                <sens>Consommation</sens>
                                <pas>1800</pas>
                                <courbeDeCharge>
                                    <mesure>
                                        <timestamp>{}</timestamp>
                                        <valeur>0.7500</valeur>
                                        <unite>kWh</unite>
                                        <qualite>MESURE</qualite>
                                    </mesure>
                                </courbeDeCharge>
                            </consulterMesuresResponse>
                        </soapenv:Body>
                    </soapenv:Envelope>"#,
                    prm, ts
                );
                (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, "text/xml; charset=utf-8")],
                    xml,
                )
            }
        }
        MockScenario::SoapFault { code, message } => {
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
                    <soapenv:Body>
                        <soapenv:Fault>
                            <faultcode>{}</faultcode>
                            <faultstring>{}</faultstring>
                            <detail>Erreur simulee par le Mock SGE</detail>
                        </soapenv:Fault>
                    </soapenv:Body>
                </soapenv:Envelope>"#,
                code, message
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/xml; charset=utf-8")],
                xml,
            )
        }
        MockScenario::BusinessConsentExpired => {
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
                    <soapenv:Body>
                        <consulterMesuresResponse xmlns="http://www.enedis.fr/sge/ws/v1">
                            <erreurFonctionnelle>
                                <prm>{}</prm>
                                <codeErreur>SGE_CONS_01</codeErreur>
                                <libelleErreur>Consentement client absent ou expire</libelleErreur>
                            </erreurFonctionnelle>
                        </consulterMesuresResponse>
                    </soapenv:Body>
                </soapenv:Envelope>"#,
                prm
            );
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/xml; charset=utf-8")],
                xml,
            )
        }
        MockScenario::BusinessPointNotFound => {
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
                    <soapenv:Body>
                        <consulterMesuresResponse xmlns="http://www.enedis.fr/sge/ws/v1">
                            <erreurFonctionnelle>
                                <prm>{}</prm>
                                <codeErreur>SGE_PRM_INTROUVABLE</codeErreur>
                                <libelleErreur>Point de livraison inconnu dans le perimetre</libelleErreur>
                            </erreurFonctionnelle>
                        </consulterMesuresResponse>
                    </soapenv:Body>
                </soapenv:Envelope>"#,
                prm
            );
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "text/xml; charset=utf-8")],
                xml,
            )
        }
        MockScenario::QuotaExceeded => (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::CONTENT_TYPE, "text/plain")],
            "Limite de requetes SGE Enedis atteinte (HTTP 429)".to_string(),
        ),
    }
}

/// Alias pour `MockSgeServer`, servant simultanément les protocoles SOAP SGE et REST Data Connect
pub type MockDataConnectServer = MockSgeServer;

impl MockSgeServer {
    /// URL racine pour Enedis Data Connect
    pub fn data_connect_url(&self) -> String {
        self.base_url()
    }

    /// URL d'obtention de jeton OAuth2 pour Enedis Data Connect
    pub fn data_connect_token_url(&self) -> String {
        self.token_url()
    }
}

// Handlers Mock Data Connect REST v5

fn check_dc_scenario(state: &MockState) -> Option<axum::response::Response> {
    let count = state.request_counter.fetch_add(1, Ordering::SeqCst);
    match &state.scenario {
        MockScenario::Success => None,
        MockScenario::TransientFailureThenSuccess { fail_count } => {
            if count < *fail_count {
                Some(
                    (
                        StatusCode::SERVICE_UNAVAILABLE,
                        [(header::CONTENT_TYPE, "application/json")],
                        r#"{"error":"temporarily_unavailable","error_description":"Service Data Connect temporairement indisponible"}"#,
                    )
                        .into_response(),
                )
            } else {
                None
            }
        }
        MockScenario::SoapFault { code, message } => Some(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                format!(r#"{{"error":"{}","error_description":"{}"}}"#, code, message),
            )
                .into_response(),
        ),
        MockScenario::BusinessConsentExpired => Some(
            (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, "application/json")],
                r#"{"tag":"ADSP_403","error":"access_denied","error_description":"Consentement client absent ou expire"}"#,
            )
                .into_response(),
        ),
        MockScenario::BusinessPointNotFound => Some(
            (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, "application/json")],
                r#"{"tag":"ADSP_404","error":"not_found","error_description":"Point de livraison introuvable"}"#,
            )
                .into_response(),
        ),
        MockScenario::QuotaExceeded => Some(
            (
                StatusCode::TOO_MANY_REQUESTS,
                [(header::CONTENT_TYPE, "application/json")],
                r#"{"tag":"ADSP_429","error":"too_many_requests","error_description":"Quota de requetes Data Connect depasse"}"#,
            )
                .into_response(),
        ),
    }
}

#[derive(serde::Deserialize)]
struct MockAuthorizeQuery {
    #[allow(dead_code)]
    client_id: Option<String>,
    #[allow(dead_code)]
    response_type: Option<String>,
    state: Option<String>,
    redirect_uri: Option<String>,
    #[allow(dead_code)]
    duration: Option<String>,
}

async fn handle_mock_authorize(
    State(state): State<MockState>,
    Query(q): Query<MockAuthorizeQuery>,
) -> impl IntoResponse {
    state.request_counter.fetch_add(1, Ordering::SeqCst);
    let redirect_uri = q
        .redirect_uri
        .unwrap_or_else(|| "http://localhost:8080/consent/callback".to_string());
    let st = q.state.unwrap_or_default();
    let code = "mock_auth_code_789";
    let prm = "01234567890123";

    let sep = if redirect_uri.contains('?') { "&" } else { "?" };
    let target = format!(
        "{}{}code={}&state={}&usage_point_id={}",
        redirect_uri, sep, code, st, prm
    );

    (
        StatusCode::FOUND,
        [(header::LOCATION, target)],
        "Redirecting to callback...",
    )
}

async fn handle_oauth2_token(State(state): State<MockState>) -> impl IntoResponse {
    state.request_counter.fetch_add(1, Ordering::SeqCst);
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"access_token":"mock-dataconnect-bearer-token","token_type":"Bearer","expires_in":3600,"usage_point_id":"01234567890123"}"#,
    )
}

async fn handle_dc_consumption(State(state): State<MockState>) -> axum::response::Response {
    if let Some(err_resp) = check_dc_scenario(&state) {
        return err_resp;
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{
            "meter_reading": {
                "usage_point_id": "01234567890123",
                "start": "2026-09-28",
                "end": "2026-09-29",
                "reading_type": { "unit": "W", "interval_length": "1800" },
                "interval_reading": [
                    { "date": "2026-09-28 08:00:00", "value": "1250" },
                    { "date": "2026-09-28 08:30:00", "value": "1410" }
                ]
            }
        }"#,
    )
        .into_response()
}

async fn handle_dc_production(State(state): State<MockState>) -> axum::response::Response {
    if let Some(err_resp) = check_dc_scenario(&state) {
        return err_resp;
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{
            "meter_reading": {
                "usage_point_id": "01234567890123",
                "start": "2026-09-28",
                "end": "2026-09-29",
                "reading_type": { "unit": "W", "interval_length": "1800" },
                "interval_reading": [
                    { "date": "2026-09-28 12:00:00", "value": "2400" },
                    { "date": "2026-09-28 12:30:00", "value": "2600" }
                ]
            }
        }"#,
    )
        .into_response()
}

async fn handle_dc_max_power(State(state): State<MockState>) -> axum::response::Response {
    if let Some(err_resp) = check_dc_scenario(&state) {
        return err_resp;
    }
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{
            "meter_reading": {
                "usage_point_id": "01234567890123",
                "reading_type": { "unit": "VA" },
                "interval_reading": [
                    { "date": "2026-09-28 19:30:00", "value": "4820" },
                    { "date": "2026-09-29 20:15:00", "value": "5200" }
                ]
            }
        }"#,
    )
        .into_response()
}

async fn handle_dc_contract(
    State(state): State<MockState>,
    Path(prm): Path<String>,
) -> axum::response::Response {
    if let Some(err_resp) = check_dc_scenario(&state) {
        return err_resp;
    }
    let json = format!(
        r#"{{
            "customer": {{
                "usage_points": [
                    {{
                        "usage_point": {{
                            "usage_point_id": "{}",
                            "contract": {{
                                "subscribed_power": "9 kVA",
                                "pricing_system": "HEURES_PLEINES_HEURES_CREUSES",
                                "offpeak_hours": "HC (22H00-6H00)",
                                "contract_status": "ACTIF"
                            }},
                            "meter": {{
                                "serial_number": "211975001234",
                                "meter_type": "LINKY",
                                "phase_count": "MONOPHASE"
                            }}
                        }}
                    }}
                ]
            }}
        }}"#,
        prm
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json,
    )
        .into_response()
}

async fn handle_dc_consents(
    State(state): State<MockState>,
    Path(prm): Path<String>,
) -> axum::response::Response {
    if let Some(err_resp) = check_dc_scenario(&state) {
        return err_resp;
    }
    let json = format!(
        r#"{{
            "usage_point_id": "{}",
            "consent_status": "ACTIVE",
            "start_date": "2026-01-01T00:00:00Z",
            "end_date": "2027-01-01T00:00:00Z",
            "authorized_usages": ["consumption_load_curve", "daily_consumption_max_power"]
        }}"#,
        prm
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        json,
    )
        .into_response()
}

fn extract_prm_from_payload(payload: &str) -> Option<String> {
    if let Some(start) = payload.find("<pointDeLivraison>") {
        let after = &payload[start + "<pointDeLivraison>".len()..];
        if let Some(end) = after.find("</pointDeLivraison>") {
            return Some(after[..end].trim().to_string());
        }
    }
    None
}

fn extract_date_debut_from_payload(payload: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Some(start) = payload.find("<dateDebut>") {
        let after = &payload[start + "<dateDebut>".len()..];
        if let Some(end) = after.find("</dateDebut>") {
            let s = after[..end].trim();
            return chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc));
        }
    }
    None
}

fn extract_date_fin_from_payload(payload: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Some(start) = payload.find("<dateFin>") {
        let after = &payload[start + "<dateFin>".len()..];
        if let Some(end) = after.find("</dateFin>") {
            let s = after[..end].trim();
            return chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|dt| dt.with_timezone(&chrono::Utc));
        }
    }
    None
}
