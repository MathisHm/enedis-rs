use crate::error::{EnedisError, SoapFault};
use quick_xml::events::Event;
use quick_xml::Reader;

pub const SOAP_ENV_1_1: &str = "http://schemas.xmlsoap.org/soap/envelope/";
pub const SOAP_ENV_1_2: &str = "http://www.w3.org/2003/05/soap-envelope";

/// Construit une enveloppe SOAP 1.1 conforme avec le payload métier dans le Body
pub fn build_soap_envelope(body_payload: &str, header_payload: Option<&str>) -> String {
    let mut out = String::with_capacity(body_payload.len() + 256);
    out.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    out.push_str(r#"<soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">"#);

    if let Some(header) = header_payload {
        out.push_str("<soapenv:Header>");
        out.push_str(header);
        out.push_str("</soapenv:Header>");
    } else {
        out.push_str("<soapenv:Header/>");
    }

    out.push_str("<soapenv:Body>");
    out.push_str(body_payload);
    out.push_str("</soapenv:Body>");
    out.push_str("</soapenv:Envelope>");
    out
}

/// Extrait le contenu du `<soapenv:Body>` et vérifie la présence d'un éventuel `<soapenv:Fault>`
pub fn parse_soap_response(xml_data: &[u8]) -> Result<String, EnedisError> {
    let mut reader = Reader::from_reader(xml_data);
    let mut buf = Vec::with_capacity(1024);

    let mut in_fault = false;
    let mut depth: usize = 0;
    let mut soap_body_depth: Option<usize> = None;
    let mut body_start_pos: Option<usize> = None;
    let mut body_end_pos: Option<usize> = None;

    let mut fault_code = String::new();
    let mut fault_string = String::new();
    let mut fault_detail = String::new();
    let mut current_tag = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                depth += 1;
                let local_name = e.local_name();
                let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                current_tag = name_str.to_string();

                if name_str.eq_ignore_ascii_case("Body") && soap_body_depth.is_none() {
                    soap_body_depth = Some(depth);
                    body_start_pos = Some(reader.buffer_position() as usize);
                } else if name_str.eq_ignore_ascii_case("Fault") {
                    in_fault = true;
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local_name = e.local_name();
                let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                if name_str.eq_ignore_ascii_case("Body") && soap_body_depth.is_none() {
                    return Ok(String::new());
                }
            }
            Ok(Event::Text(ref e)) => {
                if in_fault {
                    let text = e.unescape().unwrap_or_default().trim().to_string();
                    if !text.is_empty() {
                        match current_tag.to_lowercase().as_str() {
                            "faultcode" | "code" | "value" if fault_code.is_empty() => {
                                fault_code = text;
                            }
                            "faultstring" | "reason" | "text" if fault_string.is_empty() => {
                                fault_string = text;
                            }
                            "detail" | "message" if fault_detail.is_empty() => {
                                fault_detail = text;
                            }
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                let local_name = e.local_name();
                let name_str = std::str::from_utf8(local_name.as_ref()).unwrap_or("");
                if soap_body_depth == Some(depth) {
                    // Position exacte juste avant l'ouverture de la balise fermante </...Body>
                    let current_pos = reader.buffer_position() as usize;
                    let end_pos = xml_data[..current_pos.min(xml_data.len())]
                        .windows(2)
                        .rposition(|w| w == b"</")
                        .unwrap_or_else(|| current_pos.saturating_sub(e.name().as_ref().len() + 3));
                    body_end_pos = Some(end_pos);
                    soap_body_depth = None;
                } else if name_str.eq_ignore_ascii_case("Fault") {
                    in_fault = false;
                }
                depth = depth.saturating_sub(1);
                current_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(EnedisError::Xml(format!("Erreur XML SOAP: {}", e))),
            _ => {}
        }
        buf.clear();
    }

    // Si une faute SOAP a été détectée, la propager sous forme d'erreur structurée
    if !fault_code.is_empty() || !fault_string.is_empty() {
        return Err(EnedisError::Soap(SoapFault {
            code: if fault_code.is_empty() {
                "SOAP-ENV:Server".to_string()
            } else {
                fault_code
            },
            message: if fault_string.is_empty() {
                "Erreur SOAP indéterminée".to_string()
            } else {
                fault_string
            },
            subcode: None,
            detail: if fault_detail.is_empty() {
                None
            } else {
                Some(fault_detail)
            },
        }));
    }

    // Récupérer la portion brute du Body pour le parseur métier
    match (body_start_pos, body_end_pos) {
        (Some(start), Some(end)) if start <= end && end <= xml_data.len() => {
            let slice = &xml_data[start..end];
            let body_str = String::from_utf8_lossy(slice).trim().to_string();
            Ok(body_str)
        }
        _ => {
            // Repli : si les positions n'ont pas pu être délimitées, conversion directe
            Ok(String::from_utf8_lossy(xml_data).into_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_soap_envelope() {
        let payload = "<consultationPoint><id>12345678901234</id></consultationPoint>";
        let env = build_soap_envelope(payload, None);
        assert!(env.contains("<soapenv:Envelope"));
        assert!(env.contains(payload));
        assert!(env.contains("<soapenv:Header/>"));
    }

    #[test]
    fn test_parse_soap_fault() {
        let fault_xml = br#"<?xml version="1.0" encoding="UTF-8"?>
        <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
            <soapenv:Body>
                <soapenv:Fault>
                    <faultcode>soapenv:Client</faultcode>
                    <faultstring>PRM inexistant ou non valide</faultstring>
                    <detail>Code erreur Enedis SGE_0042</detail>
                </soapenv:Fault>
            </soapenv:Body>
        </soapenv:Envelope>"#;

        let res = parse_soap_response(fault_xml);
        assert!(res.is_err());
        match res.unwrap_err() {
            EnedisError::Soap(fault) => {
                assert_eq!(fault.code, "soapenv:Client");
                assert_eq!(fault.message, "PRM inexistant ou non valide");
                assert_eq!(
                    fault.detail,
                    Some("Code erreur Enedis SGE_0042".to_string())
                );
            }
            other => panic!("Type d'erreur inattendu: {:?}", other),
        }
    }

    #[test]
    fn test_parse_soap_success_body() {
        let success_xml = br#"<?xml version="1.0" encoding="UTF-8"?>
        <soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/">
            <soapenv:Body>
                <mesuresResponse><point>OK</point></mesuresResponse>
            </soapenv:Body>
        </soapenv:Envelope>"#;

        let res = parse_soap_response(success_xml);
        assert!(res.is_ok());
        let body = res.unwrap();
        assert!(body.contains("<mesuresResponse><point>OK</point></mesuresResponse>"));
    }

    #[test]
    fn test_parse_soap_minified_unindented() {
        let raw = br#"<soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/"><soapenv:Body><consulterMesuresResponse xmlns="http://www.enedis.fr/sge/ws/v1"><prm>01234567890123</prm></consulterMesuresResponse></soapenv:Body></soapenv:Envelope>"#;
        let body = parse_soap_response(raw).expect("Doit extraire le body minifie");
        assert_eq!(
            body,
            r#"<consulterMesuresResponse xmlns="http://www.enedis.fr/sge/ws/v1"><prm>01234567890123</prm></consulterMesuresResponse>"#
        );
    }

    #[test]
    fn test_parse_soap_empty_body() {
        let raw = br#"<soapenv:Envelope xmlns:soapenv="http://schemas.xmlsoap.org/soap/envelope/"><soapenv:Header/><soapenv:Body/></soapenv:Envelope>"#;
        let body = parse_soap_response(raw).expect("Doit extraire un body vide sans erreur");
        assert_eq!(body, "");
    }
}
