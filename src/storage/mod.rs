pub mod models;
pub mod retention;
pub mod traits;

pub mod influxdb;

#[cfg(feature = "parquet")]
pub mod parquet;

pub mod duckdb;

#[cfg(feature = "storage-sqlite")]
pub mod sqlite;

#[cfg(feature = "storage-postgres")]
pub mod postgres;

pub use crate::models::{AggregatedMeasurement, AggregationInterval};
pub use models::{BackfillStats, SyncState, UpsertStats};
pub use retention::{RetentionPolicy, RollupStats};
pub use traits::{StorageBackend, StorageFuture};

#[cfg(feature = "storage-sqlite")]
pub use sqlite::SqliteStorage;

#[cfg(feature = "storage-postgres")]
pub use postgres::PostgresStorage;
