use enedis_rs::xml::{validate_xml_security, XmlSecurityLimits};
use enedis_rs::EnedisError;

#[test]
fn test_security_rejection_of_billion_laughs_and_dtd() {
    let malicious_xmls = [
        // DTD basique
        r#"<!DOCTYPE test [ <!ENTITY xxe "malicious"> ]><root>&xxe;</root>"#,
        // Billion laughs
        r#"<?xml version="1.0"?>
        <!DOCTYPE lolz [
         <!ENTITY lol "lol">
         <!ELEMENT lolz (#PCDATA)>
         <!ENTITY lol1 "&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;&lol;">
         <!ENTITY lol2 "&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;&lol1;">
        ]>
        <lolz>&lol2;</lolz>"#,
        // External entity (XXE)
        r#"<!DOCTYPE foo [ <!ENTITY xxe SYSTEM "file:///etc/passwd"> ]><foo>&xxe;</foo>"#,
    ];

    let limits = XmlSecurityLimits::default();
    for payload in malicious_xmls {
        let res = validate_xml_security(payload.as_bytes(), &limits);
        assert!(
            res.is_err(),
            "Le payload suivant aurait dû être rejeté: {}",
            payload
        );
        match res.unwrap_err() {
            EnedisError::XmlSecurity(msg) => {
                assert!(msg.contains("<!DOCTYPE>"));
            }
            err => panic!("Erreur inattendue: {:?}", err),
        }
    }
}

#[test]
fn test_security_rejection_of_excessive_payload_size() {
    let huge_payload = vec![b'a'; 1000];
    let limits = XmlSecurityLimits {
        max_depth: 32,
        max_size_bytes: 500, // Limite à 500 octets
    };

    let res = validate_xml_security(&huge_payload, &limits);
    assert!(res.is_err());
    match res.unwrap_err() {
        EnedisError::XmlSecurity(msg) => {
            assert!(msg.contains("trop volumineuse"));
        }
        err => panic!("Erreur inattendue: {:?}", err),
    }
}

#[test]
fn test_security_rejection_of_excessive_nesting_depth() {
    let mut nested = String::new();
    for _ in 0..50 {
        nested.push_str("<node>");
    }
    for _ in 0..50 {
        nested.push_str("</node>");
    }

    let limits = XmlSecurityLimits {
        max_depth: 10,
        max_size_bytes: 1024 * 1024,
    };

    let res = validate_xml_security(nested.as_bytes(), &limits);
    assert!(res.is_err());
    match res.unwrap_err() {
        EnedisError::XmlSecurity(msg) => {
            assert!(msg.contains("Profondeur maximale"));
        }
        err => panic!("Erreur inattendue: {:?}", err),
    }
}
