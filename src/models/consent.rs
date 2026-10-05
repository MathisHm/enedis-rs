use crate::models::PointId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Statut du consentement client Enedis
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConsentStatus {
    /// Consentement valide et actif
    Active,
    /// Consentement arrivé à son terme temporel
    Expired,
    /// Consentement révoqué explicitement par le titulaire
    Revoked,
    /// Consentement en attente de signature ou de validation
    Pending,
    /// Statut non standard
    #[serde(untagged)]
    Other(String),
}

impl ConsentStatus {
    pub fn from_sge_code(code: &str) -> Self {
        match code.trim().to_uppercase().as_str() {
            "ACTIF" | "ACTIVE" | "VALIDE" => Self::Active,
            "EXPIRE" | "EXPIRED" | "ECHU" => Self::Expired,
            "REVOQUE" | "REVOKED" | "ANNULE" => Self::Revoked,
            "EN_ATTENTE" | "PENDING" => Self::Pending,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Active => "ACTIF",
            Self::Expired => "EXPIRE",
            Self::Revoked => "REVOQUE",
            Self::Pending => "EN_ATTENTE",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Informations sur le cycle de vie du consentement client pour un PRM
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentInfo {
    /// Point de livraison (PRM)
    pub point_id: PointId,
    /// Statut actuel du consentement
    pub status: ConsentStatus,
    /// Date d'effet du consentement
    pub valid_from: Option<DateTime<Utc>>,
    /// Date d'expiration du consentement
    pub valid_to: Option<DateTime<Utc>>,
    /// Liste des usages / opérations autorisés (ex: "consulterMesures", "consulterPuissanceMax")
    pub authorized_usages: Vec<String>,
}

/// Niveau d'urgence d'une alerte d'expiration de consentement
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsentAlertSeverity {
    /// Consentement déjà expiré ou résilié
    Expired,
    /// Expiration critique imminente (<= 7 jours)
    Critical,
    /// Expiration proche (<= seuil d'avertissement configuré, ex: 30 jours)
    Warning,
    /// Consentement valide sans alerte
    Healthy,
}

/// Alerte proactive sur l'expiration d'un consentement client
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentAlert {
    /// Point de livraison (PRM)
    pub point_id: PointId,
    /// Niveau d'alerte
    pub severity: ConsentAlertSeverity,
    /// Nombre de jours restants (négatif si déjà expiré)
    pub days_remaining: i64,
    /// Date d'expiration constatée
    pub valid_to: Option<DateTime<Utc>>,
    /// Message explicite d'alerte
    pub message: String,
}

impl ConsentInfo {
    /// Calcule le nombre de jours calendaires restant avant expiration
    pub fn days_until_expiration(&self, now: DateTime<Utc>) -> Option<i64> {
        self.valid_to.map(|to| (to - now).num_days())
    }

    /// Évalue l'état du consentement et génère une alerte proactive selon le seuil fourni
    pub fn check_alert(&self, warning_threshold_days: u32, now: DateTime<Utc>) -> ConsentAlert {
        if !self.status.is_active() {
            return ConsentAlert {
                point_id: self.point_id,
                severity: ConsentAlertSeverity::Expired,
                days_remaining: 0,
                valid_to: self.valid_to,
                message: format!(
                    "Consentement non actif ({}) pour le PRM {}.",
                    self.status.as_str(),
                    self.point_id
                ),
            };
        }

        match self.valid_to {
            None => ConsentAlert {
                point_id: self.point_id,
                severity: ConsentAlertSeverity::Healthy,
                days_remaining: i64::MAX,
                valid_to: None,
                message: format!(
                    "Consentement actif à durée indéterminée pour le PRM {}.",
                    self.point_id
                ),
            },
            Some(exp) => {
                let diff = exp - now;
                let days = diff.num_days();

                if days <= 0 {
                    ConsentAlert {
                        point_id: self.point_id,
                        severity: ConsentAlertSeverity::Expired,
                        days_remaining: days,
                        valid_to: Some(exp),
                        message: format!(
                            "Consentement expiré pour le PRM {} (échu le {}).",
                            self.point_id,
                            exp.format("%Y-%m-%d")
                        ),
                    }
                } else if days <= 7 {
                    ConsentAlert {
                        point_id: self.point_id,
                        severity: ConsentAlertSeverity::Critical,
                        days_remaining: days,
                        valid_to: Some(exp),
                        message: format!(
                            "URGENT: Le consentement pour le PRM {} expire dans {} jour(s) (le {}). Renouvellement requis!",
                            self.point_id,
                            days,
                            exp.format("%Y-%m-%d")
                        ),
                    }
                } else if days <= warning_threshold_days as i64 {
                    ConsentAlert {
                        point_id: self.point_id,
                        severity: ConsentAlertSeverity::Warning,
                        days_remaining: days,
                        valid_to: Some(exp),
                        message: format!(
                            "Avertissement: Le consentement pour le PRM {} expire dans {} jours (le {}).",
                            self.point_id,
                            days,
                            exp.format("%Y-%m-%d")
                        ),
                    }
                } else {
                    ConsentAlert {
                        point_id: self.point_id,
                        severity: ConsentAlertSeverity::Healthy,
                        days_remaining: days,
                        valid_to: Some(exp),
                        message: format!(
                            "Consentement valide pour le PRM {} (encore {} jours de validité).",
                            self.point_id, days
                        ),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    #[test]
    fn test_consent_alert_severities() {
        let prm = PointId::new("01234567890123").unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 29, 12, 0, 0).unwrap();

        // Expired
        let expired_info = ConsentInfo {
            point_id: prm,
            status: ConsentStatus::Active,
            valid_from: Some(now - Duration::days(365)),
            valid_to: Some(now - Duration::days(2)),
            authorized_usages: vec!["COURBE".to_string()],
        };
        let alert = expired_info.check_alert(30, now);
        assert_eq!(alert.severity, ConsentAlertSeverity::Expired);

        // Critical (<= 7 days)
        let critical_info = ConsentInfo {
            point_id: prm,
            status: ConsentStatus::Active,
            valid_from: Some(now - Duration::days(360)),
            valid_to: Some(now + Duration::days(4)),
            authorized_usages: vec!["COURBE".to_string()],
        };
        let alert = critical_info.check_alert(30, now);
        assert_eq!(alert.severity, ConsentAlertSeverity::Critical);

        // Warning (<= 30 days)
        let warning_info = ConsentInfo {
            point_id: prm,
            status: ConsentStatus::Active,
            valid_from: Some(now - Duration::days(340)),
            valid_to: Some(now + Duration::days(20)),
            authorized_usages: vec!["COURBE".to_string()],
        };
        let alert = warning_info.check_alert(30, now);
        assert_eq!(alert.severity, ConsentAlertSeverity::Warning);

        // Healthy (> 30 days)
        let healthy_info = ConsentInfo {
            point_id: prm,
            status: ConsentStatus::Active,
            valid_from: Some(now - Duration::days(100)),
            valid_to: Some(now + Duration::days(90)),
            authorized_usages: vec!["COURBE".to_string()],
        };
        let alert = healthy_info.check_alert(30, now);
        assert_eq!(alert.severity, ConsentAlertSeverity::Healthy);
    }
}
