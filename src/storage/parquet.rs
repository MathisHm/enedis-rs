use crate::error::EnedisError;
use crate::models::Measurement;
use arrow::array::{Float64Array, StringArray, TimestampMicrosecondArray, UInt32Array};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use rust_decimal::prelude::ToPrimitive;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

/// Constructeur et exportateur de fichiers Apache Parquet pour l'analyse OLAP (DuckDB, ClickHouse, Polars)
pub struct ParquetExporter;

impl ParquetExporter {
    /// Définit le schéma Arrow normalisé pour les séries temporelles Enedis
    pub fn schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("point_id", DataType::Utf8, false),
            Field::new(
                "timestamp",
                DataType::Timestamp(TimeUnit::Microsecond, Some("+00:00".into())),
                false,
            ),
            Field::new("direction", DataType::Utf8, false),
            Field::new("interval_seconds", DataType::UInt32, false),
            Field::new("value", DataType::Float64, false),
            Field::new("unit", DataType::Utf8, false),
            Field::new("quality", DataType::Utf8, false),
            Field::new("energy_kwh", DataType::Float64, false),
            Field::new("power_w", DataType::Float64, false),
        ]))
    }

    /// Convertit une tranche de mesures en un `RecordBatch` Arrow Apache
    pub fn to_record_batch(measurements: &[Measurement]) -> Result<RecordBatch, EnedisError> {
        let schema = Self::schema();

        let point_ids: Vec<&str> = measurements.iter().map(|m| m.point_id.as_str()).collect();
        let timestamps: Vec<i64> = measurements
            .iter()
            .map(|m| m.timestamp.timestamp_micros())
            .collect();
        let directions: Vec<&str> = measurements.iter().map(|m| m.direction.as_str()).collect();
        let intervals: Vec<u32> = measurements.iter().map(|m| m.interval_seconds).collect();
        let values: Vec<f64> = measurements
            .iter()
            .map(|m| m.value.to_f64().unwrap_or(0.0))
            .collect();
        let units: Vec<&str> = measurements.iter().map(|m| m.unit.as_str()).collect();
        let qualities: Vec<String> = measurements.iter().map(|m| m.quality.to_string()).collect();
        let energy_kwhs: Vec<f64> = measurements
            .iter()
            .map(|m| m.energy_kwh().to_f64().unwrap_or(0.0))
            .collect();
        let power_ws: Vec<f64> = measurements
            .iter()
            .map(|m| m.power_w().to_f64().unwrap_or(0.0))
            .collect();

        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(point_ids)),
                Arc::new(TimestampMicrosecondArray::from(timestamps).with_timezone("+00:00")),
                Arc::new(StringArray::from(directions)),
                Arc::new(UInt32Array::from(intervals)),
                Arc::new(Float64Array::from(values)),
                Arc::new(StringArray::from(units)),
                Arc::new(StringArray::from_iter_values(
                    qualities.iter().map(|s| s.as_str()),
                )),
                Arc::new(Float64Array::from(energy_kwhs)),
                Arc::new(Float64Array::from(power_ws)),
            ],
        )
        .map_err(|e| {
            EnedisError::Configuration(format!("Erreur création RecordBatch Arrow: {}", e))
        })?;

        Ok(batch)
    }

    /// Exporte les mesures dans un fichier Parquet compressé (Snappy) avec indexation dictionnaire
    pub fn export_to_file<P: AsRef<Path>>(
        path: P,
        measurements: &[Measurement],
    ) -> Result<usize, EnedisError> {
        let file = File::create(path.as_ref()).map_err(|e| {
            EnedisError::Configuration(format!(
                "Impossible d'ouvrir le fichier Parquet '{:?}': {}",
                path.as_ref(),
                e
            ))
        })?;

        let batch = Self::to_record_batch(measurements)?;

        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .set_dictionary_enabled(true)
            .build();

        let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(props)).map_err(|e| {
            EnedisError::Configuration(format!("Erreur initialisation Parquet: {}", e))
        })?;

        writer
            .write(&batch)
            .map_err(|e| EnedisError::Configuration(format!("Erreur écriture Parquet: {}", e)))?;

        writer
            .close()
            .map_err(|e| EnedisError::Configuration(format!("Erreur clôture Parquet: {}", e)))?;

        Ok(measurements.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{FlowDirection, MeasurementQuality, PointId, Unit};
    use chrono::TimeZone;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    #[test]
    fn test_parquet_record_batch_generation() {
        let prm = PointId::new("01234567890123").unwrap();
        let ts = chrono::Utc
            .with_ymd_and_hms(2026, 9, 28, 14, 30, 0)
            .unwrap();
        let m = Measurement {
            point_id: prm,
            timestamp: ts,
            interval_seconds: 1800,
            direction: FlowDirection::Consumption,
            value: Decimal::from_str("1.5000").unwrap(),
            unit: Unit::KiloWattHour,
            quality: MeasurementQuality::Validated,
        };

        let batch = ParquetExporter::to_record_batch(&[m]).unwrap();
        assert_eq!(batch.num_rows(), 1);
        assert_eq!(batch.num_columns(), 9);
    }
}
