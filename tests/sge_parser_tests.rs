use chrono::{TimeZone, Utc};
use enedis_rs::xml::SgeResponseParser;
use enedis_rs::{EnedisError, FlowDirection, MeasurementQuality, PointId, SgeBusinessError, Unit};
use rust_decimal::Decimal;
use std::str::FromStr;

#[test]
fn test_quality_hierarchy_ordering() {
    assert!(MeasurementQuality::Estimated < MeasurementQuality::Corrected);
    assert!(MeasurementQuality::Corrected < MeasurementQuality::Validated);
    assert!(MeasurementQuality::Estimated < MeasurementQuality::Validated);

    // Egalité
    assert!(MeasurementQuality::Validated >= MeasurementQuality::Validated);
    assert!(MeasurementQuality::Validated >= MeasurementQuality::Corrected);
    assert!(MeasurementQuality::Corrected >= MeasurementQuality::Estimated);

    // Une donnée estimée ne doit pas écraser une validée
    assert!(!(MeasurementQuality::Estimated >= MeasurementQuality::Validated));
}

#[test]
fn test_parse_complete_load_curve() {
    let sge_xml = br#"
    <consulterCourbeDeChargeResponse xmlns="http://www.enedis.fr/sge/ws/v1">
        <prm>01234567890123</prm>
        <sens>Consommation</sens>
        <pas>1800</pas>
        <donnees>
            <mesure>
                <timestamp>2026-09-28T00:00:00Z</timestamp>
                <valeur>0.4500</valeur>
                <unite>kWh</unite>
                <qualite>ESTIME</qualite>
            </mesure>
            <mesure>
                <timestamp>2026-09-28T00:30:00Z</timestamp>
                <valeur>0.4850</valeur>
                <unite>kWh</unite>
                <qualite>REDRESSE</qualite>
            </mesure>
            <mesure>
                <timestamp>2026-09-28T01:00:00Z</timestamp>
                <valeur>0.5120</valeur>
                <unite>kWh</unite>
                <qualite>MESURE</qualite>
            </mesure>
        </donnees>
    </consulterCourbeDeChargeResponse>"#;

    let measurements = SgeResponseParser::parse_measurements(sge_xml)
        .expect("Doit parser la courbe de charge sans erreur");

    assert_eq!(measurements.len(), 3);

    let prm = PointId::new("01234567890123").unwrap();

    // 1ère mesure : Estimée
    assert_eq!(measurements[0].point_id, prm);
    assert_eq!(
        measurements[0].timestamp,
        Utc.with_ymd_and_hms(2026, 9, 28, 0, 0, 0).unwrap()
    );
    assert_eq!(measurements[0].interval_seconds, 1800);
    assert_eq!(measurements[0].direction, FlowDirection::Consumption);
    assert_eq!(measurements[0].value, Decimal::from_str("0.4500").unwrap());
    assert_eq!(measurements[0].unit, Unit::KiloWattHour);
    assert_eq!(measurements[0].quality, MeasurementQuality::Estimated);

    // 2ème mesure : Redressée
    assert_eq!(measurements[1].value, Decimal::from_str("0.4850").unwrap());
    assert_eq!(measurements[1].quality, MeasurementQuality::Corrected);

    // 3ème mesure : Validée
    assert_eq!(measurements[2].value, Decimal::from_str("0.5120").unwrap());
    assert_eq!(measurements[2].quality, MeasurementQuality::Validated);
}

#[test]
fn test_parse_functional_sge_error_consent_expired() {
    let error_xml = br#"
    <consulterMesuresResponse xmlns="http://www.enedis.fr/sge/ws/v1">
        <erreurFonctionnelle>
            <prm>01234567890123</prm>
            <codeErreur>SGE_ERR_CONSENTEMENT_EXPIRE</codeErreur>
            <libelleErreur>Le consentement de l'usager est echu</libelleErreur>
        </erreurFonctionnelle>
    </consulterMesuresResponse>"#;

    let res = SgeResponseParser::parse_measurements(error_xml);
    assert!(res.is_err());
    match res.unwrap_err() {
        EnedisError::Business(SgeBusinessError::ConsentMissingOrExpired { point_id }) => {
            assert_eq!(point_id.as_str(), "01234567890123");
        }
        other => panic!("Type d'erreur inattendu: {:?}", other),
    }
}

#[test]
fn test_parse_functional_sge_error_point_not_found() {
    let error_xml = br#"
    <consulterMesuresResponse xmlns="http://www.enedis.fr/sge/ws/v1">
        <erreurFonctionnelle>
            <prm>99999999999999</prm>
            <codeErreur>SGE_PRM_INTROUVABLE</codeErreur>
            <libelleErreur>Le point de livraison est introuvable</libelleErreur>
        </erreurFonctionnelle>
    </consulterMesuresResponse>"#;

    let res = SgeResponseParser::parse_measurements(error_xml);
    assert!(res.is_err());
    match res.unwrap_err() {
        EnedisError::Business(SgeBusinessError::PointNotFound { point_id }) => {
            assert_eq!(point_id.as_str(), "99999999999999");
        }
        other => panic!("Type d'erreur inattendu: {:?}", other),
    }
}

#[test]
fn test_parse_iso8601_duration_interval() {
    let sge_xml = br#"
    <consulterCourbeDeChargeResponse xmlns="http://www.enedis.fr/sge/ws/v1">
        <prm>01234567890123</prm>
        <sens>Consommation</sens>
        <pas>PT30M</pas>
        <donnees>
            <mesure>
                <timestamp>2026-09-28T00:00:00Z</timestamp>
                <valeur>1.5000</valeur>
                <unite>kWh</unite>
                <qualite>MESURE</qualite>
            </mesure>
        </donnees>
    </consulterCourbeDeChargeResponse>"#;

    let measurements = SgeResponseParser::parse_measurements(sge_xml)
        .expect("Doit parser la mesure avec pas PT30M");
    assert_eq!(measurements.len(), 1);
    assert_eq!(measurements[0].interval_seconds, 1800);
}
