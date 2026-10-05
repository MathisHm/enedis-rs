use crate::client::config::ClientIdentitySource;
use crate::client::SgeClient;
use crate::error::EnedisError;
use crate::models::{FlowDirection, PointId};
#[cfg(feature = "storage")]
use crate::storage::StorageBackend;
use chrono::{Duration as ChronoDuration, Utc};
use secrecy::ExposeSecret;
use std::fs;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DoctorStatus {
    Success(String),
    Warning { message: String, advice: String },
    Failure { reason: String, remediation: String },
}

#[derive(Debug, Clone)]
pub struct DoctorReportItem {
    pub step: &'static str,
    pub status: DoctorStatus,
}

pub struct EnedisDoctor;

impl EnedisDoctor {
    /// Valide l'existence, la lisibilité et le format du certificat client
    pub fn check_certificate(identity: Option<&ClientIdentitySource>) -> DoctorReportItem {
        let step = "1. Certificat d'authentification mTLS client";

        let ident = match identity {
            Some(i) => i,
            None => {
                return DoctorReportItem {
                    step,
                    status: DoctorStatus::Warning {
                        message: "Aucune identité client mTLS configurée.".to_string(),
                        advice: "Renseignez le chemin vers votre certificat SGE (.p12 ou PEM) via la configuration ou la CLI.".to_string(),
                    },
                };
            }
        };

        match ident {
            ClientIdentitySource::Pkcs12File { path, password } => {
                if !path.exists() {
                    return DoctorReportItem {
                        step,
                        status: DoctorStatus::Failure {
                            reason: format!("Fichier PKCS#12 introuvable: {:?}", path),
                            remediation:
                                "Vérifiez le chemin du fichier de certificat SGE fourni par Enedis."
                                    .to_string(),
                        },
                    };
                }

                match fs::read(path) {
                    Err(e) => DoctorReportItem {
                        step,
                        status: DoctorStatus::Failure {
                            reason: format!(
                                "Permission refusée lors de la lecture de {:?}: {}",
                                path, e
                            ),
                            remediation:
                                "Ajustez les droits d'accès système (ex: chmod 600 cert.p12)."
                                    .to_string(),
                        },
                    },
                    Ok(der) => {
                        // Tenter le déchiffrement PKCS#12
                        match reqwest::Identity::from_pkcs12_der(&der, password.expose_secret()) {
                            Ok(_) => DoctorReportItem {
                                step,
                                status: DoctorStatus::Success(format!(
                                    "Fichier PKCS#12 {:?} valide et mot de passe vérifié avec succès.",
                                    path
                                )),
                            },
                            Err(e) => DoctorReportItem {
                                step,
                                status: DoctorStatus::Failure {
                                    reason: format!("Échec du déchiffrement du conteneur PKCS#12: {}", e),
                                    remediation: "Le mot de passe du certificat est incorrect ou le conteneur est corrompu.".to_string(),
                                },
                            },
                        }
                    }
                }
            }
            ClientIdentitySource::PemFile {
                cert_path,
                key_path,
            } => {
                if !cert_path.exists() || !key_path.exists() {
                    return DoctorReportItem {
                        step,
                        status: DoctorStatus::Failure {
                            reason: "Certificat PEM ou clé privée manquante sur disque."
                                .to_string(),
                            remediation:
                                "Vérifiez que les fichiers PEM existent et sont accessibles."
                                    .to_string(),
                        },
                    };
                }
                DoctorReportItem {
                    step,
                    status: DoctorStatus::Success("Fichiers PEM client présents.".to_string()),
                }
            }
            ClientIdentitySource::Pkcs12Der { .. } | ClientIdentitySource::PemBytes { .. } => {
                DoctorReportItem {
                    step,
                    status: DoctorStatus::Success(
                        "Certificats mTLS chargés en mémoire.".to_string(),
                    ),
                }
            }
        }
    }

    /// Teste la connectivité et la négociation TLS vers l'endpoint SGE
    pub async fn check_network_and_tls(client: &SgeClient) -> DoctorReportItem {
        let step = "2. Négociation mTLS & Connexion réseau SGE";

        // Envoi d'une requête SOAP minimale pour vérifier la poignée de main TLS et le certificat
        let test_payload = "<ping xmlns=\"http://www.enedis.fr/sge/ws/v1\"/>";
        match client
            .send_soap_request("http://www.enedis.fr/sge/ws/v1/ping", test_payload)
            .await
        {
            Ok(_) => DoctorReportItem {
                step,
                status: DoctorStatus::Success(
                    "Connectivité et mTLS validés avec Enedis SGE.".to_string(),
                ),
            },
            Err(err) => {
                let explanation = err.doctor_explanation();
                match err {
                    EnedisError::Http { status: 401 | 403, .. } => DoctorReportItem {
                        step,
                        status: DoctorStatus::Failure {
                            reason: "Authentification rejetée par le portail Enedis.".to_string(),
                            remediation: "Votre certificat client mTLS n'a pas été associé à votre compte SGE ou a été révoqué par Enedis.".to_string(),
                        },
                    },
                    EnedisError::Tls(msg) => DoctorReportItem {
                        step,
                        status: DoctorStatus::Failure {
                            reason: format!("Échec poignée de main TLS: {}", msg),
                            remediation: "Vérifiez que vous utilisez bien la suite cryptographique ou la CA requise par Enedis (activez native-tls-fallback si nécessaire).".to_string(),
                        },
                    },
                    _ => DoctorReportItem {
                        step,
                        status: DoctorStatus::Success(format!(
                            "Négociation TLS réussie (le serveur a répondu au niveau applicatif: {}).",
                            explanation
                        )),
                    },
                }
            }
        }
    }

    /// Valide l'éligibilité et le consentement pour un PRM de test
    pub async fn check_prm(client: &SgeClient, prm: PointId) -> DoctorReportItem {
        let step = "3. Vérification du consentement et des habilitations SGE";
        let now = Utc::now();
        let from = now - ChronoDuration::days(1);

        match client
            .fetch_measurements(prm, from, now, FlowDirection::Consumption)
            .await
        {
            Ok(m) => DoctorReportItem {
                step,
                status: DoctorStatus::Success(format!(
                    "PRM {} accessible et consentement actif ({} mesures relevées).",
                    prm,
                    m.len()
                )),
            },
            Err(err) => {
                let explanation = err.doctor_explanation();
                DoctorReportItem {
                    step,
                    status: DoctorStatus::Failure {
                        reason: explanation,
                        remediation: "Si le consentement est échu, demander à l'usager de renouveler son accord sur le portail Enedis.".to_string(),
                    },
                }
            }
        }
    }

    /// Contrôle proactif de l'échéance et du cycle de vie du consentement client
    pub async fn check_consent_proactive(
        provider: &dyn crate::client::EnedisProvider,
        prm: PointId,
        warning_threshold_days: u32,
    ) -> DoctorReportItem {
        let step = "3b. Suivi du cycle de vie du consentement client";
        match provider.fetch_consent_status(prm).await {
            Ok(consent) => {
                let alert = consent.check_alert(warning_threshold_days, Utc::now());
                match alert.severity {
                    crate::models::ConsentAlertSeverity::Healthy => DoctorReportItem {
                        step,
                        status: DoctorStatus::Success(format!(
                            "Consentement valide pour le PRM {} (encore {} jours de validité).",
                            prm, alert.days_remaining
                        )),
                    },
                    crate::models::ConsentAlertSeverity::Warning => DoctorReportItem {
                        step,
                        status: DoctorStatus::Warning {
                            message: alert.message,
                            advice: "Prévoyez le renouvellement du consentement dans les prochaines semaines.".to_string(),
                        },
                    },
                    crate::models::ConsentAlertSeverity::Critical => DoctorReportItem {
                        step,
                        status: DoctorStatus::Warning {
                            message: alert.message,
                            advice: "URGENT : Renouvelez le consentement avant coupure de la collecte!".to_string(),
                        },
                    },
                    crate::models::ConsentAlertSeverity::Expired => DoctorReportItem {
                        step,
                        status: DoctorStatus::Failure {
                            reason: alert.message,
                            remediation: "Demandez au titulaire du contrat de renouveler son autorisation Enedis.".to_string(),
                        },
                    },
                }
            }
            Err(err) => DoctorReportItem {
                step,
                status: DoctorStatus::Failure {
                    reason: err.doctor_explanation(),
                    remediation:
                        "Vérifiez que le compte dispose des droits d'interrogation du consentement."
                            .to_string(),
                },
            },
        }
    }

    /// Contrôle de l'accès aux données contractuelles
    pub async fn check_contract(
        provider: &dyn crate::client::EnedisProvider,
        prm: PointId,
    ) -> DoctorReportItem {
        let step = "3c. Données contractuelles et caractéristiques compteur";
        match provider.fetch_contract_data(prm).await {
            Ok(contract) => {
                let meter_type = contract
                    .meter
                    .as_ref()
                    .map(|m| format!("{:?}", m.meter_type))
                    .unwrap_or_else(|| "Inconnu".to_string());
                DoctorReportItem {
                    step,
                    status: DoctorStatus::Success(format!(
                        "Données contractuelles récupérées (Puissance: {} kVA, Option: {:?}, Compteur: {}).",
                        contract.subscribed_power_kva, contract.tariff_option, meter_type
                    )),
                }
            }
            Err(err) => DoctorReportItem {
                step,
                status: DoctorStatus::Failure {
                    reason: err.doctor_explanation(),
                    remediation:
                        "Vérifiez l'habilitation contractuelle pour ce point de livraison."
                            .to_string(),
                },
            },
        }
    }

    /// Valide la disponibilité de la base de données
    #[cfg(feature = "storage")]
    pub async fn check_storage(storage: &(dyn StorageBackend + '_)) -> DoctorReportItem {
        let step = "4. Persistance & Base de données temporelle";
        match storage.list_sync_points().await {
            Ok(points) => DoctorReportItem {
                step,
                status: DoctorStatus::Success(format!(
                    "Base de données opérationnelle ({} PRM enregistrés pour synchronisation).",
                    points.len()
                )),
            },
            Err(err) => DoctorReportItem {
                step,
                status: DoctorStatus::Failure {
                    reason: format!("Erreur de connexion SQL: {}", err),
                    remediation: "Vérifiez la chaîne de connexion (DATABASE_URL) et que les migrations ont été exécutées.".to_string(),
                },
            },
        }
    }

    /// Affiche un rapport complet formaté en console
    pub fn format_report(items: &[DoctorReportItem]) -> String {
        let mut out = String::new();
        out.push_str("\n🏥 === RAPPORT DE DIAGNOSTIC ENEDIS DOCTOR ===\n\n");

        for item in items {
            match &item.status {
                DoctorStatus::Success(msg) => {
                    out.push_str(&format!("  ✅ {}\n     {}\n\n", item.step, msg));
                }
                DoctorStatus::Warning { message, advice } => {
                    out.push_str(&format!(
                        "  ⚠️  {}\n     {}\n     👉 Conseil : {}\n\n",
                        item.step, message, advice
                    ));
                }
                DoctorStatus::Failure {
                    reason,
                    remediation,
                } => {
                    out.push_str(&format!(
                        "  ❌ {}\n     {}\n     👉 Action corrective : {}\n\n",
                        item.step, reason, remediation
                    ));
                }
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_check_certificate_none() {
        let report = EnedisDoctor::check_certificate(None);
        assert_eq!(report.step, "1. Certificat d'authentification mTLS client");
        match report.status {
            DoctorStatus::Warning { message, advice } => {
                assert!(message.contains("Aucune identité"));
                assert!(advice.contains("Renseignez le chemin"));
            }
            other => panic!("Statut inattendu: {:?}", other),
        }
    }

    #[test]
    fn test_check_certificate_pkcs12_file_not_found() {
        let ident = ClientIdentitySource::Pkcs12File {
            path: PathBuf::from("does_not_exist_cert.p12"),
            password: secrecy::SecretString::new("test".to_string()),
        };
        let report = EnedisDoctor::check_certificate(Some(&ident));
        match report.status {
            DoctorStatus::Failure {
                reason,
                remediation,
            } => {
                assert!(reason.contains("introuvable"));
                assert!(remediation.contains("Vérifiez le chemin"));
            }
            other => panic!("Statut inattendu: {:?}", other),
        }
    }

    #[test]
    fn test_check_certificate_pem_file_not_found() {
        let ident = ClientIdentitySource::PemFile {
            cert_path: PathBuf::from("does_not_exist.crt"),
            key_path: PathBuf::from("does_not_exist.key"),
        };
        let report = EnedisDoctor::check_certificate(Some(&ident));
        match report.status {
            DoctorStatus::Failure { reason, .. } => {
                assert!(reason.contains("manquante sur disque"));
            }
            other => panic!("Statut inattendu: {:?}", other),
        }
    }

    #[test]
    fn test_format_report() {
        let items = vec![
            DoctorReportItem {
                step: "1. Certificat",
                status: DoctorStatus::Success("OK".to_string()),
            },
            DoctorReportItem {
                step: "2. Réseau",
                status: DoctorStatus::Warning {
                    message: "Latence élevée".to_string(),
                    advice: "Vérifiez votre liaison".to_string(),
                },
            },
            DoctorReportItem {
                step: "3. Consentement",
                status: DoctorStatus::Failure {
                    reason: "Expiré".to_string(),
                    remediation: "Renouveler sur l'espace client".to_string(),
                },
            },
        ];

        let formatted = EnedisDoctor::format_report(&items);
        assert!(formatted.contains("RAPPORT DE DIAGNOSTIC ENEDIS DOCTOR"));
        assert!(formatted.contains("✅ 1. Certificat"));
        assert!(formatted.contains("⚠️  2. Réseau"));
        assert!(formatted.contains("❌ 3. Consentement"));
    }
}
