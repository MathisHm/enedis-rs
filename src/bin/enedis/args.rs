use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "enedis")]
#[command(about = "Infrastructure Rust de collecte et d'analyse Enedis SGE & Data Connect", long_about = None)]
pub struct Cli {
    /// Fournisseur Enedis à utiliser : "sge" (SOAP mTLS) ou "dataconnect" (REST OAuth2)
    #[arg(long, env = "ENEDIS_PROVIDER", default_value = "sge")]
    pub provider: String,

    /// URL du Web Service SGE Enedis
    #[arg(
        long,
        env = "ENEDIS_ENDPOINT",
        default_value = "https://sge-services.enedis.fr/services/"
    )]
    pub endpoint: String,

    /// Jeton Bearer pour Enedis Data Connect
    #[arg(long, env = "ENEDIS_DATA_CONNECT_TOKEN")]
    pub dc_token: Option<String>,

    /// URL de base pour Enedis Data Connect
    #[arg(
        long,
        env = "ENEDIS_DATA_CONNECT_URL",
        default_value = "https://ext.prod.api.enedis.fr"
    )]
    pub dc_url: String,

    /// Client ID OAuth2 pour Enedis Data Connect
    #[arg(long, env = "ENEDIS_DATA_CONNECT_CLIENT_ID")]
    pub dc_client_id: Option<String>,

    /// Client Secret OAuth2 pour Enedis Data Connect
    #[arg(long, env = "ENEDIS_DATA_CONNECT_CLIENT_SECRET")]
    pub dc_client_secret: Option<String>,

    /// Chemin du certificat client PKCS#12 (.p12)
    #[arg(long, env = "ENEDIS_CERT_P12")]
    pub cert_p12: Option<PathBuf>,

    /// Mot de passe du certificat client PKCS#12
    #[arg(long, env = "ENEDIS_CERT_PASSWORD")]
    pub cert_password: Option<String>,

    /// URL de la base de données (PostgreSQL ou SQLite)
    #[arg(long, env = "DATABASE_URL", default_value = "sqlite://enedis.db")]
    pub db_url: String,

    #[command(subcommand)]
    pub command: Commands,
}

impl std::fmt::Debug for Cli {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cli")
            .field("provider", &self.provider)
            .field("endpoint", &self.endpoint)
            .field("dc_token", &self.dc_token.as_ref().map(|_| "[REDACTED]"))
            .field("dc_url", &self.dc_url)
            .field("dc_client_id", &self.dc_client_id)
            .field(
                "dc_client_secret",
                &self.dc_client_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field("cert_p12", &self.cert_p12)
            .field(
                "cert_password",
                &self.cert_password.as_ref().map(|_| "[REDACTED]"),
            )
            .field("db_url", &self.db_url)
            .field("command", &self.command)
            .finish()
    }
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Diagnostic complet de la chaîne Enedis (certificats, mTLS, consentements, stockage)
    Doctor(DoctorArgs),

    /// Tests et gestion de l'authentification
    Auth(AuthArgs),

    /// Gestion des points de livraison (PRM)
    Point(PointArgs),

    /// Collecte et export des mesures
    Measurements(MeasurementsArgs),

    /// Lancement du daemon de collecte continue
    Agent(AgentArgs),

    /// Démarre le serveur API HTTP REST
    Serve(ServeArgs),

    /// Démarre le simulateur Enedis SGE pour tests et démonstrations locales
    #[cfg(feature = "mock-sge")]
    Mock(MockArgs),

    /// Consultation des données contractuelles (puissance souscrite, option tarifaire, compteur)
    Contract(ContractArgs),

    /// Pointes de puissance atteinte et audit de dimensionnement d'abonnement
    MaxPower(MaxPowerArgs),

    /// Suivi proactif du cycle de vie et alertes d'expiration du consentement client
    Consent(ConsentArgs),

    /// Calcul des coûts financiers en Euros (€) selon différentes formules tarifaires
    Costs(CostsArgs),

    /// Signaux réseau (RTE EcoWatt & EDF Tempo) et analyse de corrélation
    Signals(SignalsArgs),

    /// Marché spot Day-Ahead EPEX SPOT France, tarification dynamique et opportunités d'arbitrage
    Spot(SpotArgs),
}

#[derive(Args, Debug)]
pub struct ContractArgs {
    /// PRM cible (14 chiffres)
    #[arg(long)]
    pub prm: String,
}

#[derive(Args, Debug)]
pub struct MaxPowerArgs {
    /// PRM cible (14 chiffres)
    #[arg(long)]
    pub prm: String,
    /// Date de début (ex: 2026-09-01T00:00:00Z)
    #[arg(long)]
    pub from: String,
    /// Date de fin (ex: 2026-09-29T00:00:00Z)
    #[arg(long)]
    pub to: String,
    /// Effectuer l'analyse d'adéquation et recommandation de palier
    #[arg(long, default_value_t = true)]
    pub audit: bool,
}

#[derive(Args, Debug)]
pub struct ConsentArgs {
    /// PRM cible (14 chiffres)
    #[arg(long)]
    pub prm: String,
    /// Seuil d'avertissement en jours avant échéance (défaut: 30)
    #[arg(long, default_value = "30")]
    pub warning_days: u32,
}

#[derive(Args, Debug)]
pub struct CostsArgs {
    /// PRM cible (14 chiffres)
    #[arg(long)]
    pub prm: String,
    /// Date de début (ex: 2026-09-01T00:00:00Z)
    #[arg(long)]
    pub from: String,
    /// Date de fin (ex: 2026-09-29T00:00:00Z)
    #[arg(long)]
    pub to: String,
    /// Formule tarifaire: "base", "hphc", "tempo", "dynamic"
    #[arg(long, default_value = "base")]
    pub tariff: String,
    /// Prix Base (€/kWh)
    #[arg(long)]
    pub base_price: Option<String>,
    /// Prix Heures Pleines (€/kWh)
    #[arg(long)]
    pub hp_price: Option<String>,
    /// Prix Heures Creuses (€/kWh)
    #[arg(long)]
    pub hc_price: Option<String>,
    /// Abonnement mensuel (€/mois)
    #[arg(long)]
    pub subscription: Option<String>,
    /// Plages horaires Heures Creuses (ex: "22:00-06:00" ou "12:00-14:00,01:30-07:30")
    #[arg(long)]
    pub off_peak_slots: Option<String>,
}

#[derive(Args, Debug)]
pub struct SignalsArgs {
    /// Sous-commande signaux : "tempo", "ecowatt", ou "correlation"
    #[arg(default_value = "tempo")]
    pub action: String,
    /// PRM pour la corrélation
    #[arg(long)]
    pub prm: Option<String>,
    /// Date de début pour la corrélation
    #[arg(long)]
    pub from: Option<String>,
    /// Date de fin pour la corrélation
    #[arg(long)]
    pub to: Option<String>,
    /// Forcer la synchronisation avec les API distantes
    #[arg(long, default_value_t = false)]
    pub sync: bool,
}

#[derive(Args, Debug)]
pub struct DoctorArgs {
    /// PRM optionnel pour tester le consentement et l'accès aux données
    #[arg(long)]
    pub prm: Option<String>,
}

#[derive(Args, Debug)]
pub struct AuthArgs {
    #[command(subcommand)]
    pub command: AuthSubcommands,
}

#[derive(Subcommand, Debug)]
pub enum AuthSubcommands {
    /// Teste la poignée de main TLS et l'authentification du certificat
    Test,
}

#[derive(Args, Debug)]
pub struct PointArgs {
    #[command(subcommand)]
    pub command: PointSubcommands,
}

#[derive(Subcommand, Debug)]
pub enum PointSubcommands {
    /// Liste les points enregistrés et leur état de synchronisation
    List,
    /// Affiche les informations et l'état de synchro d'un PRM
    Get { prm: String },
    /// Publie les configurations Home Assistant Discovery et l'état consolidé vers le broker MQTT
    #[cfg(feature = "mqtt")]
    MqttPublish {
        /// PRM cible (14 chiffres)
        prm: String,
        /// URL du broker MQTT (ex: mqtt://localhost:1883)
        #[arg(
            long = "mqtt-broker",
            env = "MQTT_BROKER",
            default_value = "mqtt://localhost:1883"
        )]
        broker: String,
        /// Préfixe racine des topics d'état (défaut: enedis)
        #[arg(long = "mqtt-prefix", env = "MQTT_PREFIX", default_value = "enedis")]
        prefix: String,
    },
}

#[derive(Args, Debug)]
pub struct MeasurementsArgs {
    #[command(subcommand)]
    pub command: MeasurementsSubcommands,
}

#[derive(Subcommand, Debug)]
pub enum MeasurementsSubcommands {
    /// Récupère les mesures pour un PRM sur une plage donnée
    Fetch {
        #[arg(long)]
        prm: String,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long, default_value = "consumption")]
        direction: String,
    },
    /// Détecte et rattrape chirurgicalement les jours manquants (Backfill)
    Backfill {
        #[arg(long)]
        prm: String,
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
        #[arg(long, default_value = "consumption")]
        direction: String,
    },
    /// Compacte les historiques anciens sous forme d'agrégats pour libérer l'espace disque (Rollup / Downsampling)
    Rollup {
        /// PRM optionnel (si omis, applique à toute la base)
        #[arg(long)]
        prm: Option<String>,
        /// Âge de rétention des données brutes fines en jours (défaut: 730 jours = 2 ans)
        #[arg(long, default_value = "730")]
        older_than_days: u32,
        /// Intervalle de compactage : "hourly" ou "daily" (défaut: daily)
        #[arg(long, default_value = "daily")]
        interval: String,
        /// Déclencher une défragmentation physique (VACUUM SQLite)
        #[arg(long, default_value_t = true)]
        vacuum: bool,
    },
    /// Exporte les mesures enregistrées (format CSV, JSON, InfluxDB Line Protocol, Parquet ou DuckDB script)
    Export {
        #[arg(long)]
        prm: String,
        /// Format d'export : "json", "csv", "influxdb", "parquet", "duckdb"
        #[arg(long, default_value = "json")]
        format: String,
        /// Intervalle d'agrégation optionnel (hour, day, month, year)
        #[arg(long)]
        interval: Option<String>,
        /// Fichier de sortie optionnel (par défaut: stdout)
        #[arg(long)]
        output: Option<PathBuf>,
    },
}

#[derive(Args, Debug)]
pub struct AgentArgs {
    /// Intervalle entre deux cycles complets en secondes (défaut: 3600s / 1h)
    #[arg(long, default_value = "3600")]
    pub interval_secs: u64,

    /// Expression Cron pour la planification (ex: "0 4 * * *" pour lancer chaque nuit à 04h00)
    #[arg(long, env = "ENEDIS_AGENT_CRON")]
    pub cron: Option<String>,

    /// Nombre maximal d'appels autorisés par minute (défaut: 20)
    #[arg(long, default_value = "20")]
    pub rate_limit_per_minute: u32,

    /// Nombre maximal de PRM synchronisés simultanément (défaut: 4)
    #[arg(long, default_value = "4")]
    pub concurrency: usize,

    /// Active la détection fine et le rattrapage chirurgical automatique des trous (Backfill)
    #[arg(long, default_value_t = true)]
    pub backfill: bool,

    /// Profondeur de recherche pour le rattrapage chirurgical en jours (défaut: 30)
    #[arg(long, default_value = "30")]
    pub backfill_lookback: i64,

    /// Durée de rétention des données brutes en jours avant compactage rollup (ex: 730 pour 2 ans)
    #[arg(long)]
    pub retention_days: Option<u32>,

    /// Intervalle de compactage pour la rétention: "hourly" ou "daily" (défaut: daily)
    #[arg(long, default_value = "daily")]
    pub rollup_interval: String,

    /// URL du broker MQTT pour Home Assistant (ex: mqtt://localhost:1883)
    #[arg(long = "mqtt-broker", env = "MQTT_BROKER")]
    pub mqtt_broker: Option<String>,

    /// Préfixe racine des topics MQTT (défaut: enedis)
    #[arg(long = "mqtt-prefix", env = "MQTT_PREFIX", default_value = "enedis")]
    pub mqtt_prefix: String,

    /// Nom d'utilisateur MQTT optionnel
    #[arg(long = "mqtt-user", env = "MQTT_USER")]
    pub mqtt_user: Option<String>,

    /// Mot de passe MQTT optionnel
    #[arg(long = "mqtt-password", env = "MQTT_PASSWORD")]
    pub mqtt_password: Option<String>,
}

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Adresse IP d'écoute
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,

    /// Port d'écoute HTTP
    #[arg(long, default_value = "8080")]
    pub port: u16,

    /// Clé d'API secrète requise pour sécuriser les endpoints REST
    #[arg(long, env = "ENEDIS_API_KEY")]
    pub api_key: Option<String>,
}

#[cfg(feature = "mock-sge")]
#[derive(Args, Debug)]
pub struct MockArgs {
    /// Adresse IP d'écoute
    #[arg(long, default_value = "0.0.0.0")]
    pub host: String,

    /// Port d'écoute HTTP
    #[arg(long, default_value = "9090")]
    pub port: u16,

    /// Scénario de simulation (success, consent-expired, point-not-found, quota-exceeded)
    #[arg(long, default_value = "success")]
    pub scenario: String,
}

#[derive(Args, Debug)]
pub struct SpotArgs {
    #[command(subcommand)]
    pub command: SpotCommand,
}

#[derive(Subcommand, Debug)]
pub enum SpotCommand {
    /// Consultation des cours horaires du marché de gros Day-Ahead
    Prices(SpotPricesArgs),
    /// Analyse de corrélation Linky et calcul du potentiel d'arbitrage (€/an)
    Arbitrage(SpotArbitrageArgs),
}

#[derive(Args, Debug)]
pub struct SpotPricesArgs {
    /// Date de début (ex: 2026-09-25T00:00:00Z)
    #[arg(long)]
    pub from: Option<String>,

    /// Date de fin (ex: 2026-09-30T00:00:00Z)
    #[arg(long)]
    pub to: Option<String>,

    /// Nombre maximum de créneaux horaires à afficher
    #[arg(long, default_value = "48")]
    pub limit: usize,

    /// Force la synchronisation en ligne auprès de l'API de marché avant affichage
    #[arg(long)]
    pub sync: bool,

    /// Format de sortie : "table" (affichage console) ou "json"
    #[arg(long, default_value = "table")]
    pub format: String,
}

#[derive(Args, Debug)]
pub struct SpotArbitrageArgs {
    /// PRM cible (14 chiffres)
    #[arg(long)]
    pub prm: String,

    /// Date de début (ex: 2026-09-01T00:00:00Z)
    #[arg(long)]
    pub from: Option<String>,

    /// Date de fin (ex: 2026-09-30T00:00:00Z)
    #[arg(long)]
    pub to: Option<String>,

    /// Marge fournisseur en €/kWh (défaut: 0.0150)
    #[arg(long)]
    pub margin: Option<f64>,

    /// Format de sortie : "table" (affichage console) ou "json"
    #[arg(long, default_value = "table")]
    pub format: String,
}
