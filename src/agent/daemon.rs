use crate::agent::rate_limiter::SgeRateLimiter;
use crate::agent::signal::ShutdownSignal;
use crate::client::{EnedisProvider, IntoProvider};
use crate::error::{EnedisError, ResilienceAction};
use crate::models::{FlowDirection, PointId};
use crate::storage::{StorageBackend, SyncState};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, instrument, warn};

/// Stratégie de planification de l'Agent de collecte
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentSchedule {
    /// Intervalle périodique fixe (ex: toutes les 3600 secondes)
    Interval(Duration),
    /// Expression Cron standard (ex: "0 4 * * *" pour une exécution quotidienne à 04h00)
    Cron(String),
}

/// Configuration du daemon de collecte
#[derive(Clone, Debug)]
pub struct CollectorConfig {
    /// Fenêtre maximale par requête SGE (en jours, max recommandé Enedis: 7 jours)
    pub chunk_size_days: i64,
    /// Historique initial si aucun état de synchronisation n'existe en base
    pub initial_lookback_days: i64,
    /// Pause entre deux cycles complets de scrutation (conservé pour rétrocompatibilité)
    pub cycle_interval: Duration,
    /// Stratégie de planification avancée (Intervalle fixe ou Expression Cron)
    pub schedule: AgentSchedule,
    /// Nombre maximal d'essais en cas d'erreur transitoire
    pub max_retries: u32,
    /// Nombre maximal de PRM synchronisés simultanément (concurrence contrôlée)
    pub concurrency: usize,
    /// Active la détection fine et le rattrapage chirurgical automatique des trous (Backfill)
    pub enable_backfill: bool,
    /// Profondeur d'inspection pour le rattrapage chirurgical (en jours, défaut: 30)
    pub backfill_lookback_days: i64,
    /// Politique optionnelle de compression et rétention des données anciennes (Rollup / Downsampling)
    pub retention_policy: Option<crate::storage::RetentionPolicy>,
}

impl Default for CollectorConfig {
    fn default() -> Self {
        Self {
            chunk_size_days: 7,
            initial_lookback_days: 14,
            cycle_interval: Duration::from_secs(3600),
            schedule: AgentSchedule::Interval(Duration::from_secs(3600)),
            max_retries: 3,
            concurrency: 4,
            enable_backfill: true,
            backfill_lookback_days: 30,
            retention_policy: None,
        }
    }
}

impl CollectorConfig {
    /// Configure une planification basée sur une expression Cron (ex: "0 4 * * *")
    pub fn with_cron(mut self, expr: &str) -> Result<Self, EnedisError> {
        #[cfg(feature = "agent")]
        {
            use std::str::FromStr as _;
            croner::Cron::from_str(expr).map_err(|e| {
                EnedisError::Configuration(format!("Expression Cron invalide '{}': {}", expr, e))
            })?;
        }
        self.schedule = AgentSchedule::Cron(expr.to_string());
        Ok(self)
    }

    /// Configure une planification à intervalle périodique régulier
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.cycle_interval = interval;
        self.schedule = AgentSchedule::Interval(interval);
        self
    }

    /// Active ou désactive le rattrapage chirurgical automatique des trous
    pub fn with_backfill(mut self, enabled: bool, lookback_days: i64) -> Self {
        self.enable_backfill = enabled;
        self.backfill_lookback_days = lookback_days;
        self
    }

    /// Associe une politique de rétention et compression pour l'agent
    pub fn with_retention_policy(mut self, policy: crate::storage::RetentionPolicy) -> Self {
        self.retention_policy = Some(policy);
        self
    }

    /// Calcule le délai nécessaire jusqu'à la prochaine occurrence d'exécution
    pub fn next_delay(&self, from: DateTime<Utc>) -> Result<Duration, EnedisError> {
        match &self.schedule {
            AgentSchedule::Interval(d) => Ok(*d),
            AgentSchedule::Cron(expr) => {
                #[cfg(feature = "agent")]
                {
                    use std::str::FromStr as _;
                    let cron = croner::Cron::from_str(expr).map_err(|e| {
                        EnedisError::Configuration(format!("Erreur Cron '{}': {}", expr, e))
                    })?;
                    let next = cron.find_next_occurrence(&from, false).map_err(|e| {
                        EnedisError::Configuration(format!(
                            "Calcul de la prochaine occurrence Cron impossible: {}",
                            e
                        ))
                    })?;
                    let diff = next - from;
                    let delay = diff.to_std().unwrap_or(Duration::from_secs(60));
                    Ok(delay)
                }
                #[cfg(not(feature = "agent"))]
                Ok(self.cycle_interval)
            }
        }
    }
}

/// Daemon asynchrone de collecte continue
#[derive(Clone)]
pub struct CollectorDaemon {
    client: Arc<dyn EnedisProvider>,
    storage: Arc<dyn StorageBackend>,
    rate_limiter: SgeRateLimiter,
    config: CollectorConfig,
    shutdown_signal: ShutdownSignal,
    #[cfg(feature = "mqtt")]
    mqtt_publisher: Option<Arc<crate::mqtt::MqttPublisher>>,
}

impl CollectorDaemon {
    pub fn new(
        client: impl IntoProvider,
        storage: Arc<dyn StorageBackend>,
        rate_limiter: SgeRateLimiter,
        config: CollectorConfig,
        shutdown_signal: ShutdownSignal,
    ) -> Self {
        Self {
            client: client.into_provider(),
            storage,
            rate_limiter,
            config,
            shutdown_signal,
            #[cfg(feature = "mqtt")]
            mqtt_publisher: None,
        }
    }

    #[cfg(feature = "mqtt")]
    pub fn with_mqtt_publisher(mut self, publisher: Arc<crate::mqtt::MqttPublisher>) -> Self {
        self.mqtt_publisher = Some(publisher);
        self
    }

    /// Démarre la boucle infinie de collecte (s'interrompt proprement sur signal cancel_token)
    pub async fn run(&self) -> Result<(), EnedisError> {
        info!("Démarrage de l'Agent de collecte enedis-rs");

        while !self.shutdown_signal.is_cancelled() {
            let start_cycle = Utc::now();
            let points = self.storage.list_sync_points().await?;

            if points.is_empty() {
                info!("Aucun PRM configuré en base. En attente du prochain cycle...");
            } else {
                let concurrency = self.config.concurrency.max(1);
                info!(
                    "Début du cycle de collecte pour {} PRM (concurrence: {})",
                    points.len(),
                    concurrency
                );

                if concurrency == 1 {
                    for &point_id in &points {
                        if self.shutdown_signal.is_cancelled() {
                            break;
                        }
                        if let Err(err) = self.sync_point(point_id).await {
                            error!("Erreur non récupérable sur le PRM {}: {}", point_id, err);
                            let action = err.classify();
                            if matches!(action, ResilienceAction::FatalStop) {
                                return Err(err);
                            }
                        }
                    }
                } else {
                    let semaphore = Arc::new(tokio::sync::Semaphore::new(concurrency));
                    let mut join_set = tokio::task::JoinSet::new();

                    for &point_id in &points {
                        if self.shutdown_signal.is_cancelled() {
                            break;
                        }

                        let permit = tokio::select! {
                            _ = self.shutdown_signal.cancelled() => break,
                            p = semaphore.clone().acquire_owned() => {
                                match p {
                                    Ok(permit) => permit,
                                    Err(_) => break,
                                }
                            }
                        };

                        let daemon = self.clone();
                        join_set.spawn(async move {
                            let _permit = permit;
                            let res = daemon.sync_point(point_id).await;
                            (point_id, res)
                        });

                        // Traiter les tâches qui se terminent au fil de l'eau
                        while let Some(res) = join_set.try_join_next() {
                            match res {
                                Ok((pid, Err(err))) => {
                                    error!("Erreur non récupérable sur le PRM {}: {}", pid, err);
                                    if matches!(err.classify(), ResilienceAction::FatalStop) {
                                        join_set.shutdown().await;
                                        return Err(err);
                                    }
                                }
                                Ok((_, Ok(()))) => {}
                                Err(join_err) => {
                                    warn!("Tâche de synchronisation interrompue: {}", join_err);
                                }
                            }
                        }
                    }

                    // Attendre toutes les tâches restantes du cycle en cours
                    while let Some(res) = join_set.join_next().await {
                        match res {
                            Ok((pid, Err(err))) => {
                                error!("Erreur non récupérable sur le PRM {}: {}", pid, err);
                                if matches!(err.classify(), ResilienceAction::FatalStop) {
                                    return Err(err);
                                }
                            }
                            Ok((_, Ok(()))) => {}
                            Err(join_err) => {
                                warn!("Tâche de synchronisation interrompue: {}", join_err);
                            }
                        }
                    }
                }
            }

            // Rattrapage chirurgical automatique (Gap backfill) sur l'historique récent si activé
            if self.config.enable_backfill && !points.is_empty() {
                let now = Utc::now();
                let backfill_from = now - ChronoDuration::days(self.config.backfill_lookback_days);
                let backfill_to = now - ChronoDuration::hours(24);
                if backfill_from < backfill_to {
                    for point_id in &points {
                        if self.shutdown_signal.is_cancelled() {
                            break;
                        }
                        match self
                            .backfill_point(*point_id, backfill_from, backfill_to)
                            .await
                        {
                            Ok(b_stats) => {
                                if b_stats.measurements_recovered > 0 {
                                    info!(
                                        "PRM {} : {} mesure(s) récupérée(s) lors du rattrapage chirurgical ({} trou(s))",
                                        point_id, b_stats.measurements_recovered, b_stats.ranges_detected
                                    );
                                }
                            }
                            Err(err) => {
                                warn!(
                                    "Échec du rattrapage chirurgical pour le PRM {}: {}",
                                    point_id, err
                                );
                            }
                        }
                    }
                }
            }

            // Application de la politique de rétention et compression (rollup) si configurée
            if let Some(ref policy) = self.config.retention_policy {
                info!("Application de la politique de rétention et compression (rollup)...");
                match self.storage.apply_retention_policy(None, policy).await {
                    Ok(r_stats) => {
                        if r_stats.raw_measurements_processed > 0 {
                            info!(
                                "Rollup terminé : {} brutes archivées en {} agrégats, {} supprimées (vacuum: {})",
                                r_stats.raw_measurements_processed,
                                r_stats.rollups_created,
                                r_stats.raw_measurements_deleted,
                                r_stats.vacuum_executed
                            );
                        }
                    }
                    Err(err) => {
                        error!(
                            "Erreur lors de l'application de la politique de rétention: {}",
                            err
                        );
                    }
                }
            }

            let delay = match self.config.next_delay(Utc::now()) {
                Ok(d) => d,
                Err(e) => {
                    warn!(
                        "Erreur calcul délai planification: {}. Repli sur intervalle.",
                        e
                    );
                    self.config.cycle_interval
                }
            };

            info!(
                "Cycle de collecte terminé en {:?}. Prochaine exécution dans {:?}",
                Utc::now() - start_cycle,
                delay
            );

            tokio::select! {
                _ = self.shutdown_signal.cancelled() => {
                    info!("Signal d'arrêt reçu. Arrêt propre du collecteur.");
                    break;
                }
                _ = tokio::time::sleep(delay) => {}
            }
        }

        Ok(())
    }

    /// Synchronise un PRM donné sur toute sa plage temporelle manquante
    #[instrument(skip(self), fields(prm = %point_id))]
    pub async fn sync_point(&self, point_id: PointId) -> Result<(), EnedisError> {
        let mut cons_err = None;
        let mut any_success = false;

        // Tentative de synchronisation du flux Consommation
        match self
            .sync_point_direction(point_id, FlowDirection::Consumption)
            .await
        {
            Ok(()) => {
                any_success = true;
            }
            Err(err) => {
                if matches!(err.classify(), ResilienceAction::FatalStop) {
                    return Err(err);
                }
                warn!(
                    "Flux consommation non disponible ou en pause pour {} : {}",
                    point_id, err
                );
                cons_err = Some(err);
            }
        }

        // Tentative de synchronisation du flux Production (notamment pour les PRMs en injection pure)
        match self
            .sync_point_direction(point_id, FlowDirection::Production)
            .await
        {
            Ok(()) => {
                any_success = true;
            }
            Err(err) => {
                if matches!(err.classify(), ResilienceAction::FatalStop) {
                    return Err(err);
                }
                warn!(
                    "Flux production non disponible ou en pause pour {} : {}",
                    point_id, err
                );
            }
        }

        if !any_success {
            if let Some(err) = cons_err {
                return Err(err);
            }
        }

        // Publication Home Assistant MQTT non-bloquante
        #[cfg(feature = "mqtt")]
        if let Some(ref publisher) = self.mqtt_publisher {
            let pub_clone = Arc::clone(publisher);
            let storage_clone = Arc::clone(&self.storage);
            tokio::spawn(async move {
                if let Err(err) = pub_clone
                    .publish_prm_update(storage_clone.as_ref(), point_id)
                    .await
                {
                    error!(
                        "Échec de la publication MQTT pour le PRM {}: {}",
                        point_id, err
                    );
                }
            });
        }

        Ok(())
    }

    async fn sync_point_direction(
        &self,
        point_id: PointId,
        direction: FlowDirection,
    ) -> Result<(), EnedisError> {
        let now = Utc::now();
        let state = self.storage.get_sync_state(point_id, direction).await?;

        let mut start_date = match state {
            Some(ref s) => {
                // Si le point est en erreur, vérifier si la durée de mise en quarantaine s'est écoulée
                if s.sync_status.starts_with("ERROR_") {
                    let elapsed = now - s.last_sync_attempt;
                    let quarantine_duration = if s.sync_status.contains("POINT_NOT_FOUND") {
                        ChronoDuration::days(7)
                    } else {
                        ChronoDuration::hours(24)
                    };
                    if elapsed < quarantine_duration {
                        info!(
                            "PRM {} [{}] temporairement en pause ({}). Temps restant: {:?}. Ignoré pour ce cycle.",
                            point_id, direction, s.sync_status, quarantine_duration - elapsed
                        );
                        return Ok(());
                    }
                }
                s.last_synced_timestamp
            }
            None => now - ChronoDuration::days(self.config.initial_lookback_days),
        };

        // Si le dernier sync est très récent (< 30 min), rien à faire
        if now - start_date < ChronoDuration::minutes(30) {
            return Ok(());
        }

        while start_date < now && !self.shutdown_signal.is_cancelled() {
            let chunk_end =
                (start_date + ChronoDuration::days(self.config.chunk_size_days)).min(now);
            info!(
                "Collecte SGE pour {} [{}] de {} à {}",
                point_id, direction, start_date, chunk_end
            );

            // 1. Respect strict du rate limit
            self.rate_limiter.acquire().await;

            // 2. Appel avec retry géré
            let mut attempts = 0;
            let measurements = loop {
                attempts += 1;
                match self
                    .client
                    .fetch_measurements(point_id, start_date, chunk_end, direction)
                    .await
                {
                    Ok(m) => break Ok(m),
                    Err(err) => {
                        let action = err.classify();
                        match action {
                            ResilienceAction::RetryAfter(delay)
                                if attempts <= self.config.max_retries =>
                            {
                                warn!(
                                    "Erreur transitoire pour {} (essai {}/{}): {}. Attente de {:?}",
                                    point_id, attempts, self.config.max_retries, err, delay
                                );
                                tokio::time::sleep(delay).await;
                            }
                            ResilienceAction::PauseCollector(delay) => {
                                warn!("Pause globale demandée par SGE ({:?}): {}", delay, err);
                                attempts = 0; // Réinitialisation pour ne pas épuiser max_retries sur quota
                                tokio::time::sleep(delay).await;
                            }
                            ResilienceAction::BlacklistPoint { duration, .. } => {
                                warn!(
                                    "Mise en quarantaine du PRM {} pendant {:?}: {}",
                                    point_id, duration, err
                                );
                                self.storage
                                    .update_sync_state(&SyncState {
                                        point_id,
                                        direction,
                                        last_synced_timestamp: start_date,
                                        last_sync_attempt: Utc::now(),
                                        sync_status: format!(
                                            "ERROR_{}",
                                            err.classify_status_code()
                                        ),
                                    })
                                    .await?;
                                return Ok(());
                            }
                            _ => break Err(err),
                        }
                    }
                }
            }?;

            // 3. Persistance en base avec UPSERT conditionnel
            if !measurements.is_empty() {
                let stats = self.storage.upsert_measurements(&measurements).await?;
                info!(
                    "PRM {} : {} mesures traitées ({} insérées/mises à jour)",
                    point_id, stats.processed, stats.affected
                );
            }

            // 4. Avancement intelligent de l'état de synchronisation
            // Protège contre la latence de publication Enedis à J-1
            let new_synced_timestamp = if let Some(max_ts) =
                measurements.iter().map(|m| m.timestamp).max()
            {
                max_ts
            } else {
                let threshold_d_minus_1 = now - ChronoDuration::hours(24);
                if start_date >= threshold_d_minus_1 {
                    info!("Données Enedis non encore publiées pour la période récente. Fin du créneau.");
                    break;
                } else {
                    chunk_end.min(threshold_d_minus_1)
                }
            };

            if new_synced_timestamp <= start_date {
                info!(
                    "Synchronisation à jour pour PRM {} [{}]",
                    point_id, direction
                );
                break;
            }

            start_date = new_synced_timestamp;
            self.storage
                .update_sync_state(&SyncState {
                    point_id,
                    direction,
                    last_synced_timestamp: start_date,
                    last_sync_attempt: Utc::now(),
                    sync_status: "OK".to_string(),
                })
                .await?;
        }

        Ok(())
    }

    /// Détecte les plages de jours manquants pour un PRM entre deux dates (défaut: flux Consommation)
    pub async fn detect_missing_ranges(
        &self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<(DateTime<Utc>, DateTime<Utc>)>, EnedisError> {
        self.detect_missing_ranges_direction(point_id, FlowDirection::Consumption, from, to)
            .await
    }

    /// Détecte les plages de jours manquants pour un PRM et un sens de flux précis
    pub async fn detect_missing_ranges_direction(
        &self,
        point_id: PointId,
        direction: FlowDirection,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<(DateTime<Utc>, DateTime<Utc>)>, EnedisError> {
        self.storage
            .detect_missing_ranges(point_id, direction, from, to)
            .await
    }

    /// Exécute un rattrapage chirurgical (backfill) des plages manquantes pour un PRM (Consommation & Production)
    pub async fn backfill_point(
        &self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<crate::storage::BackfillStats, EnedisError> {
        let mut stats = crate::storage::BackfillStats::default();

        let s1 = self
            .backfill_missing_ranges(point_id, FlowDirection::Consumption, from, to)
            .await?;
        stats.ranges_detected += s1.ranges_detected;
        stats.requests_made += s1.requests_made;
        stats.measurements_recovered += s1.measurements_recovered;

        let s2 = self
            .backfill_missing_ranges(point_id, FlowDirection::Production, from, to)
            .await?;
        stats.ranges_detected += s2.ranges_detected;
        stats.requests_made += s2.requests_made;
        stats.measurements_recovered += s2.measurements_recovered;

        Ok(stats)
    }

    /// Exécute un rattrapage chirurgical (backfill) ciblé sur un flux donné
    pub async fn backfill_missing_ranges(
        &self,
        point_id: PointId,
        direction: FlowDirection,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<crate::storage::BackfillStats, EnedisError> {
        let missing = self
            .detect_missing_ranges_direction(point_id, direction, from, to)
            .await?;

        if missing.is_empty() {
            return Ok(crate::storage::BackfillStats::default());
        }

        info!(
            "Rattrapage chirurgical pour PRM {} [{}] : {} trou(s) détecté(s)",
            point_id,
            direction,
            missing.len()
        );

        let mut stats = crate::storage::BackfillStats {
            ranges_detected: missing.len(),
            requests_made: 0,
            measurements_recovered: 0,
        };

        for (gap_start, gap_end) in missing {
            if self.shutdown_signal.is_cancelled() {
                break;
            }

            let mut cur_start = gap_start;
            while cur_start < gap_end && !self.shutdown_signal.is_cancelled() {
                let chunk_end =
                    (cur_start + ChronoDuration::days(self.config.chunk_size_days)).min(gap_end);

                info!(
                    "Rattrapage Enedis SGE pour {} [{}] de {} à {}",
                    point_id, direction, cur_start, chunk_end
                );

                self.rate_limiter.acquire().await;
                stats.requests_made += 1;

                let mut attempts = 0;
                let measurements = loop {
                    attempts += 1;
                    match self
                        .client
                        .fetch_measurements(point_id, cur_start, chunk_end, direction)
                        .await
                    {
                        Ok(m) => break Ok(m),
                        Err(err) => {
                            let action = err.classify();
                            match action {
                                ResilienceAction::RetryAfter(delay)
                                    if attempts <= self.config.max_retries =>
                                {
                                    warn!(
                                        "Erreur transitoire rattrapage {} (essai {}/{}): {}. Attente de {:?}",
                                        point_id, attempts, self.config.max_retries, err, delay
                                    );
                                    tokio::time::sleep(delay).await;
                                }
                                ResilienceAction::PauseCollector(delay) => {
                                    warn!("Pause globale demandée par SGE ({:?}): {}", delay, err);
                                    tokio::time::sleep(delay).await;
                                }
                                _ => break Err(err),
                            }
                        }
                    }
                };

                match measurements {
                    Ok(m) => {
                        if !m.is_empty() {
                            let upsert_res = self.storage.upsert_measurements(&m).await?;
                            stats.measurements_recovered += upsert_res.affected;
                        }
                    }
                    Err(err) => {
                        warn!(
                            "Échec du rattrapage pour {} [{}] sur {}..{} : {}",
                            point_id, direction, cur_start, chunk_end, err
                        );
                    }
                }

                cur_start = chunk_end;
            }
        }

        Ok(stats)
    }
}
