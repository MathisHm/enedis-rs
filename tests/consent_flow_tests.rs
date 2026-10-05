#![cfg(all(feature = "api", feature = "mock-sge", feature = "storage-sqlite"))]
#[tokio::test]
async fn test_automated_consent_onboarding_flow() {
    use enedis_rs::api::{ApiServer, AppState};
    use enedis_rs::client::{DataConnectClient, DataConnectConfig};
    use enedis_rs::metrics::MetricsRegistry;
    use enedis_rs::mock::{MockDataConnectServer, MockScenario};
    use enedis_rs::models::{FlowDirection, PointId};
    use enedis_rs::signal::ShutdownSignal;
    use enedis_rs::storage::{SqliteStorage, StorageBackend};
    use secrecy::SecretString;
    use std::sync::Arc;
    use tokio::net::TcpListener;

    // 1. Initialisation du simulateur Enedis Data Connect
    let mock = MockDataConnectServer::start(MockScenario::Success).await;

    let dc_config = DataConnectConfig {
        base_url: mock.data_connect_url(),
        token_url: mock.data_connect_token_url(),
        authorize_url: Some(mock.authorize_url()),
        client_id: Some("test-client-id-123".to_string()),
        client_secret: Some(SecretString::new("test-client-secret-xyz".to_string())),
        direct_token: None,
        ..Default::default()
    };
    let dc_client = Arc::new(DataConnectClient::new(dc_config).expect("DataConnectClient valide"));

    // 2. Initialisation du stockage SQLite en mémoire
    let storage = Arc::new(SqliteStorage::connect("sqlite::memory:").await.unwrap());
    let metrics = MetricsRegistry::new();

    let state = AppState::new(storage.clone(), None, metrics).with_data_connect_client(dc_client);

    // 3. Démarrage de l'API HTTP Rest sur port local éphémère
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

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none()) // Ne pas suivre les redirections automatiquement
        .build()
        .unwrap();

    // -------------------------------------------------------------
    // Test A : Initialisation du consentement (GET /consent/authorize)
    // -------------------------------------------------------------
    let auth_res = client
        .get(format!("http://{}/consent/authorize", addr))
        .send()
        .await
        .unwrap();

    assert!(
        auth_res.status() == reqwest::StatusCode::SEE_OTHER
            || auth_res.status() == reqwest::StatusCode::FOUND
    );
    let location = auth_res
        .headers()
        .get(reqwest::header::LOCATION)
        .expect("En-tête Location présent")
        .to_str()
        .unwrap();

    assert!(
        location.starts_with(&mock.authorize_url()),
        "L'URL de redirection doit pointer vers le serveur Enedis OAuth2"
    );
    assert!(location.contains("client_id=test-client-id-123"));
    assert!(location.contains("response_type=code"));
    assert!(location.contains("duration=P3Y"));

    // Extraction du jeton CSRF 'state' depuis l'URL de redirection
    let target_url = reqwest::Url::parse(location).unwrap();
    let state_param = target_url
        .query_pairs()
        .find(|(k, _)| k == "state")
        .expect("Paramètre state présent")
        .1
        .to_string();

    assert!(
        !state_param.is_empty(),
        "Le jeton CSRF ne doit pas être vide"
    );

    // -------------------------------------------------------------
    // Test B : Traitement du callback Enedis (GET /consent/callback)
    // -------------------------------------------------------------
    let target_prm = "01234567890123";
    let callback_res = client
        .get(format!(
            "http://{}/consent/callback?code=test_auth_code_999&state={}&usage_point_id={}",
            addr, state_param, target_prm
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(callback_res.status(), reqwest::StatusCode::OK);
    let html_body = callback_res.text().await.unwrap();

    assert!(
        html_body.contains("Compteur Linky Connecté"),
        "La page de confirmation HTML doit célébrer la connexion réussie"
    );
    assert!(
        html_body.contains("0123 4567 8901 23"),
        "Le PRM doit être affiché avec un formatage clair par bloc de 4 chiffres"
    );
    assert!(
        html_body.contains("Tableau de Bord"),
        "Un lien de retour vers le tableau de bord doit être proposé"
    );

    // Vérification de la persistance en base de données
    let prm_id = PointId::new(target_prm).unwrap();
    let sync_points = storage.list_sync_points().await.unwrap();
    assert!(
        sync_points.contains(&prm_id),
        "Le PRM doit désormais être enregistré dans la liste des points suivis"
    );

    let sync_state = storage
        .get_sync_state(prm_id, FlowDirection::Consumption)
        .await
        .unwrap()
        .expect("État de synchronisation initialisé");
    assert_eq!(sync_state.sync_status, "CONSENT_ACQUIRED");

    // -------------------------------------------------------------
    // Test C : Rejeu ou jeton CSRF expiré / invalide
    // -------------------------------------------------------------
    let replay_res = client
        .get(format!(
            "http://{}/consent/callback?code=test_auth_code_999&state={}&usage_point_id={}",
            addr, state_param, target_prm
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(
        replay_res.status(),
        reqwest::StatusCode::FORBIDDEN,
        "Le même jeton CSRF ne peut pas être réutilisé (protection anti-rejeu)"
    );

    // -------------------------------------------------------------
    // Test D : Refus du consentement par l'usager sur Enedis
    // -------------------------------------------------------------
    let denied_res = client
        .get(format!(
            "http://{}/consent/callback?error=access_denied&error_description=Utilisateur+a+annule",
            addr
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(denied_res.status(), reqwest::StatusCode::OK);
    let denied_html = denied_res.text().await.unwrap();
    assert!(
        denied_html.contains("Consentement non accordé"),
        "La page d'erreur conviviale doit expliquer le refus"
    );

    // -------------------------------------------------------------
    // Test E : Endpoints Programmatiques API JSON
    // -------------------------------------------------------------
    // 1. GET /api/v1/consent/url
    let url_api_res = client
        .get(format!("http://{}/api/v1/consent/url", addr))
        .send()
        .await
        .unwrap();

    assert_eq!(url_api_res.status(), reqwest::StatusCode::OK);
    let url_json: serde_json::Value = url_api_res.json().await.unwrap();
    let api_state = url_json["state"].as_str().expect("state présent");
    assert!(url_json["authorize_url"]
        .as_str()
        .unwrap()
        .contains("client_id=test-client-id-123"));

    // 2. POST /api/v1/consent/exchange
    let exchange_payload = serde_json::json!({
        "code": "prog_auth_code_555",
        "redirect_uri": format!("http://{}/consent/callback", addr),
        "state": api_state,
        "usage_point_id": "98765432109876"
    });

    let exchange_res = client
        .post(format!("http://{}/api/v1/consent/exchange", addr))
        .json(&exchange_payload)
        .send()
        .await
        .unwrap();

    assert_eq!(exchange_res.status(), reqwest::StatusCode::OK);
    let exchange_json: serde_json::Value = exchange_res.json().await.unwrap();
    assert_eq!(exchange_json["point_id"], "98765432109876");
    assert_eq!(exchange_json["status"], "CONSENT_ACQUIRED");

    let second_prm = PointId::new("98765432109876").unwrap();
    let updated_points = storage.list_sync_points().await.unwrap();
    assert!(
        updated_points.contains(&second_prm),
        "Le deuxième PRM doit être enregistré via l'API programmatique"
    );

    shutdown.cancel();
    mock.stop();
}
