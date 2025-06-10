// src/bin/server/main.rs - Fixed server main with proper backend handling

use std::sync::Arc;
use tracing::{error, info};
use garbagetruck::{Config, GCService, Metrics};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("garbagetruck=info,server=info")
        .init();

    info!("🚛 Starting GarbageTruck Server");

    // Load configuration from environment
    let mut config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            error!("❌ Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    // Override backend if persistent features are not available
    #[cfg(not(feature = "persistent"))]
    {
        if config.storage.backend == "persistent_file" {
            info!("⚠️ Persistent storage requested but feature not enabled, falling back to memory backend");
            config.storage.backend = "memory".to_string();
        }
    }

    // Validate configuration
    if let Err(e) = config.validate() {
        error!("❌ Configuration validation failed: {}", e);
        std::process::exit(1);
    }

    // Show configuration summary
    let summary = config.summary();
    info!("📋 Configuration Summary:");
    info!("   - Server: {}", summary.server_endpoint);
    info!("   - Storage backend: {}", summary.storage_backend);
    info!("   - WAL enabled: {}", summary.storage_features.wal_enabled);
    info!("   - Auto-recovery: {}", summary.storage_features.auto_recovery_enabled);
    info!("   - Metrics enabled: {}", summary.metrics_enabled);

    let config = Arc::new(config);
    let metrics = Metrics::new();

    // Create and start the service
    match GCService::new(config.clone(), metrics).await {
        Ok(service) => {
            let addr = format!("{}:{}", config.server.host, config.server.port)
                .parse()
                .expect("Invalid server address");

            info!("🚀 Starting server on {}", addr);
            
            // Show feature information
            let features = garbagetruck::Features::new();
            info!("🔧 Available features: {:?}", features.list_enabled());

            if let Err(e) = service.start(addr).await {
                error!("❌ Server failed: {}", e);
                std::process::exit(1);
            }
        }
        Err(e) => {
            error!("❌ Failed to create service: {}", e);
            
            // Provide helpful error messages
            match e {
                garbagetruck::GCError::Configuration(ref msg) if msg.contains("persistent") => {
                    error!("💡 Hint: To use persistent storage, compile with: cargo run --bin garbagetruck-server --features persistent");
                    error!("💡 Or set GC_STORAGE_BACKEND=memory to use memory storage");
                }
                _ => {}
            }
            
            std::process::exit(1);
        }
    }

    Ok(())
}