use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

/// Signal d'arrêt coopératif pour les daemons, serveurs HTTP et tâches d'arrière-plan
#[derive(Clone, Default)]
pub struct ShutdownSignal {
    is_cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
}

impl ShutdownSignal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Déclenche l'arrêt
    pub fn cancel(&self) {
        self.is_cancelled.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    /// Indique si l'arrêt a été demandé
    #[inline]
    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::SeqCst)
    }

    /// Future qui se résout lorsque l'arrêt est déclenché
    pub async fn cancelled(&self) {
        // Enregistrer d'abord le waiter auprès de Notify pour ne manquer aucune notification concurrente
        let notified = self.notify.notified();
        if self.is_cancelled() {
            return;
        }
        notified.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_shutdown_signal_lifecycle() {
        let signal = ShutdownSignal::new();
        assert!(!signal.is_cancelled());

        let signal_clone = signal.clone();
        let handle = tokio::spawn(async move {
            signal_clone.cancelled().await;
            true
        });

        // La tâche doit être en attente
        tokio::time::sleep(Duration::from_millis(10)).await;
        assert!(!handle.is_finished());

        // Déclenchement de l'arrêt
        signal.cancel();
        assert!(signal.is_cancelled());

        let res = handle.await.unwrap();
        assert!(res);

        // Appel ultérieur immédiat (déjà annulé)
        signal.cancelled().await;
        assert!(signal.is_cancelled());
    }

    #[test]
    fn test_shutdown_signal_idempotence() {
        let signal = ShutdownSignal::new();
        signal.cancel();
        signal.cancel();
        assert!(signal.is_cancelled());
    }
}
