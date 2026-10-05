use enedis_rs::xml::{build_soap_envelope, parse_soap_response};
use enedis_rs::EnedisError;

#[test]
fn test_soap_envelope_wrapping() {
    let payload = "<getConsumption><prm>01234567890123</prm></getConsumption>";
    let env = build_soap_envelope(payload, Some("<authHeader>TOKEN</authHeader>"));

    assert!(env.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?>"#));
    assert!(env.contains(
        r#"<soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">"#
    ));
    assert!(env.contains("<soapenv:Header><authHeader>TOKEN</authHeader></soapenv:Header>"));
    assert!(env.contains(
        "<soapenv:Body><getConsumption><prm>01234567890123</prm></getConsumption></soapenv:Body>"
    ));
}

#[test]
fn test_soap_envelope_extraction_nominal() {
    let raw_response = br#"<?xml version="1.0" encoding="utf-8"?>
    <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
        <soapenv:Header/>
        <soapenv:Body>
            <consulterDonneesResponse xmlns="http://www.enedis.fr/sge/ws/v1">
                <statut>SUCCES</statut>
                <donnees>VALEUR_TEST</donnees>
            </consulterDonneesResponse>
        </soapenv:Body>
    </soapenv:Envelope>"#;

    let body = parse_soap_response(raw_response).expect("Doit extraire le body SOAP nominal");
    assert!(body.contains("<consulterDonneesResponse"));
    assert!(body.contains("<statut>SUCCES</statut>"));
    assert!(body.contains("<donnees>VALEUR_TEST</donnees>"));
}

#[test]
fn test_soap_fault_detection() {
    let raw_fault = r#"<?xml version="1.0" encoding="utf-8"?>
    <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
        <soapenv:Body>
            <soapenv:Fault>
                <faultcode>soapenv:Server</faultcode>
                <faultstring>Erreur interne de traitement SGE</faultstring>
                <detail>
                    <codeErreur>ERR_SYS_500</codeErreur>
                    <message>La base de données SGE est temporairement indisponible</message>
                </detail>
            </soapenv:Fault>
        </soapenv:Body>
    </soapenv:Envelope>"#
        .as_bytes();

    let err = parse_soap_response(raw_fault).expect_err("Doit lever une erreur SoapFault");
    match err {
        EnedisError::Soap(fault) => {
            assert_eq!(fault.code, "soapenv:Server");
            assert_eq!(fault.message, "Erreur interne de traitement SGE");
            assert!(fault.detail.is_some());
        }
        other => panic!("Erreur inattendue: {:?}", other),
    }
}
