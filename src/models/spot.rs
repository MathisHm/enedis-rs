use crate::models::tariff::{to_french_local_time, TaxConfig};
use crate::models::{FlowDirection, Measurement, PointId};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Timelike, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Enregistrement d'un prix de marché spot de l'électricité (Day-Ahead / EPEX SPOT France)
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotPriceRecord {
    /// Horodatage de début de l'heure (UTC)
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-30T14:00:00Z"))]
    pub timestamp: DateTime<Utc>,
    /// Prix spot brut en Euros par Mégawattheure (€/MWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "65.40"))]
    pub price_eur_per_mwh: Decimal,
    /// Prix spot brut ramené au Kilowattheure (€/kWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.0654"))]
    pub price_eur_per_kwh: Decimal,
    /// Indique si le prix de gros est négatif (< 0 €/MWh)
    pub is_negative: bool,
    /// Source de la donnée de marché (ex: "EPEX_SPOT_FR", "ENTSO_E", "RTE_ECO2MIX")
    #[cfg_attr(feature = "openapi", schema(example = "EPEX_SPOT_FR"))]
    pub source: String,
}

impl SpotPriceRecord {
    /// Crée un enregistrement de prix spot horaire
    pub fn new(timestamp: DateTime<Utc>, price_eur_per_mwh: Decimal, source: Option<&str>) -> Self {
        let price_eur_per_kwh = (price_eur_per_mwh / Decimal::from(1000)).round_dp(6);
        let is_negative = price_eur_per_mwh < Decimal::ZERO;
        Self {
            timestamp,
            price_eur_per_mwh,
            price_eur_per_kwh,
            is_negative,
            source: source.unwrap_or("EPEX_SPOT_FR").to_string(),
        }
    }

    /// Calcule le prix unitaire final TTC en €/kWh pour le consommateur résidentiel
    /// incluant la marge fournisseur, l'accise (TICFE) et la TVA sur la consommation (20%).
    pub fn consumer_price_ttc(&self, margin_kwh: Decimal, tax: &TaxConfig) -> Decimal {
        let price_ht = self.price_eur_per_kwh + margin_kwh + tax.ticfe_per_kwh;
        let vat_multiplier = Decimal::ONE + tax.vat_consumption_rate;
        (price_ht * vat_multiplier).round_dp(4)
    }
}

/// Opportunité d'arbitrage et de pilotage de flexibilité sur les prix spot
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotArbitrageOpportunity {
    /// Nombre total d'heures à prix négatifs sur la période
    #[cfg_attr(feature = "openapi", schema(example = 14))]
    pub negative_hours_count: usize,
    /// Prix minimum constaté sur la période (€/MWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "-24.50"))]
    pub min_price_eur_per_mwh: Decimal,
    /// Prix maximum constaté sur la période (€/MWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "182.00"))]
    pub max_price_eur_per_mwh: Decimal,
    /// Prix spot moyen arithmétique (€/MWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "68.20"))]
    pub avg_price_eur_per_mwh: Decimal,
    /// Économie potentielle annuelle estimée (€/an) en déplaçant les usages flexibles (recharge VE, ECS)
    /// sur les créneaux spot les plus avantageux plutôt qu'aux heures pleines
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "185.50"))]
    pub annual_arbitrage_savings_euro: Decimal,
    /// Recommandation opérationnelle pour le pilotage des charges
    pub recommendation: String,
}

/// Bilan et corrélation de la courbe de consommation avec les prix de marché spot
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpotProfileAnalysis {
    /// PRM audité
    pub point_id: PointId,
    /// Consommation totale analysée (kWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "450.5"))]
    pub total_energy_kwh: Decimal,
    /// Prix spot moyen pondéré par les kWh réels consommés par le foyer (€/MWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "71.30"))]
    pub weighted_average_spot_price_mwh: Decimal,
    /// Prix spot moyen arithmétique du marché sur la période (€/MWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "65.40"))]
    pub market_average_spot_price_mwh: Decimal,
    /// Coefficient de profilage du foyer (Pondéré / Moyen marché)
    /// < 1.0 : foyer vertueux consommant en heures creuses/solaires
    /// > 1.0 : foyer accentuant la pointe de tension
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "1.09"))]
    pub profiling_coefficient: Decimal,
    /// Facture totale TTC calculée en tarification dynamique (€)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "104.25"))]
    pub dynamic_total_cost_ttc: Decimal,
    /// Énergie totale consommée pendant les heures à prix négatifs (kWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "28.5"))]
    pub negative_price_energy_kwh: Decimal,
    /// Analyse des opportunités d'arbitrage
    pub arbitrage: SpotArbitrageOpportunity,
}

/// Analyse le profil de consommation Linky par rapport à l'historique des prix spot
pub fn analyze_spot_consumption(
    point_id: PointId,
    measurements: &[Measurement],
    spot_prices: &[SpotPriceRecord],
    margin_kwh: Option<Decimal>,
    taxes: Option<&TaxConfig>,
) -> Option<SpotProfileAnalysis> {
    let consumption: Vec<&Measurement> = measurements
        .iter()
        .filter(|m| m.direction == FlowDirection::Consumption && m.interval_seconds > 0)
        .collect();

    if consumption.is_empty() || spot_prices.is_empty() {
        return None;
    }

    let default_tax = TaxConfig::default();
    let tax = taxes.unwrap_or(&default_tax);
    let margin = margin_kwh.unwrap_or_else(|| Decimal::from_str_exact("0.0150").unwrap()); // 1.5 c€/kWh marge fournisseur

    // 1. Indexation des prix spot par heure UTC
    let mut price_map: HashMap<DateTime<Utc>, &SpotPriceRecord> = HashMap::new();
    let mut min_mwh = Decimal::from(999999);
    let mut max_mwh = Decimal::from(-999999);
    let mut sum_mwh = Decimal::ZERO;
    let mut negative_hours = 0;

    for sp in spot_prices {
        // Aligner à l'heure pile
        let hour_bucket = sp
            .timestamp
            .date_naive()
            .and_hms_opt(sp.timestamp.hour(), 0, 0)
            .unwrap()
            .and_utc();
        price_map.insert(hour_bucket, sp);

        if sp.price_eur_per_mwh < min_mwh {
            min_mwh = sp.price_eur_per_mwh;
        }
        if sp.price_eur_per_mwh > max_mwh {
            max_mwh = sp.price_eur_per_mwh;
        }
        sum_mwh += sp.price_eur_per_mwh;
        if sp.is_negative {
            negative_hours += 1;
        }
    }

    let avg_market_mwh = (sum_mwh / Decimal::from(spot_prices.len())).round_dp(2);

    // 2. Croisement avec la courbe de charge
    let mut total_kwh = Decimal::ZERO;
    let mut sum_weighted_mwh = Decimal::ZERO;
    let mut negative_kwh = Decimal::ZERO;
    let mut total_dyn_cost_ht = Decimal::ZERO;

    // Pour l'arbitrage : identifier la part consommée en heures très chères vs heures pas chères
    let mut high_cost_kwh = Decimal::ZERO;

    for m in &consumption {
        let kwh = m.energy_kwh();
        total_kwh += kwh;

        let hour_bucket = m
            .timestamp
            .date_naive()
            .and_hms_opt(m.timestamp.hour(), 0, 0)
            .unwrap()
            .and_utc();

        let sp = price_map.get(&hour_bucket).copied();
        let (spot_mwh, spot_kwh, is_neg) = match sp {
            Some(record) => (
                record.price_eur_per_mwh,
                record.price_eur_per_kwh,
                record.is_negative,
            ),
            None => (
                avg_market_mwh,
                (avg_market_mwh / Decimal::from(1000)).round_dp(6),
                false,
            ),
        };

        sum_weighted_mwh += spot_mwh * kwh;
        if is_neg {
            negative_kwh += kwh;
        }
        if spot_mwh > avg_market_mwh * Decimal::from_str_exact("1.25").unwrap_or(Decimal::ONE) {
            high_cost_kwh += kwh;
        }

        // Coût HT du kWh dynamique = Spot + Marge + TICFE
        let cost_ht = (spot_kwh + margin + tax.ticfe_per_kwh) * kwh;
        total_dyn_cost_ht += cost_ht;
    }

    if total_kwh.is_zero() {
        return None;
    }

    let weighted_avg_mwh = (sum_weighted_mwh / total_kwh).round_dp(2);
    let profiling_coeff = if !avg_market_mwh.is_zero() {
        (weighted_avg_mwh / avg_market_mwh).round_dp(2)
    } else {
        Decimal::ONE
    };

    // Coût TTC = HT * 1.20 (TVA 20%) + abonnement au prorata
    let days = if consumption.len() >= 2 {
        let first = consumption.first().unwrap().timestamp;
        let last = consumption.last().unwrap().timestamp;
        (last - first).num_days().abs().max(1)
    } else {
        30
    };
    let monthly_sub = Decimal::from_str_exact("14.00").unwrap(); // abonnement dynamique standard
    let sub_ht = (monthly_sub * Decimal::from(days)) / Decimal::from(30);
    let sub_ttc = sub_ht * (Decimal::ONE + tax.vat_subscription_rate);
    let dynamic_total_cost_ttc =
        (total_dyn_cost_ht * (Decimal::ONE + tax.vat_consumption_rate) + sub_ttc).round_dp(2);

    // Économie annuelle potentielle d'arbitrage :
    // Si on déplaçait la moitié de la consommation des heures de pointe (high_cost_kwh)
    // vers les heures de prix bas (écart moyen constaté de ~0.08 €/kWh TTC)
    let annual_factor = Decimal::from(365) / Decimal::from(days);
    let shiftable_kwh_annual =
        (high_cost_kwh * Decimal::from_str_exact("0.50").unwrap()) * annual_factor;
    let delta_price_kwh = Decimal::from_str_exact("0.08").unwrap(); // gain moyen de 8 c€/kWh en déplaçant la charge
    let annual_arbitrage_savings = (shiftable_kwh_annual * delta_price_kwh).round_dp(2);

    let rec = if negative_hours > 0 {
        format!(
            "Marché dynamique actif : {} heures à prix négatifs détectées (min: {:.2} €/MWh). Programmer la recharge du VE ou le chauffe-eau sur ces plages générerait ~{:.0} €/an d'économies.",
            negative_hours, min_mwh, annual_arbitrage_savings
        )
    } else {
        format!(
            "Coefficient de profilage : {:.2}. Un pilotage des équipements de puissance vers les heures creuses du marché permettrait de gagner jusqu'à {:.0} €/an.",
            profiling_coeff, annual_arbitrage_savings
        )
    };

    let arbitrage = SpotArbitrageOpportunity {
        negative_hours_count: negative_hours,
        min_price_eur_per_mwh: min_mwh,
        max_price_eur_per_mwh: max_mwh,
        avg_price_eur_per_mwh: avg_market_mwh,
        annual_arbitrage_savings_euro: annual_arbitrage_savings,
        recommendation: rec,
    };

    Some(SpotProfileAnalysis {
        point_id,
        total_energy_kwh: total_kwh.round_dp(1),
        weighted_average_spot_price_mwh: weighted_avg_mwh,
        market_average_spot_price_mwh: avg_market_mwh,
        profiling_coefficient: profiling_coeff,
        dynamic_total_cost_ttc,
        negative_price_energy_kwh: negative_kwh.round_dp(1),
        arbitrage,
    })
}

/// Générateur de profil de prix spot synthétique ultra-réaliste pour simulations et tests hors-ligne.
/// Reproduit le comportement d'EPEX SPOT France :
/// - Creux de nuit (02h-05h)
/// - Pic du matin (08h-10h)
/// - Baisse ou prix négatifs au midi solaire (13h-16h au printemps/été avec fort PV)
/// - Pic du soir (19h-21h)
pub fn generate_synthetic_spot_profile(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Vec<SpotPriceRecord> {
    let mut records = Vec::new();
    let mut current = from
        .date_naive()
        .and_hms_opt(from.hour(), 0, 0)
        .unwrap()
        .and_utc();

    while current < to {
        let local = to_french_local_time(current);
        let hour = local.hour();
        let month = local.month();
        let weekday = local.weekday().number_from_monday(); // 6=Sam, 7=Dim
        let is_weekend = weekday >= 6;

        // Base saisonnière : plus élevé en hiver (chauffage), plus bas en été
        let season_base = match month {
            11 | 12 | 1 | 2 => 95.0,
            3 | 10 => 75.0,
            4 | 5 | 9 => 55.0,
            _ => 45.0, // Juin, Juillet, Août
        };

        // Forme intra-journalière
        let hourly_delta = match hour {
            2..=4 => -35.0, // Nuit calme
            5 | 6 => -20.0,
            7 => 15.0,
            8 | 9 => 45.0, // Pic du matin
            10 | 11 => 10.0,
            12 => -15.0,
            // Ensoleillement fort le week-end au printemps/été -> risque de prix négatif !
            13..=15 => {
                if (4..=8).contains(&month) && is_weekend {
                    -90.0 // Prix négatif !
                } else if (4..=8).contains(&month) {
                    -40.0
                } else {
                    -5.0
                }
            }
            16 => -10.0,
            17 => 10.0,
            18 => 30.0,
            19 | 20 => 55.0, // Pic du soir
            21 => 25.0,
            22 => 0.0,
            23 | 0 | 1 => -20.0,
            _ => 0.0,
        };

        let weekend_discount = if is_weekend { -15.0 } else { 0.0 };

        let price_mwh = season_base + hourly_delta + weekend_discount;
        let price_dec = Decimal::from_str_exact(&format!("{:.2}", price_mwh))
            .unwrap_or_else(|_| Decimal::from(60));

        records.push(SpotPriceRecord::new(
            current,
            price_dec,
            Some("EPEX_SPOT_FR"),
        ));
        current += ChronoDuration::hours(1);
    }

    records
}
