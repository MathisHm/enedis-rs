#![cfg(feature = "wasm")]

use chrono::{Duration, TimeZone, Utc};
use enedis_rs::models::{FlowDirection, Measurement, MeasurementQuality, PointId, Unit};
use enedis_rs::wasm::*;
use rust_decimal::Decimal;

fn make_measurement_json(prm: PointId, days: i64) -> String {
    let start = Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap();
    let mut list = Vec::new();

    for d in 0..days {
        let day_ts = start + Duration::days(d);
        for half_hour in 0..48 {
            let ts = day_ts + Duration::minutes(half_hour * 30);
            let hour = half_hour / 2;
            let val = if (1..=4).contains(&hour) {
                180 // nuit / veille
            } else if (12..=14).contains(&hour) {
                2500 // cuisson / pic midi
            } else {
                450
            };

            list.push(Measurement {
                point_id: prm,
                timestamp: ts,
                interval_seconds: 1800,
                direction: FlowDirection::Consumption,
                unit: Unit::WattHour,
                value: Decimal::from(val / 2), // 1800s Wh
                quality: MeasurementQuality::Validated,
            });
        }
    }

    serde_json::to_string(&list).unwrap()
}

#[test]
fn test_wasm_audit_subscription() {
    let prm = PointId::new("01234567890123").unwrap();
    let json_data = make_measurement_json(prm, 7);

    // Abonnement 12 kVA pour un foyer avec pointe à 2.5 kW -> surdimensionné
    let res = wasm_audit_subscription(&json_data, prm.as_str(), 12).expect("Audit Wasm réussi");
    assert!(
        res.contains("oversized") || res.contains("Oversized") || res.contains("surdimensionné")
    );
    assert!(res.contains("sizing_status"));
    assert!(res.contains("recommended_power_kva"));
}
