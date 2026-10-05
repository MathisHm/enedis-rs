use crate::models::AggregationInterval;
use serde::{Deserialize, Serialize};

/// Politique de rétention et de compression (Rollup / Downsampling)
/// Conçue pour maîtriser l'empreinte disque sur micro-serveurs (Raspberry Pi, NAS, SQLite)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicy {
    /// Âge à partir duquel les mesures fines (ex: pas de 30 minutes) sont archivées (en jours, défaut: 730 jours = 2 ans)
    pub raw_data_retention_days: u32,
    /// Intervalle cible pour le compactage : `AggregationInterval::Hourly` (1h) ou `AggregationInterval::Daily` (1 jour)
    pub rollup_interval: AggregationInterval,
    /// Seuil de purge définitive optionnel (ex: supprimer les données de plus de 10 ans)
    pub max_retention_days: Option<u32>,
    /// Déclencher une défragmentation physique (VACUUM SQLite) après le compactage
    pub auto_vacuum: bool,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            raw_data_retention_days: 730, // 2 ans
            rollup_interval: AggregationInterval::Daily,
            max_retention_days: None,
            auto_vacuum: true,
        }
    }
}

impl RetentionPolicy {
    /// Crée une politique avec seuil en jours et intervalle de regroupement
    pub fn new(raw_data_retention_days: u32, rollup_interval: AggregationInterval) -> Self {
        Self {
            raw_data_retention_days,
            rollup_interval,
            max_retention_days: None,
            auto_vacuum: true,
        }
    }

    /// Définit un seuil maximal de rétention (purge définitive au-delà)
    pub fn with_max_retention_days(mut self, days: u32) -> Self {
        self.max_retention_days = Some(days);
        self
    }

    /// Active ou désactive le VACUUM automatique
    pub fn with_auto_vacuum(mut self, enabled: bool) -> Self {
        self.auto_vacuum = enabled;
        self
    }
}

/// Statistiques issues d'une passe de compression (Rollup / Downsampling)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollupStats {
    /// Nombre de mesures brutes traitées et archivées
    pub raw_measurements_processed: usize,
    /// Nombre d'agrégats compactés créés
    pub rollups_created: usize,
    /// Nombre de mesures brutes supprimées après intégration
    pub raw_measurements_deleted: usize,
    /// Indique si la commande de défragmentation physique VACUUM a été exécutée
    pub vacuum_executed: bool,
}
