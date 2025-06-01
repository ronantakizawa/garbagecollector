// src/bin/cli/main.rs - GarbageTruck CLI

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use garbagetruck::{GCClient, ObjectType};
use std::collections::HashMap;
use tonic::transport::Channel;
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "garbagetruck")]
#[command(about = "A CLI for interacting with GarbageTruck lease-based garbage collection service")]
#[command(version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// GarbageTruck server endpoint
    #[arg(long, default_value = "http://localhost:50051")]
    endpoint: String,

    /// Service ID for lease operations
    #[arg(long, default_value = "garbagetruck")]
    service_id: String,

    /// Enable verbose output
    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check service health
    Health,
    /// Get service status and statistics
    Status,
    /// Lease management operations
    Lease {
        #[command(subcommand)]
        action: LeaseAction,
    },
}

#[derive(Subcommand)]
enum LeaseAction {
    /// Create a new lease
    Create {
        /// Object ID to lease
        #[arg(long)]
        object_id: String,

        /// Object type
        #[arg(long, value_enum)]
        object_type: CliObjectType,

        /// Lease duration in seconds
        #[arg(long)]
        duration: u64,

        /// Metadata as key=value pairs
        #[arg(long, value_parser = parse_key_val)]
        metadata: Vec<(String, String)>,

        /// HTTP cleanup endpoint
        #[arg(long)]
        cleanup_endpoint: Option<String>,

        /// Cleanup payload
        #[arg(long)]
        cleanup_payload: Option<String>,
    },
    /// List leases
    List {
        /// Filter by service ID
        #[arg(long)]
        service: Option<String>,

        /// Filter by object type
        #[arg(long, value_enum)]
        object_type: Option<CliObjectType>,

        /// Maximum number of results
        #[arg(long, default_value = "10")]
        limit: u32,
    },
    /// Get lease details
    Get {
        /// Lease ID
        lease_id: String,
    },
    /// Renew a lease
    Renew {
        /// Lease ID
        lease_id: String,

        /// Extension duration in seconds
        #[arg(long)]
        extend: u64,
    },
    /// Release a lease
    Release {
        /// Lease ID
        lease_id: String,
    },
}

#[derive(Clone, ValueEnum)]
enum CliObjectType {
    DatabaseRow,
    BlobStorage,
    TemporaryFile,
    WebsocketSession,
    CacheEntry,
    Custom,
}

impl From<CliObjectType> for ObjectType {
    fn from(cli_type: CliObjectType) -> Self {
        match cli_type {
            CliObjectType::DatabaseRow => ObjectType::DatabaseRow,
            CliObjectType::BlobStorage => ObjectType::BlobStorage,
            CliObjectType::TemporaryFile => ObjectType::TemporaryFile,
            CliObjectType::WebsocketSession => ObjectType::WebsocketSession,
            CliObjectType::CacheEntry => ObjectType::CacheEntry,
            CliObjectType::Custom => ObjectType::Custom,
        }
    }
}

fn parse_key_val(s: &str) -> Result<(String, String)> {
    let pos = s
        .find('=')
        .ok_or_else(|| anyhow::anyhow!("invalid KEY=value: no `=` found in `{s}`"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    if cli.verbose {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .init();
    } else {
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO)
            .init();
    }

    // Create client
    let mut client = match GCClient::new(&cli.endpoint, cli.service_id.clone()).await {
        Ok(client) => client,
        Err(e) => {
            error!(
                "Failed to connect to GarbageTruck at {}: {}",
                cli.endpoint, e
            );
            std::process::exit(1);
        }
    };

    // Execute command
    let result = match cli.command {
        Commands::Health => handle_health(&mut client).await,
        Commands::Status => handle_status(&mut client).await,
        Commands::Lease { action } => handle_lease_command(&mut client, action).await,
    };

    if let Err(e) = result {
        error!("Command failed: {}", e);
        std::process::exit(1);
    }

    Ok(())
}

async fn handle_health(client: &mut GCClient) -> Result<()> {
    info!("🔍 Checking GarbageTruck health...");

    match client.health_check().await {
        Ok(true) => {
            println!("✅ GarbageTruck service is healthy");
            Ok(())
        }
        Ok(false) => {
            println!("⚠️  GarbageTruck service reports unhealthy");
            std::process::exit(1);
        }
        Err(e) => {
            println!("❌ Health check failed: {}", e);
            std::process::exit(1);
        }
    }
}

async fn handle_status(client: &mut GCClient) -> Result<()> {
    info!("📊 Getting GarbageTruck status...");

    match client.health_check().await {
        Ok(is_healthy) => {
            println!("🚛 GarbageTruck Service Status");
            println!("═══════════════════════════════");
            println!(
                "Health: {}",
                if is_healthy {
                    "✅ Healthy"
                } else {
                    "❌ Unhealthy"
                }
            );
        }
        Err(e) => {
            println!("❌ Failed to get status: {}", e);
            return Err(e.into());
        }
    }

    // Get lease statistics
    match client.list_leases(None, 1000).await {
        Ok(leases) => {
            let total_leases = leases.len();
            let mut by_service = HashMap::new();
            let mut by_type = HashMap::new();

            for lease in &leases {
                *by_service.entry(lease.service_id.clone()).or_insert(0) += 1;
                let type_name = format!(
                    "{:?}",
                    ObjectType::from(
                        garbagetruck::proto::ObjectType::try_from(lease.object_type)
                            .unwrap_or_default()
                    )
                );
                *by_type.entry(type_name).or_insert(0) += 1;
            }

            println!("Total Active Leases: {}", total_leases);

            if !by_service.is_empty() {
                println!("\nLeases by Service:");
                for (service, count) in by_service {
                    println!("  {} → {} leases", service, count);
                }
            }

            if !by_type.is_empty() {
                println!("\nLeases by Type:");
                for (obj_type, count) in by_type {
                    println!("  {} → {} leases", obj_type, count);
                }
            }
        }
        Err(e) => {
            println!("⚠️  Could not retrieve lease statistics: {}", e);
        }
    }

    Ok(())
}

async fn handle_lease_command(client: &mut GCClient, action: LeaseAction) -> Result<()> {
    match action {
        LeaseAction::Create {
            object_id,
            object_type,
            duration,
            metadata,
            cleanup_endpoint,
            cleanup_payload,
        } => {
            info!("📝 Creating lease for object '{}'...", object_id);

            let metadata_map: HashMap<String, String> = metadata.into_iter().collect();

            let cleanup_config = cleanup_endpoint.map(|endpoint| garbagetruck::CleanupConfig {
                cleanup_endpoint: String::new(),
                cleanup_http_endpoint: endpoint,
                cleanup_payload: cleanup_payload.unwrap_or_default(),
                max_retries: 3,
                retry_delay_seconds: 2,
            });

            match client
                .create_lease(
                    object_id.clone(),
                    object_type.into(),
                    duration,
                    metadata_map,
                    cleanup_config,
                )
                .await
            {
                Ok(lease_id) => {
                    println!("✅ Lease created successfully");
                    println!("   Lease ID: {}", lease_id);
                    println!("   Object ID: {}", object_id);
                    println!("   Duration: {}s", duration);
                }
                Err(e) => {
                    println!("❌ Failed to create lease: {}", e);
                    return Err(e.into());
                }
            }
        }
        LeaseAction::List {
            service,
            object_type: _,
            limit,
        } => {
            info!("📋 Listing leases...");

            match client.list_leases(service.clone(), limit).await {
                Ok(leases) => {
                    if leases.is_empty() {
                        println!("📭 No leases found");
                        return Ok(());
                    }

                    println!("📋 Found {} lease(s)", leases.len());
                    println!("═══════════════════════════════════════════════════════════");

                    for lease in leases {
                        let obj_type = garbagetruck::proto::ObjectType::try_from(lease.object_type)
                            .map(|t| format!("{:?}", ObjectType::from(t)))
                            .unwrap_or_else(|_| "Unknown".to_string());

                        println!("Lease ID: {}", lease.lease_id);
                        println!("  Object ID: {}", lease.object_id);
                        println!("  Service: {}", lease.service_id);
                        println!("  Type: {}", obj_type);
                        println!(
                            "  State: {:?}",
                            garbagetruck::proto::LeaseState::try_from(lease.state)
                                .unwrap_or_default()
                        );

                        if let Some(expires_at) = lease.expires_at {
                            let expires = chrono::DateTime::<chrono::Utc>::from_timestamp(
                                expires_at.seconds,
                                expires_at.nanos as u32,
                            )
                            .unwrap_or_default();
                            println!("  Expires: {}", expires.format("%Y-%m-%d %H:%M:%S UTC"));
                        }

                        println!("  Renewals: {}", lease.renewal_count);
                        println!("───────────────────────────────────────────────────────────");
                    }
                }
                Err(e) => {
                    println!("❌ Failed to list leases: {}", e);
                    return Err(e.into());
                }
            }
        }
        LeaseAction::Get { lease_id } => {
            info!("🔍 Getting lease details for '{}'...", lease_id);

            match client.get_lease(lease_id.clone()).await {
                Ok(Some(lease)) => {
                    let obj_type = garbagetruck::proto::ObjectType::try_from(lease.object_type)
                        .map(|t| format!("{:?}", ObjectType::from(t)))
                        .unwrap_or_else(|_| "Unknown".to_string());

                    println!("🔍 Lease Details");
                    println!("═══════════════════════════════════════════════════════════");
                    println!("Lease ID: {}", lease.lease_id);
                    println!("Object ID: {}", lease.object_id);
                    println!("Service ID: {}", lease.service_id);
                    println!("Object Type: {}", obj_type);
                    println!(
                        "State: {:?}",
                        garbagetruck::proto::LeaseState::try_from(lease.state).unwrap_or_default()
                    );

                    if let Some(created_at) = lease.created_at {
                        let created = chrono::DateTime::<chrono::Utc>::from_timestamp(
                            created_at.seconds,
                            created_at.nanos as u32,
                        )
                        .unwrap_or_default();
                        println!("Created: {}", created.format("%Y-%m-%d %H:%M:%S UTC"));
                    }

                    if let Some(expires_at) = lease.expires_at {
                        let expires = chrono::DateTime::<chrono::Utc>::from_timestamp(
                            expires_at.seconds,
                            expires_at.nanos as u32,
                        )
                        .unwrap_or_default();
                        println!("Expires: {}", expires.format("%Y-%m-%d %H:%M:%S UTC"));
                    }

                    println!("Renewal Count: {}", lease.renewal_count);

                    if !lease.metadata.is_empty() {
                        println!("\nMetadata:");
                        for (key, value) in &lease.metadata {
                            println!("  {} = {}", key, value);
                        }
                    }

                    if lease.cleanup_config.is_some() {
                        println!("\nCleanup configured: ✅");
                    }
                }
                Ok(None) => {
                    println!("❌ Lease '{}' not found", lease_id);
                    std::process::exit(1);
                }
                Err(e) => {
                    println!("❌ Failed to get lease: {}", e);
                    return Err(e.into());
                }
            }
        }
        LeaseAction::Renew { lease_id, extend } => {
            info!("🔄 Renewing lease '{}'...", lease_id);

            match client.renew_lease(lease_id.clone(), extend).await {
                Ok(_) => {
                    println!("✅ Lease '{}' renewed successfully", lease_id);
                    println!("   Extended by: {}s", extend);
                }
                Err(e) => {
                    println!("❌ Failed to renew lease: {}", e);
                    return Err(e.into());
                }
            }
        }
        LeaseAction::Release { lease_id } => {
            info!("🗑️  Releasing lease '{}'...", lease_id);

            match client.release_lease(lease_id.clone()).await {
                Ok(_) => {
                    println!("✅ Lease '{}' released successfully", lease_id);
                    println!("   Cleanup will be triggered if configured");
                }
                Err(e) => {
                    println!("❌ Failed to release lease: {}", e);
                    return Err(e.into());
                }
            }
        }
    }

    Ok(())
}
