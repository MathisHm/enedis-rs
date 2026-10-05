pub mod error;
pub mod models;

#[cfg(feature = "xml")]
pub mod xml;

#[cfg(feature = "client")]
pub mod client;

#[cfg(feature = "storage")]
pub mod storage;

#[cfg(feature = "mock-sge")]
pub mod mock;

#[cfg(feature = "agent")]
pub mod agent;

#[cfg(feature = "client")]
pub mod doctor;

pub mod metrics;

#[cfg(any(feature = "client", feature = "agent", feature = "api"))]
pub mod signal;

#[cfg(feature = "api")]
pub mod api;

#[cfg(feature = "wasm")]
pub mod wasm;

#[cfg(feature = "wasm")]
pub use wasm::*;

// Re-exports ergonomiques
pub use error::{EnedisError, ResilienceAction, SgeBusinessError, SoapFault, TransportError};
pub use metrics::MetricsRegistry;
pub use models::{
    aggregate_measurements, analyze_spot_consumption, audit_subscription_sizing,
    calculate_energy_costs, correlate_measurements_with_grid, from_french_local_time,
    generate_synthetic_spot_profile, tempo_date_for_time, to_french_local_time,
    AggregatedMeasurement, AggregationInterval, AggregationIntervalError, BaseTariff,
    CalendarSchedule, ConsentAlert, ConsentAlertSeverity, ConsentInfo, ConsentStatus, ContractData,
    CostCalculation, DynamicTariff, EcoWattCorrelation, EcoWattLevel, EcoWattSignal, FlowDirection,
    GridCorrelationReport, HpHcTariff, MaxPowerRecord, Measurement, MeasurementQuality,
    MeterCharacteristics, MeterType, PhaseCount, PointId, PointIdError, SizingStatus,
    SpotArbitrageOpportunity, SpotPriceRecord, SpotProfileAnalysis, SubscriptionAudit,
    TariffComparison, TariffConfig, TariffCostBucket, TariffOption, TaxConfig, TempoColor,
    TempoCorrelation, TempoDayRecord, TempoTariff, TimeSlot, Unit,
};

#[cfg(any(feature = "client", feature = "agent", feature = "api"))]
pub use signal::ShutdownSignal;

#[cfg(feature = "client")]
pub use client::{
    ClientIdentitySource, DataConnectClient, DataConnectConfig, DataConnectEnvironment,
    EnedisProvider, IntoProvider, NetworkSignalClient, OAuthTokenResponse, ProviderFuture,
    SgeClient, SgeClientConfig, SgeEnvironment, SignalClientConfig,
};

#[cfg(feature = "xml")]
pub use xml::{build_soap_envelope, parse_soap_response, validate_xml_security, SgeResponseParser};

#[cfg(feature = "storage")]
pub use storage::{StorageBackend, StorageFuture, SyncState, UpsertStats};

#[cfg(feature = "mock-sge")]
pub use mock::{MockDataConnectServer, MockScenario, MockSgeServer};

#[cfg(feature = "agent")]
pub use agent::{CollectorConfig, CollectorDaemon, SgeRateLimiter};

#[cfg(feature = "client")]
pub use doctor::{DoctorReportItem, DoctorStatus, EnedisDoctor};

#[cfg(feature = "api")]
pub use api::{ApiServer, AppState};

#[cfg(feature = "mqtt")]
pub mod mqtt;

#[cfg(feature = "mqtt")]
pub use mqtt::{
    all_discovery_configs, baseload_sensor_discovery, diagnostic_sensors_discovery,
    energy_sensor_discovery, energy_sensor_discovery_topic, max_power_sensor_discovery,
    power_sensor_discovery, solar_autoconsumption_sensor_discovery,
    solar_autoproduction_sensor_discovery, spot_price_sensor_discovery,
    subscribed_power_sensor_discovery, EnedisPrmState, HaDevice, HaSensorConfig, MqttPublisher,
    MqttPublisherConfig,
};
