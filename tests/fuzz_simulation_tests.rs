use enedis_rs::xml::{
    parse_soap_response, validate_xml_security, SgeResponseParser, XmlSecurityLimits,
};

/// Test de robustesse (fuzzing simulé) vérifiant qu'aucun payload hostile ou corrompu ne provoque de panic
#[test]
fn test_fuzz_xml_parser_no_panics() {
    let limits = XmlSecurityLimits {
        max_depth: 32,
        max_size_bytes: 64 * 1024,
    };

    // 1. Corpus de départ (graines de fuzzing)
    let seeds: Vec<&[u8]> = vec![
        b"",
        b"<",
        b">",
        b"</",
        b"<?xml",
        b"<!DOCTYPE",
        b"<soap:Envelope><soap:Body></soap:Body></soap:Envelope>",
        b"<soapenv:Fault><faultcode>Client</faultcode><faultstring>Err</faultstring></soapenv:Fault>",
        b"<mesure><valeur>NaN</valeur><timestamp>invalid</timestamp></mesure>",
        b"\x00\x01\x02\xFF\xFE\xFD",
        b"<prm>01234567890123</prm><valeur>99999999999999999999999999999999999999999999</valeur>",
    ];

    // 2. Générateur déterministe pseudo-aléatoire (Xorshift32)
    let mut rng_state: u32 = 0x12345678;
    let mut rand_u32 = || {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 17;
        rng_state ^= rng_state << 5;
        rng_state
    };

    // 3. Exécution de 2 500 mutations aléatoires
    for i in 0..2500 {
        let seed = seeds[i % seeds.len()];
        let mut mutated = seed.to_vec();

        let mutation_count = (rand_u32() % 8) as usize;
        for _ in 0..mutation_count {
            let mutation_type = rand_u32() % 5;
            match mutation_type {
                0 => {
                    // Insertion d'un octet aléatoire
                    let pos = if mutated.is_empty() {
                        0
                    } else {
                        (rand_u32() as usize) % mutated.len()
                    };
                    mutated.insert(pos, (rand_u32() % 256) as u8);
                }
                1 => {
                    // Suppression d'un octet
                    if !mutated.is_empty() {
                        let pos = (rand_u32() as usize) % mutated.len();
                        mutated.remove(pos);
                    }
                }
                2 => {
                    // Remplacement d'un octet
                    if !mutated.is_empty() {
                        let pos = (rand_u32() as usize) % mutated.len();
                        mutated[pos] = (rand_u32() % 256) as u8;
                    }
                }
                3 => {
                    // Injection de fragments XML spécifiques
                    let fragments: &[&[u8]] = &[
                        b"<!ENTITY xxe SYSTEM \"file:///\">",
                        b"<soap:Body>",
                        b"</soap:Body>",
                        b"&#x10000;",
                        b"<nested><nested><nested>",
                    ];
                    let frag = fragments[(rand_u32() as usize) % fragments.len()];
                    let pos = if mutated.is_empty() {
                        0
                    } else {
                        (rand_u32() as usize) % mutated.len()
                    };
                    mutated.splice(pos..pos, frag.iter().copied());
                }
                _ => {
                    // Tronquage
                    if mutated.len() > 2 {
                        let new_len = (rand_u32() as usize) % mutated.len();
                        mutated.truncate(new_len);
                    }
                }
            }
        }

        // Le test s'assure qu'absolument AUCUNE panic ne survient
        let _ = validate_xml_security(&mutated, &limits);
        let _ = parse_soap_response(&mutated);
        let _ = SgeResponseParser::parse_measurements(&mutated);
    }
}
