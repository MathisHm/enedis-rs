#[cfg(feature = "agent")]
use crate::agent::SgeRateLimiter;
#[cfg(feature = "client")]
use crate::client::{EnedisProvider, NetworkSignalClient};
#[cfg(not(feature = "client"))]
pub trait EnedisProvider: Send + Sync {}
use crate::metrics::MetricsRegistry;
#[allow(unused_imports)]
use crate::models::{
    calculate_energy_costs, correlate_measurements_with_grid, to_french_local_time,
    AggregationInterval, BaseTariff, CostCalculation, DynamicTariff, EcoWattCorrelation,
    EcoWattLevel, EcoWattSignal, FlowDirection, GridCorrelationReport, HpHcTariff, Measurement,
    MeasurementQuality, PointId, SpotArbitrageOpportunity, SpotPriceRecord, SpotProfileAnalysis,
    TariffComparison, TariffConfig, TariffCostBucket, TaxConfig, TempoColor, TempoCorrelation,
    TempoDayRecord, TempoTariff, TimeSlot, Unit,
};
use crate::storage::{StorageBackend, SyncState};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::sync::Arc;

/// État partagé entre les handlers HTTP
#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<dyn StorageBackend>,
    #[cfg(feature = "client")]
    pub client: Option<Arc<dyn EnedisProvider>>,
    #[cfg(feature = "client")]
    pub data_connect_client: Option<Arc<crate::client::DataConnectClient>>,
    pub metrics: MetricsRegistry,
    #[cfg(feature = "agent")]
    pub rate_limiter: Option<Arc<SgeRateLimiter>>,
    #[cfg(feature = "client")]
    pub signal_client: Option<Arc<NetworkSignalClient>>,
    pub api_key: Option<String>,
    pub consent_manager: Arc<crate::api::consent::PendingConsentStore>,
}

impl AppState {
    pub fn new(
        storage: Arc<dyn StorageBackend>,
        #[cfg(feature = "client")] client: Option<Arc<dyn EnedisProvider>>,
        metrics: MetricsRegistry,
    ) -> Self {
        #[cfg(feature = "client")]
        let data_connect_client = {
            if let Ok(client_id) = std::env::var("ENEDIS_DATA_CONNECT_CLIENT_ID") {
                let client_secret = std::env::var("ENEDIS_DATA_CONNECT_CLIENT_SECRET").ok();
                let base_url = std::env::var("ENEDIS_DATA_CONNECT_URL")
                    .unwrap_or_else(|_| "https://ext.prod.api.enedis.fr".to_string());
                let token_url = format!("{}/oauth2/v3/token", base_url.trim_end_matches('/'));
                let config = crate::client::DataConnectConfig {
                    base_url,
                    token_url,
                    authorize_url: None,
                    client_id: Some(client_id),
                    client_secret: client_secret.map(secrecy::SecretString::new),
                    direct_token: None,
                    connect_timeout: std::time::Duration::from_secs(10),
                    request_timeout: std::time::Duration::from_secs(30),
                    user_agent: format!("enedis-rs/{}", env!("CARGO_PKG_VERSION")),
                };
                crate::client::DataConnectClient::new(config)
                    .ok()
                    .map(Arc::new)
            } else {
                None
            }
        };

        Self {
            storage,
            #[cfg(feature = "client")]
            client,
            #[cfg(feature = "client")]
            data_connect_client,
            metrics,
            #[cfg(feature = "agent")]
            rate_limiter: Some(Arc::new(SgeRateLimiter::enedis_default())),
            #[cfg(feature = "client")]
            signal_client: NetworkSignalClient::with_default_config()
                .ok()
                .map(Arc::new),
            api_key: None,
            consent_manager: Arc::new(crate::api::consent::PendingConsentStore::new()),
        }
    }

    pub fn with_api_key(mut self, api_key: Option<String>) -> Self {
        self.api_key = api_key;
        self
    }

    #[cfg(feature = "agent")]
    pub fn with_rate_limiter(mut self, rate_limiter: Arc<SgeRateLimiter>) -> Self {
        self.rate_limiter = Some(rate_limiter);
        self
    }

    #[cfg(feature = "client")]
    pub fn with_signal_client(mut self, signal_client: Arc<NetworkSignalClient>) -> Self {
        self.signal_client = Some(signal_client);
        self
    }

    #[cfg(feature = "client")]
    pub fn with_data_connect_client(mut self, dc: Arc<crate::client::DataConnectClient>) -> Self {
        self.data_connect_client = Some(dc);
        self
    }
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Serialize)]
pub struct HealthResponse {
    #[cfg_attr(feature = "openapi", schema(example = "UP"))]
    pub status: &'static str,
    #[cfg_attr(feature = "openapi", schema(example = "0.1.0"))]
    pub version: &'static str,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Serialize)]
pub struct PointSummary {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "01234567890123"))]
    pub point_id: PointId,
    pub consumption: Option<SyncState>,
    pub production: Option<SyncState>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[derive(Deserialize)]
pub struct MeasurementsQuery {
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-21T00:00:00Z"))]
    pub from: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T00:00:00Z"))]
    pub to: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(example = "consumption"))]
    pub direction: Option<String>,
    #[cfg_attr(feature = "openapi", schema(example = 1000))]
    pub limit: Option<usize>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[derive(Deserialize, Default)]
pub struct AggregatesQuery {
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-01T00:00:00Z"))]
    pub from: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T00:00:00Z"))]
    pub to: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(example = "day"))]
    pub interval: Option<String>,
    #[cfg_attr(feature = "openapi", schema(example = "consumption"))]
    pub direction: Option<String>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[derive(Deserialize, Default)]
pub struct SyncQuery {
    #[cfg_attr(feature = "openapi", schema(example = "consumption"))]
    pub direction: Option<String>,
    #[cfg_attr(feature = "openapi", schema(example = 7))]
    pub days: Option<i64>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[derive(Deserialize, Default)]
pub struct CostsQuery {
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-01T00:00:00Z"))]
    pub from: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T00:00:00Z"))]
    pub to: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(example = "tempo"))]
    pub tariff_type: Option<String>,
    #[cfg_attr(feature = "openapi", schema(example = "0.2516"))]
    pub base_price: Option<Decimal>,
    #[cfg_attr(feature = "openapi", schema(example = "0.2700"))]
    pub hp_price: Option<Decimal>,
    #[cfg_attr(feature = "openapi", schema(example = "0.2068"))]
    pub hc_price: Option<Decimal>,
    #[cfg_attr(feature = "openapi", schema(example = "13.00"))]
    pub monthly_subscription: Option<Decimal>,
    #[cfg_attr(feature = "openapi", schema(example = "22:00-06:00"))]
    pub off_peak_slots: Option<String>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema, utoipa::IntoParams))]
#[derive(Deserialize, Default)]
pub struct GridCorrelationQuery {
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-01T00:00:00Z"))]
    pub from: Option<DateTime<Utc>>,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T00:00:00Z"))]
    pub to: Option<DateTime<Utc>>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Serialize)]
pub struct TempoSignalsResponse {
    pub today: Option<TempoDayRecord>,
    pub tomorrow: Option<TempoDayRecord>,
    pub history: Vec<TempoDayRecord>,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Serialize)]
pub struct ApiErrorResponse {
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Message descriptif de l'erreur")
    )]
    pub error: String,
}

#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Serialize)]
pub struct SyncPointResponse {
    #[cfg_attr(feature = "openapi", schema(example = "SYNCED"))]
    pub status: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "01234567890123"))]
    pub point_id: String,
    #[cfg_attr(feature = "openapi", schema(example = 48))]
    pub processed: usize,
    #[cfg_attr(feature = "openapi", schema(example = 48))]
    pub affected: usize,
}

/// Endpoint /health : Liveness & Readiness probe
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/health",
        tag = "Health",
        responses(
            (status = 200, description = "Service opérationnel", body = HealthResponse)
        )
    )
)]
pub async fn health_handler() -> impl IntoResponse {
    Json(HealthResponse {
        status: "UP",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Endpoint /metrics : Export au format standard Prometheus
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/metrics",
        tag = "Metrics",
        responses(
            (status = 200, description = "Métriques au format Prometheus standard", content_type = "text/plain; version=0.0.4; charset=utf-8")
        )
    )
)]
pub async fn metrics_handler(State(state): State<AppState>) -> impl IntoResponse {
    let rendered = state.metrics.render_prometheus();
    Response::builder()
        .header(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )
        .body(rendered)
        .unwrap()
}

/// Endpoint GET /api/v1/points : Liste de tous les PRM gérés
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/points",
        tag = "Points",
        responses(
            (status = 200, description = "Liste de tous les PRM supervisés", body = Vec<PointSummary>),
            (status = 500, description = "Erreur de stockage interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn list_points_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<PointSummary>>, (StatusCode, Json<ApiErrorResponse>)> {
    let points = state
        .storage
        .list_sync_points()
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let mut summaries = Vec::with_capacity(points.len());
    for point_id in points {
        let cons = state
            .storage
            .get_sync_state(point_id, FlowDirection::Consumption)
            .await
            .unwrap_or(None);
        let prod = state
            .storage
            .get_sync_state(point_id, FlowDirection::Production)
            .await
            .unwrap_or(None);

        summaries.push(PointSummary {
            point_id,
            consumption: cons,
            production: prod,
        });
    }

    Ok(Json(summaries))
}

/// Endpoint GET /api/v1/points/:prm : Détails de synchronisation d'un PRM
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/points/{prm}",
        tag = "Points",
        params(
            ("prm" = String, Path, description = "Identifiant PRM Enedis (14 chiffres)", example = "01234567890123")
        ),
        responses(
            (status = 200, description = "Détails et état de synchronisation du PRM", body = PointSummary),
            (status = 400, description = "Format de PRM invalide", body = ApiErrorResponse),
            (status = 404, description = "PRM introuvable", body = ApiErrorResponse),
            (status = 500, description = "Erreur de stockage interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn get_point_handler(
    State(state): State<AppState>,
    Path(prm): Path<String>,
) -> Result<Json<PointSummary>, (StatusCode, Json<ApiErrorResponse>)> {
    let point_id = PointId::new(&prm).map_err(|e| bad_request(e.to_string()))?;

    let cons = state
        .storage
        .get_sync_state(point_id, FlowDirection::Consumption)
        .await
        .map_err(|e| internal_error(e.to_string()))?;
    let prod = state
        .storage
        .get_sync_state(point_id, FlowDirection::Production)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    if cons.is_none() && prod.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: format!("Aucune information pour le PRM {}", point_id),
            }),
        ));
    }

    Ok(Json(PointSummary {
        point_id,
        consumption: cons,
        production: prod,
    }))
}

/// Endpoint GET /api/v1/points/:prm/measurements : Récupération des mesures normalisées
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/points/{prm}/measurements",
        tag = "Measurements",
        params(
            ("prm" = String, Path, description = "Identifiant PRM Enedis (14 chiffres)", example = "01234567890123"),
            MeasurementsQuery
        ),
        responses(
            (status = 200, description = "Liste des mesures normalisées", body = Vec<crate::models::Measurement>),
            (status = 400, description = "Paramètres de requête ou format de PRM invalide", body = ApiErrorResponse),
            (status = 500, description = "Erreur de stockage interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn get_measurements_handler(
    State(state): State<AppState>,
    Path(prm): Path<String>,
    Query(query): Query<MeasurementsQuery>,
) -> Result<Json<Vec<crate::models::Measurement>>, (StatusCode, Json<ApiErrorResponse>)> {
    let point_id = PointId::new(&prm).map_err(|e| bad_request(e.to_string()))?;

    let now = Utc::now();
    let from = query.from.unwrap_or_else(|| now - ChronoDuration::days(7));
    let to = query.to.unwrap_or(now);

    if from > to {
        return Err(bad_request(
            "Le paramètre 'from' doit être antérieur ou égal à 'to'.".to_string(),
        ));
    }

    let direction = query.direction.as_deref().map(FlowDirection::from_sge_code);

    const MAX_API_LIMIT: usize = 20_000;
    let effective_limit = query.limit.unwrap_or(MAX_API_LIMIT).min(MAX_API_LIMIT);

    let measurements = state
        .storage
        .get_measurements_limited(point_id, from, to, direction, Some(effective_limit))
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(measurements))
}

/// Endpoint GET /api/v1/points/:prm/aggregates : Calcul des agrégations temporelles (hour, day, month, year)
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/points/{prm}/aggregates",
        tag = "Measurements",
        params(
            ("prm" = String, Path, description = "Identifiant PRM Enedis (14 chiffres)", example = "01234567890123"),
            AggregatesQuery
        ),
        responses(
            (status = 200, description = "Liste des agrégations temporelles calculées", body = Vec<crate::models::AggregatedMeasurement>),
            (status = 400, description = "Paramètres de requête, intervalle ou format de PRM invalide", body = ApiErrorResponse),
            (status = 500, description = "Erreur de stockage interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn get_aggregates_handler(
    State(state): State<AppState>,
    Path(prm): Path<String>,
    Query(query): Query<AggregatesQuery>,
) -> Result<Json<Vec<crate::models::AggregatedMeasurement>>, (StatusCode, Json<ApiErrorResponse>)> {
    let point_id = PointId::new(&prm).map_err(|e| bad_request(e.to_string()))?;

    let now = Utc::now();
    let from = query.from.unwrap_or_else(|| now - ChronoDuration::days(30));
    let to = query.to.unwrap_or(now);

    if from > to {
        return Err(bad_request(
            "Le paramètre 'from' doit être antérieur ou égal à 'to'.".to_string(),
        ));
    }

    let interval = match query.interval.as_deref() {
        Some(s) => AggregationInterval::from_str(s)
            .map_err(|e| bad_request(format!("Intervalle invalide: {}", e)))?,
        None => AggregationInterval::Daily,
    };

    let direction = query.direction.as_deref().map(FlowDirection::from_sge_code);

    let aggregates = state
        .storage
        .get_aggregated_measurements(point_id, from, to, interval, direction)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(aggregates))
}

/// Endpoint POST /api/v1/points/:prm/sync : Déclenche une synchronisation immédiate à la demande
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/points/{prm}/sync",
        tag = "Sync",
        params(
            ("prm" = String, Path, description = "Identifiant PRM Enedis (14 chiffres)", example = "01234567890123"),
            SyncQuery,
        ),
        responses(
            (status = 200, description = "Synchronisation effectuée avec succès", body = SyncPointResponse),
            (status = 400, description = "Format de PRM ou paramètres invalides", body = ApiErrorResponse),
            (status = 500, description = "Erreur de stockage interne", body = ApiErrorResponse),
            (status = 502, description = "Erreur renvoyée par le serveur SGE distant", body = ApiErrorResponse),
            (status = 503, description = "Client SGE non configuré sur ce serveur d'API", body = ApiErrorResponse)
        )
    )
)]
pub async fn sync_point_handler(
    State(state): State<AppState>,
    Path(prm): Path<String>,
    Query(query): Query<SyncQuery>,
) -> Result<Json<SyncPointResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let point_id = PointId::new(&prm).map_err(|e| bad_request(e.to_string()))?;

    let client = match &state.client {
        Some(c) => c,
        None => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ApiErrorResponse {
                    error: "Le client Enedis n'est pas configuré sur ce serveur d'API.".to_string(),
                }),
            ));
        }
    };

    #[cfg(feature = "agent")]
    if let Some(ref limiter) = state.rate_limiter {
        limiter.acquire().await;
    }

    let direction = query
        .direction
        .as_deref()
        .map(FlowDirection::from_sge_code)
        .unwrap_or(FlowDirection::Consumption);

    let days = query.days.unwrap_or(7).clamp(1, 31);
    let now = Utc::now();
    let from = now - ChronoDuration::days(days);

    let res = client
        .fetch_measurements(point_id, from, now, direction)
        .await;

    match res {
        Ok(measurements) => {
            let stats = state
                .storage
                .upsert_measurements(&measurements)
                .await
                .map_err(|e| internal_error(e.to_string()))?;

            let max_ts_opt = measurements.iter().map(|m| m.timestamp).max();
            if let Some(max_ts) = max_ts_opt {
                let _ = state
                    .storage
                    .update_sync_state(&SyncState {
                        point_id,
                        direction,
                        last_synced_timestamp: max_ts,
                        last_sync_attempt: now,
                        sync_status: "OK".to_string(),
                    })
                    .await;
            } else {
                let current_sync = state
                    .storage
                    .get_sync_state(point_id, direction)
                    .await
                    .ok()
                    .flatten();
                let last_ts = current_sync
                    .map(|s| s.last_synced_timestamp)
                    .unwrap_or(from);
                let _ = state
                    .storage
                    .update_sync_state(&SyncState {
                        point_id,
                        direction,
                        last_synced_timestamp: last_ts,
                        last_sync_attempt: now,
                        sync_status: "OK".to_string(),
                    })
                    .await;
            }

            state.metrics.inc_requests("success", direction.as_str());
            state.metrics.set_last_sync_timestamp(
                point_id.as_str(),
                direction.as_str(),
                now.timestamp() as u64,
            );

            Ok(Json(SyncPointResponse {
                status: "SYNCED".to_string(),
                point_id: point_id.to_string(),
                processed: stats.processed,
                affected: stats.affected,
            }))
        }
        Err(err) => {
            state.metrics.inc_requests("error", direction.as_str());
            state
                .metrics
                .inc_errors(err.classify_status_code(), point_id.as_str());
            Err((
                StatusCode::BAD_GATEWAY,
                Json(ApiErrorResponse {
                    error: format!("Erreur SGE lors de la synchronisation: {}", err),
                }),
            ))
        }
    }
}

fn bad_request(msg: String) -> (StatusCode, Json<ApiErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiErrorResponse { error: msg }),
    )
}

fn internal_error(msg: String) -> (StatusCode, Json<ApiErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiErrorResponse { error: msg }),
    )
}

#[allow(dead_code)]
fn not_found(msg: String) -> (StatusCode, Json<ApiErrorResponse>) {
    (StatusCode::NOT_FOUND, Json(ApiErrorResponse { error: msg }))
}

/// Endpoint GET /api/v1/points/:prm/costs : Estimation financière de la facture énergétique
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/points/{prm}/costs",
        tag = "Costs",
        params(
            ("prm" = String, Path, description = "Identifiant PRM Enedis (14 chiffres)", example = "01234567890123"),
            CostsQuery
        ),
        responses(
            (status = 200, description = "Facture énergétique et décomposition financière calculée", body = CostCalculation),
            (status = 400, description = "Format de PRM ou paramètres de dates invalides", body = ApiErrorResponse),
            (status = 500, description = "Erreur de stockage interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn get_costs_handler(
    State(state): State<AppState>,
    Path(prm): Path<String>,
    Query(query): Query<CostsQuery>,
) -> Result<Json<CostCalculation>, (StatusCode, Json<ApiErrorResponse>)> {
    let point_id = PointId::new(&prm).map_err(|e| bad_request(e.to_string()))?;

    let now = Utc::now();
    let from = query.from.unwrap_or_else(|| now - ChronoDuration::days(30));
    let to = query.to.unwrap_or(now);

    if from > to {
        return Err(bad_request(
            "Le paramètre 'from' doit être antérieur ou égal à 'to'.".to_string(),
        ));
    }

    let measurements = state
        .storage
        .get_measurements(point_id, from, to, Some(FlowDirection::Consumption))
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let tariff_type_str = query
        .tariff_type
        .as_deref()
        .unwrap_or("base")
        .to_lowercase();
    let (tariff, tempo_days) = match tariff_type_str.as_str() {
        "tempo" => {
            let from_date = from.date_naive() - ChronoDuration::days(1);
            let to_date = to.date_naive() + ChronoDuration::days(1);
            let days = state
                .storage
                .get_tempo_days(from_date, to_date)
                .await
                .unwrap_or_default();
            let mut t = TempoTariff::default();
            if let Some(sub) = query.monthly_subscription {
                t.monthly_subscription = sub;
            }
            (TariffConfig::Tempo(t), Some(days))
        }
        "hp_hc" | "hphc" | "heures_creuses" => {
            let mut h = HpHcTariff::default();
            if let Some(hp) = query.hp_price {
                h.hp_price_per_kwh = hp;
            }
            if let Some(hc) = query.hc_price {
                h.hc_price_per_kwh = hc;
            }
            if let Some(sub) = query.monthly_subscription {
                h.monthly_subscription = sub;
            }
            if let Some(ref slots_str) = query.off_peak_slots {
                let mut slots = Vec::new();
                for part in slots_str.split(',') {
                    if let Ok(slot) = TimeSlot::parse(part) {
                        slots.push(slot);
                    }
                }
                if !slots.is_empty() {
                    h.off_peak_slots = slots;
                }
            }
            (TariffConfig::HpHc(h), None)
        }
        "dynamic" | "dynamique" | "spot" => {
            let mut d = DynamicTariff::default();
            if let Some(sub) = query.monthly_subscription {
                d.monthly_subscription = sub;
            }
            (TariffConfig::Dynamic(d), None)
        }
        _ => {
            let mut b = BaseTariff::default();
            if let Some(price) = query.base_price {
                b.price_per_kwh = price;
            }
            if let Some(sub) = query.monthly_subscription {
                b.monthly_subscription = sub;
            }
            (TariffConfig::Base(b), None)
        }
    };

    let costs = calculate_energy_costs(
        point_id,
        from,
        to,
        &measurements,
        &tariff,
        None,
        tempo_days.as_deref(),
    )
    .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(costs))
}

/// Endpoint POST /api/v1/points/:prm/costs : Calcul financier avec grille tarifaire personnalisée
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/points/{prm}/costs",
        tag = "Costs",
        params(
            ("prm" = String, Path, description = "Identifiant PRM Enedis (14 chiffres)", example = "01234567890123"),
            CostsQuery
        ),
        request_body = TariffConfig,
        responses(
            (status = 200, description = "Facture énergétique et décomposition financière calculée", body = CostCalculation),
            (status = 400, description = "Format de PRM ou paramètres de dates invalides", body = ApiErrorResponse),
            (status = 500, description = "Erreur de stockage interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn post_costs_handler(
    State(state): State<AppState>,
    Path(prm): Path<String>,
    Query(query): Query<CostsQuery>,
    Json(tariff): Json<TariffConfig>,
) -> Result<Json<CostCalculation>, (StatusCode, Json<ApiErrorResponse>)> {
    let point_id = PointId::new(&prm).map_err(|e| bad_request(e.to_string()))?;

    let now = Utc::now();
    let from = query.from.unwrap_or_else(|| now - ChronoDuration::days(30));
    let to = query.to.unwrap_or(now);

    if from > to {
        return Err(bad_request(
            "Le paramètre 'from' doit être antérieur ou égal à 'to'.".to_string(),
        ));
    }

    let measurements = state
        .storage
        .get_measurements(point_id, from, to, Some(FlowDirection::Consumption))
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let tempo_days = match &tariff {
        TariffConfig::Tempo(_) => {
            let from_date = from.date_naive() - ChronoDuration::days(1);
            let to_date = to.date_naive() + ChronoDuration::days(1);
            Some(
                state
                    .storage
                    .get_tempo_days(from_date, to_date)
                    .await
                    .unwrap_or_default(),
            )
        }
        _ => None,
    };

    let costs = calculate_energy_costs(
        point_id,
        from,
        to,
        &measurements,
        &tariff,
        None,
        tempo_days.as_deref(),
    )
    .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(costs))
}

/// Endpoint GET /api/v1/signals/tempo : État et calendrier des couleurs Tempo (J / J+1)
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/signals/tempo",
        tag = "Signals",
        responses(
            (status = 200, description = "Couleur du jour (J) et du lendemain (J+1) avec historique", body = TempoSignalsResponse),
            (status = 500, description = "Erreur de stockage interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn get_tempo_signals_handler(
    State(state): State<AppState>,
) -> Result<Json<TempoSignalsResponse>, (StatusCode, Json<ApiErrorResponse>)> {
    let now = Utc::now();
    let today_date = now.date_naive();
    let tomorrow_date = today_date + ChronoDuration::days(1);
    let from_date = today_date - ChronoDuration::days(7);

    #[cfg(feature = "client")]
    if let Some(ref client) = state.signal_client {
        let existing = state
            .storage
            .get_tempo_days(today_date, tomorrow_date)
            .await
            .unwrap_or_default();
        if existing.len() < 2 {
            let _ = client.sync_to_storage(&*state.storage).await;
        }
    }

    let records = state
        .storage
        .get_tempo_days(from_date, tomorrow_date)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let today = records.iter().find(|r| r.date == today_date).cloned();
    let tomorrow = records.iter().find(|r| r.date == tomorrow_date).cloned();

    Ok(Json(TempoSignalsResponse {
        today,
        tomorrow,
        history: records,
    }))
}

/// Endpoint GET /api/v1/signals/ecowatt : Signaux de tension du réseau électrique RTE EcoWatt
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/signals/ecowatt",
        tag = "Signals",
        responses(
            (status = 200, description = "Liste des signaux EcoWatt (vert/orange/rouge)", body = Vec<crate::models::EcoWattSignal>),
            (status = 500, description = "Erreur de stockage interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn get_ecowatt_signals_handler(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::models::EcoWattSignal>>, (StatusCode, Json<ApiErrorResponse>)> {
    let now = Utc::now();
    let from = now - ChronoDuration::hours(24);
    let to = now + ChronoDuration::hours(72);

    #[cfg(feature = "client")]
    if let Some(ref client) = state.signal_client {
        let existing = state
            .storage
            .get_ecowatt_signals(from, to)
            .await
            .unwrap_or_default();
        if existing.is_empty() {
            let _ = client.sync_to_storage(&*state.storage).await;
        }
    }

    let signals = state
        .storage
        .get_ecowatt_signals(from, to)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    Ok(Json(signals))
}

/// Endpoint GET /api/v1/points/:prm/grid-correlation : Corrélation entre consommation mesurée et alertes réseau
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/points/{prm}/grid-correlation",
        tag = "Signals",
        params(
            ("prm" = String, Path, description = "Identifiant PRM Enedis (14 chiffres)", example = "01234567890123"),
            GridCorrelationQuery
        ),
        responses(
            (status = 200, description = "Rapport de corrélation et d'alignement avec les tensions du réseau", body = GridCorrelationReport),
            (status = 400, description = "Format de PRM ou paramètres de dates invalides", body = ApiErrorResponse),
            (status = 500, description = "Erreur de stockage interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn get_grid_correlation_handler(
    State(state): State<AppState>,
    Path(prm): Path<String>,
    Query(query): Query<GridCorrelationQuery>,
) -> Result<Json<GridCorrelationReport>, (StatusCode, Json<ApiErrorResponse>)> {
    let point_id = PointId::new(&prm).map_err(|e| bad_request(e.to_string()))?;
    let now = Utc::now();
    let from = query.from.unwrap_or_else(|| now - ChronoDuration::days(30));
    let to = query.to.unwrap_or(now);

    if from > to {
        return Err(bad_request(
            "Le paramètre 'from' doit être antérieur ou égal à 'to'.".to_string(),
        ));
    }

    let measurements = state
        .storage
        .get_measurements(point_id, from, to, Some(FlowDirection::Consumption))
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    let ecowatt_signals = state
        .storage
        .get_ecowatt_signals(from, to)
        .await
        .unwrap_or_default();

    let from_date = from.date_naive() - ChronoDuration::days(1);
    let to_date = to.date_naive() + ChronoDuration::days(1);
    let tempo_days = state.storage.get_tempo_days(from_date, to_date).await.ok();

    let report = correlate_measurements_with_grid(
        point_id,
        from,
        to,
        &measurements,
        &ecowatt_signals,
        tempo_days.as_deref(),
    );

    Ok(Json(report))
}
/// Paramètres de requête pour la consultation des prix spot Day-Ahead
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams, utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct SpotPricesQuery {
    /// Date/heure de début (ISO 8601 / RFC 3339)
    #[cfg_attr(feature = "openapi", param(example = "2026-09-01T00:00:00Z"))]
    pub from: Option<DateTime<Utc>>,
    /// Date/heure de fin (ISO 8601 / RFC 3339)
    #[cfg_attr(feature = "openapi", param(example = "2026-09-30T00:00:00Z"))]
    pub to: Option<DateTime<Utc>>,
    /// Plafond maximal de prix renvoyés (défaut : 168 = 7 jours)
    #[cfg_attr(feature = "openapi", param(example = 168))]
    pub limit: Option<usize>,
}

/// Paramètres de requête pour l'analyse de corrélation spot et flexibilité
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams, utoipa::ToSchema))]
#[derive(Debug, Deserialize)]
pub struct SpotAnalysisQuery {
    /// Date/heure de début (ISO 8601 / RFC 3339)
    #[cfg_attr(feature = "openapi", param(example = "2026-09-01T00:00:00Z"))]
    pub from: Option<DateTime<Utc>>,
    /// Date/heure de fin (ISO 8601 / RFC 3339)
    #[cfg_attr(feature = "openapi", param(example = "2026-09-30T00:00:00Z"))]
    pub to: Option<DateTime<Utc>>,
    /// Marge fournisseur fixe en €/kWh (défaut : 0.0150 €/kWh)
    #[cfg_attr(feature = "openapi", param(example = "0.0150"))]
    pub margin_kwh: Option<Decimal>,
}

/// Endpoint GET /api/v1/spot/prices : Récupération des cours horaires du marché spot Day-Ahead (EPEX SPOT)
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/spot/prices",
        tag = "Spot",
        params(SpotPricesQuery),
        responses(
            (status = 200, description = "Historique et cours Day-Ahead du marché de gros de l'électricité", body = Vec<crate::models::SpotPriceRecord>),
            (status = 400, description = "Paramètres de requête invalides", body = ApiErrorResponse),
            (status = 500, description = "Erreur interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn get_spot_prices_handler(
    State(state): State<AppState>,
    Query(query): Query<SpotPricesQuery>,
) -> Result<Json<Vec<crate::models::SpotPriceRecord>>, (StatusCode, Json<ApiErrorResponse>)> {
    let now = Utc::now();
    let to = query.to.unwrap_or_else(|| now + ChronoDuration::days(1));
    let from = query.from.unwrap_or_else(|| to - ChronoDuration::days(7));

    let mut prices = state
        .storage
        .get_spot_prices(from, to)
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    if prices.is_empty() {
        #[cfg(feature = "client")]
        if let Some(ref sc) = state.signal_client {
            if let Ok(fetched) = sc.fetch_spot_prices(from, to).await {
                let _ = state.storage.upsert_spot_prices(&fetched).await;
                prices = fetched;
            }
        }
    }

    if prices.is_empty() {
        prices = crate::models::generate_synthetic_spot_profile(from, to);
    }

    if let Some(limit) = query.limit {
        prices.truncate(limit);
    }

    Ok(Json(prices))
}

/// Endpoint GET /api/v1/points/:prm/spot/analysis : Analyse de corrélation et calcul du potentiel d'arbitrage
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/points/{prm}/spot/analysis",
        tag = "Spot",
        params(
            ("prm" = String, Path, description = "Identifiant PRM Enedis (14 chiffres)", example = "01234567890123"),
            SpotAnalysisQuery
        ),
        responses(
            (status = 200, description = "Rapport d'analyse de marché spot, coefficient de profilage et opportunités d'arbitrage", body = crate::models::SpotProfileAnalysis),
            (status = 400, description = "Paramètres de requête invalides", body = ApiErrorResponse),
            (status = 404, description = "Aucune mesure pour ce PRM", body = ApiErrorResponse),
            (status = 500, description = "Erreur interne", body = ApiErrorResponse)
        )
    )
)]
pub async fn get_spot_analysis_handler(
    State(state): State<AppState>,
    Path(prm): Path<String>,
    Query(query): Query<SpotAnalysisQuery>,
) -> Result<Json<crate::models::SpotProfileAnalysis>, (StatusCode, Json<ApiErrorResponse>)> {
    let point_id = PointId::new(&prm).map_err(|e| bad_request(e.to_string()))?;
    let now = Utc::now();
    let to = query.to.unwrap_or(now);
    let from = query.from.unwrap_or_else(|| to - ChronoDuration::days(30));

    let measurements = state
        .storage
        .get_measurements(point_id, from, to, Some(FlowDirection::Consumption))
        .await
        .map_err(|e| internal_error(e.to_string()))?;

    if measurements.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: format!(
                    "Aucune mesure de consommation pour le PRM {} sur la période demandée",
                    point_id
                ),
            }),
        ));
    }

    let mut spot_prices = state
        .storage
        .get_spot_prices(from, to)
        .await
        .unwrap_or_default();

    if spot_prices.is_empty() {
        spot_prices = crate::models::generate_synthetic_spot_profile(from, to);
    }

    match crate::models::analyze_spot_consumption(
        point_id,
        &measurements,
        &spot_prices,
        query.margin_kwh,
        None,
    ) {
        Some(analysis) => Ok(Json(analysis)),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ApiErrorResponse {
                error: format!(
                    "Impossible d'analyser la corrélation spot pour le PRM {}",
                    point_id
                ),
            }),
        )),
    }
}

#[cfg(feature = "openapi")]
#[derive(utoipa::OpenApi)]
#[openapi(
    paths(
        health_handler,
        metrics_handler,
        list_points_handler,
        get_point_handler,
        get_measurements_handler,
        get_aggregates_handler,
        sync_point_handler,
        get_costs_handler,
        post_costs_handler,
        get_tempo_signals_handler,
        get_ecowatt_signals_handler,
        get_grid_correlation_handler,
        get_spot_prices_handler,
        get_spot_analysis_handler,
    ),
    components(
        schemas(
            crate::models::PointId,
            crate::models::FlowDirection,
            crate::models::Unit,
            crate::models::MeasurementQuality,
            crate::models::Measurement,
            crate::models::AggregationInterval,
            crate::models::AggregatedMeasurement,
            crate::storage::SyncState,
            crate::models::BaseTariff,
            crate::models::HpHcTariff,
            crate::models::TempoTariff,
            crate::models::DynamicTariff,
            crate::models::TariffConfig,
            crate::models::CostCalculation,
            crate::models::TariffCostBucket,
            crate::models::TariffComparison,
            crate::models::TimeSlot,
            crate::models::TempoColor,
            crate::models::TempoDayRecord,
            crate::models::EcoWattLevel,
            crate::models::EcoWattSignal,
            crate::models::EcoWattCorrelation,
            crate::models::TempoCorrelation,
            crate::models::GridCorrelationReport,
            crate::models::SpotPriceRecord,
            crate::models::SpotProfileAnalysis,
            crate::models::SpotArbitrageOpportunity,
            PointSummary,
            HealthResponse,
            ApiErrorResponse,
            SyncPointResponse,
            MeasurementsQuery,
            AggregatesQuery,
            SyncQuery,
            CostsQuery,
            GridCorrelationQuery,
            TempoSignalsResponse,
            SpotPricesQuery,
            SpotAnalysisQuery,
            crate::api::consent::ConsentUrlResponse,
            crate::api::consent::ConsentExchangeRequest,
            crate::api::consent::ConsentExchangeResponse,
            crate::client::OAuthTokenResponse,
        )
    ),
    tags(
        (name = "Health", description = "Vérification de l'état de santé du service"),
        (name = "Metrics", description = "Export des métriques Prometheus standard"),
        (name = "Points", description = "Gestion des Points de Référence Mesure (PRM)"),
        (name = "Measurements", description = "Consultation des séries temporelles de consommation et production"),
        (name = "Sync", description = "Synchronisation à la demande auprès d'Enedis SGE"),
        (name = "Costs", description = "Moteur tarifaire et calculs de facturation en Euros (€)"),
        (name = "Signals", description = "Signaux de tension réseau RTE EcoWatt & calendrier Tempo"),
        (name = "Spot", description = "Marché spot de gros Day-Ahead EPEX SPOT France, tarification dynamique et opportunités d'arbitrage"),
        (name = "Consent", description = "Onboarding et recueil automatisé du consentement client Enedis Data Connect OAuth2"),
    ),
    info(
        title = "Enedis-RS REST API",
        version = env!("CARGO_PKG_VERSION"),
        description = "API REST Axum open-source pour la collecte, la normalisation, la tarification et la synchronisation des données de compteurs communicants Enedis (Linky / SGE)",
        license(name = "MIT OR Apache-2.0")
    )
)]
pub struct ApiDoc;
