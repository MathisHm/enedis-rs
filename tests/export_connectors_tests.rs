#![cfg(all(feature = "storage", feature = "parquet"))]

use chrono::{TimeZone, Utc};
use enedis_rs::models::{FlowDirection, Measurement, MeasurementQuality, PointId, Unit};
use enedis_rs::storage::duckdb::DuckDbHelper;
use enedis_rs::storage::influxdb::{
    measurements_to_line_protocol, InfluxDbConfig, InfluxDbExporter,
};
use rust_decimal::Decimal;
use std::str::FromStr;

#[test]
fn test_influxdb_line_protocol_export() {
    let point_id = PointId::new("01234567890123").unwrap();
    let ts = Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap();

    let m1 = Measurement {
        point_id,
        timestamp: ts,
        interval_seconds: 1800,
        direction: FlowDirection::Consumption,
        value: Decimal::from_str("2.5").unwrap(),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Validated,
    };
    let m2 = Measurement {
        point_id,
        timestamp: ts + chrono::Duration::minutes(30),
        interval_seconds: 1800,
        direction: FlowDirection::Production,
        value: Decimal::from_str("0.75").unwrap(),
        unit: Unit::KiloWattHour,
        quality: MeasurementQuality::Estimated,
    };

    let lines = measurements_to_line_protocol(&[m1.clone(), m2.clone()], Some("enedis_power"));
    let split_lines: Vec<&str> = lines.trim().lines().collect();
    assert_eq!(split_lines.len(), 2);

    assert!(split_lines[0].starts_with(
        "enedis_power,prm=01234567890123,direction=CONSUMPTION,unit=kWh,quality=VALIDATED"
    ));
    assert!(split_lines[0].contains("value=2.5"));
    assert!(split_lines[0].contains("energy_kwh=2.5"));

    assert!(split_lines[1].starts_with(
        "enedis_power,prm=01234567890123,direction=PRODUCTION,unit=kWh,quality=ESTIMATED"
    ));
    assert!(split_lines[1].contains("value=0.75"));

    // Test export to file
    let tmp_dir = std::env::temp_dir();
    let file_path = tmp_dir.join("enedis_test.lp");
    let exporter =
        InfluxDbExporter::new(InfluxDbConfig::default().with_measurement_name("enedis_power"));
    exporter.export_to_file(&file_path, &[m1, m2]).unwrap();

    let content = std::fs::read_to_string(&file_path).unwrap();
    assert_eq!(content, lines);
    let _ = std::fs::remove_file(file_path);
}

#[cfg(feature = "parquet")]
#[test]
fn test_parquet_export_and_readback() {
    use enedis_rs::storage::parquet::ParquetExporter;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use std::fs::File;

    let point_id = PointId::new("01234567890123").unwrap();
    let ts = Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap();

    let measurements: Vec<Measurement> = (0..10)
        .map(|i| Measurement {
            point_id,
            timestamp: ts + chrono::Duration::minutes(i * 30),
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str(&format!("{}.5", i)).unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        })
        .collect();

    let tmp_dir = std::env::temp_dir();
    let parquet_path = tmp_dir.join("test_enedis_export.parquet");

    let count = ParquetExporter::export_to_file(&parquet_path, &measurements).unwrap();
    assert_eq!(count, 10);

    // Vérifier l'en-tête binaire Magic 'PAR1'
    let bytes = std::fs::read(&parquet_path).unwrap();
    assert!(bytes.len() > 12);
    assert_eq!(&bytes[0..4], b"PAR1");
    assert_eq!(&bytes[bytes.len() - 4..], b"PAR1");

    // Relire le fichier Parquet avec le lecteur Arrow
    let file = File::open(&parquet_path).unwrap();
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();
    let mut reader = builder.build().unwrap();

    let batch = reader.next().unwrap().unwrap();
    assert_eq!(batch.num_rows(), 10);
    assert_eq!(batch.num_columns(), 9);

    let _ = std::fs::remove_file(parquet_path);
}

#[test]
fn test_duckdb_script_generation() {
    let tmp_parquet = std::path::PathBuf::from("/var/data/enedis_history.parquet");
    let script = DuckDbHelper::generate_parquet_view_script(&tmp_parquet, Some("vue_enedis"));

    assert!(script.contains("CREATE OR REPLACE VIEW vue_enedis AS"));
    assert!(script.contains("read_parquet('/var/data/enedis_history.parquet')"));
    assert!(script.contains("energy_kwh"));
    assert!(script.contains("power_w"));

    let sqlite_script =
        DuckDbHelper::generate_sqlite_attach_script(&std::path::PathBuf::from("local.db"));
    assert!(sqlite_script.contains("INSTALL sqlite;"));
    assert!(sqlite_script.contains("ATTACH 'local.db' AS enedis_db (TYPE SQLITE);"));
}
