// src/bin/cli/main.rs - GarbageTruck CLI client

use clap::{Parser, Subcommand};
use tracing::{error, info, Level};
use tracing_subscriber;

use garbagetruck::{GCClient, ObjectType};

#[derive(Parser)]
#[command(name = "garbagetruck")]
#[command(about = "GarbageTruck CLI - Lease-based distributed garbage collection")]
#[command(version)]
struct Cli {
    /// GarbageTruck server endpoint
    #[arg(long, default_value = "http://localhost:50051")]
    endpoint: String,

    /// Service ID for this client
    #[arg(long, default_value = "cli-client")]
    service_id: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Check server health
    Health,
    /// Lease management commands
    Lease {
        #[command(subcommand)]
        action: LeaseAction,
    },
}

#[derive(Subcommand)]
enum LeaseAction {
    /// Create a new lease
    Create {
        /// Object ID to create lease for
        #[arg(long)]
        object_id: String,

        /// Object type
        #[arg(long, value_enum)]
        object_type: CliObjectType,

        /// Lease duration in seconds
        #[arg(long, default_value = "300")]
        duration: u64,

        /// Cleanup HTTP endpoint (optional)
        #[arg(long)]
        cleanup_endpoint: Option<String>,
    },
    /// Renew an existing lease
    Renew {
        /// Lease ID to renew
        #[arg(long)]
        lease_id: String,

        /// Extension duration in seconds
        #[arg(long, default_value = "300")]
        duration: u64,
    },
    /// Release a lease
    Release {
        /// Lease ID to release
        #[arg(long)]
        lease_id: String,
    },
    /// Get lease information
    Get {
        /// Lease ID to get
        #[arg(long)]
        lease_id: String,
    },
    /// List leases
    List {
        /// Filter by service ID
        #[arg(long)]
        service: Option<String>,

        /// Maximum number of leases to return
        #[arg(long, default_value = "10")]
        limit: u32,
    },
}

#[derive(clap::ValueEnum, Clone)]
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();

    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    // Create client
    let mut client = match GCClient::new(&cli.endpoint, cli.service_id.clone()).await {
        Ok(client) => {
            info!("✅ Connected to GarbageTruck server at {}", cli.endpoint);
            client
        }
        Err(e) => {
            error!("❌ Failed to connect to GarbageTruck server: {}", e);
            error!("💡 Make sure the server is running on {}", cli.endpoint);
            std::process::exit(1);
        }
    };

    match cli.command {
        Commands::Health => match client.health_check().await {
            Ok(healthy) => {
                if healthy {
                    println!("✅ GarbageTruck server is healthy");
                } else {
                    println!("⚠️  GarbageTruck server reports unhealthy status");
                    std::process::exit(1);
                }
            }
            Err(e) => {
                error!("❌ Health check failed: {}", e);
                std::process::exit(1);
            }
        },

        Commands::Lease { action } => match action {
            LeaseAction::Create {
                object_id,
                object_type,
                duration,
                cleanup_endpoint,
            } => {
                let cleanup_config = cleanup_endpoint.map(|endpoint| garbagetruck::CleanupConfig {
                    cleanup_endpoint: String::new(),
                    cleanup_http_endpoint: endpoint,
                    cleanup_payload: format!(
                        r#"{{"action": "delete", "object_id": "{}"}}"#,
                        object_id
                    ),
                    max_retries: 3,
                    retry_delay_seconds: 2,
                });

                match client
                    .create_lease(
                        object_id.clone(),
                        object_type.into(),
                        duration,
                        std::collections::HashMap::new(),
                        cleanup_config,
                    )
                    .await
                {
                    Ok(lease_id) => {
                        println!("✅ Created lease: {}", lease_id);
                        println!("   Object ID: {}", object_id);
                        println!("   Duration: {}s", duration);
                    }
                    Err(e) => {
                        error!("❌ Failed to create lease: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            LeaseAction::Renew { lease_id, duration } => {
                match client.renew_lease(lease_id.clone(), duration).await {
                    Ok(_) => {
                        println!("✅ Renewed lease: {}", lease_id);
                        println!("   Extended by: {}s", duration);
                    }
                    Err(e) => {
                        error!("❌ Failed to renew lease: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            LeaseAction::Release { lease_id } => {
                match client.release_lease(lease_id.clone()).await {
                    Ok(_) => {
                        println!("✅ Released lease: {}", lease_id);
                    }
                    Err(e) => {
                        error!("❌ Failed to release lease: {}", e);
                        std::process::exit(1);
                    }
                }
            }

            LeaseAction::Get { lease_id } => match client.get_lease(lease_id.clone()).await {
                Ok(Some(lease)) => {
                    println!("✅ Lease found: {}", lease_id);
                    println!("   Object ID: {}", lease.object_id);
                    println!("   Service ID: {}", lease.service_id);
                    println!("   State: {:?}", lease.state);
                    if let Some(created_at) = lease.created_at {
                        println!("   Created: {} seconds ago", created_at.seconds);
                    }
                    if let Some(expires_at) = lease.expires_at {
                        println!("   Expires: {} seconds from epoch", expires_at.seconds);
                    }
                }
                Ok(None) => {
                    println!("❌ Lease not found: {}", lease_id);
                    std::process::exit(1);
                }
                Err(e) => {
                    error!("❌ Failed to get lease: {}", e);
                    std::process::exit(1);
                }
            },

            LeaseAction::List { service, limit } => {
                match client.list_leases(service.clone(), limit).await {
                    Ok(leases) => {
                        if leases.is_empty() {
                            println!("📭 No leases found");
                        } else {
                            println!("📋 Found {} lease(s):", leases.len());
                            for lease in leases {
                                println!("  🎫 {} ({})", lease.lease_id, lease.object_id);
                                println!("     Service: {}", lease.service_id);
                                println!("     State: {:?}", lease.state);
                            }
                        }
                    }
                    Err(e) => {
                        error!("❌ Failed to list leases: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        },
    }

    Ok(())
}
