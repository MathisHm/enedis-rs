use crate::models::{audit_subscription_sizing, MaxPowerRecord, Measurement, PointId};
use wasm_bindgen::prelude::*;

/// Initialise le gestionnaire de panique pour afficher les erreurs détaillées dans la console JS
#[wasm_bindgen]
pub fn wasm_init() {
    console_error_panic_hook::set_once();
}

/// Parse une chaîne JSON contenant un tableau de mesures Linky
fn parse_measurements(json_str: &str) -> Result<Vec<Measurement>, String> {
    serde_json::from_str::<Vec<Measurement>>(json_str)
        .map_err(|e| format!("Erreur de désérialisation JSON des mesures : {}", e))
}

/// Audit du dimensionnement de puissance souscrite (kVA)
///
/// Renvoie un JSON contenant le diagnostic de puissance (Oversized, Undersized, WellSized),
/// la pointe observée, la recommandation et le potentiel d'économies annuelles sur l'abonnement.
#[wasm_bindgen]
pub fn wasm_audit_subscription(
    measurements_json: &str,
    prm: &str,
    subscribed_power_kva: u32,
) -> Result<String, String> {
    let measurements = parse_measurements(measurements_json)?;
    let point_id = PointId::new(prm).map_err(|e| e.to_string())?;

    let max_records = if !measurements.is_empty() {
        let max_m = measurements.iter().max_by_key(|m| m.power_w()).cloned();
        if let Some(m) = max_m {
            vec![MaxPowerRecord {
                point_id,
                timestamp: m.timestamp,
                value: m.power_w(),
                unit: crate::models::Unit::Watt,
            }]
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let audit = audit_subscription_sizing(point_id, subscribed_power_kva, &max_records);
    serde_json::to_string(&audit)
        .map_err(|e| format!("Erreur de sérialisation JSON de l'audit : {}", e))
}
