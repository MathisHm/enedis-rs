use crate::error::EnedisError;
use std::fs::File;
use std::io::Write;
use std::path::Path;

fn escape_sql_literal(s: &str) -> String {
    s.replace('\'', "''")
}

fn sanitize_sql_identifier(s: &str) -> String {
    let sanitized: String = s
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if sanitized.is_empty() {
        "enedis_analytics".to_string()
    } else {
        sanitized
    }
}

/// Outils d'intégration et d'analyse haute performance avec DuckDB
pub struct DuckDbHelper;

impl DuckDbHelper {
    /// Génère un script SQL DuckDB créant des vues analytiques optimisées au-dessus d'un fichier Parquet
    pub fn generate_parquet_view_script(parquet_path: &Path, view_name: Option<&str>) -> String {
        let name = sanitize_sql_identifier(view_name.unwrap_or("enedis_analytics"));
        let path_str = escape_sql_literal(&parquet_path.to_string_lossy());

        format!(
            r#"-- DuckDB Analytics Script pour Enedis
-- Généré automatiquement par enedis-rs

CREATE OR REPLACE VIEW {name} AS 
SELECT 
    point_id,
    timestamp,
    direction,
    interval_seconds,
    value,
    unit,
    quality,
    energy_kwh,
    power_w,
    date_trunc('hour', timestamp) AS hour_bucket,
    date_trunc('day', timestamp) AS day_bucket,
    date_trunc('month', timestamp) AS month_bucket,
    date_trunc('year', timestamp) AS year_bucket
FROM read_parquet('{path_str}');

-- Exemple 1 : Consommation totale par mois et par PRM
-- SELECT point_id, month_bucket, SUM(energy_kwh) as total_kwh, MAX(power_w) as peak_power_w 
-- FROM {name} WHERE direction = 'CONSUMPTION' GROUP BY point_id, month_bucket ORDER BY month_bucket DESC;

-- Exemple 2 : Profil journalier moyen (heure par heure)
-- SELECT strftime(timestamp, '%H:00') as heure, AVG(power_w) as avg_power_w 
-- FROM {name} GROUP BY heure ORDER BY heure;
"#,
            name = name,
            path_str = path_str
        )
    }

    /// Génère un script DuckDB pour analyser directement une base SQLite en mode vectorisé
    pub fn generate_sqlite_attach_script(sqlite_path: &Path) -> String {
        let path_str = escape_sql_literal(&sqlite_path.to_string_lossy());
        format!(
            r#"-- DuckDB SQLite Direct Query
INSTALL sqlite;
LOAD sqlite;
ATTACH '{path_str}' AS enedis_db (TYPE SQLITE);

-- Analyse vectorisée directe des mesures SQLite :
SELECT 
    point_id,
    direction,
    strftime('%Y-%m', timestamp) as mois,
    SUM(CAST(value AS DOUBLE)) as total_raw,
    COUNT(*) as total_mesures
FROM enedis_db.measurements
GROUP BY point_id, direction, mois
ORDER BY mois DESC;
"#,
            path_str = path_str
        )
    }

    /// Écrit un script DuckDB prêt à l'exécution dans un fichier .sql
    pub fn write_script_file<P: AsRef<Path>>(
        output_script_path: P,
        content: &str,
    ) -> Result<(), EnedisError> {
        let mut file = File::create(output_script_path.as_ref()).map_err(|e| {
            EnedisError::Configuration(format!(
                "Impossible de créer le script DuckDB '{:?}': {}",
                output_script_path.as_ref(),
                e
            ))
        })?;

        file.write_all(content.as_bytes()).map_err(|e| {
            EnedisError::Configuration(format!(
                "Erreur d'écriture du script DuckDB '{:?}': {}",
                output_script_path.as_ref(),
                e
            ))
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_duckdb_script_generation() {
        let script = DuckDbHelper::generate_parquet_view_script(
            &PathBuf::from("/data/enedis.parquet"),
            None,
        );
        assert!(script.contains("CREATE OR REPLACE VIEW enedis_analytics"));
        assert!(script.contains("read_parquet('/data/enedis.parquet')"));
    }
}
