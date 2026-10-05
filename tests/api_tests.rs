#![cfg(all(feature = "api", feature = "storage-sqlite"))]
#[tokio::test]
async fn test_api_rest_endpoints() {
    use chrono::{TimeZone, Utc};
    use enedis_rs::api::{ApiServer, AppState};
    use enedis_rs::metrics::MetricsRegistry;
    use enedis_rs::signal::ShutdownSignal;
    use enedis_rs::storage::{SqliteStorage, StorageBackend, SyncState};
    use enedis_rs::{FlowDirection, Measurement, MeasurementQuality, PointId, Unit};
    use rust_decimal::Decimal;
    use std::str::FromStr;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    // 1. Initialisation de la base SQLite et injection de données de test
    let storage = Arc::new(SqliteStorage::connect("sqlite::memory:").await.unwrap());
    let prm = PointId::new("01234567890123").unwrap();
    let ts = Utc.with_ymd_and_hms(2026, 9, 28, 10, 0, 0).unwrap();

    let measurement = Measurement {
        point_id: prm,
        timestamp: ts,
        interval_seconds: 1800,
        direction: FlowDirection::Consumption,
        value: Decimal::from_str("3.1415").unwrap(),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };
    storage.upsert_measurements(&[measurement]).await.unwrap();

    storage
        .update_sync_state(&SyncState {
            point_id: prm,
            direction: FlowDirection::Consumption,
            last_synced_timestamp: ts,
            last_sync_attempt: ts,
            sync_status: "OK".to_string(),
        })
        .await
        .unwrap();

    // 2. Démarrage du serveur API sur port éphémère
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let metrics = MetricsRegistry::new();
    metrics.inc_requests("success", "CONSUMPTION");
    metrics.set_last_sync_timestamp(prm.as_str(), "CONSUMPTION", ts.timestamp() as u64);

    let state = AppState::new(storage, None, metrics);

    let router = ApiServer::router(state);
    let shutdown = ShutdownSignal::new();
    let shutdown_rx = shutdown.clone();

    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                shutdown_rx.cancelled().await;
            })
            .await
            .unwrap();
    });

    let http_client = reqwest::Client::new();
    let base_url = format!("http://{}", addr);

    // 3. Test /health
    let res = http_client
        .get(format!("{}/health", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let health_json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(health_json["status"], "UP");

    // 4. Test /metrics
    let res = http_client
        .get(format!("{}/metrics", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let metrics_text = res.text().await.unwrap();
    assert!(metrics_text.contains("enedis_requests_total"));
    assert!(metrics_text.contains("enedis_collector_last_sync_timestamp"));

    // 5. Test /api/v1/points/:prm/measurements
    let res = http_client
        .get(format!(
            "{}/api/v1/points/{}/measurements?direction=consumption",
            base_url, prm
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let measurements_json: Vec<serde_json::Value> = res.json().await.unwrap();
    assert_eq!(measurements_json.len(), 1);
    assert_eq!(measurements_json[0]["point_id"], "01234567890123");
    assert_eq!(measurements_json[0]["value"], "3.1415");
    assert_eq!(measurements_json[0]["quality"], "Validated");

    // 5b. Test /api/v1/points/:prm/aggregates
    let res = http_client
        .get(format!(
            "{}/api/v1/points/{}/aggregates?interval=day&direction=consumption",
            base_url, prm
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let aggregates_json: Vec<serde_json::Value> = res.json().await.unwrap();
    assert_eq!(aggregates_json.len(), 1);
    assert_eq!(aggregates_json[0]["point_id"], "01234567890123");
    assert_eq!(aggregates_json[0]["direction"], "Consumption");
    assert_eq!(aggregates_json[0]["sample_count"], 1);
    assert_eq!(aggregates_json[0]["total_energy_kwh"], "3.1415");

    // 5c. Test /api/v1/points/:prm/aggregates avec intervalle invalide -> 400 Bad Request
    let res = http_client
        .get(format!(
            "{}/api/v1/points/{}/aggregates?interval=unknown_interval",
            base_url, prm
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 6. Test GET /api/v1/points (liste globale)
    let res = http_client
        .get(format!("{}/api/v1/points", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let points_json: Vec<serde_json::Value> = res.json().await.unwrap();
    assert_eq!(points_json.len(), 1);
    assert_eq!(points_json[0]["point_id"], "01234567890123");
    assert_eq!(points_json[0]["consumption"]["sync_status"], "OK");

    // 7. Test GET /api/v1/points/:prm (détails d'un PRM)
    let res = http_client
        .get(format!("{}/api/v1/points/{}", base_url, prm))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let point_json: serde_json::Value = res.json().await.unwrap();
    assert_eq!(point_json["point_id"], "01234567890123");
    assert_eq!(point_json["consumption"]["sync_status"], "OK");

    // 8. Test GET /api/v1/points/:prm - 404 Not Found sur PRM inconnu
    let res = http_client
        .get(format!("{}/api/v1/points/99999999999999", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::NOT_FOUND);

    // 9. Test GET /api/v1/points/:prm - 400 Bad Request sur PRM invalide
    let res = http_client
        .get(format!("{}/api/v1/points/invalid_prm", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 10. Test POST /api/v1/points/:prm/sync - 503 Service Unavailable quand le client SGE n'est pas configuré
    let res = http_client
        .post(format!("{}/api/v1/points/{}/sync", base_url, prm))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::SERVICE_UNAVAILABLE);

    // 11. Test POST /api/v1/points/:prm/sync - 400 Bad Request sur PRM invalide
    let res = http_client
        .post(format!("{}/api/v1/points/invalid_prm/sync", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);

    // 12. Test OpenAPI specification (/api-docs/openapi.json)
    let res = http_client
        .get(format!("{}/api-docs/openapi.json", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let openapi_json: serde_json::Value = res.json().await.unwrap();
    assert!(openapi_json["openapi"].as_str().unwrap().starts_with("3."));
    assert_eq!(openapi_json["info"]["title"], "Enedis-RS REST API");
    // Vérification des routes documentées et de leurs statuts
    let paths = &openapi_json["paths"];
    assert!(paths.get("/health").is_some());
    assert!(paths["/health"]["get"]["responses"].get("200").is_some());

    assert!(paths.get("/metrics").is_some());
    assert!(paths["/metrics"]["get"]["responses"].get("200").is_some());

    assert!(paths.get("/api/v1/points").is_some());
    assert!(paths["/api/v1/points"]["get"]["responses"]
        .get("200")
        .is_some());
    assert!(paths["/api/v1/points"]["get"]["responses"]
        .get("500")
        .is_some());

    assert!(paths.get("/api/v1/points/{prm}").is_some());
    let point_responses = &paths["/api/v1/points/{prm}"]["get"]["responses"];
    assert!(point_responses.get("200").is_some());
    assert!(point_responses.get("400").is_some());
    assert!(point_responses.get("404").is_some());
    assert!(point_responses.get("500").is_some());

    assert!(paths.get("/api/v1/points/{prm}/measurements").is_some());
    let measurements_responses = &paths["/api/v1/points/{prm}/measurements"]["get"]["responses"];
    assert!(measurements_responses.get("200").is_some());
    assert!(measurements_responses.get("400").is_some());
    assert!(measurements_responses.get("500").is_some());

    assert!(paths.get("/api/v1/points/{prm}/aggregates").is_some());
    let aggregates_responses = &paths["/api/v1/points/{prm}/aggregates"]["get"]["responses"];
    assert!(aggregates_responses.get("200").is_some());
    assert!(aggregates_responses.get("400").is_some());
    assert!(aggregates_responses.get("500").is_some());

    assert!(paths.get("/api/v1/points/{prm}/sync").is_some());
    let sync_responses = &paths["/api/v1/points/{prm}/sync"]["post"]["responses"];
    assert!(sync_responses.get("200").is_some());
    assert!(sync_responses.get("400").is_some());
    assert!(sync_responses.get("500").is_some());
    assert!(sync_responses.get("502").is_some());
    assert!(sync_responses.get("503").is_some());

    // Vérification des schémas de composants
    let schemas = &openapi_json["components"]["schemas"];
    assert!(schemas.get("PointId").is_some());
    let point_id_schema = &schemas["PointId"];
    assert_eq!(point_id_schema["type"], "string");
    assert_eq!(point_id_schema["example"], "01234567890123");

    assert!(schemas.get("PointSummary").is_some());
    assert!(schemas.get("SyncState").is_some());
    assert!(schemas.get("Measurement").is_some());
    assert!(schemas.get("AggregatedMeasurement").is_some());
    assert!(schemas.get("AggregationInterval").is_some());
    assert!(schemas.get("AggregatesQuery").is_some());
    assert!(schemas.get("HealthResponse").is_some());
    assert!(schemas.get("ApiErrorResponse").is_some());
    assert!(schemas.get("SyncPointResponse").is_some());
    assert!(schemas.get("FlowDirection").is_some());
    assert!(schemas.get("Unit").is_some());
    assert!(schemas.get("MeasurementQuality").is_some());

    // 13. Test Swagger UI (/swagger-ui/ et /swagger-ui)
    let res = http_client
        .get(format!("{}/swagger-ui/", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let html = res.text().await.unwrap();
    assert!(html.contains("swagger-ui") || html.contains("SwaggerUIBundle"));

    let res_no_slash = http_client
        .get(format!("{}/swagger-ui", base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(res_no_slash.status(), reqwest::StatusCode::OK);

    // 14. Arrêt gracieux
    shutdown.cancel();
}

#[cfg(all(feature = "api", feature = "storage-sqlite"))]
#[tokio::test]
async fn test_api_authentication_enforcement() {
    use enedis_rs::api::{ApiServer, AppState};
    use enedis_rs::metrics::MetricsRegistry;
    use enedis_rs::signal::ShutdownSignal;
    use enedis_rs::storage::SqliteStorage;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    let storage = Arc::new(SqliteStorage::connect("sqlite::memory:").await.unwrap());
    let metrics = MetricsRegistry::new();
    let state =
        AppState::new(storage, None, metrics).with_api_key(Some("secret-api-key-xyz".to_string()));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = ApiServer::router(state);
    let shutdown = ShutdownSignal::new();
    let shutdown_rx = shutdown.clone();

    tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                shutdown_rx.cancelled().await;
            })
            .await
            .unwrap();
    });

    let client = reqwest::Client::new();
    let base = format!("http://{}", addr);

    // 1. Endpoint public (/health) accessible sans clé
    let res = client.get(format!("{}/health", base)).send().await.unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);

    // 2. Endpoint protégé sans clé -> 401 Unauthorized
    let res = client
        .get(format!("{}/api/v1/points", base))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 3. Endpoint protégé avec clé erronée -> 401 Unauthorized
    let res = client
        .get(format!("{}/api/v1/points", base))
        .header("X-API-Key", "bad-key")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 4. Endpoint protégé avec X-API-Key valide -> 200 OK
    let res = client
        .get(format!("{}/api/v1/points", base))
        .header("X-API-Key", "secret-api-key-xyz")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);

    // 5. Endpoint protégé avec Authorization: Bearer valide -> 200 OK
    let res = client
        .get(format!("{}/api/v1/points", base))
        .header("Authorization", "Bearer secret-api-key-xyz")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    // 6. Endpoints Web Dashboard publics (/ et /dashboard) accessibles sans clé
    let res_root = client.get(format!("{}/", base)).send().await.unwrap();
    assert_eq!(res_root.status(), reqwest::StatusCode::OK);
    assert_eq!(
        res_root.headers().get("content-type").unwrap(),
        "text/html; charset=utf-8"
    );
    let html = res_root.text().await.unwrap();
    assert!(html.contains("enedis-rs"));
    assert!(html.contains("Community"));

    let res_dash = client
        .get(format!("{}/dashboard", base))
        .send()
        .await
        .unwrap();
    assert_eq!(res_dash.status(), reqwest::StatusCode::OK);

    shutdown.cancel();
}
