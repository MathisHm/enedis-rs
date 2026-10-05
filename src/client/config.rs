use secrecy::SecretString;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// Source d'identité client pour la négociation mTLS
#[derive(Clone)]
pub enum ClientIdentitySource {
    /// Fichier PKCS#12 (.p12 / .pfx) sur disque
    Pkcs12File {
        path: PathBuf,
        password: SecretString,
    },
    /// Contenu binaire PKCS#12 en mémoire (ex: lu depuis un Secret Store ou une variable d'environnement)
    Pkcs12Der {
        der_bytes: Vec<u8>,
        password: SecretString,
    },
    /// Certificat et clé privée au format PEM sur disque
    PemFile {
        cert_path: PathBuf,
        key_path: PathBuf,
    },
    /// Paires PEM en mémoire
    PemBytes { cert_pem: Vec<u8>, key_pem: Vec<u8> },
}

impl fmt::Debug for ClientIdentitySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pkcs12File { path, .. } => f
                .debug_struct("Pkcs12File")
                .field("path", path)
                .field("password", &"[REDACTED]")
                .finish(),
            Self::Pkcs12Der { der_bytes, .. } => f
                .debug_struct("Pkcs12Der")
                .field("len", &der_bytes.len())
                .field("password", &"[REDACTED]")
                .finish(),
            Self::PemFile {
                cert_path,
                key_path,
            } => f
                .debug_struct("PemFile")
                .field("cert_path", cert_path)
                .field("key_path", key_path)
                .finish(),
            Self::PemBytes { cert_pem, key_pem } => f
                .debug_struct("PemBytes")
                .field("cert_len", &cert_pem.len())
                .field("key_len", &key_pem.len())
                .finish(),
        }
    }
}

/// Environnement cible pour les Web Services Enedis SGE
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SgeEnvironment {
    /// Environnement de production officiel
    #[default]
    Production,
    /// Environnement d'homologation / bac à sable Enedis
    Homologation,
    /// Environnement local de développement ou mock
    LocalMock,
}

impl SgeEnvironment {
    pub fn default_endpoint(&self) -> &'static str {
        match self {
            Self::Production => "https://sge-services.enedis.fr/services/",
            Self::Homologation => "https://sge-services-homologation.enedis.fr/services/",
            Self::LocalMock => "http://127.0.0.1:8080/services/",
        }
    }
}

/// Configuration du client de communication SGE
#[derive(Clone, Debug)]
pub struct SgeClientConfig {
    /// URL racine des Web Services Enedis SGE
    pub endpoint_url: String,
    /// Identité client mTLS
    pub identity: Option<ClientIdentitySource>,
    /// CA racine personnalisée optionnelle (format PEM)
    pub custom_ca_pem: Option<Vec<u8>>,
    /// Délai maximal d'établissement de connexion TCP / TLS
    pub connect_timeout: Duration,
    /// Délai maximal d'attente de réponse HTTP
    pub request_timeout: Duration,
    /// Chaîne User-Agent envoyée à Enedis
    pub user_agent: String,
}

impl SgeClientConfig {
    /// Instancie une configuration pré-paramétrée pour l'environnement cible
    pub fn with_environment(env: SgeEnvironment) -> Self {
        Self {
            endpoint_url: env.default_endpoint().to_string(),
            ..Self::default()
        }
    }
}

impl Default for SgeClientConfig {
    fn default() -> Self {
        Self {
            endpoint_url: SgeEnvironment::Production.default_endpoint().to_string(),
            identity: None,
            custom_ca_pem: None,
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(45),
            user_agent: format!("enedis-rs/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Environnement cible pour l'API REST Enedis Data Connect (OAuth2)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DataConnectEnvironment {
    /// Environnement de production officiel
    #[default]
    Production,
    /// Environnement Sandbox / Bac à sable Enedis
    Sandbox,
    /// Environnement local de développement ou mock
    LocalMock,
}

impl DataConnectEnvironment {
    pub fn default_base_url(&self) -> &'static str {
        match self {
            Self::Production => "https://ext.prod.api.enedis.fr",
            Self::Sandbox => "https://ext.sandbox.api.enedis.fr",
            Self::LocalMock => "http://127.0.0.1:8081",
        }
    }

    pub fn default_token_url(&self) -> &'static str {
        match self {
            Self::Production => "https://ext.prod.api.enedis.fr/oauth2/v3/token",
            Self::Sandbox => "https://ext.sandbox.api.enedis.fr/oauth2/v3/token",
            Self::LocalMock => "http://127.0.0.1:8081/oauth2/v3/token",
        }
    }

    pub fn default_authorize_url(&self) -> &'static str {
        match self {
            Self::Production => {
                "https://mon-compte-client.enedis.fr/dataconnect/v1/oauth2/authorize"
            }
            Self::Sandbox => "https://mon-compte-client.enedis.fr/dataconnect/v1/oauth2/authorize",
            Self::LocalMock => "http://127.0.0.1:8081/oauth2/v3/authorize",
        }
    }
}

/// Configuration du client Enedis Data Connect (REST v5 / OAuth2)
#[derive(Clone)]
pub struct DataConnectConfig {
    /// URL de base de l'API REST (ex: https://ext.prod.api.enedis.fr)
    pub base_url: String,
    /// URL du serveur OAuth2 de jetons
    pub token_url: String,
    /// URL d'autorisation Enedis pour le recueil de consentement client
    pub authorize_url: Option<String>,
    /// Identifiant Client ID OAuth2
    pub client_id: Option<String>,
    /// Secret client OAuth2
    pub client_secret: Option<SecretString>,
    /// Jeton Bearer pré-acquis ou injecté directement
    pub direct_token: Option<SecretString>,
    /// Délai maximal d'établissement de connexion TCP / TLS
    pub connect_timeout: Duration,
    /// Délai maximal d'attente de réponse HTTP
    pub request_timeout: Duration,
    /// Chaîne User-Agent envoyée
    pub user_agent: String,
}

impl fmt::Debug for DataConnectConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataConnectConfig")
            .field("base_url", &self.base_url)
            .field("token_url", &self.token_url)
            .field("authorize_url", &self.authorize_url)
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "direct_token",
                &self.direct_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("connect_timeout", &self.connect_timeout)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

impl DataConnectConfig {
    pub fn with_environment(env: DataConnectEnvironment) -> Self {
        Self {
            base_url: env.default_base_url().to_string(),
            token_url: env.default_token_url().to_string(),
            authorize_url: Some(env.default_authorize_url().to_string()),
            ..Self::default()
        }
    }

    pub fn with_direct_token(token: impl Into<String>) -> Self {
        Self {
            direct_token: Some(SecretString::new(token.into())),
            ..Self::default()
        }
    }

    pub fn with_credentials(
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        Self {
            client_id: Some(client_id.into()),
            client_secret: Some(SecretString::new(client_secret.into())),
            ..Self::default()
        }
    }

    pub fn with_authorize_url(mut self, url: impl Into<String>) -> Self {
        self.authorize_url = Some(url.into());
        self
    }

    pub fn get_authorize_url(&self) -> &str {
        self.authorize_url
            .as_deref()
            .unwrap_or("https://mon-compte-client.enedis.fr/dataconnect/v1/oauth2/authorize")
    }
}

impl Default for DataConnectConfig {
    fn default() -> Self {
        Self {
            base_url: DataConnectEnvironment::Production
                .default_base_url()
                .to_string(),
            token_url: DataConnectEnvironment::Production
                .default_token_url()
                .to_string(),
            authorize_url: Some(
                DataConnectEnvironment::Production
                    .default_authorize_url()
                    .to_string(),
            ),
            client_id: None,
            client_secret: None,
            direct_token: None,
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(45),
            user_agent: format!("enedis-rs/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}
