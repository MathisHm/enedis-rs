#![cfg(feature = "agent")]

use chrono::{TimeZone, Utc};
use enedis_rs::agent::{AgentSchedule, CollectorConfig};
use std::time::Duration;

#[test]
fn test_collector_config_cron_valid() {
    let config = CollectorConfig::default()
        .with_cron("0 4 * * *")
        .expect("0 4 * * * doit être une expression cron valide");

    assert!(matches!(config.schedule, AgentSchedule::Cron(ref s) if s == "0 4 * * *"));
}

#[test]
fn test_collector_config_cron_invalid() {
    let res = CollectorConfig::default().with_cron("not a cron pattern");
    assert!(res.is_err());
}

#[test]
fn test_collector_config_cron_next_delay_calculation() {
    let config = CollectorConfig::default()
        .with_cron("0 4 * * *")
        .expect("Expression valide");

    // 03:30:00 UTC -> la prochaine exécution à 04:00:00 UTC doit être dans exactement 30 minutes (1800s)
    let current_time = Utc.with_ymd_and_hms(2026, 9, 29, 3, 30, 0).unwrap();
    let delay = config.next_delay(current_time).unwrap();

    assert_eq!(delay, Duration::from_secs(1800));

    // 04:00:01 UTC -> la prochaine exécution doit être le lendemain à 04:00:00 UTC (23h 59m 59s = 86399s)
    let after_trigger = Utc.with_ymd_and_hms(2026, 9, 29, 4, 0, 1).unwrap();
    let delay_next_day = config.next_delay(after_trigger).unwrap();

    assert_eq!(delay_next_day, Duration::from_secs(86399));
}

#[test]
fn test_collector_config_interval_fallback() {
    let config = CollectorConfig::default().with_interval(Duration::from_secs(900));

    assert_eq!(
        config.schedule,
        AgentSchedule::Interval(Duration::from_secs(900))
    );
    let now = Utc::now();
    let delay = config.next_delay(now).unwrap();
    assert_eq!(delay, Duration::from_secs(900));
}
