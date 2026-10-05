use crate::api::consent::{
    consent_authorize_handler, consent_callback_handler, consent_exchange_api_handler,
    consent_url_api_handler,
};
use crate::api::dashboard::dashboard_handler;
use crate::api::routes::{
    get_aggregates_handler, get_costs_handler, get_ecowatt_signals_handler,
    get_grid_correlation_handler, get_measurements_handler, get_point_handler,
    get_spot_analysis_handler, get_spot_prices_handler, get_tempo_signals_handler, health_handler,
    list_points_handler, metrics_handler, post_costs_handler, sync_point_handler, ApiErrorResponse,
    AppState,
};
use crate::error::EnedisError;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::net::SocketAddr;
use tokio::net::TcpListener;
use tracing::info;

pub struct ApiServer;

async fn auth_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ApiErrorResponse>)> {
    if let Some(ref expected_key) = state.api_key {
        let path = req.uri().path();
        // Endpoints publics exemptés d'authentification
        if path == "/"
            || path == "/dashboard"
            || path == "/health"
            || path == "/metrics"
            || path.starts_with("/consent")
            || path.starts_with("/api/v1/consent")
            || path.starts_with("/swagger-ui")
            || path.starts_with("/api-docs")
        {
            return Ok(next.run(req).await);
        }

        let key_from_header = req.headers().get("X-API-Key").and_then(|v| v.to_str().ok());

        let key_from_bearer = req
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));

        let key_to_check = key_from_header.or(key_from_bearer);
        let authorized = match key_to_check {
            Some(k) => {
                let k_bytes = k.as_bytes();
                let exp_bytes = expected_key.as_bytes();
                if k_bytes.len() == exp_bytes.len() {
                    bool::from(subtle::ConstantTimeEq::ct_eq(k_bytes, exp_bytes))
                } else {
                    false
                }
            }
            None => false,
        };

        if !authorized {
            return Err((
                StatusCode::UNAUTHORIZED,
                Json(ApiErrorResponse {
                    error:
                        "Accès refusé : clé d'API (header X-API-Key ou Bearer) invalide ou absente."
                            .to_string(),
                }),
            ));
        }
    }

    Ok(next.run(req).await)
}

async fn security_headers_middleware(req: Request, next: Next) -> Response {
    let is_options = req.method() == axum::http::Method::OPTIONS;
    if is_options {
        let mut res = Response::default();
        let headers = res.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            header::HeaderValue::from_static("*"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            header::HeaderValue::from_static("GET, POST, PATCH, OPTIONS"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            header::HeaderValue::from_static("Content-Type, Authorization, X-API-Key"),
        );
        return res;
    }

    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        header::HeaderValue::from_static("GET, POST, PATCH, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        header::HeaderValue::from_static("Content-Type, Authorization, X-API-Key"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::X_FRAME_OPTIONS,
        header::HeaderValue::from_static("DENY"),
    );
    response
}

impl ApiServer {
    /// Construit le routeur HTTP Axum avec tous les endpoints enregistrés
    pub fn router(state: AppState) -> Router {
        let app = Router::new()
            .route("/", get(dashboard_handler))
            .route("/dashboard", get(dashboard_handler))
            .route("/health", get(health_handler))
            .route("/metrics", get(metrics_handler))
            .route("/consent/authorize", get(consent_authorize_handler))
            .route("/consent/callback", get(consent_callback_handler))
            .route("/api/v1/consent/url", get(consent_url_api_handler))
            .route(
                "/api/v1/consent/exchange",
                post(consent_exchange_api_handler),
            )
            .route("/api/v1/points", get(list_points_handler))
            .route("/api/v1/points/:prm", get(get_point_handler))
            .route(
                "/api/v1/points/:prm/measurements",
                get(get_measurements_handler),
            )
            .route(
                "/api/v1/points/:prm/aggregates",
                get(get_aggregates_handler),
            )
            .route(
                "/api/v1/points/:prm/costs",
                get(get_costs_handler).post(post_costs_handler),
            )
            .route(
                "/api/v1/points/:prm/grid-correlation",
                get(get_grid_correlation_handler),
            )
            .route("/api/v1/signals/tempo", get(get_tempo_signals_handler))
            .route("/api/v1/signals/ecowatt", get(get_ecowatt_signals_handler))
            .route("/api/v1/spot/prices", get(get_spot_prices_handler))
            .route(
                "/api/v1/points/:prm/spot/analysis",
                get(get_spot_analysis_handler),
            )
            .route("/api/v1/points/:prm/sync", post(sync_point_handler));

        #[cfg(feature = "openapi")]
        let app = {
            use utoipa::OpenApi;
            use utoipa_swagger_ui::SwaggerUi;

            let openapi = crate::api::routes::ApiDoc::openapi();
            app.merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", openapi))
        };

        app.layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(axum::middleware::from_fn(security_headers_middleware))
        .with_state(state)
    }

    /// Lance l'écoute HTTP sur l'adresse et le port indiqués
    pub async fn run(
        state: AppState,
        addr: SocketAddr,
        shutdown: crate::signal::ShutdownSignal,
    ) -> Result<(), EnedisError> {
        let listener = TcpListener::bind(addr).await.map_err(|e| {
            EnedisError::Configuration(format!(
                "Impossible d'écouter sur l'adresse HTTP {:?}: {}",
                addr, e
            ))
        })?;

        let app = Self::router(state);
        info!("API HTTP REST d'enedis-rs active sur http://{}", addr);

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown.cancelled().await;
                info!("Arrêt gracieux de l'API HTTP en cours...");
            })
            .await
            .map_err(|e| {
                EnedisError::Transport(crate::error::TransportError::Network(e.to_string()))
            })?;

        Ok(())
    }
}
