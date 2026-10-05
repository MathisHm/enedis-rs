use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, Timelike, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

pub mod consent;
pub mod contract;
pub mod power;
pub mod signals;
pub mod spot;
pub mod tariff;

pub use consent::{ConsentAlert, ConsentAlertSeverity, ConsentInfo, ConsentStatus};
pub use contract::{
    CalendarSchedule, ContractData, MeterCharacteristics, MeterType, PhaseCount, TariffOption,
};
pub use power::{audit_subscription_sizing, MaxPowerRecord, SizingStatus, SubscriptionAudit};
pub use signals::{
    correlate_measurements_with_grid, EcoWattCorrelation, EcoWattLevel, EcoWattSignal,
    GridCorrelationReport, TempoColor, TempoCorrelation, TempoDayRecord,
};
pub use spot::{
    analyze_spot_consumption, generate_synthetic_spot_profile, SpotArbitrageOpportunity,
    SpotPriceRecord, SpotProfileAnalysis,
};
pub use tariff::{
    calculate_energy_costs, from_french_local_time, tempo_date_for_time, to_french_local_time,
    BaseTariff, CostCalculation, DynamicTariff, HpHcTariff, TariffComparison, TariffConfig,
    TariffCostBucket, TaxConfig, TempoTariff, TimeSlot,
};

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum PointIdError {
    #[error("Le PRM doit contenir exactement 14 chiffres (reçu: '{0}')")]
    InvalidFormat(String),
}

/// Identifiant unique d'un Point de Référence Mesure (PRM) Enedis (14 chiffres décimaux).
/// Garanti sans allocation superflue et vérifié à la construction.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "openapi", schema(value_type = String, example = "01234567890123", format = "string"))]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PointId([u8; 14]);

impl PointId {
    /// Identifiant factice par défaut (14 zéros)
    pub const DUMMY: Self = Self([b'0'; 14]);

    /// Valide et instancie un PointId à partir d'une chaîne ou tranche d'octets.
    pub fn new(prm: &str) -> Result<Self, PointIdError> {
        let bytes = prm.trim().as_bytes();
        if bytes.len() != 14 || !bytes.iter().all(|b| b.is_ascii_digit()) {
            return Err(PointIdError::InvalidFormat(prm.to_string()));
        }
        let mut arr = [0u8; 14];
        arr.copy_from_slice(bytes);
        Ok(Self(arr))
    }

    /// Accès zero-copy à la chaîne sous-jacente.
    #[inline]
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0)
            .expect("PointId contient uniquement des chiffres ASCII valides")
    }

    /// Vérifie la conformité éventuelle à la clé de contrôle de l'algorithme de Luhn
    pub fn is_luhn_valid(&self) -> bool {
        let mut sum = 0;
        let mut double = false;
        for &byte in self.0.iter().rev() {
            let mut digit = (byte - b'0') as u32;
            if double {
                digit *= 2;
                if digit > 9 {
                    digit -= 9;
                }
            }
            sum += digit;
            double = !double;
        }
        sum % 10 == 0
    }
}

impl AsRef<str> for PointId {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for PointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for PointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PointId({})", self.as_str())
    }
}

impl FromStr for PointId {
    type Err = PointIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for PointId {
    type Error = PointIdError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl Serialize for PointId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PointId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        PointId::from_str(&s).map_err(serde::de::Error::custom)
    }
}

/// Qualité de la mesure fournie par Enedis.
/// L'ordre total strict `Estimated` < `Corrected` < `Validated` permet l'UPSERT conditionnel sans régression.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum MeasurementQuality {
    /// Valeur estimée par Enedis (algorithmique / historique)
    Estimated = 1,
    /// Valeur redressée ou reconstituée après incident de relève
    Corrected = 2,
    /// Valeur mesurée, certifiée et validée pour facturation
    Validated = 3,
}

impl MeasurementQuality {
    #[inline]
    pub fn as_u8(&self) -> u8 {
        *self as u8
    }

    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Estimated),
            2 => Some(Self::Corrected),
            3 => Some(Self::Validated),
            _ => None,
        }
    }

    /// Analyse les libellés de qualité utilisés dans les flux SGE d'Enedis
    pub fn from_sge_code(code: &str) -> Self {
        match code.trim().to_uppercase().as_str() {
            "MESURE" | "VALIDE" | "CERTIFIE" | "BRUT" | "MEASURED" => Self::Validated,
            "CORRIGE" | "REDRESSE" | "RECONSTITUE" | "CORRECTED" => Self::Corrected,
            _ => Self::Estimated,
        }
    }
}

impl fmt::Display for MeasurementQuality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Estimated => write!(f, "ESTIMATED"),
            Self::Corrected => write!(f, "CORRECTED"),
            Self::Validated => write!(f, "VALIDATED"),
        }
    }
}

/// Sens du flux énergétique
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FlowDirection {
    /// Énergie soutirée du réseau (Consommation)
    Consumption,
    /// Énergie injectée sur le réseau (Production)
    Production,
}

impl FlowDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Consumption => "CONSUMPTION",
            Self::Production => "PRODUCTION",
        }
    }

    pub fn from_sge_code(code: &str) -> Self {
        match code.trim().to_uppercase().as_str() {
            "INJECTION" | "PROD" | "PRODUCTION" => Self::Production,
            _ => Self::Consumption,
        }
    }
}

impl fmt::Display for FlowDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Unités physiques normalisées selon les flux Enedis
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Unit {
    Watt,
    WattHour,
    KiloWatt,
    KiloWattHour,
    VoltAmpere,
    KiloVoltAmpere,
    KiloVoltAmpereReactiveHour,
}

impl Unit {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Watt => "W",
            Self::WattHour => "Wh",
            Self::KiloWatt => "kW",
            Self::KiloWattHour => "kWh",
            Self::VoltAmpere => "VA",
            Self::KiloVoltAmpere => "kVA",
            Self::KiloVoltAmpereReactiveHour => "kvarh",
        }
    }

    pub fn from_sge_code(code: &str) -> Self {
        match code.trim() {
            "W" => Self::Watt,
            "Wh" | "WH" => Self::WattHour,
            "kW" | "KW" => Self::KiloWatt,
            "kVA" | "KVA" => Self::KiloVoltAmpere,
            "VA" => Self::VoltAmpere,
            "kvarh" | "KVARH" => Self::KiloVoltAmpereReactiveHour,
            _ => Self::KiloWattHour,
        }
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Enregistrement d'une mesure horodatée
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Measurement {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "01234567890123"))]
    pub point_id: PointId,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T10:00:00Z"))]
    pub timestamp: DateTime<Utc>,
    /// Durée de l'intervalle d'intégration en secondes (ex: 1800 pour 30m, 86400 pour 1 jour)
    #[cfg_attr(feature = "openapi", schema(example = 1800))]
    pub interval_seconds: u32,
    pub direction: FlowDirection,
    /// Valeur décimale exacte (zéro flottant pour éviter toute imprécision)
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "3.1415"))]
    pub value: Decimal,
    pub unit: Unit,
    pub quality: MeasurementQuality,
}

impl Measurement {
    /// Énergie intégrée de cette mesure en kWh
    pub fn energy_kwh(&self) -> Decimal {
        match self.unit {
            Unit::KiloWattHour => self.value,
            Unit::WattHour => self.value / Decimal::from(1000),
            Unit::KiloWatt | Unit::KiloVoltAmpere => {
                (self.value * Decimal::from(self.interval_seconds)) / Decimal::from(3600)
            }
            Unit::Watt | Unit::VoltAmpere => {
                (self.value * Decimal::from(self.interval_seconds)) / Decimal::from(3_600_000)
            }
            // L'énergie réactive (kvarh) ne représente pas un travail actif en kWh
            Unit::KiloVoltAmpereReactiveHour => Decimal::ZERO,
        }
    }

    /// Puissance moyenne de cette mesure sur son intervalle en Watts (W)
    pub fn power_w(&self) -> Decimal {
        match self.unit {
            Unit::Watt | Unit::VoltAmpere => self.value,
            Unit::KiloWatt | Unit::KiloVoltAmpere => self.value * Decimal::from(1000),
            Unit::WattHour => {
                if self.interval_seconds > 0 {
                    (self.value * Decimal::from(3600)) / Decimal::from(self.interval_seconds)
                } else {
                    self.value
                }
            }
            Unit::KiloWattHour => {
                if self.interval_seconds > 0 {
                    (self.value * Decimal::from(3_600_000)) / Decimal::from(self.interval_seconds)
                } else {
                    self.value * Decimal::from(1000)
                }
            }
            // La puissance réactive (kvar) ne représente pas une puissance active en W
            Unit::KiloVoltAmpereReactiveHour => Decimal::ZERO,
        }
    }
}

/// Erreur survenue lors de l'analyse d'un intervalle d'agrégation
#[derive(Error, Debug, Clone, PartialEq, Eq)]
#[error("Intervalle d'agrégation invalide '{0}' (attendu: hour/hourly, day/daily, month/monthly, year/yearly)")]
pub struct AggregationIntervalError(pub String);

/// Pas d'agrégation temporelle pour l'analyse des courbes de charge
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AggregationInterval {
    #[serde(alias = "hour", alias = "1h", alias = "h")]
    Hourly,
    #[serde(alias = "day", alias = "1d", alias = "d")]
    Daily,
    #[serde(alias = "month", alias = "1m", alias = "m")]
    Monthly,
    #[serde(alias = "year", alias = "1y", alias = "y")]
    Yearly,
}

impl AggregationInterval {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hourly => "hourly",
            Self::Daily => "daily",
            Self::Monthly => "monthly",
            Self::Yearly => "yearly",
        }
    }

    /// Détermine le début de fenêtre temporelle (borne inférieure inclusive) pour un horodatage donné
    pub fn bucket_start(&self, ts: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            Self::Hourly => ts
                .date_naive()
                .and_hms_opt(ts.hour(), 0, 0)
                .unwrap_or_else(|| ts.date_naive().and_time(chrono::NaiveTime::MIN))
                .and_utc(),
            Self::Daily => ts.date_naive().and_time(chrono::NaiveTime::MIN).and_utc(),
            Self::Monthly => NaiveDate::from_ymd_opt(ts.year(), ts.month(), 1)
                .unwrap_or_else(|| ts.date_naive())
                .and_time(chrono::NaiveTime::MIN)
                .and_utc(),
            Self::Yearly => NaiveDate::from_ymd_opt(ts.year(), 1, 1)
                .unwrap_or_else(|| ts.date_naive())
                .and_time(chrono::NaiveTime::MIN)
                .and_utc(),
        }
    }

    /// Calcule la fin de fenêtre temporelle (borne supérieure exclusive) à partir du début de fenêtre
    pub fn bucket_end(&self, start: DateTime<Utc>) -> DateTime<Utc> {
        match self {
            Self::Hourly => start + ChronoDuration::hours(1),
            Self::Daily => start + ChronoDuration::days(1),
            Self::Monthly => {
                let (y, m) = if start.month() == 12 {
                    (start.year() + 1, 1)
                } else {
                    (start.year(), start.month() + 1)
                };
                NaiveDate::from_ymd_opt(y, m, 1)
                    .unwrap_or_else(|| start.date_naive() + ChronoDuration::days(31))
                    .and_time(chrono::NaiveTime::MIN)
                    .and_utc()
            }
            Self::Yearly => NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)
                .unwrap_or_else(|| start.date_naive() + ChronoDuration::days(366))
                .and_time(chrono::NaiveTime::MIN)
                .and_utc(),
        }
    }
}

impl fmt::Display for AggregationInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for AggregationInterval {
    type Err = AggregationIntervalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "hour" | "hourly" | "1h" | "h" => Ok(Self::Hourly),
            "day" | "daily" | "1d" | "d" => Ok(Self::Daily),
            "month" | "monthly" | "1m" | "m" => Ok(Self::Monthly),
            "year" | "yearly" | "1y" | "y" => Ok(Self::Yearly),
            _ => Err(AggregationIntervalError(s.to_string())),
        }
    }
}

/// Mesure agrégée calculée sur une fenêtre temporelle
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregatedMeasurement {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "01234567890123"))]
    pub point_id: PointId,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T00:00:00Z"))]
    pub bucket_start: DateTime<Utc>,
    #[cfg_attr(feature = "openapi", schema(example = "2026-09-28T01:00:00Z"))]
    pub bucket_end: DateTime<Utc>,
    pub direction: FlowDirection,
    /// Intégrale de l'énergie sur la période en kWh
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "42.5000"))]
    pub total_energy_kwh: Decimal,
    /// Puissance de pointe atteinte en Watts
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "3500.0000"))]
    pub max_power_w: Option<Decimal>,
    /// Puissance minimale / bruit de fond en Watts
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "250.0000"))]
    pub min_power_w: Option<Decimal>,
    /// Puissance moyenne en Watts
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "1250.5000"))]
    pub avg_power_w: Option<Decimal>,
    /// Nombre de pas de mesure agrégés
    #[cfg_attr(feature = "openapi", schema(example = 2))]
    pub sample_count: u32,
}

/// Agrège une collection de mesures en mémoire selon un intervalle temporel donné
pub fn aggregate_measurements(
    measurements: &[Measurement],
    interval: AggregationInterval,
) -> Vec<AggregatedMeasurement> {
    use std::collections::BTreeMap;

    if measurements.is_empty() {
        return Vec::new();
    }

    let mut groups: BTreeMap<(PointId, FlowDirection, DateTime<Utc>), Vec<&Measurement>> =
        BTreeMap::new();
    for m in measurements {
        let b_start = interval.bucket_start(m.timestamp);
        groups
            .entry((m.point_id, m.direction, b_start))
            .or_default()
            .push(m);
    }

    let mut results = Vec::with_capacity(groups.len());
    for ((point_id, direction, bucket_start), items) in groups {
        let bucket_end = interval.bucket_end(bucket_start);
        let sample_count = items.len() as u32;

        let total_energy_kwh: Decimal = items
            .iter()
            .map(|m| m.energy_kwh())
            .sum::<Decimal>()
            .round_dp(4);
        let powers: Vec<Decimal> = items.iter().map(|m| m.power_w()).collect();

        let max_power_w = powers.iter().max().copied().map(|p| p.round_dp(4));
        let min_power_w = powers.iter().min().copied().map(|p| p.round_dp(4));
        let avg_power_w = if !powers.is_empty() {
            let sum: Decimal = powers.iter().sum();
            Some((sum / Decimal::from(sample_count)).round_dp(4))
        } else {
            None
        };

        results.push(AggregatedMeasurement {
            point_id,
            bucket_start,
            bucket_end,
            direction,
            total_energy_kwh,
            max_power_w,
            min_power_w,
            avg_power_w,
            sample_count,
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_id_valid() {
        let prm = PointId::new("01234567890123").unwrap();
        assert_eq!(prm.as_str(), "01234567890123");
        assert_eq!(prm.to_string(), "01234567890123");
        assert_eq!(format!("{:?}", prm), "PointId(01234567890123)");
        assert_eq!(prm.as_ref(), "01234567890123");
    }

    #[test]
    fn test_point_id_trim_and_errors() {
        // Trim whitespace
        let prm = PointId::new("  01234567890123  ").unwrap();
        assert_eq!(prm.as_str(), "01234567890123");

        // Trop court (13 chiffres)
        assert!(matches!(
            PointId::new("1234567890123"),
            Err(PointIdError::InvalidFormat(_))
        ));

        // Trop long (15 chiffres)
        assert!(matches!(
            PointId::new("123456789012345"),
            Err(PointIdError::InvalidFormat(_))
        ));

        // Contient des lettres
        assert!(matches!(
            PointId::new("0123456789012A"),
            Err(PointIdError::InvalidFormat(_))
        ));

        // Vide
        assert!(matches!(
            PointId::new(""),
            Err(PointIdError::InvalidFormat(_))
        ));
    }

    #[test]
    fn test_point_id_luhn_validation() {
        // "12345678901237" vérifie la somme de Luhn mod 10 == 0
        let valid_luhn = PointId::new("12345678901237").unwrap();
        assert!(valid_luhn.is_luhn_valid());

        // "12345678901234" a une somme de 57 (invalide)
        let invalid_luhn = PointId::new("12345678901234").unwrap();
        assert!(!invalid_luhn.is_luhn_valid());
    }

    #[test]
    fn test_point_id_serde() {
        let prm = PointId::new("01234567890123").unwrap();
        let json = serde_json::to_string(&prm).unwrap();
        assert_eq!(json, "\"01234567890123\"");

        let deserialized: PointId = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, prm);

        // Erreur de désérialisation sur format invalide
        let invalid_json = "\"123\"";
        assert!(serde_json::from_str::<PointId>(invalid_json).is_err());
    }

    #[test]
    fn test_measurement_quality_conversion() {
        assert_eq!(
            MeasurementQuality::from_u8(1),
            Some(MeasurementQuality::Estimated)
        );
        assert_eq!(
            MeasurementQuality::from_u8(2),
            Some(MeasurementQuality::Corrected)
        );
        assert_eq!(
            MeasurementQuality::from_u8(3),
            Some(MeasurementQuality::Validated)
        );
        assert_eq!(MeasurementQuality::from_u8(4), None);

        assert_eq!(MeasurementQuality::Estimated.as_u8(), 1);
        assert_eq!(MeasurementQuality::Corrected.as_u8(), 2);
        assert_eq!(MeasurementQuality::Validated.as_u8(), 3);

        assert_eq!(
            MeasurementQuality::from_sge_code("MESURE"),
            MeasurementQuality::Validated
        );
        assert_eq!(
            MeasurementQuality::from_sge_code("valide"),
            MeasurementQuality::Validated
        );
        assert_eq!(
            MeasurementQuality::from_sge_code("certifie"),
            MeasurementQuality::Validated
        );
        assert_eq!(
            MeasurementQuality::from_sge_code("brut"),
            MeasurementQuality::Validated
        );
        assert_eq!(
            MeasurementQuality::from_sge_code("measured"),
            MeasurementQuality::Validated
        );

        assert_eq!(
            MeasurementQuality::from_sge_code("corrige"),
            MeasurementQuality::Corrected
        );
        assert_eq!(
            MeasurementQuality::from_sge_code("redresse"),
            MeasurementQuality::Corrected
        );
        assert_eq!(
            MeasurementQuality::from_sge_code("reconstitue"),
            MeasurementQuality::Corrected
        );
        assert_eq!(
            MeasurementQuality::from_sge_code("corrected"),
            MeasurementQuality::Corrected
        );

        assert_eq!(
            MeasurementQuality::from_sge_code("AUTRE"),
            MeasurementQuality::Estimated
        );
        assert_eq!(
            MeasurementQuality::from_sge_code("ESTIME"),
            MeasurementQuality::Estimated
        );
    }

    #[test]
    fn test_flow_direction_and_units() {
        assert_eq!(
            FlowDirection::from_sge_code("INJECTION"),
            FlowDirection::Production
        );
        assert_eq!(
            FlowDirection::from_sge_code("prod"),
            FlowDirection::Production
        );
        assert_eq!(
            FlowDirection::from_sge_code("production"),
            FlowDirection::Production
        );
        assert_eq!(
            FlowDirection::from_sge_code("CONSOMMATION"),
            FlowDirection::Consumption
        );
        assert_eq!(FlowDirection::Consumption.as_str(), "CONSUMPTION");
        assert_eq!(FlowDirection::Production.as_str(), "PRODUCTION");

        assert_eq!(Unit::from_sge_code("W"), Unit::Watt);
        assert_eq!(Unit::from_sge_code("Wh"), Unit::WattHour);
        assert_eq!(Unit::from_sge_code("kW"), Unit::KiloWatt);
        assert_eq!(Unit::from_sge_code("kWh"), Unit::KiloWattHour);
        assert_eq!(Unit::from_sge_code("VA"), Unit::VoltAmpere);
        assert_eq!(Unit::from_sge_code("kVA"), Unit::KiloVoltAmpere);
        assert_eq!(
            Unit::from_sge_code("kvarh"),
            Unit::KiloVoltAmpereReactiveHour
        );
        assert_eq!(Unit::from_sge_code("inconnu"), Unit::KiloWattHour);
    }

    #[test]
    fn test_aggregation_interval_parsing_and_serde() {
        assert_eq!(
            AggregationInterval::from_str("hour").unwrap(),
            AggregationInterval::Hourly
        );
        assert_eq!(
            AggregationInterval::from_str("hourly").unwrap(),
            AggregationInterval::Hourly
        );
        assert_eq!(
            AggregationInterval::from_str("1h").unwrap(),
            AggregationInterval::Hourly
        );
        assert_eq!(
            AggregationInterval::from_str("day").unwrap(),
            AggregationInterval::Daily
        );
        assert_eq!(
            AggregationInterval::from_str("daily").unwrap(),
            AggregationInterval::Daily
        );
        assert_eq!(
            AggregationInterval::from_str("month").unwrap(),
            AggregationInterval::Monthly
        );
        assert_eq!(
            AggregationInterval::from_str("monthly").unwrap(),
            AggregationInterval::Monthly
        );
        assert_eq!(
            AggregationInterval::from_str("year").unwrap(),
            AggregationInterval::Yearly
        );
        assert_eq!(
            AggregationInterval::from_str("yearly").unwrap(),
            AggregationInterval::Yearly
        );

        assert!(AggregationInterval::from_str("invalid").is_err());

        // Serde
        let json = serde_json::to_string(&AggregationInterval::Daily).unwrap();
        assert_eq!(json, "\"daily\"");
        let deserialized: AggregationInterval = serde_json::from_str("\"day\"").unwrap();
        assert_eq!(deserialized, AggregationInterval::Daily);
        let deserialized_hourly: AggregationInterval = serde_json::from_str("\"hour\"").unwrap();
        assert_eq!(deserialized_hourly, AggregationInterval::Hourly);
    }

    #[test]
    fn test_aggregation_interval_bucket_boundaries() {
        use chrono::TimeZone;

        // Horodatage: 2026-09-28 14:35:22 UTC
        let ts = Utc.with_ymd_and_hms(2026, 9, 28, 14, 35, 22).unwrap();

        // Hourly
        let h_start = AggregationInterval::Hourly.bucket_start(ts);
        assert_eq!(
            h_start,
            Utc.with_ymd_and_hms(2026, 9, 28, 14, 0, 0).unwrap()
        );
        let h_end = AggregationInterval::Hourly.bucket_end(h_start);
        assert_eq!(h_end, Utc.with_ymd_and_hms(2026, 9, 28, 15, 0, 0).unwrap());

        // Daily
        let d_start = AggregationInterval::Daily.bucket_start(ts);
        assert_eq!(d_start, Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap());
        let d_end = AggregationInterval::Daily.bucket_end(d_start);
        assert_eq!(d_end, Utc.with_ymd_and_hms(2026, 9, 29, 0, 0, 0).unwrap());

        // Monthly
        let m_start = AggregationInterval::Monthly.bucket_start(ts);
        assert_eq!(m_start, Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap());
        let m_end = AggregationInterval::Monthly.bucket_end(m_start);
        assert_eq!(m_end, Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap());

        // Month boundary rollover (December to January)
        let dec_ts = Utc.with_ymd_and_hms(2026, 12, 15, 10, 0, 0).unwrap();
        let dec_start = AggregationInterval::Monthly.bucket_start(dec_ts);
        assert_eq!(
            dec_start,
            Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap()
        );
        let dec_end = AggregationInterval::Monthly.bucket_end(dec_start);
        assert_eq!(dec_end, Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap());

        // Yearly
        let y_start = AggregationInterval::Yearly.bucket_start(ts);
        assert_eq!(y_start, Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap());
        let y_end = AggregationInterval::Yearly.bucket_end(y_start);
        assert_eq!(y_end, Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap());
    }

    #[test]
    fn test_measurement_energy_and_power_conversions() {
        let prm = PointId::new("01234567890123").unwrap();
        let ts = Utc::now();

        // 1. 2000 W sur 30 min (1800s) -> 1.0 kWh, 2000 W
        let m_watt = Measurement {
            point_id: prm,
            timestamp: ts,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from(2000),
            unit: Unit::Watt,
            quality: MeasurementQuality::Validated,
        };
        assert_eq!(m_watt.energy_kwh(), Decimal::from_str("1.0").unwrap());
        assert_eq!(m_watt.power_w(), Decimal::from(2000));

        // 2. 2 kW sur 30 min (1800s) -> 1.0 kWh, 2000 W
        let m_kw = Measurement {
            point_id: prm,
            timestamp: ts,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from(2),
            unit: Unit::KiloWatt,
            quality: MeasurementQuality::Validated,
        };
        assert_eq!(m_kw.energy_kwh(), Decimal::from_str("1.0").unwrap());
        assert_eq!(m_kw.power_w(), Decimal::from(2000));

        // 3. 1000 Wh sur 30 min (1800s) -> 1.0 kWh, 2000 W
        let m_wh = Measurement {
            point_id: prm,
            timestamp: ts,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from(1000),
            unit: Unit::WattHour,
            quality: MeasurementQuality::Validated,
        };
        assert_eq!(m_wh.energy_kwh(), Decimal::from_str("1.0").unwrap());
        assert_eq!(m_wh.power_w(), Decimal::from(2000));

        // 4. 1 kWh sur 30 min (1800s) -> 1.0 kWh, 2000 W
        let m_kwh = Measurement {
            point_id: prm,
            timestamp: ts,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from(1),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        };
        assert_eq!(m_kwh.energy_kwh(), Decimal::from_str("1.0").unwrap());
        assert_eq!(m_kwh.power_w(), Decimal::from(2000));
    }

    #[test]
    fn test_aggregate_measurements_mathematical_accuracy_variable_intervals() {
        use chrono::TimeZone;
        let prm = PointId::new("01234567890123").unwrap();

        // 3 mesures dans la même heure (10:00 - 11:00 UTC) avec des pas variables :
        // 1. 10 minutes (600s) à 1200 W -> 1.2 kW * (1/6) h = 0.2 kWh
        // 2. 15 minutes (900s) à 2000 W -> 2.0 kW * (1/4) h = 0.5 kWh
        // 3. 30 minutes (1800s) à 600 W -> 0.6 kW * (1/2) h = 0.3 kWh
        // Total énergie = 0.2 + 0.5 + 0.3 = 1.0 kWh
        // Puissances: min = 600 W, max = 2000 W, avg = (1200 + 2000 + 600) / 3 = 1266.6667 W
        let m1 = Measurement {
            point_id: prm,
            timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 10, 0, 0).unwrap(),
            interval_seconds: 600,
            direction: FlowDirection::Consumption,
            value: Decimal::from(1200),
            unit: Unit::Watt,
            quality: MeasurementQuality::Validated,
        };
        let m2 = Measurement {
            point_id: prm,
            timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 10, 10, 0).unwrap(),
            interval_seconds: 900,
            direction: FlowDirection::Consumption,
            value: Decimal::from(2000),
            unit: Unit::Watt,
            quality: MeasurementQuality::Validated,
        };
        let m3 = Measurement {
            point_id: prm,
            timestamp: Utc.with_ymd_and_hms(2026, 9, 28, 10, 25, 0).unwrap(),
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from(600),
            unit: Unit::Watt,
            quality: MeasurementQuality::Validated,
        };

        let aggregates = aggregate_measurements(&[m1, m2, m3], AggregationInterval::Hourly);
        assert_eq!(aggregates.len(), 1);

        let agg = &aggregates[0];
        assert_eq!(agg.point_id, prm);
        assert_eq!(agg.direction, FlowDirection::Consumption);
        assert_eq!(
            agg.bucket_start,
            Utc.with_ymd_and_hms(2026, 9, 28, 10, 0, 0).unwrap()
        );
        assert_eq!(
            agg.bucket_end,
            Utc.with_ymd_and_hms(2026, 9, 28, 11, 0, 0).unwrap()
        );
        assert_eq!(agg.sample_count, 3);
        assert_eq!(agg.total_energy_kwh, Decimal::from_str("1.0000").unwrap());
        assert_eq!(
            agg.max_power_w,
            Some(Decimal::from_str("2000.0000").unwrap())
        );
        assert_eq!(
            agg.min_power_w,
            Some(Decimal::from_str("600.0000").unwrap())
        );
        assert_eq!(
            agg.avg_power_w,
            Some(Decimal::from_str("1266.6667").unwrap())
        );
    }
}
