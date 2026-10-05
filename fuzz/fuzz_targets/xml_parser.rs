#![no_main]

use enedis_rs::xml::{
    parse_soap_response, validate_xml_security, SgeResponseParser, XmlSecurityLimits,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let limits = XmlSecurityLimits::default();

    // 1. Fuzzing de la validation de sécurité
    let _ = validate_xml_security(data, &limits);

    // 2. Fuzzing de l'extraction SOAP
    let _ = parse_soap_response(data);

    // 3. Fuzzing du parseur métier SGE
    let _ = SgeResponseParser::parse_measurements(data);
});
