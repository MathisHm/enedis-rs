use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

type MetricMap = Arc<RwLock<HashMap<(String, String), Arc<AtomicU64>>>>;

/// Registre de métriques Prometheus léger et sans dépendance externe
#[derive(Clone, Default)]
pub struct MetricsRegistry {
    requests_total: MetricMap,
    sync_errors_total: MetricMap,
    last_sync_timestamp: MetricMap,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Incrémente le compteur de requêtes SGE effectuées
    pub fn inc_requests(&self, status: &str, direction: &str) {
        let key = (status.to_string(), direction.to_string());
        let counter = {
            let mut map = self.requests_total.write().unwrap();
            map.entry(key)
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Incrémente le compteur d'erreurs de synchronisation par code et PRM
    pub fn inc_errors(&self, code: &str, point_id: &str) {
        let key = (code.to_string(), point_id.to_string());
        let counter = {
            let mut map = self.sync_errors_total.write().unwrap();
            map.entry(key)
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Met à jour l'horodatage de la dernière synchronisation réussie (epoch en secondes)
    pub fn set_last_sync_timestamp(&self, point_id: &str, direction: &str, timestamp_secs: u64) {
        let key = (point_id.to_string(), direction.to_string());
        let gauge = {
            let mut map = self.last_sync_timestamp.write().unwrap();
            map.entry(key)
                .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                .clone()
        };
        gauge.store(timestamp_secs, Ordering::Relaxed);
    }

    /// Exporte toutes les métriques au format texte standard Prometheus
    pub fn render_prometheus(&self) -> String {
        let mut out = String::with_capacity(2048);
        let now_secs = chrono::Utc::now().timestamp() as u64;

        // 1. Requests total counter
        out.push_str("# HELP enedis_requests_total Nombre total de requetes SGE effectuees\n");
        out.push_str("# TYPE enedis_requests_total counter\n");
        {
            let map = self.requests_total.read().unwrap();
            for ((status, direction), count) in map.iter() {
                out.push_str(&format!(
                    "enedis_requests_total{{status=\"{}\",direction=\"{}\"}} {}\n",
                    status,
                    direction,
                    count.load(Ordering::Relaxed)
                ));
            }
        }

        // 2. Sync errors counter
        out.push_str("\n# HELP enedis_sync_errors_total Nombre d'erreurs rencontrees lors des collectes SGE\n");
        out.push_str("# TYPE enedis_sync_errors_total counter\n");
        {
            let map = self.sync_errors_total.read().unwrap();
            for ((code, point_id), count) in map.iter() {
                out.push_str(&format!(
                    "enedis_sync_errors_total{{code=\"{}\",point_id=\"{}\"}} {}\n",
                    code,
                    point_id,
                    count.load(Ordering::Relaxed)
                ));
            }
        }

        // 3. Last sync timestamp gauge
        out.push_str("\n# HELP enedis_collector_last_sync_timestamp Dernier horodatage synchronise (epoch secondes)\n");
        out.push_str("# TYPE enedis_collector_last_sync_timestamp gauge\n");
        {
            let map = self.last_sync_timestamp.read().unwrap();
            for ((point_id, direction), ts) in map.iter() {
                let val = ts.load(Ordering::Relaxed);
                out.push_str(&format!(
                    "enedis_collector_last_sync_timestamp{{point_id=\"{}\",direction=\"{}\"}} {}\n",
                    point_id, direction, val
                ));
            }
        }

        // 4. Data freshness gauge (now - last_sync_timestamp)
        out.push_str("\n# HELP enedis_collector_data_freshness_seconds Fraicheur de la donnee en secondes (age du dernier point)\n");
        out.push_str("# TYPE enedis_collector_data_freshness_seconds gauge\n");
        {
            let map = self.last_sync_timestamp.read().unwrap();
            for ((point_id, direction), ts) in map.iter() {
                let val = ts.load(Ordering::Relaxed);
                let age = now_secs.saturating_sub(val);
                out.push_str(&format!(
                    "enedis_collector_data_freshness_seconds{{point_id=\"{}\",direction=\"{}\"}} {}\n",
                    point_id, direction, age
                ));
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_prometheus() {
        let reg = MetricsRegistry::new();
        reg.inc_requests("success", "CONSUMPTION");
        reg.inc_requests("success", "CONSUMPTION");
        reg.inc_errors("CONSENT_EXPIRED", "01234567890123");
        reg.set_last_sync_timestamp("01234567890123", "CONSUMPTION", 1700000000);

        let output = reg.render_prometheus();
        assert!(
            output.contains(r#"enedis_requests_total{status="success",direction="CONSUMPTION"} 2"#)
        );
        assert!(output.contains(
            r#"enedis_sync_errors_total{code="CONSENT_EXPIRED",point_id="01234567890123"} 1"#
        ));
        assert!(output.contains(r#"enedis_collector_last_sync_timestamp{point_id="01234567890123",direction="CONSUMPTION"} 1700000000"#));
        assert!(output.contains(r#"enedis_collector_data_freshness_seconds{point_id="01234567890123",direction="CONSUMPTION"}"#));
    }
}
