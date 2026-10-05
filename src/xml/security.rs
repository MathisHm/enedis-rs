use crate::error::EnedisError;
use quick_xml::events::Event;
use quick_xml::Reader;

/// Paramètres de sécurité pour le parseur XML
pub struct XmlSecurityLimits {
    /// Profondeur maximale d'imbrication d'éléments (protection stack overflow / Billion Laughs)
    pub max_depth: usize,
    /// Taille maximale totale du document XML en octets (protection OOM)
    pub max_size_bytes: usize,
}

impl Default for XmlSecurityLimits {
    fn default() -> Self {
        Self {
            max_depth: 64,
            max_size_bytes: 25 * 1024 * 1024, // 25 Mo max
        }
    }
}

/// Analyse et sécurise le document XML avant / pendant la lecture.
/// Rejette formellement les DTD, entités externes et dépassements de quota mémoire.
pub fn validate_xml_security(
    xml_data: &[u8],
    limits: &XmlSecurityLimits,
) -> Result<(), EnedisError> {
    if xml_data.len() > limits.max_size_bytes {
        return Err(EnedisError::XmlSecurity(format!(
            "Charge utile XML trop volumineuse: {} octets (limite: {})",
            xml_data.len(),
            limits.max_size_bytes
        )));
    }

    let mut reader = Reader::from_reader(xml_data);
    let mut depth: usize = 0;
    let mut buf = Vec::with_capacity(1024);

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::DocType(_)) => {
                // Interdiction des DTD : rempart absolu contre le Billion Laughs et SSRF XML
                return Err(EnedisError::XmlSecurity(
                    "Déclaration <!DOCTYPE> interdite (protection contre l'injection d'entités XML / Billion Laughs)".to_string(),
                ));
            }
            Ok(Event::Start(_)) => {
                depth += 1;
                if depth > limits.max_depth {
                    return Err(EnedisError::XmlSecurity(format!(
                        "Profondeur maximale XML dépassée ({}/{})",
                        depth, limits.max_depth
                    )));
                }
            }
            Ok(Event::End(_)) => {
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(EnedisError::Xml(format!("Erreur syntaxique XML: {}", e)));
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reject_doctype_billion_laughs() {
        let malicious_xml = br#"<?xml version="1.0"?>
        <!DOCTYPE lolz [
         <!ENTITY lol "lol">
         <!ENTITY lol1 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
        ]>
        <soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body>&lol1;</soap:Body>
        </soap:Envelope>"#;

        let limits = XmlSecurityLimits::default();
        let res = validate_xml_security(malicious_xml, &limits);
        assert!(res.is_err());
        assert!(matches!(res.unwrap_err(), EnedisError::XmlSecurity(_)));
    }

    #[test]
    fn test_reject_excessive_depth() {
        let deep_xml = format!("{}{}", "<root>".repeat(100), "</root>".repeat(100));

        let limits = XmlSecurityLimits {
            max_depth: 20,
            max_size_bytes: 1024 * 1024,
        };
        let res = validate_xml_security(deep_xml.as_bytes(), &limits);
        assert!(res.is_err());
    }

    #[test]
    fn test_valid_xml() {
        let safe_xml = br#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/">
            <soap:Body><data>OK</data></soap:Body>
        </soap:Envelope>"#;

        let limits = XmlSecurityLimits::default();
        assert!(validate_xml_security(safe_xml, &limits).is_ok());
    }
}
