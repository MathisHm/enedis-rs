use crate::error::EnedisError;
use crate::models::signals::TempoDayRecord;
use crate::models::{Measurement, PointId};
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Plage horaire délimitée par une heure de début et une heure de fin
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSlot {
    #[cfg_attr(feature = "openapi", schema(example = "22:00"))]
    pub start: NaiveTime,
    #[cfg_attr(feature = "openapi", schema(example = "06:00"))]
    pub end: NaiveTime,
}

impl TimeSlot {
    pub fn new(start: NaiveTime, end: NaiveTime) -> Self {
        Self { start, end }
    }

    /// Analyse une chaîne au format `22:00-06:00`, `22h-06h`, `12:00-14:00`, etc.
    pub fn parse(s: &str) -> Result<Self, String> {
        let clean = s.trim().replace(' ', "");
        let parts: Vec<&str> = clean.split('-').collect();
        if parts.len() != 2 {
            return Err(format!("Format de plage horaire invalide: '{}'", s));
        }

        let start = parse_time_component(parts[0])?;
        let end = parse_time_component(parts[1])?;

        Ok(Self { start, end })
    }

    /// Détermine si une heure donnée est incluse dans la plage
    pub fn contains(&self, time: NaiveTime) -> bool {
        if self.start <= self.end {
            // Plage dans la même journée (ex: 12:00 à 14:00)
            time >= self.start && time < self.end
        } else {
            // Plage à cheval sur minuit (ex: 22:00 à 06:00)
            time >= self.start || time < self.end
        }
    }
}

fn parse_time_component(part: &str) -> Result<NaiveTime, String> {
    let p = part.to_lowercase().replace('h', ":");
    let subparts: Vec<&str> = p.split(':').collect();
    let hour: u32 = subparts[0]
        .parse()
        .map_err(|_| format!("Heure invalide dans '{}'", part))?;

    let min: u32 = if subparts.len() > 1 && !subparts[1].is_empty() {
        subparts[1]
            .parse()
            .map_err(|_| format!("Minutes invalides dans '{}'", part))?
    } else {
        0
    };

    NaiveTime::from_hms_opt(hour, min, 0)
        .ok_or_else(|| format!("Heure en dehors des limites (0-23:0-59): '{}'", part))
}

/// Grille tarifaire "Option Base"
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseTariff {
    /// Prix du kWh en Euros (€/kWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.2516"))]
    pub price_per_kwh: Decimal,
    /// Abonnement mensuel fixe en Euros (€/mois)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "12.50"))]
    pub monthly_subscription: Decimal,
}

impl Default for BaseTariff {
    fn default() -> Self {
        Self {
            price_per_kwh: Decimal::from_str_exact("0.2516").unwrap(),
            monthly_subscription: Decimal::from_str_exact("12.50").unwrap(),
        }
    }
}

/// Grille tarifaire "Option Heures Pleines / Heures Creuses (HP/HC)"
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HpHcTariff {
    /// Prix du kWh en Heures Pleines (€/kWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.2700"))]
    pub hp_price_per_kwh: Decimal,
    /// Prix du kWh en Heures Creuses (€/kWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.2068"))]
    pub hc_price_per_kwh: Decimal,
    /// Abonnement mensuel fixe en Euros (€/mois)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "13.00"))]
    pub monthly_subscription: Decimal,
    /// Plages horaires locales des Heures Creuses (ex: 22h-06h, 12h-14h et 01h-07h)
    pub off_peak_slots: Vec<TimeSlot>,
}

impl Default for HpHcTariff {
    fn default() -> Self {
        Self {
            hp_price_per_kwh: Decimal::from_str_exact("0.2700").unwrap(),
            hc_price_per_kwh: Decimal::from_str_exact("0.2068").unwrap(),
            monthly_subscription: Decimal::from_str_exact("13.00").unwrap(),
            off_peak_slots: vec![TimeSlot::new(
                NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
                NaiveTime::from_hms_opt(6, 0, 0).unwrap(),
            )],
        }
    }
}

impl HpHcTariff {
    pub fn is_off_peak(&self, time: NaiveTime) -> bool {
        if self.off_peak_slots.is_empty() {
            // Par défaut 22h-06h
            time.hour() >= 22 || time.hour() < 6
        } else {
            self.off_peak_slots.iter().any(|slot| slot.contains(time))
        }
    }
}

/// Grille tarifaire "Option EDF Tempo"
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TempoTariff {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.1296"))]
    pub blue_hc_price: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.1609"))]
    pub blue_hp_price: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.1486"))]
    pub white_hc_price: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.1894"))]
    pub white_hp_price: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.1568"))]
    pub red_hc_price: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.7562"))]
    pub red_hp_price: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "13.00"))]
    pub monthly_subscription: Decimal,
}

impl Default for TempoTariff {
    fn default() -> Self {
        Self {
            blue_hc_price: Decimal::from_str_exact("0.1296").unwrap(),
            blue_hp_price: Decimal::from_str_exact("0.1609").unwrap(),
            white_hc_price: Decimal::from_str_exact("0.1486").unwrap(),
            white_hp_price: Decimal::from_str_exact("0.1894").unwrap(),
            red_hc_price: Decimal::from_str_exact("0.1568").unwrap(),
            red_hp_price: Decimal::from_str_exact("0.7562").unwrap(),
            monthly_subscription: Decimal::from_str_exact("13.00").unwrap(),
        }
    }
}

/// Grille tarifaire dynamique (ex: tarification horaire / spot / marché avec marge)
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicTariff {
    /// Prix de base ou de repli (€/kWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.2000"))]
    pub fallback_price_per_kwh: Decimal,
    /// Marge fournisseur fixe ajoutée par kWh (€/kWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.0150"))]
    pub fixed_margin_per_kwh: Decimal,
    /// Abonnement mensuel fixe en Euros (€/mois)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "14.00"))]
    pub monthly_subscription: Decimal,
    /// Carte des prix horaires connus par horodatage UTC (au pas horaire)
    pub hourly_prices: HashMap<DateTime<Utc>, Decimal>,
}

impl Default for DynamicTariff {
    fn default() -> Self {
        Self {
            fallback_price_per_kwh: Decimal::from_str_exact("0.2000").unwrap(),
            fixed_margin_per_kwh: Decimal::from_str_exact("0.0150").unwrap(),
            monthly_subscription: Decimal::from_str_exact("14.00").unwrap(),
            hourly_prices: HashMap::new(),
        }
    }
}

/// Paramètres de taxes applicables à la facture d'électricité
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxConfig {
    /// Taux de TVA appliqué à l'abonnement (standard 5.5% en France)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.055"))]
    pub vat_subscription_rate: Decimal,
    /// Taux de TVA appliqué à la consommation (standard 20% en France)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.20"))]
    pub vat_consumption_rate: Decimal,
    /// Accise sur l'électricité / TICFE en €/kWh (ex: 0.021 €/kWh)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.021"))]
    pub ticfe_per_kwh: Decimal,
}

impl Default for TaxConfig {
    fn default() -> Self {
        Self {
            vat_subscription_rate: Decimal::from_str_exact("0.055").unwrap(),
            vat_consumption_rate: Decimal::from_str_exact("0.20").unwrap(),
            ticfe_per_kwh: Decimal::from_str_exact("0.021").unwrap(),
        }
    }
}

/// Configuration globale d'une grille tarifaire
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TariffConfig {
    Base(BaseTariff),
    HpHc(HpHcTariff),
    Tempo(TempoTariff),
    Dynamic(DynamicTariff),
}

impl Default for TariffConfig {
    fn default() -> Self {
        Self::Base(BaseTariff::default())
    }
}

impl TariffConfig {
    pub fn monthly_subscription(&self) -> Decimal {
        match self {
            Self::Base(b) => b.monthly_subscription,
            Self::HpHc(h) => h.monthly_subscription,
            Self::Tempo(t) => t.monthly_subscription,
            Self::Dynamic(d) => d.monthly_subscription,
        }
    }
}

/// Détail du coût pour une tranche ou couleur tarifaire
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TariffCostBucket {
    #[cfg_attr(feature = "openapi", schema(example = "Heures Creuses"))]
    pub bucket_name: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "120.5000"))]
    pub energy_kwh: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.2068"))]
    pub unit_price: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "24.9194"))]
    pub total_cost_ht: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "45.50"))]
    pub percentage_of_energy: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "40.12"))]
    pub percentage_of_cost: Decimal,
}

/// Comparaison financière avec le tarif réglementé de Base
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TariffComparison {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "150.25"))]
    pub base_total_cost_ttc: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "18.50"))]
    pub savings_amount_ttc: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "12.31"))]
    pub savings_percentage: Decimal,
}

/// Résultat complet du calcul des coûts énergétiques d'un PRM sur une période
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostCalculation {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "01234567890123"))]
    pub point_id: PointId,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-01T00:00:00Z"))]
    pub from: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T00:00:00Z"))]
    pub to: DateTime<Utc>,
    pub tariff_type: String,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "350.4500"))]
    pub total_energy_kwh: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "72.45"))]
    pub total_consumption_cost_ht: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "11.66"))]
    pub total_subscription_cost_ht: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "21.85"))]
    pub total_taxes: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "105.96"))]
    pub total_cost_ttc: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "0.3023"))]
    pub average_cost_per_kwh_ttc: Decimal,
    pub breakdown: Vec<TariffCostBucket>,
    pub comparison_with_base: Option<TariffComparison>,
}

/// Convertit un horodatage UTC en heure locale métropolitaine française (Europe/Paris)
/// en respectant exactement les règles légales de bascule heure d'été / heure d'hiver.
pub fn to_french_local_time(utc: DateTime<Utc>) -> NaiveDateTime {
    let year = utc.year();
    // Directive européenne 2000/84/CE : Dernier dimanche de mars à 01:00 UTC (bascule UTC+1 -> UTC+2)
    let last_march = NaiveDate::from_ymd_opt(year, 3, 31).expect("31 mars toujours valide");
    let march_offset = last_march.weekday().num_days_from_sunday() as i64;
    let march_sunday = last_march - chrono::Duration::days(march_offset);
    let march_transition = march_sunday
        .and_hms_opt(1, 0, 0)
        .expect("01:00:00 toujours valide")
        .and_utc();

    // Directive européenne 2000/84/CE : Dernier dimanche d'octobre à 01:00 UTC (bascule UTC+2 -> UTC+1)
    let last_october = NaiveDate::from_ymd_opt(year, 10, 31).expect("31 octobre toujours valide");
    let oct_offset = last_october.weekday().num_days_from_sunday() as i64;
    let oct_sunday = last_october - chrono::Duration::days(oct_offset);
    let oct_transition = oct_sunday
        .and_hms_opt(1, 0, 0)
        .expect("01:00:00 toujours valide")
        .and_utc();

    let offset_hours = if utc >= march_transition && utc < oct_transition {
        2
    } else {
        1
    };

    (utc + chrono::Duration::hours(offset_hours)).naive_utc()
}

/// Convertit une heure locale métropolitaine française (Europe/Paris) en horodatage UTC
/// en appliquant l'inverse exact des règles légales de la directive 2000/84/CE.
pub fn from_french_local_time(local: NaiveDateTime) -> DateTime<Utc> {
    let year = local.year();
    let last_march = NaiveDate::from_ymd_opt(year, 3, 31).expect("31 mars toujours valide");
    let march_offset = last_march.weekday().num_days_from_sunday() as i64;
    let march_sunday = last_march - chrono::Duration::days(march_offset);
    let march_transition = march_sunday
        .and_hms_opt(2, 0, 0)
        .expect("02:00:00 toujours valide");

    let last_october = NaiveDate::from_ymd_opt(year, 10, 31).expect("31 octobre toujours valide");
    let oct_offset = last_october.weekday().num_days_from_sunday() as i64;
    let oct_sunday = last_october - chrono::Duration::days(oct_offset);
    let oct_transition = oct_sunday
        .and_hms_opt(3, 0, 0)
        .expect("03:00:00 toujours valide");

    let offset_hours = if local >= march_transition && local < oct_transition {
        2
    } else {
        1
    };

    (local - chrono::Duration::hours(offset_hours)).and_utc()
}

/// Règle légale Tempo : la journée Tempo s'étend de 06:00:00 (J) à 06:00:00 (J+1).
/// Ainsi, une mesure prise à 02:00 le 15 janvier fait partie de la journée Tempo du 14 janvier.
pub fn tempo_date_for_time(local_time: NaiveDateTime) -> NaiveDate {
    if local_time.hour() < 6 {
        local_time.date() - chrono::Duration::days(1)
    } else {
        local_time.date()
    }
}

/// Calcule la facture énergétique estimée pour une série de mesures et une grille tarifaire
pub fn calculate_energy_costs(
    point_id: PointId,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    measurements: &[Measurement],
    tariff: &TariffConfig,
    tax_config: Option<&TaxConfig>,
    tempo_days: Option<&[TempoDayRecord]>,
) -> Result<CostCalculation, EnedisError> {
    let taxes = tax_config.cloned().unwrap_or_default();
    let duration_secs = (to - from).num_seconds().max(0);
    // Prorata d'abonnement : mois moyen de 30.4375 jours (365.25 / 12 * 86400 = 2_629_800 secondes)
    let seconds_in_month = Decimal::from(2_629_800);
    let subscription_prorata = (tariff.monthly_subscription() * Decimal::from(duration_secs)
        / seconds_in_month)
        .round_dp(4);

    let mut buckets: HashMap<String, (Decimal, Decimal)> = HashMap::new(); // Nom -> (kWh, Coût HT)

    match tariff {
        TariffConfig::Base(b) => {
            let mut total_kwh = Decimal::ZERO;
            for m in measurements {
                total_kwh += m.energy_kwh();
            }
            let cost = (total_kwh * b.price_per_kwh).round_dp(4);
            buckets.insert("Base".to_string(), (total_kwh, cost));
        }

        TariffConfig::HpHc(h) => {
            let mut hp_kwh = Decimal::ZERO;
            let mut hc_kwh = Decimal::ZERO;

            for m in measurements {
                let local = to_french_local_time(m.timestamp);
                let kwh = m.energy_kwh();
                if h.is_off_peak(local.time()) {
                    hc_kwh += kwh;
                } else {
                    hp_kwh += kwh;
                }
            }

            let hp_cost = (hp_kwh * h.hp_price_per_kwh).round_dp(4);
            let hc_cost = (hc_kwh * h.hc_price_per_kwh).round_dp(4);

            buckets.insert("Heures Pleines".to_string(), (hp_kwh, hp_cost));
            buckets.insert("Heures Creuses".to_string(), (hc_kwh, hc_cost));
        }

        TariffConfig::Tempo(t) => {
            use crate::models::signals::TempoColor;

            let mut tempo_map: HashMap<NaiveDate, TempoColor> = HashMap::new();
            if let Some(records) = tempo_days {
                for r in records {
                    tempo_map.insert(r.date, r.color);
                }
            }

            let mut blue_hp = Decimal::ZERO;
            let mut blue_hc = Decimal::ZERO;
            let mut white_hp = Decimal::ZERO;
            let mut white_hc = Decimal::ZERO;
            let mut red_hp = Decimal::ZERO;
            let mut red_hc = Decimal::ZERO;

            for m in measurements {
                let local = to_french_local_time(m.timestamp);
                let tempo_date = tempo_date_for_time(local);
                let color = tempo_map
                    .get(&tempo_date)
                    .copied()
                    .unwrap_or(TempoColor::Blue);

                // Heures Creuses Tempo : 22h00 à 06h00
                let is_hc = local.hour() >= 22 || local.hour() < 6;
                let kwh = m.energy_kwh();

                match color {
                    TempoColor::Blue => {
                        if is_hc {
                            blue_hc += kwh;
                        } else {
                            blue_hp += kwh;
                        }
                    }
                    TempoColor::White => {
                        if is_hc {
                            white_hc += kwh;
                        } else {
                            white_hp += kwh;
                        }
                    }
                    TempoColor::Red => {
                        if is_hc {
                            red_hc += kwh;
                        } else {
                            red_hp += kwh;
                        }
                    }
                    TempoColor::Unknown => {
                        // En l'absence d'information, repli sur Bleu
                        if is_hc {
                            blue_hc += kwh;
                        } else {
                            blue_hp += kwh;
                        }
                    }
                }
            }

            buckets.insert(
                "Tempo Bleu HP".to_string(),
                (blue_hp, (blue_hp * t.blue_hp_price).round_dp(4)),
            );
            buckets.insert(
                "Tempo Bleu HC".to_string(),
                (blue_hc, (blue_hc * t.blue_hc_price).round_dp(4)),
            );
            buckets.insert(
                "Tempo Blanc HP".to_string(),
                (white_hp, (white_hp * t.white_hp_price).round_dp(4)),
            );
            buckets.insert(
                "Tempo Blanc HC".to_string(),
                (white_hc, (white_hc * t.white_hc_price).round_dp(4)),
            );
            buckets.insert(
                "Tempo Rouge HP".to_string(),
                (red_hp, (red_hp * t.red_hp_price).round_dp(4)),
            );
            buckets.insert(
                "Tempo Rouge HC".to_string(),
                (red_hc, (red_hc * t.red_hc_price).round_dp(4)),
            );
        }

        TariffConfig::Dynamic(d) => {
            let mut dyn_kwh = Decimal::ZERO;
            let mut dyn_cost = Decimal::ZERO;

            for m in measurements {
                // Arrondi à l'heure UTC
                let hour_bucket = m
                    .timestamp
                    .date_naive()
                    .and_hms_opt(m.timestamp.hour(), 0, 0)
                    .unwrap()
                    .and_utc();

                let spot_price = d
                    .hourly_prices
                    .get(&hour_bucket)
                    .copied()
                    .unwrap_or(d.fallback_price_per_kwh);

                let price = spot_price + d.fixed_margin_per_kwh;
                let kwh = m.energy_kwh();
                dyn_kwh += kwh;
                dyn_cost += (kwh * price).round_dp(4);
            }

            buckets.insert("Dynamique (Spot + Marge)".to_string(), (dyn_kwh, dyn_cost));
        }
    }

    let total_energy_kwh: Decimal = buckets
        .values()
        .map(|(kwh, _)| *kwh)
        .sum::<Decimal>()
        .round_dp(4);
    let total_consumption_cost_ht: Decimal = buckets
        .values()
        .map(|(_, cost)| *cost)
        .sum::<Decimal>()
        .round_dp(4);

    let mut breakdown = Vec::new();
    for (name, (kwh, cost_ht)) in buckets {
        let unit_price = if kwh > Decimal::ZERO {
            (cost_ht / kwh).round_dp(4)
        } else {
            Decimal::ZERO
        };
        let percentage_of_energy = if total_energy_kwh > Decimal::ZERO {
            ((kwh / total_energy_kwh) * Decimal::from(100)).round_dp(2)
        } else {
            Decimal::ZERO
        };
        let percentage_of_cost = if total_consumption_cost_ht > Decimal::ZERO {
            ((cost_ht / total_consumption_cost_ht) * Decimal::from(100)).round_dp(2)
        } else {
            Decimal::ZERO
        };

        breakdown.push(TariffCostBucket {
            bucket_name: name,
            energy_kwh: kwh.round_dp(4),
            unit_price,
            total_cost_ht: cost_ht.round_dp(4),
            percentage_of_energy,
            percentage_of_cost,
        });
    }

    // Tri stable pour une sortie prédictible
    breakdown.sort_by(|a, b| a.bucket_name.cmp(&b.bucket_name));

    // Calcul des taxes
    let ticfe_amount = (total_energy_kwh * taxes.ticfe_per_kwh).round_dp(4);
    let vat_subscription = (subscription_prorata * taxes.vat_subscription_rate).round_dp(4);
    let vat_consumption =
        ((total_consumption_cost_ht + ticfe_amount) * taxes.vat_consumption_rate).round_dp(4);
    let total_taxes = (ticfe_amount + vat_subscription + vat_consumption).round_dp(4);

    let total_cost_ttc =
        (total_consumption_cost_ht + subscription_prorata + total_taxes).round_dp(2);

    let average_cost_per_kwh_ttc = if total_energy_kwh > Decimal::ZERO {
        (total_cost_ttc / total_energy_kwh).round_dp(4)
    } else {
        Decimal::ZERO
    };

    // Calcul de comparaison avec l'Option Base standard si le tarif courant n'est pas Base
    let comparison_with_base = match tariff {
        TariffConfig::Base(_) => None,
        _ => {
            let base_default = BaseTariff::default();
            let base_sub_cost = (base_default.monthly_subscription * Decimal::from(duration_secs)
                / seconds_in_month)
                .round_dp(4);
            let base_conso_ht = (total_energy_kwh * base_default.price_per_kwh).round_dp(4);
            let base_ticfe = (total_energy_kwh * taxes.ticfe_per_kwh).round_dp(4);
            let base_vat_sub = (base_sub_cost * taxes.vat_subscription_rate).round_dp(4);
            let base_vat_conso =
                ((base_conso_ht + base_ticfe) * taxes.vat_consumption_rate).round_dp(4);
            let base_ttc =
                (base_conso_ht + base_sub_cost + base_ticfe + base_vat_sub + base_vat_conso)
                    .round_dp(2);

            let savings_amount = (base_ttc - total_cost_ttc).round_dp(2);
            let savings_percentage = if base_ttc > Decimal::ZERO {
                ((savings_amount / base_ttc) * Decimal::from(100)).round_dp(2)
            } else {
                Decimal::ZERO
            };

            Some(TariffComparison {
                base_total_cost_ttc: base_ttc,
                savings_amount_ttc: savings_amount,
                savings_percentage,
            })
        }
    };

    let tariff_type = match tariff {
        TariffConfig::Base(_) => "BASE",
        TariffConfig::HpHc(_) => "HEURES_PLEINES_HEURES_CREUSES",
        TariffConfig::Tempo(_) => "TEMPO",
        TariffConfig::Dynamic(_) => "DYNAMIQUE",
    }
    .to_string();

    Ok(CostCalculation {
        point_id,
        from,
        to,
        tariff_type,
        total_energy_kwh,
        total_consumption_cost_ht,
        total_subscription_cost_ht: subscription_prorata,
        total_taxes,
        total_cost_ttc,
        average_cost_per_kwh_ttc,
        breakdown,
        comparison_with_base,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FlowDirection, MeasurementQuality, Unit};
    use chrono::TimeZone;

    #[test]
    fn test_timeslot_parsing_and_containment() {
        let slot = TimeSlot::parse("22:00-06:00").unwrap();
        assert!(slot.contains(NaiveTime::from_hms_opt(22, 0, 0).unwrap()));
        assert!(slot.contains(NaiveTime::from_hms_opt(23, 30, 0).unwrap()));
        assert!(slot.contains(NaiveTime::from_hms_opt(3, 15, 0).unwrap()));
        assert!(slot.contains(NaiveTime::from_hms_opt(5, 59, 0).unwrap()));
        assert!(!slot.contains(NaiveTime::from_hms_opt(6, 0, 0).unwrap()));
        assert!(!slot.contains(NaiveTime::from_hms_opt(14, 0, 0).unwrap()));

        // Plage dans la même journée
        let day_slot = TimeSlot::parse("12h00 - 14h00").unwrap();
        assert!(day_slot.contains(NaiveTime::from_hms_opt(12, 30, 0).unwrap()));
        assert!(!day_slot.contains(NaiveTime::from_hms_opt(11, 59, 0).unwrap()));
        assert!(!day_slot.contains(NaiveTime::from_hms_opt(14, 0, 0).unwrap()));
    }

    #[test]
    fn test_french_local_time_and_tempo_date() {
        // En hiver (janvier) : UTC+1
        let winter_utc = Utc.with_ymd_and_hms(2026, 1, 15, 1, 0, 0).unwrap();
        let local_winter = to_french_local_time(winter_utc);
        assert_eq!(
            local_winter.time(),
            NaiveTime::from_hms_opt(2, 0, 0).unwrap()
        );

        // À 2h00 du matin, on fait partie de la journée Tempo de la veille (14 janvier)
        let tempo_d = tempo_date_for_time(local_winter);
        assert_eq!(tempo_d, NaiveDate::from_ymd_opt(2026, 1, 14).unwrap());

        // À 7h00 locale (6h00 UTC), on bascule sur la journée Tempo du 15 janvier
        let morning_utc = Utc.with_ymd_and_hms(2026, 1, 15, 6, 0, 0).unwrap();
        let local_morning = to_french_local_time(morning_utc);
        assert_eq!(
            local_morning.time(),
            NaiveTime::from_hms_opt(7, 0, 0).unwrap()
        );
        assert_eq!(
            tempo_date_for_time(local_morning),
            NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()
        );

        // En été (juin) : UTC+2
        let summer_utc = Utc.with_ymd_and_hms(2026, 6, 15, 10, 0, 0).unwrap();
        let local_summer = to_french_local_time(summer_utc);
        assert_eq!(
            local_summer.time(),
            NaiveTime::from_hms_opt(12, 0, 0).unwrap()
        );
    }

    #[test]
    fn test_calculate_energy_costs_base_and_hphc() {
        let prm = PointId::new("01234567890123").unwrap();
        let from = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 9, 30, 0, 0, 0).unwrap();

        // 2 mesures : une à 14h00 locale (HP), une à 23h00 locale (HC)
        // 14h00 local été (UTC+2) -> 12h00 UTC
        let m_hp = Measurement {
            point_id: prm,
            timestamp: Utc.with_ymd_and_hms(2026, 9, 15, 12, 0, 0).unwrap(),
            interval_seconds: 3600,
            direction: FlowDirection::Consumption,
            value: Decimal::from(10), // 10 kWh
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        };
        // 23h00 local été (UTC+2) -> 21h00 UTC
        let m_hc = Measurement {
            point_id: prm,
            timestamp: Utc.with_ymd_and_hms(2026, 9, 15, 21, 0, 0).unwrap(),
            interval_seconds: 3600,
            direction: FlowDirection::Consumption,
            value: Decimal::from(20), // 20 kWh
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        };

        let measures = vec![m_hp, m_hc];

        // 1. Calcul Base
        let base_calc = calculate_energy_costs(
            prm,
            from,
            to,
            &measures,
            &TariffConfig::Base(BaseTariff::default()),
            None,
            None,
        )
        .unwrap();

        assert_eq!(base_calc.total_energy_kwh, Decimal::from(30));
        assert!(base_calc.total_consumption_cost_ht > Decimal::ZERO);
        assert!(base_calc.total_cost_ttc > base_calc.total_consumption_cost_ht);
        assert_eq!(base_calc.breakdown.len(), 1);

        // 2. Calcul HP/HC
        let hphc_calc = calculate_energy_costs(
            prm,
            from,
            to,
            &measures,
            &TariffConfig::HpHc(HpHcTariff::default()),
            None,
            None,
        )
        .unwrap();

        assert_eq!(hphc_calc.total_energy_kwh, Decimal::from(30));
        assert_eq!(hphc_calc.breakdown.len(), 2);
        let hp_bucket = hphc_calc
            .breakdown
            .iter()
            .find(|b| b.bucket_name == "Heures Pleines")
            .unwrap();
        let hc_bucket = hphc_calc
            .breakdown
            .iter()
            .find(|b| b.bucket_name == "Heures Creuses")
            .unwrap();
        assert_eq!(hp_bucket.energy_kwh, Decimal::from(10));
        assert_eq!(hc_bucket.energy_kwh, Decimal::from(20));
        assert!(hphc_calc.comparison_with_base.is_some());
    }
}
