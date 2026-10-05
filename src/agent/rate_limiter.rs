use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;
use tracing::debug;

/// Limiteur de débit (Rate Limiter) pragmatique pour protéger le quota IP Enedis SGE
#[derive(Clone)]
pub struct SgeRateLimiter {
    state: Arc<Mutex<RateLimiterState>>,
}

struct RateLimiterState {
    min_interval: Duration,
    last_request_time: Option<Instant>,
    max_requests_per_minute: u32,
    request_timestamps: Vec<Instant>,
}

impl SgeRateLimiter {
    /// Crée un nouveau limiteur de débit
    /// `max_per_minute`: nombre maximal d'appels autorisés sur une fenêtre de 60 secondes
    /// `min_interval`: temps minimal absolu entre deux requêtes consécutives
    pub fn new(max_per_minute: u32, min_interval: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimiterState {
                min_interval,
                last_request_time: None,
                max_requests_per_minute: max_per_minute,
                request_timestamps: Vec::with_capacity(max_per_minute as usize),
            })),
        }
    }

    /// Configuration par défaut respectant les préconisations Enedis SGE (ex: 20 req/min, min 1s d'intervalle)
    pub fn enedis_default() -> Self {
        Self::new(20, Duration::from_millis(1500))
    }

    /// Attend le temps nécessaire avant d'autoriser la requête suivante
    pub async fn acquire(&self) {
        loop {
            let sleep_duration = {
                let mut state = self.state.lock().await;
                let now = Instant::now();

                // 1. Nettoyage des requêtes antérieures à 60 secondes
                let one_minute_ago = now.checked_sub(Duration::from_secs(60)).unwrap_or(now);
                state.request_timestamps.retain(|&t| t > one_minute_ago);

                // 2. Vérification de l'intervalle minimal
                let interval_wait = if let Some(last) = state.last_request_time {
                    let elapsed = now.duration_since(last);
                    if elapsed < state.min_interval {
                        state.min_interval - elapsed
                    } else {
                        Duration::ZERO
                    }
                } else {
                    Duration::ZERO
                };

                // 3. Vérification du quota par minute
                let quota_wait =
                    if state.request_timestamps.len() >= state.max_requests_per_minute as usize {
                        let oldest = state.request_timestamps[0];
                        let elapsed = now.duration_since(oldest);
                        if elapsed < Duration::from_secs(60) {
                            Duration::from_secs(60) - elapsed
                        } else {
                            Duration::ZERO
                        }
                    } else {
                        Duration::ZERO
                    };

                let wait_time = interval_wait.max(quota_wait);

                if wait_time.is_zero() {
                    state.last_request_time = Some(now);
                    state.request_timestamps.push(now);
                    None
                } else {
                    Some(wait_time)
                }
            };

            if let Some(duration) = sleep_duration {
                debug!(
                    "Rate limit SGE : attente de {:?} avant la prochaine requête",
                    duration
                );
                tokio::time::sleep(duration).await;
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn test_rate_limiter_min_interval() {
        let limiter = SgeRateLimiter::new(100, Duration::from_millis(200));

        let t0 = Instant::now();
        limiter.acquire().await;
        let t1 = Instant::now();
        assert_eq!(t1.duration_since(t0), Duration::ZERO);

        limiter.acquire().await;
        let t2 = Instant::now();
        assert!(t2.duration_since(t1) >= Duration::from_millis(200));
    }

    #[tokio::test(start_paused = true)]
    async fn test_rate_limiter_quota_per_minute() {
        // Limiteur autorisant 2 requêtes max par minute
        let limiter = SgeRateLimiter::new(2, Duration::from_millis(10));

        let t0 = Instant::now();
        limiter.acquire().await;
        limiter.acquire().await;
        let elapsed_two = Instant::now().duration_since(t0);
        assert!(elapsed_two >= Duration::from_millis(10));
        assert!(elapsed_two < Duration::from_secs(1));

        // La 3ème requête doit attendre que la 1ère ait plus de 60 secondes
        limiter.acquire().await;
        let elapsed_total = Instant::now().duration_since(t0);
        assert!(elapsed_total >= Duration::from_secs(60));
    }

    #[test]
    fn test_enedis_default_construction() {
        let limiter = SgeRateLimiter::enedis_default();
        let state = limiter.state.try_lock().unwrap();
        assert_eq!(state.max_requests_per_minute, 20);
        assert_eq!(state.min_interval, Duration::from_millis(1500));
    }
}
