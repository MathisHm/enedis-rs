use super::args::*;
use chrono::Utc;
use enedis_rs::client::{
    ClientIdentitySource, DataConnectClient, EnedisProvider, SgeClient, SgeClientConfig,
};
use enedis_rs::doctor::EnedisDoctor;
use enedis_rs::models::{
    analyze_spot_consumption, audit_subscription_sizing, calculate_energy_costs,
    correlate_measurements_with_grid, generate_synthetic_spot_profile, BaseTariff, DynamicTariff,
    FlowDirection, HpHcTariff, PointId, TariffConfig, TempoTariff, TimeSlot,
};
#[cfg(feature = "mqtt")]
use secrecy::SecretString;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

#[cfg(feature = "storage-postgres")]
use enedis_rs::storage::PostgresStorage;
#[cfg(feature = "storage-sqlite")]
use enedis_rs::storage::SqliteStorage;
#[cfg(feature = "storage")]
use enedis_rs::storage::StorageBackend;

#[cfg(feature = "storage")]
pub async fn init_storage(
    db_url: &str,
) -> Result<Arc<dyn StorageBackend>, Box<dyn std::error::Error>> {
    #[cfg(feature = "storage-postgres")]
    if db_url.starts_with("postgres://") || db_url.starts_with("postgresql://") {
        let storage = PostgresStorage::connect(db_url).await?;
        return Ok(Arc::new(storage));
    }

    #[cfg(feature = "storage-sqlite")]
    {
        let storage = SqliteStorage::connect(db_url).await?;
        return Ok(Arc::new(storage));
    }

    #[allow(unreachable_code)]
    Err(format!(
        "Aucun backend de stockage compatible activé pour '{}' (vérifiez features storage-sqlite ou storage-postgres)",
        db_url
    ).into())
}

pub async fn handle_doctor(
    provider: &Arc<dyn EnedisProvider>,
    provider_name: &str,
    identity: Option<&ClientIdentitySource>,
    client_config: &SgeClientConfig,
    db_url: &str,
    args: DoctorArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "🔍 Démarrage du diagnostic enedis doctor (fournisseur: {})...",
        provider.provider_name()
    );
    let mut reports = Vec::new();

    // 1. Vérification du certificat (si SGE)
    if provider_name.to_lowercase() == "sge" {
        reports.push(EnedisDoctor::check_certificate(identity));

        // 2. Vérification réseau et mTLS
        if let Ok(client) = SgeClient::new(client_config.clone()) {
            reports.push(EnedisDoctor::check_network_and_tls(&client).await);

            // 3. Vérification PRM si fourni
            if let Some(prm_str) = args.prm {
                if let Ok(prm) = PointId::new(&prm_str) {
                    reports.push(EnedisDoctor::check_prm(&client, prm).await);
                    reports.push(
                        EnedisDoctor::check_consent_proactive(provider.as_ref(), prm, 30).await,
                    );
                    reports.push(EnedisDoctor::check_contract(provider.as_ref(), prm).await);
                } else {
                    eprintln!("⚠️ PRM invalide: {}", prm_str);
                }
            }
        }
    } else if let Some(prm_str) = args.prm {
        if let Ok(prm) = PointId::new(&prm_str) {
            reports.push(EnedisDoctor::check_consent_proactive(provider.as_ref(), prm, 30).await);
            reports.push(EnedisDoctor::check_contract(provider.as_ref(), prm).await);
        } else {
            eprintln!("⚠️ PRM invalide: {}", prm_str);
        }
    }

    // 4. Vérification de la base de données
    #[cfg(feature = "storage")]
    match init_storage(db_url).await {
        Ok(storage) => {
            reports.push(EnedisDoctor::check_storage(storage.as_ref()).await);
        }
        Err(err) => {
            reports.push(enedis_rs::doctor::DoctorReportItem {
                step: "4. Persistance & Base de données temporelle",
                status: enedis_rs::doctor::DoctorStatus::Failure {
                    reason: format!("Erreur d'accès à la base de données: {}", err),
                    remediation:
                        "Vérifiez que la base de données est accessible à l'adresse spécifiée."
                            .to_string(),
                },
            });
        }
    }

    println!("{}", EnedisDoctor::format_report(&reports));
    Ok(())
}

pub async fn handle_auth(
    client_config: SgeClientConfig,
    args: AuthArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        AuthSubcommands::Test => {
            println!(
                "🔒 Test de connexion mTLS vers {}...",
                client_config.endpoint_url
            );
            match SgeClient::new(client_config) {
                Ok(client) => {
                    let rep = EnedisDoctor::check_network_and_tls(&client).await;
                    println!("{:?}", rep.status);
                }
                Err(e) => eprintln!("❌ Erreur d'initialisation du client: {}", e),
            }
        }
    }
    Ok(())
}

pub async fn handle_point(db_url: &str, args: PointArgs) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        PointSubcommands::List => {
            #[cfg(feature = "storage")]
            {
                let storage = init_storage(db_url).await?;
                let points = storage.list_sync_points().await?;
                println!(
                    "📋 {} point(s) de mesure (PRM) configuré(s) en base ({}):",
                    points.len(),
                    db_url
                );
                for p in points {
                    let c_state = storage
                        .get_sync_state(p, FlowDirection::Consumption)
                        .await?;
                    let p_state = storage.get_sync_state(p, FlowDirection::Production).await?;
                    let c_info = c_state
                        .map(|s| {
                            format!(
                                "Consommation: {} ({})",
                                s.sync_status, s.last_synced_timestamp
                            )
                        })
                        .unwrap_or_else(|| "Consommation: non synchronisé".to_string());
                    let p_info = p_state
                        .map(|s| {
                            format!(
                                "Production: {} ({})",
                                s.sync_status, s.last_synced_timestamp
                            )
                        })
                        .unwrap_or_else(|| "Production: non configuré".to_string());
                    println!("   • PRM {} => {}, {}", p, c_info, p_info);
                }
            }
            #[cfg(not(feature = "storage"))]
            {
                let _ = db_url;
                eprintln!("❌ Fonctionnalité stockage non activée.");
            }
        }
        PointSubcommands::Get { prm } => {
            #[cfg(feature = "storage")]
            {
                let storage = init_storage(db_url).await?;
                let point_id = PointId::new(&prm)?;
                println!("ℹ️ Détails de synchronisation pour le PRM {} :", point_id);
                let cons = storage
                    .get_sync_state(point_id, FlowDirection::Consumption)
                    .await?;
                let prod = storage
                    .get_sync_state(point_id, FlowDirection::Production)
                    .await?;
                match cons {
                    Some(s) => println!(
                        "   - Consommation : Statut={}, Dernier sync={}, Dernier essai={}",
                        s.sync_status, s.last_synced_timestamp, s.last_sync_attempt
                    ),
                    None => println!("   - Consommation : Aucune synchronisation enregistrée"),
                }
                match prod {
                    Some(s) => println!(
                        "   - Production   : Statut={}, Dernier sync={}, Dernier essai={}",
                        s.sync_status, s.last_synced_timestamp, s.last_sync_attempt
                    ),
                    None => println!("   - Production   : Aucune synchronisation enregistrée"),
                }
            }
            #[cfg(not(feature = "storage"))]
            {
                let _ = (db_url, prm);
                eprintln!("❌ Fonctionnalité stockage non activée.");
            }
        }
        #[cfg(feature = "mqtt")]
        PointSubcommands::MqttPublish {
            prm,
            broker,
            prefix,
        } => {
            #[cfg(feature = "storage")]
            {
                use enedis_rs::mqtt::{MqttPublisher, MqttPublisherConfig};
                let storage = init_storage(db_url).await?;
                let point_id = PointId::new(&prm)?;
                println!(
                    "📡 Publication Home Assistant Discovery & État pour {} vers {} (topic: {})...",
                    point_id, broker, prefix
                );
                let mqtt_config = MqttPublisherConfig {
                    broker_url: broker,
                    topic_prefix: prefix,
                    ..Default::default()
                };
                let (publisher, _bg) = MqttPublisher::start(mqtt_config)?;
                publisher
                    .publish_prm_update(storage.as_ref(), point_id)
                    .await?;
                // Attendre brièvement que la boucle MQTT flush les paquets
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                println!(
                    "✅ Capteurs Home Assistant enregistrés et état consolidé publié avec succès !"
                );
            }
            #[cfg(not(feature = "storage"))]
            {
                let _ = (db_url, prm, broker, prefix);
                eprintln!("❌ Fonctionnalité stockage non activée.");
            }
        }
    }
    Ok(())
}

pub async fn handle_measurements(
    provider: &Arc<dyn EnedisProvider>,
    client_config: SgeClientConfig,
    db_url: &str,
    args: MeasurementsArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        MeasurementsSubcommands::Fetch {
            prm,
            from,
            to,
            direction,
        } => {
            let point_id = PointId::from_str(&prm)?;
            let from_dt = chrono::DateTime::parse_from_rfc3339(&from)?.with_timezone(&Utc);
            let to_dt = chrono::DateTime::parse_from_rfc3339(&to)?.with_timezone(&Utc);
            let flow_dir = FlowDirection::from_sge_code(&direction);

            println!(
                "⚡ Téléchargement des mesures pour le PRM {} ({} via {})...",
                point_id,
                flow_dir,
                provider.provider_name()
            );

            match provider
                .fetch_measurements(point_id, from_dt, to_dt, flow_dir)
                .await
            {
                Ok(measurements) => {
                    println!("✅ {} mesures reçues avec succès !", measurements.len());
                    for m in measurements.iter().take(5) {
                        println!(
                            "   - {} : {} {} ({:?})",
                            m.timestamp, m.value, m.unit, m.quality
                        );
                    }
                    if measurements.len() > 5 {
                        println!("   ... et {} autres mesures.", measurements.len() - 5);
                    }

                    #[cfg(feature = "storage")]
                    if let Ok(storage) = init_storage(db_url).await {
                        let stats = storage.upsert_measurements(&measurements).await?;
                        println!(
                            "💾 Mesures enregistrées en base ({} traitées, {} affectées).",
                            stats.processed, stats.affected
                        );
                    }
                }
                Err(err) => {
                    eprintln!("❌ Échec de la collecte : {}", err);
                    eprintln!("💡 Diagnostic doctor : {}", err.doctor_explanation());
                }
            }
        }
        MeasurementsSubcommands::Backfill {
            prm,
            from,
            to,
            direction,
        } => {
            #[cfg(all(feature = "agent", feature = "storage", feature = "client"))]
            {
                use chrono::DateTime;
                use enedis_rs::agent::{
                    CollectorConfig, CollectorDaemon, SgeRateLimiter, ShutdownSignal,
                };
                let storage = init_storage(db_url).await?;
                let client = SgeClient::new(client_config)?;
                let point_id = PointId::new(&prm)?;
                let from_dt = DateTime::parse_from_rfc3339(&from)?.with_timezone(&Utc);
                let to_dt = DateTime::parse_from_rfc3339(&to)?.with_timezone(&Utc);
                let flow_dir = FlowDirection::from_sge_code(&direction);
                let rate_limiter = SgeRateLimiter::new(20, std::time::Duration::from_millis(1500));
                let daemon = CollectorDaemon::new(
                    client,
                    storage,
                    rate_limiter,
                    CollectorConfig::default(),
                    ShutdownSignal::new(),
                );

                println!(
                    "🔍 Analyse fine des trous pour {} [{}] de {} à {}...",
                    point_id, flow_dir, from_dt, to_dt
                );
                let stats = daemon
                    .backfill_missing_ranges(point_id, flow_dir, from_dt, to_dt)
                    .await?;
                println!("✅ Rattrapage chirurgical terminé avec succès :");
                println!(
                    "   - Plages manquantes détectées : {}",
                    stats.ranges_detected
                );
                println!("   - Requêtes SGE effectuées : {}", stats.requests_made);
                println!(
                    "   - Mesures récupérées et persistées : {}",
                    stats.measurements_recovered
                );
            }
            #[cfg(not(all(feature = "agent", feature = "storage", feature = "client")))]
            {
                let _ = (prm, from, to, direction, client_config, db_url);
                eprintln!("❌ Le rattrapage (backfill) requiert les features 'agent', 'storage' et 'client'.");
            }
        }
        MeasurementsSubcommands::Rollup {
            prm,
            older_than_days,
            interval,
            vacuum,
        } => {
            #[cfg(feature = "storage")]
            {
                use enedis_rs::models::AggregationInterval;
                use enedis_rs::storage::RetentionPolicy;
                let storage = init_storage(db_url).await?;
                let point_id = prm.as_deref().map(PointId::new).transpose()?;
                let rollup_interval = match interval.to_lowercase().as_str() {
                    "hourly" | "hour" | "1h" => AggregationInterval::Hourly,
                    _ => AggregationInterval::Daily,
                };
                let policy =
                    RetentionPolicy::new(older_than_days, rollup_interval).with_auto_vacuum(vacuum);

                println!("📦 Application du compactage (Rollup / Downsampling) :");
                println!(
                    "   - Seuil d'archivage des données fines : {} jours",
                    older_than_days
                );
                println!("   - Intervalle cible : {:?}", rollup_interval);
                println!(
                    "   - PRM cible : {}",
                    point_id
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "Tous les compteurs".to_string())
                );

                let stats = storage.apply_retention_policy(point_id, &policy).await?;
                println!("✅ Compactage terminé :");
                println!(
                    "   - Mesures brutes traitées : {}",
                    stats.raw_measurements_processed
                );
                println!(
                    "   - Agrégats compactés insérés : {}",
                    stats.rollups_created
                );
                println!(
                    "   - Mesures brutes supprimées : {}",
                    stats.raw_measurements_deleted
                );
                if stats.vacuum_executed {
                    println!("   - Nettoyage VACUUM exécuté avec succès (espace libéré).");
                }
            }
            #[cfg(not(feature = "storage"))]
            {
                let _ = (prm, older_than_days, interval, vacuum, db_url);
                eprintln!("❌ Fonctionnalité stockage non activée.");
            }
        }
        MeasurementsSubcommands::Export {
            prm,
            format,
            interval,
            output,
        } => {
            #[cfg(feature = "storage")]
            {
                let storage = init_storage(db_url).await?;
                let point_id = PointId::new(&prm)?;
                let from = Utc::now() - chrono::Duration::days(365 * 3);
                let to = Utc::now();

                if let Some(interval_str) = interval {
                    use enedis_rs::models::AggregationInterval;
                    let agg_interval = AggregationInterval::from_str(&interval_str)?;
                    let aggregates = storage
                        .get_aggregated_measurements(point_id, from, to, agg_interval, None)
                        .await?;

                    if format.eq_ignore_ascii_case("csv") {
                        let mut csv = String::from("point_id,bucket_start,bucket_end,direction,total_energy_kwh,max_power_w,min_power_w,avg_power_w,sample_count\n");
                        for a in &aggregates {
                            csv.push_str(&format!(
                                "{},{},{},{},{},{},{},{},{}\n",
                                a.point_id,
                                a.bucket_start.to_rfc3339(),
                                a.bucket_end.to_rfc3339(),
                                a.direction,
                                a.total_energy_kwh,
                                a.max_power_w.map(|v| v.to_string()).unwrap_or_default(),
                                a.min_power_w.map(|v| v.to_string()).unwrap_or_default(),
                                a.avg_power_w.map(|v| v.to_string()).unwrap_or_default(),
                                a.sample_count
                            ));
                        }
                        if let Some(ref path) = output {
                            std::fs::write(path, csv)?;
                            println!(
                                "✅ {} agrégats exportés au format CSV vers {:?}",
                                aggregates.len(),
                                path
                            );
                        } else {
                            print!("{}", csv);
                        }
                    } else {
                        let json = serde_json::to_string_pretty(&aggregates)?;
                        if let Some(ref path) = output {
                            std::fs::write(path, json)?;
                            println!(
                                "✅ {} agrégats exportés au format JSON vers {:?}",
                                aggregates.len(),
                                path
                            );
                        } else {
                            println!("{}", json);
                        }
                    }
                } else {
                    let measurements = storage.get_measurements(point_id, from, to, None).await?;

                    match format.to_lowercase().as_str() {
                        "csv" => {
                            let mut csv = String::from("point_id,timestamp,direction,interval_seconds,value,unit,quality\n");
                            for m in &measurements {
                                csv.push_str(&format!(
                                    "{},{},{},{},{},{},{}\n",
                                    m.point_id,
                                    m.timestamp.to_rfc3339(),
                                    m.direction,
                                    m.interval_seconds,
                                    m.value,
                                    m.unit,
                                    m.quality
                                ));
                            }
                            if let Some(ref path) = output {
                                std::fs::write(path, csv)?;
                                println!(
                                    "✅ {} mesures exportées au format CSV vers {:?}",
                                    measurements.len(),
                                    path
                                );
                            } else {
                                print!("{}", csv);
                            }
                        }
                        "influxdb" | "lineprotocol" | "lp" => {
                            let lp = enedis_rs::storage::influxdb::measurements_to_line_protocol(
                                &measurements,
                                None,
                            );
                            if let Some(ref path) = output {
                                std::fs::write(path, lp)?;
                                println!("✅ {} mesures exportées au format InfluxDB Line Protocol vers {:?}", measurements.len(), path);
                            } else {
                                print!("{}", lp);
                            }
                        }
                        "parquet" => {
                            #[cfg(feature = "parquet")]
                            {
                                let out_path = output.unwrap_or_else(|| {
                                    PathBuf::from(format!("{}_measurements.parquet", point_id))
                                });
                                enedis_rs::storage::parquet::ParquetExporter::export_to_file(
                                    &out_path,
                                    &measurements,
                                )?;
                                println!(
                                    "✅ {} mesures exportées au format Apache Parquet vers {:?}",
                                    measurements.len(),
                                    out_path
                                );
                            }
                            #[cfg(not(feature = "parquet"))]
                            eprintln!(
                                "❌ L'export Parquet nécessite d'activer la feature 'parquet'."
                            );
                        }
                        "duckdb" => {
                            #[cfg(feature = "parquet")]
                            {
                                let parquet_path =
                                    PathBuf::from(format!("{}_measurements.parquet", point_id));
                                enedis_rs::storage::parquet::ParquetExporter::export_to_file(
                                    &parquet_path,
                                    &measurements,
                                )?;
                                let sql_path = output.unwrap_or_else(|| {
                                    PathBuf::from(format!("{}_duckdb.sql", point_id))
                                });
                                let script = enedis_rs::storage::duckdb::DuckDbHelper::generate_parquet_view_script(&parquet_path, Some("enedis_measurements"));
                                enedis_rs::storage::duckdb::DuckDbHelper::write_script_file(
                                    &sql_path, &script,
                                )?;
                                println!("✅ Fichier Parquet créé ({:?}) et script analytique DuckDB généré avec succès dans {:?}", parquet_path, sql_path);
                            }
                            #[cfg(not(feature = "parquet"))]
                            {
                                let sql_path =
                                    output.unwrap_or_else(|| PathBuf::from("duckdb_sqlite.sql"));
                                let script = enedis_rs::storage::duckdb::DuckDbHelper::generate_sqlite_attach_script(&PathBuf::from("enedis.db"));
                                enedis_rs::storage::duckdb::DuckDbHelper::write_script_file(
                                    &sql_path, &script,
                                )?;
                                println!("✅ Script DuckDB SQLite généré dans {:?}", sql_path);
                            }
                        }
                        _ => {
                            let json = serde_json::to_string_pretty(&measurements)?;
                            if let Some(ref path) = output {
                                std::fs::write(path, json)?;
                                println!(
                                    "✅ {} mesures exportées au format JSON vers {:?}",
                                    measurements.len(),
                                    path
                                );
                            } else {
                                println!("{}", json);
                            }
                        }
                    }
                }
            }
            #[cfg(not(feature = "storage"))]
            {
                let _ = (prm, format, interval, output, db_url);
                eprintln!("❌ Fonctionnalité stockage non activée.");
            }
        }
    }
    Ok(())
}

pub async fn handle_contract(
    provider: &Arc<dyn EnedisProvider>,
    args: ContractArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let prm = PointId::new(&args.prm)?;
    println!(
        "📋 Données contractuelles pour le PRM {} (via {})...",
        prm,
        provider.provider_name()
    );
    match provider.fetch_contract_data(prm).await {
        Ok(contract) => {
            println!("   - Point de livraison : {}", contract.point_id);
            println!(
                "   - Puissance souscrite : {} kVA",
                contract.subscribed_power_kva
            );
            println!("   - Option tarifaire    : {:?}", contract.tariff_option);
            if let Some(cal) = &contract.calendar {
                println!(
                    "   - Calendrier          : {}",
                    cal.schedule_name.as_deref().unwrap_or("Standard")
                );
                if !cal.off_peak_ranges.is_empty() {
                    println!(
                        "   - Heures creuses      : {}",
                        cal.off_peak_ranges.join(", ")
                    );
                }
            }
            if let Some(meter) = &contract.meter {
                println!("   - Type de compteur    : {:?}", meter.meter_type);
                println!("   - Raccordement        : {:?}", meter.phase_count);
                if let Some(sn) = &meter.serial_number {
                    println!("   - N° Série compteur   : {}", sn);
                }
            }
            if let Some(st) = &contract.status {
                println!("   - Statut contrat      : {}", st);
            }
        }
        Err(err) => {
            eprintln!("❌ Erreur données contractuelles : {}", err);
            eprintln!("💡 Diagnostic doctor : {}", err.doctor_explanation());
        }
    }
    Ok(())
}

pub async fn handle_max_power(
    provider: &Arc<dyn EnedisProvider>,
    args: MaxPowerArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let prm = PointId::new(&args.prm)?;
    let from_dt = chrono::DateTime::parse_from_rfc3339(&args.from)?.with_timezone(&Utc);
    let to_dt = chrono::DateTime::parse_from_rfc3339(&args.to)?.with_timezone(&Utc);

    println!(
        "⚡ Pointes maximales de puissance pour le PRM {} (via {})...",
        prm,
        provider.provider_name()
    );
    match provider.fetch_daily_max_power(prm, from_dt, to_dt).await {
        Ok(records) => {
            println!("✅ {} pointes de puissance relevées.", records.len());
            for r in &records {
                println!(
                    "   - {} : {} {} ({:.2} kVA)",
                    r.timestamp,
                    r.value,
                    r.unit,
                    r.value_kva()
                );
            }
            if args.audit {
                let sub_kva = if let Ok(c) = provider.fetch_contract_data(prm).await {
                    c.subscribed_power_kva
                } else {
                    6
                };
                let audit = audit_subscription_sizing(prm, sub_kva, &records);
                println!("\n🔍 --- AUDIT DE DIMENSIONNEMENT D'ABONNEMENT ---");
                println!(
                    "   - Puissance souscrite actuelle : {} kVA",
                    audit.subscribed_power_kva
                );
                println!(
                    "   - Pointe maximale observée     : {:.2} kVA",
                    audit.peak_reached_kva
                );
                println!(
                    "   - Diagnostic d'adéquation      : {:?}",
                    audit.sizing_status
                );
                println!(
                    "   - Puissance recommandée        : {} kVA",
                    audit.recommended_power_kva
                );
                println!(
                    "   - Préconisation                : {}",
                    audit.recommendation
                );
            }
        }
        Err(err) => {
            eprintln!("❌ Erreur puissance maximale : {}", err);
            eprintln!("💡 Diagnostic doctor : {}", err.doctor_explanation());
        }
    }
    Ok(())
}

pub async fn handle_consent(
    provider: &Arc<dyn EnedisProvider>,
    args: ConsentArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let prm = PointId::new(&args.prm)?;
    println!(
        "🔐 Suivi proactif du consentement pour le PRM {} (via {})...",
        prm,
        provider.provider_name()
    );
    match provider.fetch_consent_status(prm).await {
        Ok(consent) => {
            println!("   - Statut courant : {:?}", consent.status);
            println!("   - Début validité : {:?}", consent.valid_from);
            println!("   - Fin validité   : {:?}", consent.valid_to);
            if !consent.authorized_usages.is_empty() {
                println!(
                    "   - Usages autorisés : {}",
                    consent.authorized_usages.join(", ")
                );
            }
            let alert = consent.check_alert(args.warning_days, Utc::now());
            println!("   - Niveau alerte  : {:?}", alert.severity);
            println!("   - Jours restants : {}", alert.days_remaining);
            println!("   - Diagnostic     : {}", alert.message);
        }
        Err(err) => {
            eprintln!("❌ Erreur vérification consentement : {}", err);
            eprintln!("💡 Diagnostic doctor : {}", err.doctor_explanation());
        }
    }
    Ok(())
}

pub async fn handle_costs(db_url: &str, args: CostsArgs) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "storage")]
    {
        let storage = init_storage(db_url).await?;
        let point_id = PointId::new(&args.prm)?;
        let from = chrono::DateTime::parse_from_rfc3339(&args.from)?.with_timezone(&Utc);
        let to = chrono::DateTime::parse_from_rfc3339(&args.to)?.with_timezone(&Utc);

        let measurements = storage
            .get_measurements(point_id, from, to, Some(FlowDirection::Consumption))
            .await?;

        let tariff_type = args.tariff.to_lowercase();
        let (tariff, tempo_days) = match tariff_type.as_str() {
            "tempo" => {
                let from_date = from.date_naive() - chrono::Duration::days(1);
                let to_date = to.date_naive() + chrono::Duration::days(1);
                let days = storage
                    .get_tempo_days(from_date, to_date)
                    .await
                    .unwrap_or_default();
                (TariffConfig::Tempo(TempoTariff::default()), Some(days))
            }
            "hphc" | "hp_hc" | "heures_creuses" => {
                let mut h = HpHcTariff::default();
                if let Some(ref hp) = args.hp_price {
                    h.hp_price_per_kwh = rust_decimal::Decimal::from_str(hp)?;
                }
                if let Some(ref hc) = args.hc_price {
                    h.hc_price_per_kwh = rust_decimal::Decimal::from_str(hc)?;
                }
                if let Some(ref sub) = args.subscription {
                    h.monthly_subscription = rust_decimal::Decimal::from_str(sub)?;
                }
                if let Some(ref slots_str) = args.off_peak_slots {
                    let mut slots = Vec::new();
                    for part in slots_str.split(',') {
                        if let Ok(slot) = TimeSlot::parse(part) {
                            slots.push(slot);
                        }
                    }
                    if !slots.is_empty() {
                        h.off_peak_slots = slots;
                    }
                }
                (TariffConfig::HpHc(h), None)
            }
            "dynamic" | "dynamique" => (TariffConfig::Dynamic(DynamicTariff::default()), None),
            _ => {
                let mut b = BaseTariff::default();
                if let Some(ref price) = args.base_price {
                    b.price_per_kwh = rust_decimal::Decimal::from_str(price)?;
                }
                if let Some(ref sub) = args.subscription {
                    b.monthly_subscription = rust_decimal::Decimal::from_str(sub)?;
                }
                (TariffConfig::Base(b), None)
            }
        };

        let costs = calculate_energy_costs(
            point_id,
            from,
            to,
            &measurements,
            &tariff,
            None,
            tempo_days.as_deref(),
        )?;

        println!("============================================================");
        println!(" 💶 FACTURE ÉNERGÉTIQUE ESTIMÉE - PRM {}", costs.point_id);
        println!("============================================================");
        println!(
            " Période       : {} à {}",
            costs.from.to_rfc3339(),
            costs.to.to_rfc3339()
        );
        println!(" Formule       : {}", costs.tariff_type);
        println!(" Énergie totale: {} kWh", costs.total_energy_kwh);
        println!("------------------------------------------------------------");
        for b in &costs.breakdown {
            println!(
                " - {:<24} : {:>8} kWh ({:>5}%) | {} €/kWh -> {:>8} € HT",
                b.bucket_name, b.energy_kwh, b.percentage_of_energy, b.unit_price, b.total_cost_ht
            );
        }
        println!("------------------------------------------------------------");
        println!(
            " Consommation HT : {:>8} €",
            costs.total_consumption_cost_ht
        );
        println!(
            " Abonnement fixe : {:>8} €",
            costs.total_subscription_cost_ht
        );
        println!(" Taxes (TVA+TICFE): {:>8} €", costs.total_taxes);
        println!(" TOTAL ESTIMÉ TTC: {:>8} €", costs.total_cost_ttc);
        println!(
            " Coût moyen kWh  : {:>8} €/kWh",
            costs.average_cost_per_kwh_ttc
        );

        if let Some(ref comp) = costs.comparison_with_base {
            println!("------------------------------------------------------------");
            println!(
                " Comparatif vs Tarif Base : Base = {} € TTC",
                comp.base_total_cost_ttc
            );
            if comp.savings_amount_ttc >= rust_decimal::Decimal::ZERO {
                println!(
                    " Économies estimées       : +{} € ({}%)",
                    comp.savings_amount_ttc, comp.savings_percentage
                );
            } else {
                println!(
                    " Surcoût estimé           : {} € ({}%)",
                    comp.savings_amount_ttc, comp.savings_percentage
                );
            }
        }
        println!("============================================================");
    }
    #[cfg(not(feature = "storage"))]
    {
        let _ = (db_url, args);
        eprintln!("❌ Fonctionnalité stockage non activée.");
    }
    Ok(())
}

pub async fn handle_signals(
    db_url: &str,
    args: SignalsArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(all(feature = "storage", feature = "client"))]
    {
        use enedis_rs::client::NetworkSignalClient;
        let storage = init_storage(db_url).await?;
        let client = NetworkSignalClient::with_default_config()?;

        if args.sync {
            println!("🔄 Synchronisation des signaux réseau (RTE EcoWatt & Tempo)...");
            client.sync_to_storage(&*storage).await?;
        }

        match args.action.to_lowercase().as_str() {
            "ecowatt" => {
                let now = Utc::now();
                let signals = storage
                    .get_ecowatt_signals(
                        now - chrono::Duration::hours(24),
                        now + chrono::Duration::hours(48),
                    )
                    .await?;
                println!("============================================================");
                println!(" ⚡ SIGNAUX DU RÉSEAU ÉLECTRIQUE - RTE ECOWATT");
                println!("============================================================");
                for s in signals {
                    let icon = match s.level {
                        enedis_rs::models::EcoWattLevel::Green => "🟢",
                        enedis_rs::models::EcoWattLevel::Orange => "🟠",
                        enedis_rs::models::EcoWattLevel::Red => "🔴",
                    };
                    println!(
                        " {} {:<24} : {:<7} | {}",
                        icon,
                        s.timestamp.to_rfc3339(),
                        s.level.as_str(),
                        s.message.unwrap_or_default()
                    );
                }
                println!("============================================================");
            }
            "correlation" => {
                let prm_str = args.prm.ok_or_else(|| {
                    enedis_rs::EnedisError::Configuration(
                        "Paramètre --prm requis pour correlation".to_string(),
                    )
                })?;
                let point_id = PointId::new(&prm_str)?;
                let from = args
                    .from
                    .as_deref()
                    .map(|s| chrono::DateTime::parse_from_rfc3339(s).map(|d| d.with_timezone(&Utc)))
                    .transpose()?
                    .unwrap_or_else(|| Utc::now() - chrono::Duration::days(30));
                let to = args
                    .to
                    .as_deref()
                    .map(|s| chrono::DateTime::parse_from_rfc3339(s).map(|d| d.with_timezone(&Utc)))
                    .transpose()?
                    .unwrap_or_else(Utc::now);

                let measurements = storage
                    .get_measurements(point_id, from, to, Some(FlowDirection::Consumption))
                    .await?;
                let ecowatt_signals = storage
                    .get_ecowatt_signals(from, to)
                    .await
                    .unwrap_or_default();
                let tempo_days = storage
                    .get_tempo_days(
                        from.date_naive() - chrono::Duration::days(1),
                        to.date_naive() + chrono::Duration::days(1),
                    )
                    .await
                    .ok();

                let rep = correlate_measurements_with_grid(
                    point_id,
                    from,
                    to,
                    &measurements,
                    &ecowatt_signals,
                    tempo_days.as_deref(),
                );
                println!("============================================================");
                println!(
                    " 📊 CORRÉLATION CONSOMMATION & ALERTES RÉSEAU - PRM {}",
                    rep.point_id
                );
                println!("============================================================");
                println!(" Énergie totale      : {} kWh", rep.total_consumption_kwh);
                println!(
                    " Part en tension (EcoWatt): {}% ({} kWh)",
                    rep.ecowatt.tension_energy_percentage,
                    rep.ecowatt.orange_energy_kwh + rep.ecowatt.red_energy_kwh
                );
                println!(
                    " Effacement en pic (Rouge): {}%",
                    rep.ecowatt.red_flexibility_score_percentage
                );
                if let Some(ref t) = rep.tempo {
                    println!(
                        " Report HC Jours Rouges   : {}%",
                        t.red_days_off_peak_ratio_percentage
                    );
                }
                println!(" Diagnostic          : {}", rep.assessment);
                println!("============================================================");
            }
            _ => {
                let now = Utc::now().date_naive();
                let records = storage
                    .get_tempo_days(
                        now - chrono::Duration::days(7),
                        now + chrono::Duration::days(1),
                    )
                    .await?;
                println!("============================================================");
                println!(" 🎨 CALENDRIER TEMPO (EDF / RTE)");
                println!("============================================================");
                for r in records {
                    let icon = match r.color {
                        enedis_rs::models::TempoColor::Blue => "🔵 BLEU",
                        enedis_rs::models::TempoColor::White => "⚪ BLANC",
                        enedis_rs::models::TempoColor::Red => "🔴 ROUGE",
                        enedis_rs::models::TempoColor::Unknown => "❓ INCONNU",
                    };
                    println!(" - Date : {} -> {}", r.date, icon);
                }
                println!("============================================================");
            }
        }
    }
    #[cfg(not(all(feature = "storage", feature = "client")))]
    {
        let _ = (db_url, args);
        eprintln!("❌ Signaux réseau nécessitent les features 'storage' et 'client'.");
    }
    Ok(())
}

pub async fn handle_agent(
    provider: &Arc<dyn EnedisProvider>,
    db_url: &str,
    args: AgentArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(all(feature = "agent", feature = "storage"))]
    {
        use enedis_rs::agent::{CollectorConfig, CollectorDaemon, SgeRateLimiter, ShutdownSignal};
        let storage = init_storage(db_url).await?;
        let rate_limiter = SgeRateLimiter::new(
            args.rate_limit_per_minute,
            std::time::Duration::from_millis(1500),
        );
        let mut config = CollectorConfig {
            cycle_interval: std::time::Duration::from_secs(args.interval_secs),
            concurrency: args.concurrency,
            enable_backfill: args.backfill,
            backfill_lookback_days: args.backfill_lookback,
            ..CollectorConfig::default()
        };

        if let Some(ref cron_expr) = args.cron {
            config = config.with_cron(cron_expr)?;
            println!("⏱️ Planification avancée Cron configurée : '{}'", cron_expr);
        }

        if let Some(days) = args.retention_days {
            use enedis_rs::models::AggregationInterval;
            use enedis_rs::storage::RetentionPolicy;
            let rollup_interval = match args.rollup_interval.to_lowercase().as_str() {
                "hourly" | "hour" | "1h" => AggregationInterval::Hourly,
                _ => AggregationInterval::Daily,
            };
            config = config.with_retention_policy(RetentionPolicy::new(days, rollup_interval));
            println!("🧹 Politique de rétention et compression activée (compactage des données > {} jours en {:?})", days, rollup_interval);
        }

        let shutdown = ShutdownSignal::new();
        let shutdown_clone = shutdown.clone();

        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            println!("\n🛑 Interruption reçue, arrêt ordonné de l'agent en cours...");
            shutdown_clone.cancel();
        });

        let mut daemon = CollectorDaemon::new(
            Arc::clone(provider),
            storage,
            rate_limiter,
            config,
            shutdown,
        );

        #[cfg(feature = "mqtt")]
        if let Some(broker_url) = args.mqtt_broker {
            use enedis_rs::mqtt::{MqttPublisher, MqttPublisherConfig};
            let mqtt_config = MqttPublisherConfig {
                broker_url: broker_url.clone(),
                topic_prefix: args.mqtt_prefix.clone(),
                username: args.mqtt_user,
                password: args.mqtt_password.map(SecretString::new),
                ..Default::default()
            };
            match MqttPublisher::start(mqtt_config) {
                Ok((publisher, _bg_handle)) => {
                    println!(
                        "📡 Publication MQTT activée vers {} (préfixe: {})...",
                        broker_url, args.mqtt_prefix
                    );
                    daemon = daemon.with_mqtt_publisher(Arc::new(publisher));
                }
                Err(e) => {
                    eprintln!("⚠️ Impossible d'initialiser le client MQTT: {}", e);
                }
            }
        }

        #[cfg(not(feature = "mqtt"))]
        if args.mqtt_broker.is_some() {
            eprintln!("⚠️ Option --mqtt-broker spécifiée mais la feature 'mqtt' n'a pas été activée lors de la compilation.");
        }

        let sched_desc = if let Some(ref c) = args.cron {
            format!("cron '{}'", c)
        } else {
            format!("cycle: {}s", args.interval_secs)
        };
        println!(
            "🚀 Démarrage de l'agent de collecte continue ({}, limite: {} req/min, concurrence: {} PRM)...",
            sched_desc, args.rate_limit_per_minute, args.concurrency
        );
        daemon.run().await?;
    }
    #[cfg(not(all(feature = "agent", feature = "storage")))]
    {
        let _ = (provider, db_url, args);
        eprintln!("❌ L'agent nécessite les features 'agent' et 'storage'.");
    }
    Ok(())
}

pub async fn handle_serve(
    provider: &Arc<dyn EnedisProvider>,
    dc_client: Option<&Arc<DataConnectClient>>,
    db_url: &str,
    args: ServeArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(all(feature = "api", feature = "storage"))]
    {
        use enedis_rs::api::{ApiServer, AppState};
        use enedis_rs::metrics::MetricsRegistry;
        use enedis_rs::signal::ShutdownSignal;
        use std::net::SocketAddr;

        let storage = init_storage(db_url).await?;
        let metrics = MetricsRegistry::new();
        let mut state = AppState::new(storage, Some(Arc::clone(provider)), metrics);
        if let Some(dc) = dc_client {
            state = state.with_data_connect_client(Arc::clone(dc));
        }
        if let Some(key) = args.api_key {
            state = state.with_api_key(Some(key));
        }

        let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
        let shutdown = ShutdownSignal::new();
        let shutdown_clone = shutdown.clone();

        tokio::spawn(async move {
            tokio::signal::ctrl_c().await.ok();
            println!("\n🛑 Interruption reçue, arrêt gracieux du serveur HTTP...");
            shutdown_clone.cancel();
        });

        println!(
            "🌐 Démarrage du serveur API HTTP REST sur http://{}...",
            addr
        );
        ApiServer::run(state, addr, shutdown).await?;
    }
    #[cfg(not(all(feature = "api", feature = "storage")))]
    {
        let _ = (provider, db_url, args);
        eprintln!("❌ Le serveur API nécessite les features 'api' et 'storage'.");
    }
    Ok(())
}

#[cfg(feature = "mock-sge")]
pub async fn handle_mock(args: MockArgs) -> Result<(), Box<dyn std::error::Error>> {
    use enedis_rs::mock::{MockScenario, MockSgeServer};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use tokio::sync::oneshot;

    let scenario = MockScenario::from_str(&args.scenario)?;
    let addr: SocketAddr = format!("{}:{}", args.host, args.port).parse()?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        println!("\n🛑 Interruption reçue, arrêt du simulateur Mock SGE...");
        let _ = shutdown_tx.send(());
    });

    println!(
        "🎭 Démarrage du simulateur Enedis (SGE SOAP & Data Connect REST) sur http://{} (scénario: {:?})...",
        addr, scenario
    );
    println!(
        "   Endpoint SGE SOAP         : http://{}:{}/services/",
        args.host, args.port
    );
    println!(
        "   Endpoint Data Connect REST: http://{}:{}",
        args.host, args.port
    );
    println!(
        "   Endpoint OAuth2 Token     : http://{}:{}/oauth2/v3/token",
        args.host, args.port
    );
    MockSgeServer::run_server(addr, scenario, shutdown_rx).await?;
    Ok(())
}

pub async fn handle_spot(
    provider: &Arc<dyn EnedisProvider>,
    db_url: &str,
    args: SpotArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    match args.command {
        SpotCommand::Prices(a) => handle_spot_prices(db_url, a).await,
        SpotCommand::Arbitrage(a) => handle_spot_arbitrage(provider, db_url, a).await,
    }
}

async fn handle_spot_prices(
    db_url: &str,
    args: SpotPricesArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let now = Utc::now();
    let to = args
        .to
        .as_deref()
        .and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        })
        .unwrap_or_else(|| now + chrono::Duration::days(1));
    let from = args
        .from
        .as_deref()
        .and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        })
        .unwrap_or_else(|| to - chrono::Duration::days(2));

    let storage_opt = init_storage(db_url).await.ok();
    let mut prices = Vec::new();

    if args.sync {
        #[cfg(feature = "client")]
        {
            println!("🔄 Synchronisation des cours spot Day-Ahead auprès de l'API de marché...");
            let client = enedis_rs::client::NetworkSignalClient::with_default_config()?;
            if let Ok(fetched) = client.fetch_spot_prices(from, to).await {
                if let Some(ref storage) = storage_opt {
                    let _ = storage.upsert_spot_prices(&fetched).await;
                }
                prices = fetched;
            }
        }
    }

    if prices.is_empty() {
        if let Some(ref storage) = storage_opt {
            if let Ok(p) = storage.get_spot_prices(from, to).await {
                prices = p;
            }
        }
    }

    if prices.is_empty() {
        prices = generate_synthetic_spot_profile(from, to);
    }

    if args.format.to_lowercase() == "json" {
        let display_prices: Vec<_> = prices.into_iter().take(args.limit).collect();
        println!("{}", serde_json::to_string_pretty(&display_prices)?);
        return Ok(());
    }

    println!("\n✨ ===========================================================================");
    println!(" ⚡ COURS DU MARCHÉ SPOT DAY-AHEAD ÉLECTRICITÉ (EPEX SPOT FRANCE)");
    println!(" ===========================================================================");
    println!(
        " Période observée : du {} au {}",
        from.format("%d/%m/%Y %H:%M"),
        to.format("%d/%m/%Y %H:%M")
    );
    println!(" ---------------------------------------------------------------------------");
    println!(
        "  {:<20} | {:>12} | {:>12} | {:<20}",
        "Date / Heure (Local)", "Prix €/MWh", "Prix €/kWh", "Statut"
    );
    println!(" ---------------------------------------------------------------------------");

    for p in prices.iter().take(args.limit) {
        let local = enedis_rs::models::to_french_local_time(p.timestamp);
        let status = if p.is_negative {
            "🟢 PRIX NÉGATIF !"
        } else if p.price_eur_per_mwh > rust_decimal::Decimal::from(100) {
            "🔴 Pic de tension"
        } else if p.price_eur_per_mwh > rust_decimal::Decimal::from(60) {
            "🟡 Heure normale"
        } else {
            "🔵 Heure creuse"
        };

        println!(
            "  {:<20} | {:>10.2} € | {:>10.4} € | {:<20}",
            local.format("%d/%m/%Y %H:%M"),
            p.price_eur_per_mwh,
            p.price_eur_per_kwh,
            status
        );
    }
    println!(" ===========================================================================\n");

    Ok(())
}

async fn handle_spot_arbitrage(
    provider: &Arc<dyn EnedisProvider>,
    db_url: &str,
    args: SpotArbitrageArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let prm = PointId::new(&args.prm)?;
    let now = Utc::now();
    let to = args
        .to
        .as_deref()
        .and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        })
        .unwrap_or(now);
    let from = args
        .from
        .as_deref()
        .and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s)
                .ok()
                .map(|d| d.with_timezone(&Utc))
        })
        .unwrap_or_else(|| to - chrono::Duration::days(30));

    println!(
        "⚡ Analyse de corrélation et calcul d'arbitrage spot pour le PRM {}...",
        prm
    );

    let storage_opt = init_storage(db_url).await.ok();
    let mut measurements = Vec::new();

    if let Some(ref storage) = storage_opt {
        if let Ok(m) = storage
            .get_measurements(prm, from, to, Some(FlowDirection::Consumption))
            .await
        {
            measurements = m;
        }
    }

    if measurements.is_empty() {
        if let Ok(m) = provider
            .fetch_measurements(prm, from, to, FlowDirection::Consumption)
            .await
        {
            measurements = m;
        }
    }

    if measurements.is_empty() {
        return Err(format!(
            "Aucune mesure de consommation disponible pour le PRM {}.",
            prm
        )
        .into());
    }

    let mut spot_prices = Vec::new();
    if let Some(ref storage) = storage_opt {
        if let Ok(sp) = storage.get_spot_prices(from, to).await {
            spot_prices = sp;
        }
    }
    if spot_prices.is_empty() {
        spot_prices = generate_synthetic_spot_profile(from, to);
    }

    let margin_dec = args
        .margin
        .and_then(|m| rust_decimal::Decimal::from_str_exact(&format!("{:.4}", m)).ok());

    match analyze_spot_consumption(prm, &measurements, &spot_prices, margin_dec, None) {
        Some(analysis) => {
            if args.format.to_lowercase() == "json" {
                println!("{}", serde_json::to_string_pretty(&analysis)?);
            } else {
                println!("\n✨ ===========================================================================");
                println!(" ⚡ ANALYSE DU PROFIL LINKY & ARBITRAGE MARCHÉ SPOT (EPEX SPOT FRANCE)");
                println!(
                    " ==========================================================================="
                );
                println!(" PRM                       : {}", analysis.point_id);
                println!(
                    " Consommation analysée     : {:.1} kWh",
                    analysis.total_energy_kwh
                );
                println!(
                    " Prix moyen pondéré Linky  : {:.2} € / MWh",
                    analysis.weighted_average_spot_price_mwh
                );
                println!(
                    " Prix moyen marché spot    : {:.2} € / MWh",
                    analysis.market_average_spot_price_mwh
                );
                println!(
                    " Coefficient de profilage  : {:.2} ({} 1.0)",
                    analysis.profiling_coefficient,
                    if analysis.profiling_coefficient <= rust_decimal::Decimal::ONE {
                        "inférieur à"
                    } else {
                        "supérieur à"
                    }
                );
                println!(
                    " Heures à prix négatifs    : {} heures (énergie consommée : {:.1} kWh)",
                    analysis.arbitrage.negative_hours_count, analysis.negative_price_energy_kwh
                );
                println!(
                    " Facture dynamique TTC     : {:.2} € (sur la période)",
                    analysis.dynamic_total_cost_ttc
                );
                println!(
                    " ---------------------------------------------------------------------------"
                );
                println!(
                    " 💡 Potentiel d'arbitrage  : ~{:.0} € / an",
                    analysis.arbitrage.annual_arbitrage_savings_euro
                );
                println!(
                    " 📋 Préconisation          : {}",
                    analysis.arbitrage.recommendation
                );
                println!(" ===========================================================================\n");
            }
        }
        None => {
            println!(
                "⚠️ Impossible d'analyser la corrélation spot pour le PRM {}.",
                prm
            );
        }
    }

    Ok(())
}
