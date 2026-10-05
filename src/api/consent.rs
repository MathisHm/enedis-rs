use crate::api::routes::{ApiErrorResponse, AppState};
use crate::models::{FlowDirection, PointId};
use crate::storage::SyncState;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// Génère un jeton CSRF unique et cryptographiquement sécurisé pour lier l'autorisation et le callback OAuth2
pub fn generate_csrf_state() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{:02x}", b);
    }
    out
}

/// Session de consentement en attente de validation lors du retour OAuth2
#[derive(Clone, Debug)]
pub struct ConsentSession {
    pub created_at: DateTime<Utc>,
    pub redirect_uri: String,
}

/// Gestionnaire thread-safe des sessions CSRF et des redirections de consentement
#[derive(Clone, Default)]
pub struct PendingConsentStore {
    sessions: Arc<RwLock<HashMap<String, ConsentSession>>>,
}

impl PendingConsentStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Enregistre une nouvelle session en attente (expire après 15 minutes)
    pub async fn create_session(&self, state: String, redirect_uri: String) {
        let mut map = self.sessions.write().await;
        // Purge automatique des sessions expirées (> 15 min)
        let cutoff = Utc::now() - chrono::Duration::minutes(15);
        map.retain(|_, v| v.created_at > cutoff);

        map.insert(
            state,
            ConsentSession {
                created_at: Utc::now(),
                redirect_uri,
            },
        );
    }

    /// Valide et consomme le jeton CSRF de manière atomique (usage unique)
    pub async fn validate_and_consume(&self, state: &str) -> Option<ConsentSession> {
        let mut map = self.sessions.write().await;
        let session = map.remove(state)?;
        if Utc::now() - session.created_at < chrono::Duration::minutes(15) {
            Some(session)
        } else {
            None
        }
    }
}

/// Paramètres de requête reçus sur `GET /consent/authorize`
#[derive(Debug, Deserialize)]
pub struct AuthorizeQueryParams {
    pub redirect_uri: Option<String>,
    pub duration: Option<String>,
}

/// Paramètres de requête reçus sur `GET /consent/callback` depuis Enedis
#[derive(Debug, Deserialize)]
pub struct CallbackQueryParams {
    pub code: Option<String>,
    pub state: Option<String>,
    pub usage_point_id: Option<String>,
    pub prm: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Réponse retournée par l'endpoint JSON `GET /api/v1/consent/url`
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize, Deserialize)]
pub struct ConsentUrlResponse {
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "https://mon-compte-client.enedis.fr/dataconnect/v1/oauth2/authorize?..."
        )
    )]
    pub authorize_url: String,
    #[cfg_attr(feature = "openapi", schema(example = "1a2b3c4d5e6f7a8b"))]
    pub state: String,
    #[cfg_attr(
        feature = "openapi",
        schema(example = "http://localhost:8080/consent/callback")
    )]
    pub redirect_uri: String,
}

/// Corps de requête pour l'échange programmatique `POST /api/v1/consent/exchange`
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct ConsentExchangeRequest {
    #[cfg_attr(feature = "openapi", schema(example = "auth_code_xyz123"))]
    pub code: String,
    #[cfg_attr(
        feature = "openapi",
        schema(example = "http://localhost:8080/consent/callback")
    )]
    pub redirect_uri: String,
    #[cfg_attr(feature = "openapi", schema(example = "1a2b3c4d5e6f7a8b"))]
    pub state: Option<String>,
    #[cfg_attr(feature = "openapi", schema(example = "01234567890123"))]
    pub usage_point_id: Option<String>,
}

/// Réponse retournée par `POST /api/v1/consent/exchange`
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Serialize)]
pub struct ConsentExchangeResponse {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "01234567890123"))]
    pub point_id: PointId,
    #[cfg_attr(feature = "openapi", schema(example = "CONSENT_ACQUIRED"))]
    pub status: String,
    pub message: String,
    pub token_expires_in: Option<i64>,
}

/// Échappe les caractères spéciaux HTML pour se prémunir contre les failles XSS
pub fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Détermine l'URL de callback publique en inspectant les en-têtes HTTP de la requête (Host, X-Forwarded-Proto)
fn resolve_redirect_uri(headers: &HeaderMap, query_redirect: Option<&str>) -> String {
    let host = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .filter(|h| {
            !h.contains('/')
                && !h.contains('\\')
                && !h.contains('@')
                && !h.contains('\n')
                && !h.contains('\r')
        })
        .unwrap_or("localhost:8080");

    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .filter(|p| *p == "http" || *p == "https")
        .unwrap_or_else(|| {
            if host.starts_with("localhost") || host.starts_with("127.0.0.1") {
                "http"
            } else {
                "https"
            }
        });

    if let Some(uri) = query_redirect {
        if !uri.is_empty() {
            // Validation stricte de sécurité : l'URL personnalisée doit être absolue, en http/https,
            // se terminer par le chemin autorisé de callback, et correspondre à l'hôte autorisé.
            if let Ok(parsed) = reqwest::Url::parse(uri) {
                if (parsed.scheme() == "http" || parsed.scheme() == "https")
                    && (parsed.path().ends_with("/consent/callback")
                        || parsed.path() == "/consent/callback")
                {
                    let host_matches = match parsed.host_str() {
                        Some(h) => {
                            let expected_clean = host.split(':').next().unwrap_or(host);
                            h.eq_ignore_ascii_case(expected_clean)
                                || h.eq_ignore_ascii_case("localhost")
                                || h == "127.0.0.1"
                        }
                        None => false,
                    };

                    if host_matches {
                        return uri.to_string();
                    } else {
                        warn!("URL redirect_uri rejetée (hôte non autorisé) : {}", uri);
                    }
                } else {
                    warn!(
                        "URL redirect_uri rejetée (doit se terminer par /consent/callback) : {}",
                        uri
                    );
                }
            }
        }
    }

    format!("{}://{}/consent/callback", proto, host)
}

/// Handler HTTP pour initier le tunnel de consentement : `GET /consent/authorize`
pub async fn consent_authorize_handler(
    State(state): State<AppState>,
    Query(params): Query<AuthorizeQueryParams>,
    headers: HeaderMap,
) -> Response {
    #[cfg(feature = "client")]
    {
        let redirect_uri = resolve_redirect_uri(&headers, params.redirect_uri.as_deref());
        let csrf_state = generate_csrf_state();

        // Si aucun client Data Connect n'est initialisé
        let dc_client = match state.data_connect_client.as_ref() {
            Some(c) => c,
            None => {
                let html = render_consent_error_html(
                    "Configuration Data Connect manquante",
                    "Le client OAuth2 Enedis Data Connect n'est pas initialisé sur ce serveur.",
                    "Définissez les variables d'environnement ENEDIS_DATA_CONNECT_CLIENT_ID et ENEDIS_DATA_CONNECT_CLIENT_SECRET, ou activez le simulateur avec --provider dataconnect.",
                );
                return (StatusCode::BAD_REQUEST, Html(html)).into_response();
            }
        };

        // Enregistrement de la session CSRF
        state
            .consent_manager
            .create_session(csrf_state.clone(), redirect_uri.clone())
            .await;

        match dc_client.get_authorize_url(&redirect_uri, &csrf_state, params.duration.as_deref()) {
            Ok(auth_url) => {
                info!(
                    "Redirection vers l'autorisation Enedis OAuth2 (redirect_uri: {}, state: {})",
                    redirect_uri, csrf_state
                );
                Redirect::to(&auth_url).into_response()
            }
            Err(err) => {
                let html = render_consent_error_html(
                    "Erreur de génération du lien d'autorisation",
                    &err.to_string(),
                    "Vérifiez que votre client_id et redirect_uri sont déclarés dans le portail Enedis Développeur.",
                );
                (StatusCode::INTERNAL_SERVER_ERROR, Html(html)).into_response()
            }
        }
    }

    #[cfg(not(feature = "client"))]
    {
        let _ = (state, params, headers);
        (
            StatusCode::NOT_IMPLEMENTED,
            Html("<h1>Fonctionnalité client désactivée</h1>"),
        )
            .into_response()
    }
}

/// Handler HTTP pour la réception du retour Enedis : `GET /consent/callback`
pub async fn consent_callback_handler(
    State(state): State<AppState>,
    Query(params): Query<CallbackQueryParams>,
) -> Response {
    #[cfg(feature = "client")]
    {
        // 1. Vérification si Enedis a retourné une erreur (ex: refus du consentement par l'usager)
        if let Some(err_code) = params.error {
            let desc = params
                .error_description
                .unwrap_or_else(|| "L'autorisation a été interrompue ou refusée.".to_string());
            warn!(
                "Consentement refusé ou interrompu par Enedis : {} - {}",
                err_code, desc
            );
            let html = render_consent_error_html(
                "Consentement non accordé",
                &format!("Enedis a retourné : {} ({})", err_code, desc),
                "Vous pouvez relancer l'autorisation à tout moment lorsque vous souhaitez connecter votre compteur Linky.",
            );
            return (StatusCode::OK, Html(html)).into_response();
        }

        // 2. Vérification des paramètres requis
        let code = match params.code {
            Some(c) if !c.is_empty() => c,
            _ => {
                let html = render_consent_error_html(
                    "Paramètre de code manquant",
                    "La réponse Enedis ne contient aucun paramètre 'code'.",
                    "Vérifiez l'URL de rappel configurée dans votre portail Enedis.",
                );
                return (StatusCode::BAD_REQUEST, Html(html)).into_response();
            }
        };

        let csrf_state = match params.state {
            Some(s) if !s.is_empty() => s,
            _ => {
                let html = render_consent_error_html(
                    "Paramètre de sécurité manquant",
                    "Le paramètre anti-CSRF 'state' est absent.",
                    "Veuillez initier le flux directement depuis le tableau de bord.",
                );
                return (StatusCode::BAD_REQUEST, Html(html)).into_response();
            }
        };

        // 3. Validation CSRF
        let session = match state
            .consent_manager
            .validate_and_consume(&csrf_state)
            .await
        {
            Some(s) => s,
            None => {
                warn!(
                    "Tentative de callback avec jeton CSRF invalide ou expiré : {}",
                    csrf_state
                );
                let html = render_consent_error_html(
                    "Session de consentement expirée",
                    "La requête d'autorisation a expiré ou le jeton de sécurité ne correspond pas.",
                    "Pour des raisons de sécurité, chaque lien de consentement est valable 15 minutes. Veuillez recommencer.",
                );
                return (StatusCode::FORBIDDEN, Html(html)).into_response();
            }
        };

        // 4. Client Data Connect requis
        let dc_client = match state.data_connect_client.as_ref() {
            Some(c) => c,
            None => {
                let html = render_consent_error_html(
                    "Client Data Connect non configuré",
                    "Impossible d'échanger le code d'autorisation sans configuration client active.",
                    "Définissez vos identifiants Data Connect sur le serveur.",
                );
                return (StatusCode::INTERNAL_SERVER_ERROR, Html(html)).into_response();
            }
        };

        // 5. Échange du code d'autorisation contre le jeton Bearer
        let token_resp = match dc_client
            .exchange_authorization_code(&code, &session.redirect_uri)
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                error!(
                    "Erreur lors de l'échange OAuth2 du code d'autorisation : {}",
                    e
                );
                let html = render_consent_error_html(
                    "Échec d'obtention du jeton Enedis",
                    &format!("L'échange OAuth2 a échoué : {}", e),
                    "Vérifiez que votre client_secret est valide et que le code n'a pas déjà été consommé.",
                );
                return (StatusCode::BAD_GATEWAY, Html(html)).into_response();
            }
        };

        // 6. Extraction du PointId (PRM)
        let prm_str = params
            .usage_point_id
            .or(params.prm)
            .or(token_resp.usage_point_id);

        let prm = match prm_str {
            Some(ref s) => match PointId::new(s) {
                Ok(p) => p,
                Err(e) => {
                    let html = render_consent_error_html(
                        "Numéro de PRM invalide",
                        &format!("Le PRM '{}' retourné n'est pas conforme : {}", s, e),
                        "Un Point de Référence et de Mesure Enedis doit contenir exactement 14 chiffres.",
                    );
                    return (StatusCode::BAD_REQUEST, Html(html)).into_response();
                }
            },
            None => {
                let html = render_consent_error_html(
                    "Identifiant de compteur introuvable",
                    "Enedis n'a renvoyé aucun numéro de PRM dans l'autorisation ou le jeton.",
                    "Assurez-vous d'avoir bien sélectionné au moins un compteur Linky lors de l'étape de consentement.",
                );
                return (StatusCode::BAD_REQUEST, Html(html)).into_response();
            }
        };

        // 7. Enregistrement automatique du PRM dans le StorageBackend
        let sync_state = SyncState {
            point_id: prm,
            direction: FlowDirection::Consumption,
            last_synced_timestamp: Utc::now() - chrono::Duration::days(365),
            last_sync_attempt: Utc::now(),
            sync_status: "CONSENT_ACQUIRED".to_string(),
        };

        if let Err(e) = state.storage.update_sync_state(&sync_state).await {
            error!(
                "Erreur lors de l'enregistrement du PRM en base de données : {}",
                e
            );
            let html = render_consent_error_html(
                "Erreur de persistance",
                &format!(
                    "Le consentement est validé mais le compteur n'a pu être stocké : {}",
                    e
                ),
                "Vérifiez la connexion à votre base de données.",
            );
            return (StatusCode::INTERNAL_SERVER_ERROR, Html(html)).into_response();
        }

        info!(
            "PRM {} enregistré avec succès suite au consentement client Enedis !",
            prm
        );

        let html = render_consent_success_html(&prm);
        (StatusCode::OK, Html(html)).into_response()
    }

    #[cfg(not(feature = "client"))]
    {
        let _ = (state, params);
        (
            StatusCode::NOT_IMPLEMENTED,
            Html("<h1>Fonctionnalité client désactivée</h1>"),
        )
            .into_response()
    }
}

/// Endpoint JSON : `GET /api/v1/consent/url`
pub async fn consent_url_api_handler(
    State(state): State<AppState>,
    Query(params): Query<AuthorizeQueryParams>,
    headers: HeaderMap,
) -> Result<Json<ConsentUrlResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    #[cfg(feature = "client")]
    {
        let redirect_uri = resolve_redirect_uri(&headers, params.redirect_uri.as_deref());
        let csrf_state = generate_csrf_state();

        let dc_client = state.data_connect_client.as_ref().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResponse {
                    error: "Client Enedis Data Connect non configuré".to_string(),
                }),
            )
        })?;

        state
            .consent_manager
            .create_session(csrf_state.clone(), redirect_uri.clone())
            .await;

        let authorize_url = dc_client
            .get_authorize_url(&redirect_uri, &csrf_state, params.duration.as_deref())
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResponse {
                        error: e.to_string(),
                    }),
                )
            })?;

        Ok(Json(ConsentUrlResponse {
            authorize_url,
            state: csrf_state,
            redirect_uri,
        }))
    }

    #[cfg(not(feature = "client"))]
    {
        let _ = (state, params, headers);
        Err((
            StatusCode::NOT_IMPLEMENTED,
            Json(ApiErrorResponse {
                error: "Fonctionnalité client désactivée".to_string(),
            }),
        ))
    }
}

/// Endpoint JSON : `POST /api/v1/consent/exchange`
pub async fn consent_exchange_api_handler(
    State(state): State<AppState>,
    Json(payload): Json<ConsentExchangeRequest>,
) -> Result<Json<ConsentExchangeResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    #[cfg(feature = "client")]
    {
        let dc_client = state.data_connect_client.as_ref().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResponse {
                    error: "Client Enedis Data Connect non configuré".to_string(),
                }),
            )
        })?;

        // Validation CSRF optionnelle si fourni
        if let Some(ref st) = payload.state {
            if state
                .consent_manager
                .validate_and_consume(st)
                .await
                .is_none()
            {
                return Err((
                    StatusCode::FORBIDDEN,
                    Json(ApiErrorResponse {
                        error: "Jeton CSRF invalide ou expiré".to_string(),
                    }),
                ));
            }
        }

        let token_resp = dc_client
            .exchange_authorization_code(&payload.code, &payload.redirect_uri)
            .await
            .map_err(|e| {
                (
                    StatusCode::BAD_GATEWAY,
                    Json(ApiErrorResponse {
                        error: format!("Échec échange OAuth2 : {}", e),
                    }),
                )
            })?;

        let prm_str = payload
            .usage_point_id
            .or(token_resp.usage_point_id)
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    Json(ApiErrorResponse {
                        error: "Aucun numéro de PRM fourni ou retourné par Enedis".to_string(),
                    }),
                )
            })?;

        let point_id = PointId::new(&prm_str).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ApiErrorResponse {
                    error: format!("PRM invalide : {}", e),
                }),
            )
        })?;

        let sync_state = SyncState {
            point_id,
            direction: FlowDirection::Consumption,
            last_synced_timestamp: Utc::now() - chrono::Duration::days(365),
            last_sync_attempt: Utc::now(),
            sync_status: "CONSENT_ACQUIRED".to_string(),
        };

        state
            .storage
            .update_sync_state(&sync_state)
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ApiErrorResponse {
                        error: format!("Erreur stockage PRM : {}", e),
                    }),
                )
            })?;

        info!(
            "PRM {} synchronisé et activé via API d'échange OAuth2",
            point_id
        );

        Ok(Json(ConsentExchangeResponse {
            point_id,
            status: "CONSENT_ACQUIRED".to_string(),
            message: format!("Compteur Linky {} associé avec succès", point_id),
            token_expires_in: token_resp.expires_in,
        }))
    }

    #[cfg(not(feature = "client"))]
    {
        let _ = (state, payload);
        Err((
            StatusCode::NOT_IMPLEMENTED,
            Json(ApiErrorResponse {
                error: "Fonctionnalité client désactivée".to_string(),
            }),
        ))
    }
}

/// Formate un numéro de PRM en groupes de 4 chiffres pour une lecture conviviale
fn format_prm_display(prm: &str) -> String {
    if prm.len() == 14 {
        format!(
            "{} {} {} {}",
            &prm[0..4],
            &prm[4..8],
            &prm[8..12],
            &prm[12..14]
        )
    } else {
        prm.to_string()
    }
}

/// Rendu HTML de la page de confirmation de succès du recueil de consentement
pub fn render_consent_success_html(point_id: &PointId) -> String {
    let formatted_prm = format_prm_display(point_id.as_str());

    format!(
        r#"<!DOCTYPE html>
<html lang="fr">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Consentement Validé • enedis-rs</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;500;600;700;800&family=JetBrains+Mono:wght@500;600&display=swap" rel="stylesheet">
    <style>
        :root {{
            --bg-base: #070a12;
            --bg-surface: #0e1526;
            --bg-card: #131d33;
            --border-subtle: rgba(255, 255, 255, 0.08);
            --border-strong: rgba(255, 255, 255, 0.16);
            --text-primary: #f8fafc;
            --text-secondary: #94a3b8;
            --text-muted: #64748b;
            --cyan: #06b6d4;
            --emerald: #10b981;
            --glow-emerald: 0 0 35px -5px rgba(16, 185, 129, 0.4);
        }}
        * {{ margin: 0; padding: 0; box-sizing: border-box; font-family: 'Plus Jakarta Sans', sans-serif; }}
        body {{
            background-color: var(--bg-base);
            color: var(--text-primary);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 2rem 1rem;
        }}
        .container {{
            width: 100%;
            max-width: 540px;
            background: var(--bg-surface);
            border: 1px solid var(--border-subtle);
            border-radius: 24px;
            padding: 2.75rem 2.25rem;
            box-shadow: 0 20px 40px -15px rgba(0, 0, 0, 0.7);
            text-align: center;
        }}
        .icon-badge {{
            width: 76px;
            height: 76px;
            margin: 0 auto 1.5rem;
            background: rgba(16, 185, 129, 0.12);
            border: 2px solid var(--emerald);
            border-radius: 20px;
            display: flex;
            align-items: center;
            justify-content: center;
            box-shadow: var(--glow-emerald);
        }}
        .icon-badge svg {{
            width: 38px;
            height: 38px;
            stroke: var(--emerald);
        }}
        h1 {{
            font-size: 1.6rem;
            font-weight: 800;
            letter-spacing: -0.02em;
            margin-bottom: 0.5rem;
        }}
        .subtitle {{
            font-size: 0.95rem;
            color: var(--text-secondary);
            margin-bottom: 2rem;
            line-height: 1.5;
        }}
        .prm-box {{
            background: var(--bg-card);
            border: 1px solid var(--border-strong);
            border-radius: 14px;
            padding: 1.25rem;
            margin-bottom: 2rem;
            display: flex;
            flex-direction: column;
            gap: 0.35rem;
        }}
        .prm-label {{
            font-size: 0.75rem;
            font-weight: 700;
            text-transform: uppercase;
            letter-spacing: 0.08em;
            color: var(--text-muted);
        }}
        .prm-value {{
            font-family: 'JetBrains Mono', monospace;
            font-size: 1.45rem;
            font-weight: 700;
            color: var(--cyan);
            letter-spacing: 0.06em;
        }}
        .checklist {{
            text-align: left;
            background: rgba(255, 255, 255, 0.02);
            border: 1px solid var(--border-subtle);
            border-radius: 12px;
            padding: 1.25rem;
            margin-bottom: 2rem;
            display: flex;
            flex-direction: column;
            gap: 0.85rem;
        }}
        .check-item {{
            display: flex;
            align-items: center;
            gap: 0.75rem;
            font-size: 0.88rem;
            color: var(--text-secondary);
        }}
        .check-item svg {{
            width: 18px;
            height: 18px;
            stroke: var(--emerald);
            flex-shrink: 0;
        }}
        .actions {{
            display: flex;
            flex-direction: column;
            gap: 0.85rem;
        }}
        .btn {{
            display: inline-flex;
            align-items: center;
            justify-content: center;
            gap: 0.65rem;
            padding: 0.9rem 1.5rem;
            font-size: 0.92rem;
            font-weight: 700;
            border-radius: 12px;
            text-decoration: none;
            transition: all 0.2s ease;
            cursor: pointer;
        }}
        .btn-primary {{
            background: linear-gradient(135deg, #10b981 0%, #06b6d4 100%);
            color: #ffffff;
            box-shadow: 0 4px 15px rgba(16, 185, 129, 0.3);
            border: none;
        }}
        .btn-primary:hover {{
            transform: translateY(-2px);
            box-shadow: 0 6px 20px rgba(16, 185, 129, 0.45);
        }}
        .btn-secondary {{
            background: rgba(255, 255, 255, 0.05);
            color: var(--text-primary);
            border: 1px solid var(--border-subtle);
        }}
        .btn-secondary:hover {{
            background: rgba(255, 255, 255, 0.08);
            border-color: var(--border-strong);
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="icon-badge">
            <svg fill="none" stroke-width="2.5" viewBox="0 0 24 24">
                <polyline points="20 6 9 17 4 12"></polyline>
            </svg>
        </div>
        <h1>Compteur Linky Connecté !</h1>
        <p class="subtitle">Votre autorisation Enedis Data Connect a été validée avec succès. Les télémesures sont désormais prêtes pour la collecte.</p>

        <div class="prm-box">
            <span class="prm-label">Point de Référence et de Mesure (PRM)</span>
            <span class="prm-value">{}</span>
        </div>

        <div class="checklist">
            <div class="check-item">
                <svg fill="none" stroke-width="2.5" viewBox="0 0 24 24"><polyline points="20 6 9 17 4 12"></polyline></svg>
                <span>Consentement client actif pour une durée de 3 ans</span>
            </div>
            <div class="check-item">
                <svg fill="none" stroke-width="2.5" viewBox="0 0 24 24"><polyline points="20 6 9 17 4 12"></polyline></svg>
                <span>Point de livraison enregistré dans la base de données temporelle</span>
            </div>
            <div class="check-item">
                <svg fill="none" stroke-width="2.5" viewBox="0 0 24 24"><polyline points="20 6 9 17 4 12"></polyline></svg>
                <span>Synchronisation continue et analyse d'optimisation prêtes</span>
            </div>
        </div>

        <div class="actions">
            <a href="/" class="btn btn-primary">
                <svg width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
                    <rect x="3" y="3" width="7" height="7"></rect>
                    <rect x="14" y="3" width="7" height="7"></rect>
                    <rect x="14" y="14" width="7" height="7"></rect>
                    <rect x="3" y="14" width="7" height="7"></rect>
                </svg>
                Accéder au Tableau de Bord
            </a>
            <a href="/api/v1/points/{}" target="_blank" class="btn btn-secondary">
                <svg width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
                    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"></path>
                    <polyline points="14 2 14 8 20 8"></polyline>
                    <line x1="16" y1="13" x2="8" y2="13"></line>
                    <line x1="16" y1="17" x2="8" y2="17"></line>
                </svg>
                Détails du Point de Livraison
            </a>
        </div>
    </div>
</body>
</html>"#,
        formatted_prm, point_id
    )
}

/// Rendu HTML de la page d'erreur ou d'interruption du recueil de consentement
pub fn render_consent_error_html(title: &str, details: &str, remediation: &str) -> String {
    let safe_title = escape_html(title);
    let safe_details = escape_html(details);
    let safe_remediation = escape_html(remediation);

    format!(
        r#"<!DOCTYPE html>
<html lang="fr">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Erreur de Consentement • enedis-rs</title>
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Plus+Jakarta+Sans:wght@400;500;600;700;800&display=swap" rel="stylesheet">
    <style>
        :root {{
            --bg-base: #070a12;
            --bg-surface: #0e1526;
            --bg-card: #131d33;
            --border-subtle: rgba(255, 255, 255, 0.08);
            --border-strong: rgba(255, 255, 255, 0.16);
            --text-primary: #f8fafc;
            --text-secondary: #94a3b8;
            --rose: #ef4444;
            --amber: #f59e0b;
            --glow-rose: 0 0 35px -5px rgba(239, 68, 68, 0.35);
        }}
        * {{ margin: 0; padding: 0; box-sizing: border-box; font-family: 'Plus Jakarta Sans', sans-serif; }}
        body {{
            background-color: var(--bg-base);
            color: var(--text-primary);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 2rem 1rem;
        }}
        .container {{
            width: 100%;
            max-width: 520px;
            background: var(--bg-surface);
            border: 1px solid var(--border-subtle);
            border-radius: 24px;
            padding: 2.75rem 2.25rem;
            box-shadow: 0 20px 40px -15px rgba(0, 0, 0, 0.7);
            text-align: center;
        }}
        .icon-badge {{
            width: 76px;
            height: 76px;
            margin: 0 auto 1.5rem;
            background: rgba(239, 68, 68, 0.12);
            border: 2px solid var(--rose);
            border-radius: 20px;
            display: flex;
            align-items: center;
            justify-content: center;
            box-shadow: var(--glow-rose);
        }}
        .icon-badge svg {{
            width: 38px;
            height: 38px;
            stroke: var(--rose);
        }}
        h1 {{
            font-size: 1.55rem;
            font-weight: 800;
            letter-spacing: -0.02em;
            margin-bottom: 0.75rem;
        }}
        .error-desc {{
            font-size: 0.92rem;
            color: var(--text-secondary);
            margin-bottom: 1.5rem;
            line-height: 1.5;
        }}
        .remediation-box {{
            background: var(--bg-card);
            border: 1px solid rgba(245, 158, 11, 0.25);
            border-radius: 12px;
            padding: 1.15rem;
            margin-bottom: 2rem;
            text-align: left;
            font-size: 0.85rem;
            color: #fde68a;
            line-height: 1.45;
        }}
        .actions {{
            display: flex;
            flex-direction: column;
            gap: 0.85rem;
        }}
        .btn {{
            display: inline-flex;
            align-items: center;
            justify-content: center;
            gap: 0.65rem;
            padding: 0.85rem 1.5rem;
            font-size: 0.92rem;
            font-weight: 700;
            border-radius: 12px;
            text-decoration: none;
            transition: all 0.2s ease;
            cursor: pointer;
        }}
        .btn-primary {{
            background: linear-gradient(135deg, #ef4444 0%, #f59e0b 100%);
            color: #ffffff;
            border: none;
        }}
        .btn-primary:hover {{
            opacity: 0.95;
            transform: translateY(-2px);
        }}
        .btn-secondary {{
            background: rgba(255, 255, 255, 0.05);
            color: var(--text-primary);
            border: 1px solid var(--border-subtle);
        }}
        .btn-secondary:hover {{
            background: rgba(255, 255, 255, 0.08);
            border-color: var(--border-strong);
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="icon-badge">
            <svg fill="none" stroke-width="2.5" viewBox="0 0 24 24">
                <circle cx="12" cy="12" r="10"></circle>
                <line x1="15" y1="9" x2="9" y2="15"></line>
                <line x1="9" y1="9" x2="15" y2="15"></line>
            </svg>
        </div>
        <h1>{}</h1>
        <p class="error-desc">{}</p>

        <div class="remediation-box">
            <strong>Conseil :</strong> {}
        </div>

        <div class="actions">
            <a href="/consent/authorize" class="btn btn-primary">
                <svg width="18" height="18" fill="none" stroke="currentColor" stroke-width="2" viewBox="0 0 24 24">
                    <path d="M21.5 2v6h-6M21.34 15.57a10 10 0 1 1-.57-8.38l5.67-5.67"></path>
                </svg>
                Réessayer la Connexion
            </a>
            <a href="/" class="btn btn-secondary">
                Retour au Tableau de Bord
            </a>
        </div>
    </div>
</body>
</html>"#,
        safe_title, safe_details, safe_remediation
    )
}
