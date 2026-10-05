use crate::models::PointId;
use chrono::{DateTime, Utc};
use std::time::Duration;
use thiserror::Error;

/// Erreur racine pour toutes les opérations d'`enedis-rs`
#[derive(Error, Debug)]
pub enum EnedisError {
    #[error("Erreur de configuration: {0}")]
    Configuration(String),

    #[error("Erreur mTLS / Certificat client: {0}")]
    Tls(String),

    #[error("Erreur de transport réseau: {0}")]
    Transport(#[from] TransportError),

    #[error("Erreur HTTP {status}: {body}")]
    Http { status: u16, body: String },

    #[error("Erreur SOAP Fault: {0}")]
    Soap(#[from] SoapFault),

    #[error("Erreur métier Enedis SGE: {0}")]
    Business(#[from] SgeBusinessError),

    #[error("Erreur de parsing XML: {0}")]
    Xml(String),

    #[error("Violation de sécurité XML: {0}")]
    XmlSecurity(String),

    #[cfg(feature = "storage")]
    #[error("Erreur base de données: {0}")]
    Database(#[from] sqlx::Error),

    #[cfg(feature = "storage")]
    #[error("Erreur de décodage de données en base: {0}")]
    StorageDecode(String),
}

/// Erreurs de transport réseau bas niveau
#[derive(Error, Debug)]
pub enum TransportError {
    #[error("Délai de connexion dépassé (Timeout)")]
    Timeout,

    #[error("Connexion interrompue ou refusée: {0}")]
    Network(String),
}

#[cfg(feature = "client")]
impl From<reqwest::Error> for TransportError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Timeout
        } else {
            Self::Network(err.to_string())
        }
    }
}

/// Représentation structurée d'un SOAP Fault (SOAP 1.1 / 1.2)
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("SOAP Fault [{code}]: {message}")]
pub struct SoapFault {
    pub code: String,
    pub message: String,
    pub subcode: Option<String>,
    pub detail: Option<String>,
}

/// Erreurs métier spécifiques aux Web Services SGE d'Enedis
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum SgeBusinessError {
    #[error("Consentement client absent ou révoqué pour le PRM {point_id}")]
    ConsentMissingOrExpired { point_id: PointId },

    #[error("PRM {point_id} introuvable, inactif ou hors périmètre contractuel")]
    PointNotFound { point_id: PointId },

    #[error("Plage temporelle invalide pour le PRM {point_id}: {reason}")]
    InvalidDateRange { point_id: PointId, reason: String },

    #[error("Quota d'appels SGE dépassé. Réinitialisation estimée à {reset_at:?}")]
    QuotaExceeded { reset_at: Option<DateTime<Utc>> },

    #[error("Erreur métier SGE non répertoriée [{code}]: {message}")]
    Generic { code: String, message: String },
}

/// Conduite recommandée pour l'Agent de collecte ou le superviseur
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResilienceAction {
    /// Réessayer après un délai calculé (Backoff exponentiel avec Jitter)
    RetryAfter(Duration),
    /// Arrêt fatal du processus (configuration corrompue, certificat révoqué ou mot de passe faux)
    FatalStop,
    /// Mettre ce PRM spécifique en pause et continuer le traitement des autres
    BlacklistPoint {
        point_id: PointId,
        duration: Duration,
    },
    /// Suspendre l'ensemble des requêtes du collecteur (ex: 429 ou quota SGE global)
    PauseCollector(Duration),
}

impl EnedisError {
    /// Analyse la nature de l'erreur pour déterminer la stratégie de résilience idoine
    pub fn classify(&self) -> ResilienceAction {
        match self {
            Self::Transport(TransportError::Timeout) => {
                ResilienceAction::RetryAfter(Duration::from_secs(5))
            }
            Self::Transport(TransportError::Network(_)) => {
                ResilienceAction::RetryAfter(Duration::from_secs(10))
            }
            Self::Http {
                status: 500..=504, ..
            } => ResilienceAction::RetryAfter(Duration::from_secs(30)),
            Self::Http { status: 429, .. } => {
                ResilienceAction::PauseCollector(Duration::from_secs(300))
            }
            Self::Business(SgeBusinessError::QuotaExceeded { .. }) => {
                ResilienceAction::PauseCollector(Duration::from_secs(3600))
            }
            Self::Business(SgeBusinessError::ConsentMissingOrExpired { point_id }) => {
                // Inutile de solliciter Enedis en boucle sans consentement renouvelé
                ResilienceAction::BlacklistPoint {
                    point_id: *point_id,
                    duration: Duration::from_secs(86400), // 24h
                }
            }
            Self::Business(SgeBusinessError::PointNotFound { point_id }) => {
                ResilienceAction::BlacklistPoint {
                    point_id: *point_id,
                    duration: Duration::from_secs(86400 * 7), // 7 jours
                }
            }
            Self::Business(SgeBusinessError::InvalidDateRange { point_id, .. }) => {
                ResilienceAction::BlacklistPoint {
                    point_id: *point_id,
                    duration: Duration::from_secs(86400),
                }
            }
            Self::Configuration(_) | Self::Tls(_) => ResilienceAction::FatalStop,
            Self::Http {
                status: 401 | 403, ..
            } => ResilienceAction::FatalStop,
            _ => ResilienceAction::RetryAfter(Duration::from_secs(60)),
        }
    }

    /// Code statut synthétique pour la colonne sync_status
    pub fn classify_status_code(&self) -> &'static str {
        match self {
            Self::Business(SgeBusinessError::ConsentMissingOrExpired { .. }) => "CONSENT_EXPIRED",
            Self::Business(SgeBusinessError::PointNotFound { .. }) => "POINT_NOT_FOUND",
            Self::Business(SgeBusinessError::QuotaExceeded { .. }) => "QUOTA_EXCEEDED",
            Self::Business(SgeBusinessError::InvalidDateRange { .. }) => "INVALID_RANGE",
            Self::Http { status: 429, .. } => "HTTP_429",
            Self::Http { status: 401, .. } => "UNAUTHORIZED",
            Self::Http { status: 403, .. } => "FORBIDDEN",
            Self::Transport(_) => "NETWORK_ERROR",
            Self::Tls(_) => "TLS_ERROR",
            _ => "GENERIC_ERROR",
        }
    }

    /// Diagnostic intelligible pour la commande `enedis doctor`
    pub fn doctor_explanation(&self) -> String {
        match self {
            Self::Tls(msg) => format!(
                "❌ Échec de la négociation mTLS : Vérifiez que le certificat client (.p12) est valide, non expiré et accepté par Enedis. ({})",
                msg
            ),
            Self::Business(SgeBusinessError::ConsentMissingOrExpired { point_id }) => format!(
                "⚠️ Consentement manquant pour le PRM {} : Le client doit valider le partage de ses données sur son espace Enedis ou via le portail de consentement.",
                point_id
            ),
            Self::Business(SgeBusinessError::PointNotFound { point_id }) => format!(
                "⚠️ PRM {} non trouvé : Vérifiez le numéro de PRM ou l'habilitation contractuelle de votre compte SGE.",
                point_id
            ),
            Self::Business(SgeBusinessError::QuotaExceeded { reset_at }) => format!(
                "⏳ Quota SGE atteint : Limite d'appels API journalière dépassée. Reprise suggérée à : {:?}.",
                reset_at
            ),
            Self::Http { status: 401, .. } => {
                "❌ 401 Non Autorisé : Certificat SGE ou compte Enedis non reconnu.".to_string()
            }
            Self::Http { status: 403, .. } => {
                "❌ 403 Accès Refusé : Vous n'avez pas les droits contractuels SGE sur ce service ou ce périmètre.".to_string()
            }
            _ => format!("Détail technique : {}", self),
        }
    }
}
