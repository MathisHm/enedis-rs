use crate::models::{FlowDirection, PointId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Statistiques retournées après une opération d'UPSERT par lot
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpsertStats {
    /// Nombre total de mesures traitées dans le lot
    pub processed: usize,
    /// Nombre de mesures insérées ou mises à jour avec succès
    pub affected: usize,
}

/// État de synchronisation d'un PRM pour l'Agent de collecte
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "01234567890123"))]
    pub point_id: PointId,
    pub direction: FlowDirection,
    /// Dernier horodatage collecté avec succès
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T10:00:00Z"))]
    pub last_synced_timestamp: DateTime<Utc>,
    /// Horodatage de la dernière tentative de collecte
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T10:00:00Z"))]
    pub last_sync_attempt: DateTime<Utc>,
    /// Statut de synchronisation (ex: "OK", "ERROR_CONSENT", "PAUSED")
    #[cfg_attr(feature = "openapi", schema(example = "OK"))]
    pub sync_status: String,
}

/// Statistiques retournées après une passe de détection et rattrapage chirurgical (Backfill)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackfillStats {
    /// Nombre de plages temporelles de trous détectées
    pub ranges_detected: usize,
    /// Nombre de requêtes effectuées auprès d'Enedis SGE
    pub requests_made: usize,
    /// Nombre total de mesures récupérées et stockées
    pub measurements_recovered: usize,
}
