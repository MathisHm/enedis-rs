use crate::models::PointId;
use serde::{Deserialize, Serialize};

/// Option tarifaire souscrite auprès du fournisseur d'énergie
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TariffOption {
    /// Option Base (tarif unique)
    Base,
    /// Heures Pleines / Heures Creuses (HP/HC)
    #[serde(
        rename = "HEURES_PLEINES_HEURES_CREUSES",
        alias = "HP_HC",
        alias = "HPHC"
    )]
    HeuresPleinesCreuses,
    /// Option Tempo (Bleu, Blanc, Rouge)
    Tempo,
    /// Option Effacement Jour de Pointe (EJP)
    Ejp,
    /// Autre formule tarifaire spécifique
    #[serde(untagged)]
    Other(String),
}

impl TariffOption {
    pub fn from_sge_code(code: &str) -> Self {
        match code.trim().to_uppercase().as_str() {
            "BASE" | "BTINFMUT" => Self::Base,
            "HEURES_PLEINES_HEURES_CREUSES" | "HP_HC" | "HPHC" | "BTINFMUD" => {
                Self::HeuresPleinesCreuses
            }
            "TEMPO" | "BTINFMUTEMPO" => Self::Tempo,
            "EJP" | "BTINFMEJP" => Self::Ejp,
            other => Self::Other(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Base => "BASE",
            Self::HeuresPleinesCreuses => "HEURES_PLEINES_HEURES_CREUSES",
            Self::Tempo => "TEMPO",
            Self::Ejp => "EJP",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Type de compteur électrique
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MeterType {
    /// Compteur communicant Linky
    Linky,
    /// Compteur électronique (CBE)
    Electronic,
    /// Compteur électromécanique classique
    Electromechanical,
    /// Autre
    #[serde(untagged)]
    Other(String),
}

impl MeterType {
    pub fn from_sge_code(code: &str) -> Self {
        let code_up = code.trim().to_uppercase();
        if code_up.contains("LINKY") || code_up == "CME" {
            Self::Linky
        } else if code_up.contains("ELECTRO") || code_up == "CBE" {
            Self::Electronic
        } else if code_up.contains("MECAN") {
            Self::Electromechanical
        } else {
            Self::Other(code.to_string())
        }
    }
}

/// Type de raccordement réseau / Nombre de phases
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PhaseCount {
    /// Raccordement monophasé (230V)
    SinglePhase,
    /// Raccordement triphasé (400V)
    ThreePhase,
    /// Indéterminé
    #[serde(untagged)]
    Unknown(String),
}

impl PhaseCount {
    pub fn from_sge_code(code: &str) -> Self {
        let code_up = code.trim().to_uppercase();
        if code_up.contains("MONO") || code_up == "1" {
            Self::SinglePhase
        } else if code_up.contains("TRI") || code_up == "3" {
            Self::ThreePhase
        } else {
            Self::Unknown(code.to_string())
        }
    }
}

/// Caractéristiques techniques du compteur électrique
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeterCharacteristics {
    /// Numéro de série / Matricule Enedis du compteur
    pub serial_number: Option<String>,
    /// Type de compteur (Linky, électronique, électromécanique)
    pub meter_type: MeterType,
    /// Nombre de phases de l'installation
    pub phase_count: PhaseCount,
    /// Calibre ou réglage maximal en Ampères
    pub circuit_breaker_amperes: Option<u32>,
}

/// Calendrier fournisseur / Jours fériés / Plages tarifaires
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CalendarSchedule {
    /// Libellé commercial du calendrier
    pub schedule_name: Option<String>,
    /// Plages horaires des heures creuses (ex: ["22:00-06:00"] ou ["12:00-14:00", "01:30-07:30"])
    pub off_peak_ranges: Vec<String>,
    /// Couleur Tempo pour les contrats concernés (BLEU, BLANC, ROUGE)
    pub tempo_color: Option<String>,
    /// Mode saisonnier ou profil de saison
    pub seasonal_mode: Option<String>,
}

/// Données contractuelles complètes relatives à un PRM
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractData {
    /// Point de livraison (PRM)
    pub point_id: PointId,
    /// Puissance souscrite en kVA (ex: 6, 9, 12)
    pub subscribed_power_kva: u32,
    /// Option tarifaire
    pub tariff_option: TariffOption,
    /// Calendrier et plages horaires
    pub calendar: Option<CalendarSchedule>,
    /// Caractéristiques techniques du compteur
    pub meter: Option<MeterCharacteristics>,
    /// Statut contractuel du contrat d'accès au réseau (ex: "ACTIF")
    pub status: Option<String>,
}
