use crate::models::{PointId, Unit};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Enregistrement d'une pointe maximale quotidienne de puissance atteinte
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaxPowerRecord {
    /// Point de livraison (PRM)
    pub point_id: PointId,
    /// Horodatage de survenance de la pointe
    pub timestamp: DateTime<Utc>,
    /// Valeur maximale de puissance relevée
    pub value: Decimal,
    /// Unité de la mesure (généralement Watt ou VoltAmpere)
    pub unit: Unit,
}

impl MaxPowerRecord {
    /// Convertit la valeur de pointe en kVA (ou kW)
    pub fn value_kva(&self) -> Decimal {
        match self.unit {
            Unit::Watt | Unit::VoltAmpere => self.value / Decimal::from(1000),
            Unit::KiloWatt | Unit::KiloVoltAmpere => self.value,
            _ => self.value,
        }
    }

    /// Convertit la valeur de pointe en Watts (ou VoltAmpère)
    pub fn value_w(&self) -> Decimal {
        match self.unit {
            Unit::Watt | Unit::VoltAmpere => self.value,
            Unit::KiloWatt | Unit::KiloVoltAmpere => self.value * Decimal::from(1000),
            _ => self.value,
        }
    }
}

/// Statut de l'adéquation du dimensionnement de l'abonnement électrique
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SizingStatus {
    /// Puissance souscrite optimale
    Optimal,
    /// Abonnement surdimensionné (économies potentielles sur l'abonnement fixe)
    Oversized,
    /// Abonnement sous-dimensionné (risque de disjonction)
    Undersized,
}

/// Rapport d'audit de dimensionnement de la puissance souscrite
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionAudit {
    /// Point de livraison (PRM)
    pub point_id: PointId,
    /// Puissance actuellement souscrite (en kVA)
    pub subscribed_power_kva: u32,
    /// Pointe maximale constatée sur la période observée (en kVA)
    pub peak_reached_kva: Decimal,
    /// Date et heure de la pointe maximale
    pub peak_timestamp: Option<DateTime<Utc>>,
    /// Puissance recommandée selon les paliers standards Enedis (3, 6, 9, 12, 15, 18, 24, 30, 36)
    pub recommended_power_kva: u32,
    /// Évaluation du dimensionnement
    pub sizing_status: SizingStatus,
    /// Explications détaillées et préconisations concrètes
    pub recommendation: String,
}

/// Analyse un historique de pointes de puissance maximale pour évaluer le dimensionnement de l'abonnement
pub fn audit_subscription_sizing(
    point_id: PointId,
    subscribed_power_kva: u32,
    records: &[MaxPowerRecord],
) -> SubscriptionAudit {
    if records.is_empty() {
        return SubscriptionAudit {
            point_id,
            subscribed_power_kva,
            peak_reached_kva: Decimal::ZERO,
            peak_timestamp: None,
            recommended_power_kva: subscribed_power_kva,
            sizing_status: SizingStatus::Optimal,
            recommendation:
                "Aucune mesure de pointe disponible sur la période pour évaluer l'abonnement."
                    .to_string(),
        };
    }

    let mut peak_val = Decimal::ZERO;
    let mut peak_time = None;

    for r in records {
        let kva = r.value_kva();
        if kva > peak_val {
            peak_val = kva;
            peak_time = Some(r.timestamp);
        }
    }

    // Paliers contractuels monophasés et triphasés basse tension Enedis
    let standard_tiers = [3, 6, 9, 12, 15, 18, 24, 30, 36];

    // Marge de sécurité préconisée de 10%
    let peak_with_margin = peak_val * Decimal::from_str_exact("1.10").unwrap_or(Decimal::ONE);
    let peak_float = peak_with_margin.to_string().parse::<f64>().unwrap_or(0.0);

    let recommended = standard_tiers
        .iter()
        .copied()
        .find(|&tier| (tier as f64) >= peak_float)
        .unwrap_or(36);

    let subscribed_float = subscribed_power_kva as f64;
    let actual_peak_float = peak_val.to_string().parse::<f64>().unwrap_or(0.0);

    let (sizing_status, recommendation) = if actual_peak_float > subscribed_float * 0.98 {
        (
            SizingStatus::Undersized,
            format!(
                "Pointe observée ({:.2} kVA) frôlant ou dépassant la puissance souscrite ({} kVA). Risque de coupure Linky. Passage préconisé à {} kVA.",
                actual_peak_float, subscribed_power_kva, recommended
            ),
        )
    } else if subscribed_power_kva > 3 && recommended < subscribed_power_kva {
        (
            SizingStatus::Oversized,
            format!(
                "Pointe maximale constatée de {:.2} kVA pour un abonnement de {} kVA. Un palier de {} kVA (avec marge de sécurité) permettrait de réaliser des économies sur la part fixe d'abonnement.",
                actual_peak_float, subscribed_power_kva, recommended
            ),
        )
    } else {
        (
            SizingStatus::Optimal,
            format!(
                "Puissance souscrite de {} kVA adaptée à la pointe maximale constatée de {:.2} kVA.",
                subscribed_power_kva, actual_peak_float
            ),
        )
    };

    SubscriptionAudit {
        point_id,
        subscribed_power_kva,
        peak_reached_kva: peak_val,
        peak_timestamp: peak_time,
        recommended_power_kva: recommended,
        sizing_status,
        recommendation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_subscription_audit_optimal() {
        let prm = PointId::new("01234567890123").unwrap();
        let records = vec![MaxPowerRecord {
            point_id: prm,
            timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 18, 0, 0).unwrap(),
            value: Decimal::from(4500),
            unit: Unit::Watt,
        }];
        let audit = audit_subscription_sizing(prm, 6, &records);
        assert_eq!(audit.sizing_status, SizingStatus::Optimal);
        assert_eq!(audit.recommended_power_kva, 6);
    }

    #[test]
    fn test_subscription_audit_oversized() {
        let prm = PointId::new("01234567890123").unwrap();
        let records = vec![MaxPowerRecord {
            point_id: prm,
            timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 18, 0, 0).unwrap(),
            value: Decimal::from(4000), // 4 kVA max
            unit: Unit::Watt,
        }];
        let audit = audit_subscription_sizing(prm, 12, &records);
        assert_eq!(audit.sizing_status, SizingStatus::Oversized);
        assert_eq!(audit.recommended_power_kva, 6);
    }

    #[test]
    fn test_subscription_audit_undersized() {
        let prm = PointId::new("01234567890123").unwrap();
        let records = vec![MaxPowerRecord {
            point_id: prm,
            timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 18, 0, 0).unwrap(),
            value: Decimal::from(6200), // 6.2 kVA > 6 kVA
            unit: Unit::Watt,
        }];
        let audit = audit_subscription_sizing(prm, 6, &records);
        assert_eq!(audit.sizing_status, SizingStatus::Undersized);
        assert_eq!(audit.recommended_power_kva, 9);
    }
}
