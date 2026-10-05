use crate::error::{EnedisError, SgeBusinessError};
use crate::models::{
    from_french_local_time, CalendarSchedule, ConsentInfo, ConsentStatus, ContractData,
    FlowDirection, MaxPowerRecord, Measurement, MeasurementQuality, MeterCharacteristics,
    MeterType, PhaseCount, PointId, TariffOption, Unit,
};
use chrono::{DateTime, Utc};
use quick_xml::events::Event;
use quick_xml::Reader;
use rust_decimal::Decimal;
use std::str::FromStr;

/// Parseur streaming pour les charges utiles XML métier renvoyées par Enedis SGE
pub struct SgeResponseParser;

impl SgeResponseParser {
    /// Analyse le corps XML et extrait la liste des mesures ou les erreurs fonctionnelles SGE
    pub fn parse_measurements(xml_data: &[u8]) -> Result<Vec<Measurement>, EnedisError> {
        let mut reader = Reader::from_reader(xml_data);
        let mut buf = Vec::with_capacity(512);

        let mut measurements = Vec::new();
        let mut current_point_id: Option<PointId> = None;
        let mut current_direction = FlowDirection::Consumption;
        let mut current_interval: u32 = 1800; // 30 minutes par défaut pour la courbe de charge

        // Accumulateurs pour le point de mesure courant
        let mut cur_timestamp: Option<DateTime<Utc>> = None;
        let mut cur_value: Option<Decimal> = None;
        let mut cur_unit: Unit = Unit::KiloWattHour;
        let mut cur_quality: MeasurementQuality = MeasurementQuality::Estimated;

        // Détection d'erreurs fonctionnelles SGE dans le XML
        let mut in_error_block = false;
        let mut error_code = String::new();
        let mut error_msg = String::new();
        let mut error_prm = String::new();

        let mut current_tag = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let local_name = e.local_name();
                    let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                    current_tag = name_str.to_string();

                    match current_tag.to_lowercase().as_str() {
                        "erreur" | "erreurfonctionnelle" | "anomalie" => {
                            in_error_block = true;
                        }
                        "pointmesure" | "mesure" | "valeurhorodatee" => {
                            // Réinitialiser les accumulateurs du point
                            cur_timestamp = None;
                            cur_value = None;
                            cur_quality = MeasurementQuality::Estimated;
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(ref e)) => {
                    let text = e.unescape().unwrap_or_default().trim().to_string();
                    if text.is_empty() {
                        continue;
                    }

                    if in_error_block {
                        match current_tag.to_lowercase().as_str() {
                            "code" | "codeerreur" => error_code = text,
                            "libelle" | "message" | "libelleerreur" => error_msg = text,
                            "prm" | "idprm" | "pointid" => error_prm = text,
                            _ => {}
                        }
                    } else {
                        match current_tag.to_lowercase().as_str() {
                            "prm" | "idpoint" | "pointid" | "pointdelivraison" => {
                                if let Ok(prm) = PointId::new(&text) {
                                    current_point_id = Some(prm);
                                }
                            }
                            "sens" | "direction" | "typeflux" => {
                                current_direction = FlowDirection::from_sge_code(&text);
                            }
                            "pas" | "interval" | "pascalcul" => {
                                if let Ok(sec) = text.parse::<u32>() {
                                    current_interval = sec;
                                } else {
                                    match text.trim().to_uppercase().as_str() {
                                        "PT10M" => current_interval = 600,
                                        "PT15M" => current_interval = 900,
                                        "PT30M" => current_interval = 1800,
                                        "PT1H" | "PT60M" => current_interval = 3600,
                                        "P1D" => current_interval = 86400,
                                        _ => {}
                                    }
                                }
                            }
                            "date" | "horodatage" | "timestamp" => {
                                if let Ok(dt) = DateTime::parse_from_rfc3339(&text) {
                                    cur_timestamp = Some(dt.with_timezone(&Utc));
                                } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(
                                    &text,
                                    "%Y-%m-%d %H:%M:%S",
                                ) {
                                    cur_timestamp = Some(from_french_local_time(dt));
                                }
                            }
                            "valeur" | "value" => {
                                if let Ok(val) = Decimal::from_str(&text) {
                                    cur_value = Some(val);
                                }
                            }
                            "unite" | "unit" => {
                                cur_unit = Unit::from_sge_code(&text);
                            }
                            "qualite" | "statut" | "nature" => {
                                cur_quality = MeasurementQuality::from_sge_code(&text);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let local_name = e.local_name();
                    let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");

                    match name_str.to_lowercase().as_str() {
                        "erreur" | "erreurfonctionnelle" | "anomalie" => {
                            in_error_block = false;
                        }
                        "pointmesure" | "mesure" | "valeurhorodatee" => {
                            if let (Some(pid), Some(ts), Some(val)) =
                                (current_point_id, cur_timestamp, cur_value)
                            {
                                measurements.push(Measurement {
                                    point_id: pid,
                                    timestamp: ts,
                                    interval_seconds: current_interval,
                                    direction: current_direction,
                                    value: val,
                                    unit: cur_unit,
                                    quality: cur_quality,
                                });
                            }
                        }
                        _ => {}
                    }
                    current_tag.clear();
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(EnedisError::Xml(format!("Erreur XML SGE: {}", e))),
                _ => {}
            }
            buf.clear();
        }

        Self::check_functional_error(&error_code, &error_msg, &error_prm, current_point_id)?;

        Ok(measurements)
    }

    /// Analyse la réponse XML de l'opération SGE `consulterDonneesContractuelles`
    pub fn parse_contract_data(xml_data: &[u8]) -> Result<ContractData, EnedisError> {
        let mut reader = Reader::from_reader(xml_data);
        let mut buf = Vec::with_capacity(512);

        let mut point_id: Option<PointId> = None;
        let mut subscribed_power_kva: u32 = 6;
        let mut tariff_option = TariffOption::Base;
        let mut contract_status: Option<String> = None;

        // Calendrier
        let mut schedule_name: Option<String> = None;
        let mut off_peak_ranges: Vec<String> = Vec::new();
        let mut tempo_color: Option<String> = None;
        let mut seasonal_mode: Option<String> = None;

        // Compteur
        let mut meter_serial: Option<String> = None;
        let mut meter_type = MeterType::Linky;
        let mut phase_count = PhaseCount::SinglePhase;
        let mut circuit_breaker_amperes: Option<u32> = None;

        let mut in_error_block = false;
        let mut error_code = String::new();
        let mut error_msg = String::new();
        let mut error_prm = String::new();

        let mut current_tag = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let local_name = e.local_name();
                    let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                    current_tag = name_str.to_string();

                    if matches!(
                        current_tag.to_lowercase().as_str(),
                        "erreur" | "erreurfonctionnelle" | "anomalie"
                    ) {
                        in_error_block = true;
                    }
                }
                Ok(Event::Text(ref e)) => {
                    let text = e.unescape().unwrap_or_default().trim().to_string();
                    if text.is_empty() {
                        continue;
                    }

                    if in_error_block {
                        match current_tag.to_lowercase().as_str() {
                            "code" | "codeerreur" => error_code = text,
                            "libelle" | "message" | "libelleerreur" => error_msg = text,
                            "prm" | "idprm" | "pointid" => error_prm = text,
                            _ => {}
                        }
                    } else {
                        match current_tag.to_lowercase().as_str() {
                            "prm" | "idpoint" | "pointid" | "pointdelivraison" => {
                                if let Ok(prm) = PointId::new(&text) {
                                    point_id = Some(prm);
                                }
                            }
                            "puissancesouscrite" | "puissance" | "valeurpuissance" => {
                                // Gère "6", "6 kVA", "6.0", etc.
                                let clean: String = text
                                    .chars()
                                    .filter(|c| c.is_ascii_digit() || *c == '.')
                                    .collect();
                                if let Ok(val) = clean.parse::<f64>() {
                                    subscribed_power_kva = val.round() as u32;
                                }
                            }
                            "optiontarifaire" | "option" | "formuletarifaire" => {
                                tariff_option = TariffOption::from_sge_code(&text);
                            }
                            "statutcontrat" | "etatcontrat" | "statut" => {
                                contract_status = Some(text);
                            }
                            "libellecalendrier" | "nomcalendrier" | "libelle" => {
                                schedule_name = Some(text);
                            }
                            "plagesheurescreuses" | "heurescreuses" | "plagehc" => {
                                off_peak_ranges.push(text);
                            }
                            "couleurjour" | "couleurtempo" => {
                                tempo_color = Some(text);
                            }
                            "saison" | "modepointe" => {
                                seasonal_mode = Some(text);
                            }
                            "numeroserie" | "matricule" | "matriculecompteur" => {
                                meter_serial = Some(text);
                            }
                            "typecompteur" | "modelecompteur" => {
                                meter_type = MeterType::from_sge_code(&text);
                            }
                            "nombrephases" | "typebranchement" | "phases" => {
                                phase_count = PhaseCount::from_sge_code(&text);
                            }
                            "calibredisjoncteur" | "reglagedisjoncteur" => {
                                if let Ok(val) = text.parse::<u32>() {
                                    circuit_breaker_amperes = Some(val);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let local_name = e.local_name();
                    let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                    if matches!(
                        name_str.to_lowercase().as_str(),
                        "erreur" | "erreurfonctionnelle" | "anomalie"
                    ) {
                        in_error_block = false;
                    }
                    current_tag.clear();
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(EnedisError::Xml(format!("Erreur XML SGE: {}", e))),
                _ => {}
            }
            buf.clear();
        }

        Self::check_functional_error(&error_code, &error_msg, &error_prm, point_id)?;

        let final_prm = point_id.unwrap_or(PointId::DUMMY);

        let calendar = if schedule_name.is_some()
            || !off_peak_ranges.is_empty()
            || tempo_color.is_some()
            || seasonal_mode.is_some()
        {
            Some(CalendarSchedule {
                schedule_name,
                off_peak_ranges,
                tempo_color,
                seasonal_mode,
            })
        } else {
            None
        };

        let meter = if meter_serial.is_some()
            || !matches!(meter_type, MeterType::Other(_))
            || circuit_breaker_amperes.is_some()
        {
            Some(MeterCharacteristics {
                serial_number: meter_serial,
                meter_type,
                phase_count,
                circuit_breaker_amperes,
            })
        } else {
            None
        };

        Ok(ContractData {
            point_id: final_prm,
            subscribed_power_kva,
            tariff_option,
            calendar,
            meter,
            status: contract_status,
        })
    }

    /// Analyse la réponse XML de l'opération SGE `consulterPuissanceMax`
    pub fn parse_max_power(xml_data: &[u8]) -> Result<Vec<MaxPowerRecord>, EnedisError> {
        let mut reader = Reader::from_reader(xml_data);
        let mut buf = Vec::with_capacity(512);

        let mut records = Vec::new();
        let mut global_point_id: Option<PointId> = None;

        let mut cur_timestamp: Option<DateTime<Utc>> = None;
        let mut cur_value: Option<Decimal> = None;
        let mut cur_unit = Unit::VoltAmpere;

        let mut in_error_block = false;
        let mut error_code = String::new();
        let mut error_msg = String::new();
        let mut error_prm = String::new();

        let mut current_tag = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let local_name = e.local_name();
                    let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                    current_tag = name_str.to_string();

                    match current_tag.to_lowercase().as_str() {
                        "erreur" | "erreurfonctionnelle" | "anomalie" => {
                            in_error_block = true;
                        }
                        "pointe" | "puissancemax" | "mesure" | "valeurhorodatee" => {
                            cur_timestamp = None;
                            cur_value = None;
                            cur_unit = Unit::VoltAmpere;
                        }
                        _ => {}
                    }
                }
                Ok(Event::Text(ref e)) => {
                    let text = e.unescape().unwrap_or_default().trim().to_string();
                    if text.is_empty() {
                        continue;
                    }

                    if in_error_block {
                        match current_tag.to_lowercase().as_str() {
                            "code" | "codeerreur" => error_code = text,
                            "libelle" | "message" | "libelleerreur" => error_msg = text,
                            "prm" | "idprm" | "pointid" => error_prm = text,
                            _ => {}
                        }
                    } else {
                        match current_tag.to_lowercase().as_str() {
                            "prm" | "idpoint" | "pointid" | "pointdelivraison" => {
                                if let Ok(prm) = PointId::new(&text) {
                                    global_point_id = Some(prm);
                                }
                            }
                            "date" | "horodatage" | "timestamp" => {
                                if let Ok(dt) = DateTime::parse_from_rfc3339(&text) {
                                    cur_timestamp = Some(dt.with_timezone(&Utc));
                                } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(
                                    &text,
                                    "%Y-%m-%d %H:%M:%S",
                                ) {
                                    cur_timestamp = Some(from_french_local_time(dt));
                                }
                            }
                            "valeur" | "value" | "puissance" => {
                                if let Ok(val) = Decimal::from_str(&text) {
                                    cur_value = Some(val);
                                }
                            }
                            "unite" | "unit" => {
                                cur_unit = Unit::from_sge_code(&text);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let local_name = e.local_name();
                    let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");

                    match name_str.to_lowercase().as_str() {
                        "erreur" | "erreurfonctionnelle" | "anomalie" => {
                            in_error_block = false;
                        }
                        "pointe" | "puissancemax" | "mesure" | "valeurhorodatee" => {
                            if let (Some(ts), Some(val)) = (cur_timestamp, cur_value) {
                                records.push(MaxPowerRecord {
                                    point_id: global_point_id.unwrap_or(PointId::DUMMY),
                                    timestamp: ts,
                                    value: val,
                                    unit: cur_unit,
                                });
                            }
                        }
                        _ => {}
                    }
                    current_tag.clear();
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(EnedisError::Xml(format!("Erreur XML SGE: {}", e))),
                _ => {}
            }
            buf.clear();
        }

        Self::check_functional_error(&error_code, &error_msg, &error_prm, global_point_id)?;

        Ok(records)
    }

    /// Analyse la réponse XML de l'opération SGE de suivi du consentement client
    pub fn parse_consent_status(xml_data: &[u8]) -> Result<ConsentInfo, EnedisError> {
        let mut reader = Reader::from_reader(xml_data);
        let mut buf = Vec::with_capacity(512);

        let mut point_id: Option<PointId> = None;
        let mut status = ConsentStatus::Active;
        let mut valid_from: Option<DateTime<Utc>> = None;
        let mut valid_to: Option<DateTime<Utc>> = None;
        let mut authorized_usages: Vec<String> = Vec::new();

        let mut in_error_block = false;
        let mut error_code = String::new();
        let mut error_msg = String::new();
        let mut error_prm = String::new();

        let mut current_tag = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    let local_name = e.local_name();
                    let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                    current_tag = name_str.to_string();

                    if matches!(
                        current_tag.to_lowercase().as_str(),
                        "erreur" | "erreurfonctionnelle" | "anomalie"
                    ) {
                        in_error_block = true;
                    }
                }
                Ok(Event::Text(ref e)) => {
                    let text = e.unescape().unwrap_or_default().trim().to_string();
                    if text.is_empty() {
                        continue;
                    }

                    if in_error_block {
                        match current_tag.to_lowercase().as_str() {
                            "code" | "codeerreur" => error_code = text,
                            "libelle" | "message" | "libelleerreur" => error_msg = text,
                            "prm" | "idprm" | "pointid" => error_prm = text,
                            _ => {}
                        }
                    } else {
                        match current_tag.to_lowercase().as_str() {
                            "prm" | "idpoint" | "pointid" | "pointdelivraison" => {
                                if let Ok(prm) = PointId::new(&text) {
                                    point_id = Some(prm);
                                }
                            }
                            "etat" | "statut" | "status" => {
                                status = ConsentStatus::from_sge_code(&text);
                            }
                            "datedebut" | "debut" | "start" | "validfrom" => {
                                if let Ok(dt) = DateTime::parse_from_rfc3339(&text) {
                                    valid_from = Some(dt.with_timezone(&Utc));
                                } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(
                                    &text,
                                    "%Y-%m-%d %H:%M:%S",
                                ) {
                                    valid_from = Some(from_french_local_time(dt));
                                }
                            }
                            "datefin" | "fin" | "end" | "validto" | "echeance" => {
                                if let Ok(dt) = DateTime::parse_from_rfc3339(&text) {
                                    valid_to = Some(dt.with_timezone(&Utc));
                                } else if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(
                                    &text,
                                    "%Y-%m-%d %H:%M:%S",
                                ) {
                                    valid_to = Some(from_french_local_time(dt));
                                }
                            }
                            "usage" | "usageautorise" | "perimetre" => {
                                authorized_usages.push(text);
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Event::End(ref e)) => {
                    let local_name = e.local_name();
                    let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                    if matches!(
                        name_str.to_lowercase().as_str(),
                        "erreur" | "erreurfonctionnelle" | "anomalie"
                    ) {
                        in_error_block = false;
                    }
                    current_tag.clear();
                }
                Ok(Event::Eof) => break,
                Err(e) => return Err(EnedisError::Xml(format!("Erreur XML SGE: {}", e))),
                _ => {}
            }
            buf.clear();
        }

        Self::check_functional_error(&error_code, &error_msg, &error_prm, point_id)?;

        Ok(ConsentInfo {
            point_id: point_id.unwrap_or(PointId::DUMMY),
            status,
            valid_from,
            valid_to,
            authorized_usages,
        })
    }

    /// Mappe un code et message d'erreur fonctionnelle SGE vers l'erreur métier Enedis
    fn check_functional_error(
        error_code: &str,
        error_msg: &str,
        error_prm: &str,
        fallback_prm: Option<PointId>,
    ) -> Result<(), EnedisError> {
        if error_code.is_empty() && error_msg.is_empty() {
            return Ok(());
        }

        let prm = PointId::new(error_prm).unwrap_or(fallback_prm.unwrap_or(PointId::DUMMY));
        let code_upper = error_code.to_uppercase();
        let msg_upper = error_msg.to_uppercase();

        if code_upper.contains("CONS")
            || msg_upper.contains("CONSENTEMENT")
            || msg_upper.contains("AUTORISATION")
        {
            Err(EnedisError::Business(
                SgeBusinessError::ConsentMissingOrExpired { point_id: prm },
            ))
        } else if code_upper.contains("PRM_INCONNU")
            || code_upper.contains("NOT_FOUND")
            || msg_upper.contains("INTROUVABLE")
        {
            Err(EnedisError::Business(SgeBusinessError::PointNotFound {
                point_id: prm,
            }))
        } else if code_upper.contains("QUOTA") || msg_upper.contains("QUOTA") {
            Err(EnedisError::Business(SgeBusinessError::QuotaExceeded {
                reset_at: None,
            }))
        } else if code_upper.contains("PERIODE") || msg_upper.contains("PLAGE") {
            Err(EnedisError::Business(SgeBusinessError::InvalidDateRange {
                point_id: prm,
                reason: error_msg.to_string(),
            }))
        } else {
            Err(EnedisError::Business(SgeBusinessError::Generic {
                code: error_code.to_string(),
                message: error_msg.to_string(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sge_measurements_success() {
        let sample_xml = br#"
        <consultationMesuresResponse>
            <prm>12345678901234</prm>
            <sens>Consommation</sens>
            <pas>1800</pas>
            <courbeDeCharge>
                <mesure>
                    <timestamp>2026-09-28T10:00:00Z</timestamp>
                    <valeur>1.4250</valeur>
                    <unite>kWh</unite>
                    <qualite>MESURE</qualite>
                </mesure>
                <mesure>
                    <timestamp>2026-09-28T10:30:00Z</timestamp>
                    <valeur>0.8900</valeur>
                    <unite>kWh</unite>
                    <qualite>ESTIME</qualite>
                </mesure>
            </courbeDeCharge>
        </consultationMesuresResponse>"#;

        let res = SgeResponseParser::parse_measurements(sample_xml);
        assert!(res.is_ok());
        let measurements = res.unwrap();
        assert_eq!(measurements.len(), 2);

        let m1 = &measurements[0];
        assert_eq!(m1.point_id.as_str(), "12345678901234");
        assert_eq!(m1.value, Decimal::from_str("1.4250").unwrap());
        assert_eq!(m1.quality, MeasurementQuality::Validated);
        assert_eq!(m1.interval_seconds, 1800);

        let m2 = &measurements[1];
        assert_eq!(m2.value, Decimal::from_str("0.8900").unwrap());
        assert_eq!(m2.quality, MeasurementQuality::Estimated);
    }

    #[test]
    fn test_parse_sge_consent_error() {
        let error_xml = br#"
        <consultationMesuresResponse>
            <erreurFonctionnelle>
                <prm>12345678901234</prm>
                <codeErreur>SGE_CONS_01</codeErreur>
                <libelleErreur>Consentement client absent ou expire</libelleErreur>
            </erreurFonctionnelle>
        </consultationMesuresResponse>"#;

        let res = SgeResponseParser::parse_measurements(error_xml);
        assert!(res.is_err());
        match res.unwrap_err() {
            EnedisError::Business(SgeBusinessError::ConsentMissingOrExpired { point_id }) => {
                assert_eq!(point_id.as_str(), "12345678901234");
            }
            other => panic!("Erreur inattendue: {:?}", other),
        }
    }

    #[test]
    fn test_parse_contract_data_success() {
        let sample_xml = br#"
        <consulterDonneesContractuellesResponse xmlns="http://www.enedis.fr/sge/ws/v1">
            <pointDeLivraison>01234567890123</pointDeLivraison>
            <donneesContractuelles>
                <puissanceSouscrite>9</puissanceSouscrite>
                <optionTarifaire>HEURES_PLEINES_HEURES_CREUSES</optionTarifaire>
                <statutContrat>ACTIF</statutContrat>
                <calendrier>
                    <libelle>Tarif Bleu Option Creuse</libelle>
                    <plagesHeuresCreuses>22h00-06h00</plagesHeuresCreuses>
                    <plagesHeuresCreuses>12h30-14h30</plagesHeuresCreuses>
                </calendrier>
                <caracteristiquesCompteur>
                    <matricule>022176001234</matricule>
                    <typeCompteur>LINKY</typeCompteur>
                    <nombrePhases>MONOPHASE</nombrePhases>
                    <calibreDisjoncteur>45</calibreDisjoncteur>
                </caracteristiquesCompteur>
            </donneesContractuelles>
        </consulterDonneesContractuellesResponse>"#;

        let contract = SgeResponseParser::parse_contract_data(sample_xml).unwrap();
        assert_eq!(contract.point_id.as_str(), "01234567890123");
        assert_eq!(contract.subscribed_power_kva, 9);
        assert_eq!(contract.tariff_option, TariffOption::HeuresPleinesCreuses);
        assert_eq!(contract.status.as_deref(), Some("ACTIF"));

        let cal = contract.calendar.unwrap();
        assert_eq!(
            cal.schedule_name.as_deref(),
            Some("Tarif Bleu Option Creuse")
        );
        assert_eq!(cal.off_peak_ranges.len(), 2);
        assert_eq!(cal.off_peak_ranges[0], "22h00-06h00");

        let meter = contract.meter.unwrap();
        assert_eq!(meter.serial_number.as_deref(), Some("022176001234"));
        assert_eq!(meter.meter_type, MeterType::Linky);
        assert_eq!(meter.phase_count, PhaseCount::SinglePhase);
        assert_eq!(meter.circuit_breaker_amperes, Some(45));
    }

    #[test]
    fn test_parse_max_power_success() {
        let sample_xml = br#"
        <consulterPuissanceMaxResponse xmlns="http://www.enedis.fr/sge/ws/v1">
            <pointDeLivraison>01234567890123</pointDeLivraison>
            <pointesPuissance>
                <pointe>
                    <date>2026-09-28T19:30:00Z</date>
                    <valeur>4820</valeur>
                    <unite>W</unite>
                </pointe>
                <pointe>
                    <date>2026-09-29T20:15:00Z</date>
                    <valeur>5200</valeur>
                    <unite>VA</unite>
                </pointe>
            </pointesPuissance>
        </consulterPuissanceMaxResponse>"#;

        let records = SgeResponseParser::parse_max_power(sample_xml).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].point_id.as_str(), "01234567890123");
        assert_eq!(records[0].value, Decimal::from(4820));
        assert_eq!(records[0].unit, Unit::Watt);
        assert_eq!(records[0].value_kva(), Decimal::from_str("4.82").unwrap());

        assert_eq!(records[1].value, Decimal::from(5200));
        assert_eq!(records[1].unit, Unit::VoltAmpere);
    }

    #[test]
    fn test_parse_consent_status_success() {
        let sample_xml = br#"
        <consulterConsentementResponse xmlns="http://www.enedis.fr/sge/ws/v1">
            <pointDeLivraison>01234567890123</pointDeLivraison>
            <consentement>
                <etat>ACTIF</etat>
                <dateDebut>2026-01-01T00:00:00Z</dateDebut>
                <dateFin>2027-01-01T00:00:00Z</dateFin>
                <usage>COURBE_DE_CHARGE</usage>
                <usage>PUISSANCE_MAX</usage>
            </consentement>
        </consulterConsentementResponse>"#;

        let consent = SgeResponseParser::parse_consent_status(sample_xml).unwrap();
        assert_eq!(consent.point_id.as_str(), "01234567890123");
        assert_eq!(consent.status, ConsentStatus::Active);
        assert!(consent.valid_from.is_some());
        assert!(consent.valid_to.is_some());
        assert_eq!(consent.authorized_usages.len(), 2);
    }

    #[test]
    fn test_parse_naive_datetime_french_local_time() {
        // En juillet (heure d'été UTC+2), 14:00:00 local correspond à 12:00:00 UTC
        let sample_xml = br#"
        <consulterCourbeDeChargeResponse xmlns="http://www.enedis.fr/sge/ws/v1">
            <prm>01234567890123</prm>
            <sens>Consommation</sens>
            <donnees>
                <mesure>
                    <timestamp>2026-07-15 14:00:00</timestamp>
                    <valeur>1.5000</valeur>
                    <unite>kWh</unite>
                </mesure>
            </donnees>
        </consulterCourbeDeChargeResponse>"#;

        let measurements = SgeResponseParser::parse_measurements(sample_xml).unwrap();
        assert_eq!(measurements.len(), 1);
        use chrono::TimeZone;
        assert_eq!(
            measurements[0].timestamp,
            Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap()
        );
    }
}
