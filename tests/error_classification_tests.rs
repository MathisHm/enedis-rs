use enedis_rs::{EnedisError, PointId, ResilienceAction, SgeBusinessError, TransportError};
use std::time::Duration;

#[test]
fn test_resilience_classification() {
    let prm = PointId::new("12345678901234").unwrap();

    // 1. Timeout réseau -> Retry court
    let timeout_err = EnedisError::Transport(TransportError::Timeout);
    assert_eq!(
        timeout_err.classify(),
        ResilienceAction::RetryAfter(Duration::from_secs(5))
    );

    // 2. Erreur HTTP 503 Enedis indisponible -> Retry moyen
    let http_503 = EnedisError::Http {
        status: 503,
        body: "Service Unavailable".to_string(),
    };
    assert_eq!(
        http_503.classify(),
        ResilienceAction::RetryAfter(Duration::from_secs(30))
    );

    // 3. Quota HTTP 429 -> Pause collector
    let http_429 = EnedisError::Http {
        status: 429,
        body: "Too Many Requests".to_string(),
    };
    assert_eq!(
        http_429.classify(),
        ResilienceAction::PauseCollector(Duration::from_secs(300))
    );

    // 4. Consentement absent pour un PRM -> Blacklist PRM pendant 24h
    let consent_err =
        EnedisError::Business(SgeBusinessError::ConsentMissingOrExpired { point_id: prm });
    assert_eq!(
        consent_err.classify(),
        ResilienceAction::BlacklistPoint {
            point_id: prm,
            duration: Duration::from_secs(86400),
        }
    );

    // 5. Erreur mTLS (certificat révoqué / corrompu) -> Arrêt fatal
    let tls_err = EnedisError::Tls("Handshake failed".to_string());
    assert_eq!(tls_err.classify(), ResilienceAction::FatalStop);

    // 6. Erreur HTTP 500 -> RetryAfter (non fatal)
    let http_500 = EnedisError::Http {
        status: 500,
        body: "Internal Server Error".to_string(),
    };
    assert_eq!(
        http_500.classify(),
        ResilienceAction::RetryAfter(Duration::from_secs(30))
    );

    // 7. Erreur XML transitoire -> RetryAfter (ne tue pas le daemon)
    let xml_err = EnedisError::Xml("Syntax error".to_string());
    assert_eq!(
        xml_err.classify(),
        ResilienceAction::RetryAfter(Duration::from_secs(60))
    );
}

#[test]
fn test_doctor_explanations() {
    let prm = PointId::new("12345678901234").unwrap();

    let consent_err =
        EnedisError::Business(SgeBusinessError::ConsentMissingOrExpired { point_id: prm });
    let explanation = consent_err.doctor_explanation();
    assert!(explanation.contains("Consentement manquant"));
    assert!(explanation.contains("12345678901234"));

    let tls_err = EnedisError::Tls("Certificat invalide".to_string());
    let tls_expl = tls_err.doctor_explanation();
    assert!(tls_expl.contains("mTLS"));
    assert!(tls_expl.contains("certificat client"));
}
