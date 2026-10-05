use crate::models::tariff::{tempo_date_for_time, to_french_local_time};
use crate::models::{Measurement, PointId};
use chrono::{DateTime, NaiveDate, Timelike, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Couleur Tempo RTE / EDF pour une journée
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TempoColor {
    /// Jour Bleu (tarif très avantageux, 300 jours/an)
    Blue,
    /// Jour Blanc (tarif intermédiaire, 43 jours/an)
    White,
    /// Jour Rouge (tarif Heures Pleines très élevé, 22 jours/an du 1er nov au 31 mars)
    Red,
    /// Couleur non encore déterminée (J+1 avant midi)
    Unknown,
}

impl TempoColor {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blue => "BLEU",
            Self::White => "BLANC",
            Self::Red => "ROUGE",
            Self::Unknown => "INCONNU",
        }
    }

    pub fn from_str_code(s: &str) -> Self {
        match s.trim().to_uppercase().as_str() {
            "BLEU" | "BLUE" | "1" => Self::Blue,
            "BLANC" | "WHITE" | "2" => Self::White,
            "ROUGE" | "RED" | "3" => Self::Red,
            _ => Self::Unknown,
        }
    }
}

/// Enregistrement de la couleur Tempo d'un jour calendaire
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TempoDayRecord {
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-29"))]
    pub date: NaiveDate,
    pub color: TempoColor,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-29T06:00:00Z"))]
    pub updated_at: DateTime<Utc>,
}

/// Niveau d'alerte du signal réseau RTE EcoWatt
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EcoWattLevel {
    /// Niveau 1 : Situation normale (Consommation raisonnable)
    Green = 1,
    /// Niveau 2 : Système électrique tendu (Écogestes recommandés)
    Orange = 2,
    /// Niveau 3 : Système électrique très tendu (Risque élevé de coupure, écogestes indispensables)
    Red = 3,
}

impl EcoWattLevel {
    pub fn as_u8(&self) -> u8 {
        *self as u8
    }

    pub fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(Self::Green),
            2 => Some(Self::Orange),
            3 => Some(Self::Red),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Green => "VERT",
            Self::Orange => "ORANGE",
            Self::Red => "ROUGE",
        }
    }

    pub fn from_str_code(s: &str) -> Self {
        match s.trim().to_uppercase().as_str() {
            "ORANGE" | "WARNING" | "2" => Self::Orange,
            "ROUGE" | "RED" | "CRITICAL" | "3" => Self::Red,
            _ => Self::Green,
        }
    }
}

/// Pas de signal horaire EcoWatt
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EcoWattSignal {
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-29T18:00:00Z"))]
    pub timestamp: DateTime<Utc>,
    pub level: EcoWattLevel,
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Pic de consommation hivernale en soirée")
    )]
    pub message: Option<String>,
}

/// Analyse de corrélation entre la consommation et les signaux EcoWatt
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EcoWattCorrelation {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "240.5000"))]
    pub green_energy_kwh: Decimal,
    pub green_samples_count: u32,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "650.0000"))]
    pub green_avg_power_w: Decimal,

    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "12.2000"))]
    pub orange_energy_kwh: Decimal,
    pub orange_samples_count: u32,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "420.0000"))]
    pub orange_avg_power_w: Decimal,

    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "4.1000"))]
    pub red_energy_kwh: Decimal,
    pub red_samples_count: u32,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "310.0000"))]
    pub red_avg_power_w: Decimal,

    /// Pourcentage de l'énergie consommée durant des alertes (Orange ou Rouge)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "6.35"))]
    pub tension_energy_percentage: Decimal,

    /// Score d'effacement en pourcentage : réduction de la puissance moyenne lors des alertes rouges
    /// par rapport au bruit de fond / moyenne verte normale (+50% signifie que la consommation a diminué de moitié).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "52.31"))]
    pub red_flexibility_score_percentage: Decimal,
}

/// Analyse de corrélation avec le calendrier Tempo
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TempoCorrelation {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "180.5000"))]
    pub blue_hp_kwh: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "90.2000"))]
    pub blue_hc_kwh: Decimal,

    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "35.1000"))]
    pub white_hp_kwh: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "25.4000"))]
    pub white_hc_kwh: Decimal,

    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "8.2000"))]
    pub red_hp_kwh: Decimal,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "28.5000"))]
    pub red_hc_kwh: Decimal,

    /// Ratio d'évitement en Jour Rouge : part d'énergie consommée en Heures Creuses plutôt qu'en Heures Pleines
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "77.66"))]
    pub red_days_off_peak_ratio_percentage: Decimal,
}

/// Rapport complet d'alignement et de corrélation avec les tensions du réseau électrique français
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GridCorrelationReport {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "01234567890123"))]
    pub point_id: PointId,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-01T00:00:00Z"))]
    pub from: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T00:00:00Z"))]
    pub to: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "378.0000"))]
    pub total_consumption_kwh: Decimal,
    pub ecowatt: EcoWattCorrelation,
    pub tempo: Option<TempoCorrelation>,
    #[cfg_attr(
        feature = "openapi",
        schema(
            example = "Excellente sobriété observée lors des jours rouges Tempo (77% de report en HC)."
        )
    )]
    pub assessment: String,
}

/// Corrèle un ensemble de mesures avec les signaux réseau RTE EcoWatt et le calendrier Tempo
pub fn correlate_measurements_with_grid(
    point_id: PointId,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    measurements: &[Measurement],
    ecowatt_signals: &[EcoWattSignal],
    tempo_days: Option<&[TempoDayRecord]>,
) -> GridCorrelationReport {
    let mut total_kwh = Decimal::ZERO;

    // Indexation temporelle des signaux EcoWatt (par heure UTC)
    let mut ecowatt_map: HashMap<DateTime<Utc>, EcoWattLevel> = HashMap::new();
    for sig in ecowatt_signals {
        let hour_bucket = sig
            .timestamp
            .date_naive()
            .and_hms_opt(sig.timestamp.hour(), 0, 0)
            .unwrap()
            .and_utc();
        ecowatt_map.insert(hour_bucket, sig.level);
    }

    let mut green_kwh = Decimal::ZERO;
    let mut green_powers = Vec::new();
    let mut orange_kwh = Decimal::ZERO;
    let mut orange_powers = Vec::new();
    let mut red_kwh = Decimal::ZERO;
    let mut red_powers = Vec::new();

    for m in measurements {
        let kwh = m.energy_kwh();
        let power = m.power_w();
        total_kwh += kwh;

        let hour_bucket = m
            .timestamp
            .date_naive()
            .and_hms_opt(m.timestamp.hour(), 0, 0)
            .unwrap()
            .and_utc();

        let level = ecowatt_map
            .get(&hour_bucket)
            .copied()
            .unwrap_or(EcoWattLevel::Green);

        match level {
            EcoWattLevel::Green => {
                green_kwh += kwh;
                green_powers.push(power);
            }
            EcoWattLevel::Orange => {
                orange_kwh += kwh;
                orange_powers.push(power);
            }
            EcoWattLevel::Red => {
                red_kwh += kwh;
                red_powers.push(power);
            }
        }
    }

    let green_samples_count = green_powers.len() as u32;
    let orange_samples_count = orange_powers.len() as u32;
    let red_samples_count = red_powers.len() as u32;

    let green_avg_power_w = if green_samples_count > 0 {
        (green_powers.iter().sum::<Decimal>() / Decimal::from(green_samples_count)).round_dp(2)
    } else {
        Decimal::ZERO
    };

    let orange_avg_power_w = if orange_samples_count > 0 {
        (orange_powers.iter().sum::<Decimal>() / Decimal::from(orange_samples_count)).round_dp(2)
    } else {
        Decimal::ZERO
    };

    let red_avg_power_w = if red_samples_count > 0 {
        (red_powers.iter().sum::<Decimal>() / Decimal::from(red_samples_count)).round_dp(2)
    } else {
        Decimal::ZERO
    };

    let tension_energy = orange_kwh + red_kwh;
    let tension_energy_percentage = if total_kwh > Decimal::ZERO {
        ((tension_energy / total_kwh) * Decimal::from(100)).round_dp(2)
    } else {
        Decimal::ZERO
    };

    let red_flexibility_score_percentage =
        if green_avg_power_w > Decimal::ZERO && red_samples_count > 0 {
            (((green_avg_power_w - red_avg_power_w) / green_avg_power_w) * Decimal::from(100))
                .round_dp(2)
        } else {
            Decimal::ZERO
        };

    let ecowatt_corr = EcoWattCorrelation {
        green_energy_kwh: green_kwh.round_dp(4),
        green_samples_count,
        green_avg_power_w,
        orange_energy_kwh: orange_kwh.round_dp(4),
        orange_samples_count,
        orange_avg_power_w,
        red_energy_kwh: red_kwh.round_dp(4),
        red_samples_count,
        red_avg_power_w,
        tension_energy_percentage,
        red_flexibility_score_percentage,
    };

    // Corrélation Tempo
    let tempo_corr = tempo_days.map(|records| {
        let mut tempo_map: HashMap<NaiveDate, TempoColor> = HashMap::new();
        for r in records {
            tempo_map.insert(r.date, r.color);
        }

        let mut b_hp = Decimal::ZERO;
        let mut b_hc = Decimal::ZERO;
        let mut w_hp = Decimal::ZERO;
        let mut w_hc = Decimal::ZERO;
        let mut r_hp = Decimal::ZERO;
        let mut r_hc = Decimal::ZERO;

        for m in measurements {
            let local = to_french_local_time(m.timestamp);
            let tempo_date = tempo_date_for_time(local);
            let color = tempo_map
                .get(&tempo_date)
                .copied()
                .unwrap_or(TempoColor::Blue);
            let is_hc = local.hour() >= 22 || local.hour() < 6;
            let kwh = m.energy_kwh();

            match color {
                TempoColor::Blue => {
                    if is_hc {
                        b_hc += kwh;
                    } else {
                        b_hp += kwh;
                    }
                }
                TempoColor::White => {
                    if is_hc {
                        w_hc += kwh;
                    } else {
                        w_hp += kwh;
                    }
                }
                TempoColor::Red => {
                    if is_hc {
                        r_hc += kwh;
                    } else {
                        r_hp += kwh;
                    }
                }
                TempoColor::Unknown => {
                    if is_hc {
                        b_hc += kwh;
                    } else {
                        b_hp += kwh;
                    }
                }
            }
        }

        let total_red = r_hp + r_hc;
        let red_days_off_peak_ratio_percentage = if total_red > Decimal::ZERO {
            ((r_hc / total_red) * Decimal::from(100)).round_dp(2)
        } else {
            Decimal::ZERO
        };

        TempoCorrelation {
            blue_hp_kwh: b_hp.round_dp(4),
            blue_hc_kwh: b_hc.round_dp(4),
            white_hp_kwh: w_hp.round_dp(4),
            white_hc_kwh: w_hc.round_dp(4),
            red_hp_kwh: r_hp.round_dp(4),
            red_hc_kwh: r_hc.round_dp(4),
            red_days_off_peak_ratio_percentage,
        }
    });

    let assessment = if let Some(ref t) = tempo_corr {
        if t.red_hp_kwh + t.red_hc_kwh > Decimal::ZERO {
            if t.red_days_off_peak_ratio_percentage >= Decimal::from(70) {
                format!(
                    "Excellente sobriété énergétique lors des jours rouges Tempo ({}% de report en heures creuses).",
                    t.red_days_off_peak_ratio_percentage
                )
            } else if t.red_days_off_peak_ratio_percentage >= Decimal::from(50) {
                format!(
                    "Effort notable de report en heures creuses en jour rouge ({}%). Potentiel d'amélioration sur les usages en journée.",
                    t.red_days_off_peak_ratio_percentage
                )
            } else {
                format!(
                    "Consommation soutenue en heures pleines lors des jours rouges Tempo (seulement {}% en heures creuses). Risque financier important.",
                    t.red_days_off_peak_ratio_percentage
                )
            }
        } else if tension_energy_percentage > Decimal::ZERO {
            format!(
                "{}% de la consommation mesurée coïncide avec des alertes de tension EcoWatt.",
                tension_energy_percentage
            )
        } else {
            "Consommation stable sans tension notable constatée sur le réseau électrique."
                .to_string()
        }
    } else if tension_energy_percentage > Decimal::ZERO {
        format!(
            "{}% de la consommation totale mesurée coïncide avec des alertes de tension du réseau EcoWatt.",
            tension_energy_percentage
        )
    } else {
        "Consommation en phase normale du réseau électrique.".to_string()
    };

    GridCorrelationReport {
        point_id,
        from,
        to,
        total_consumption_kwh: total_kwh.round_dp(4),
        ecowatt: ecowatt_corr,
        tempo: tempo_corr,
        assessment,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FlowDirection, MeasurementQuality, Unit};
    use chrono::TimeZone;

    #[test]
    fn test_ecowatt_level_conversions() {
        assert_eq!(EcoWattLevel::from_u8(1), Some(EcoWattLevel::Green));
        assert_eq!(EcoWattLevel::from_u8(2), Some(EcoWattLevel::Orange));
        assert_eq!(EcoWattLevel::from_u8(3), Some(EcoWattLevel::Red));
        assert_eq!(EcoWattLevel::from_u8(4), None);
        assert_eq!(EcoWattLevel::from_str_code("CRITICAL"), EcoWattLevel::Red);
        assert_eq!(EcoWattLevel::from_str_code("WARNING"), EcoWattLevel::Orange);
        assert_eq!(EcoWattLevel::from_str_code("NORMAL"), EcoWattLevel::Green);
    }

    #[test]
    fn test_correlate_measurements_with_grid() {
        let prm = PointId::new("01234567890123").unwrap();
        let from = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 1, 16, 0, 0, 0).unwrap();

        // 1 mesure verte (10h UTC = 11h local), 1 mesure rouge EcoWatt (18h UTC = 19h local)
        let m_green = Measurement {
            point_id: prm,
            timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 10, 0, 0).unwrap(),
            interval_seconds: 3600,
            direction: FlowDirection::Consumption,
            value: Decimal::from(2), // 2 kWh -> 2000 W
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        };

        let m_red = Measurement {
            point_id: prm,
            timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 18, 0, 0).unwrap(),
            interval_seconds: 3600,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str_exact("0.5").unwrap(), // 0.5 kWh -> 500 W (effacement)
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        };

        let eco_signals = vec![
            EcoWattSignal {
                timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 10, 0, 0).unwrap(),
                level: EcoWattLevel::Green,
                message: None,
            },
            EcoWattSignal {
                timestamp: Utc.with_ymd_and_hms(2026, 1, 15, 18, 0, 0).unwrap(),
                level: EcoWattLevel::Red,
                message: Some("Alerte pic".to_string()),
            },
        ];

        let tempo_days = vec![TempoDayRecord {
            date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            color: TempoColor::Red,
            updated_at: Utc::now(),
        }];

        let report = correlate_measurements_with_grid(
            prm,
            from,
            to,
            &[m_green, m_red],
            &eco_signals,
            Some(&tempo_days),
        );

        assert_eq!(
            report.total_consumption_kwh,
            Decimal::from_str_exact("2.5").unwrap()
        );
        assert_eq!(report.ecowatt.green_samples_count, 1);
        assert_eq!(report.ecowatt.red_samples_count, 1);
        assert_eq!(report.ecowatt.green_avg_power_w, Decimal::from(2000));
        assert_eq!(report.ecowatt.red_avg_power_w, Decimal::from(500));
        // Effacement : (2000 - 500) / 2000 = +75%
        assert_eq!(
            report.ecowatt.red_flexibility_score_percentage,
            Decimal::from(75)
        );

        let tempo_res = report.tempo.unwrap();
        // Les deux mesures (11h et 19h) sont en Heures Pleines sur ce jour rouge
        assert_eq!(
            tempo_res.red_hp_kwh,
            Decimal::from_str_exact("2.5").unwrap()
        );
        assert_eq!(tempo_res.red_hc_kwh, Decimal::ZERO);
    }
}
