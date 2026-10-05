use clap::Parser;
use enedis_rs::client::{
    ClientIdentitySource, DataConnectClient, DataConnectConfig, EnedisProvider, SgeClient,
    SgeClientConfig,
};
use secrecy::SecretString;
use std::sync::Arc;

#[path = "enedis/args.rs"]
mod args;
#[path = "enedis/commands.rs"]
mod commands;

use args::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let identity = match (&cli.cert_p12, &cli.cert_password) {
        (Some(p), Some(pw)) => Some(ClientIdentitySource::Pkcs12File {
            path: p.clone(),
            password: SecretString::new(pw.clone()),
        }),
        _ => None,
    };

    let client_config = SgeClientConfig {
        endpoint_url: cli.endpoint.clone(),
        identity: identity.clone(),
        custom_ca_pem: None,
        connect_timeout: std::time::Duration::from_secs(10),
        request_timeout: std::time::Duration::from_secs(30),
        user_agent: format!("enedis-cli/{}", env!("CARGO_PKG_VERSION")),
    };

    let (provider, dc_client_opt): (Arc<dyn EnedisProvider>, Option<Arc<DataConnectClient>>) =
        if cli.provider.to_lowercase() == "dataconnect" {
            let dc_config = DataConnectConfig {
                base_url: cli.dc_url.clone(),
                token_url: format!("{}/oauth2/v3/token", cli.dc_url.trim_end_matches('/')),
                authorize_url: None,
                client_id: cli.dc_client_id.clone(),
                client_secret: cli
                    .dc_client_secret
                    .as_ref()
                    .map(|s| SecretString::new(s.clone())),
                direct_token: cli.dc_token.as_ref().map(|t| SecretString::new(t.clone())),
                connect_timeout: std::time::Duration::from_secs(10),
                request_timeout: std::time::Duration::from_secs(30),
                user_agent: format!("enedis-cli/{}", env!("CARGO_PKG_VERSION")),
            };
            let client = Arc::new(DataConnectClient::new(dc_config)?);
            (client.clone(), Some(client))
        } else {
            let dc_client = if cli.dc_client_id.is_some() {
                let dc_config = DataConnectConfig {
                    base_url: cli.dc_url.clone(),
                    token_url: format!("{}/oauth2/v3/token", cli.dc_url.trim_end_matches('/')),
                    authorize_url: None,
                    client_id: cli.dc_client_id.clone(),
                    client_secret: cli
                        .dc_client_secret
                        .as_ref()
                        .map(|s| SecretString::new(s.clone())),
                    direct_token: cli.dc_token.as_ref().map(|t| SecretString::new(t.clone())),
                    connect_timeout: std::time::Duration::from_secs(10),
                    request_timeout: std::time::Duration::from_secs(30),
                    user_agent: format!("enedis-cli/{}", env!("CARGO_PKG_VERSION")),
                };
                DataConnectClient::new(dc_config).ok().map(Arc::new)
            } else {
                None
            };
            (Arc::new(SgeClient::new(client_config.clone())?), dc_client)
        };

    match cli.command {
        Commands::Doctor(args) => {
            commands::handle_doctor(
                &provider,
                &cli.provider,
                identity.as_ref(),
                &client_config,
                &cli.db_url,
                args,
            )
            .await?;
        }
        Commands::Auth(args) => {
            commands::handle_auth(client_config, args).await?;
        }
        Commands::Point(args) => {
            commands::handle_point(&cli.db_url, args).await?;
        }
        Commands::Measurements(args) => {
            commands::handle_measurements(&provider, client_config, &cli.db_url, args).await?;
        }
        Commands::Contract(args) => {
            commands::handle_contract(&provider, args).await?;
        }
        Commands::MaxPower(args) => {
            commands::handle_max_power(&provider, args).await?;
        }
        Commands::Consent(args) => {
            commands::handle_consent(&provider, args).await?;
        }
        Commands::Costs(args) => {
            commands::handle_costs(&cli.db_url, args).await?;
        }
        Commands::Signals(args) => {
            commands::handle_signals(&cli.db_url, args).await?;
        }
        Commands::Spot(args) => {
            commands::handle_spot(&provider, &cli.db_url, args).await?;
        }
        Commands::Agent(args) => {
            commands::handle_agent(&provider, &cli.db_url, args).await?;
        }
        Commands::Serve(args) => {
            commands::handle_serve(&provider, dc_client_opt.as_ref(), &cli.db_url, args).await?;
        }
        #[cfg(feature = "mock-sge")]
        Commands::Mock(args) => {
            commands::handle_mock(args).await?;
        }
    }

    Ok(())
}
