use crate::error::EnedisError;
use crate::models::{
    ConsentInfo, ContractData, FlowDirection, MaxPowerRecord, Measurement, PointId,
};
use chrono::{DateTime, Utc};
use std::future::Future;
use std::pin::Pin;

/// Future asynchrone retournée par les méthodes de EnedisProvider (object-safe)
pub type ProviderFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, EnedisError>> + Send + 'a>>;

/// Trait unifié abstrayant la source d'accès aux données Enedis
///
/// Ce trait permet d'interchanger de manière transparente :
/// - `SgeClient` : Protocoles Web Services SOAP 1.1 sur TLS mutuel (mTLS) pour les acteurs de marché
/// - `DataConnectClient` : API REST v5 sécurisée par OAuth2 Bearer Tokens pour les particuliers et intégrations tierces
pub trait EnedisProvider: Send + Sync {
    /// Identifiant du fournisseur (ex: "SGE-SOAP-mTLS", "DataConnect-REST-v5")
    fn provider_name(&self) -> &'static str;

    /// Récupère la courbe de charge ou les mesures pour un PRM donné sur une plage de dates
    fn fetch_measurements<'a>(
        &'a self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        direction: FlowDirection,
    ) -> ProviderFuture<'a, Vec<Measurement>>;

    /// Récupère les données contractuelles (puissance souscrite, option tarifaire, caractéristiques compteur, etc.)
    fn fetch_contract_data<'a>(&'a self, point_id: PointId) -> ProviderFuture<'a, ContractData>;

    /// Récupère les pointes maximales quotidiennes de puissance atteinte (W / kVA)
    fn fetch_daily_max_power<'a>(
        &'a self,
        point_id: PointId,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> ProviderFuture<'a, Vec<MaxPowerRecord>>;

    /// Vérifie et récupère l'état et l'échéance du consentement client
    fn fetch_consent_status<'a>(&'a self, point_id: PointId) -> ProviderFuture<'a, ConsentInfo>;
}

/// Trait de conversion universel permettant d'injecter n'importe quel client Enedis (SGE ou Data Connect)
pub trait IntoProvider: Send + Sync {
    fn into_provider(self) -> std::sync::Arc<dyn EnedisProvider>;
}

impl IntoProvider for std::sync::Arc<dyn EnedisProvider> {
    fn into_provider(self) -> std::sync::Arc<dyn EnedisProvider> {
        self
    }
}

impl IntoProvider for crate::client::SgeClient {
    fn into_provider(self) -> std::sync::Arc<dyn EnedisProvider> {
        std::sync::Arc::new(self)
    }
}

impl IntoProvider for std::sync::Arc<crate::client::SgeClient> {
    fn into_provider(self) -> std::sync::Arc<dyn EnedisProvider> {
        self
    }
}

impl IntoProvider for crate::client::DataConnectClient {
    fn into_provider(self) -> std::sync::Arc<dyn EnedisProvider> {
        std::sync::Arc::new(self)
    }
}

impl IntoProvider for std::sync::Arc<crate::client::DataConnectClient> {
    fn into_provider(self) -> std::sync::Arc<dyn EnedisProvider> {
        self
    }
}
