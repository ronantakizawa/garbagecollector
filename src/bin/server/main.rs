// src/bin/server/main.rs - Fixed server binary that actually starts the service

use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info, Level};
use tracing_subscriber;

use garbagetruck::{
    config::Config,
    error::Result,
    metrics::Metrics,
    service::GCService,
    startup::ApplicationStartup,
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    info!("🚀 Starting GarbageTruck server");

    // Load configuration
    let config = match Config::from_env() {
        Ok(config) => {
            info!("📋 Configuration Summary:");
            info!("  Server endpoint: {}:{}", config.server.host, config.server.port);
            info!("  Storage backend: {}", config.storage.backend);
            info!("  Default lease duration: {}s", config.gc.default_lease_duration_seconds);
            
            #[cfg(feature = "persistent")]
            {
                info!("  WAL enabled: {}", config.storage.enable_wal);
                info!("  Auto-recovery: {}", config.storage.enable_auto_recovery);
            }
            
            info!("  Cleanup interval: {}s", config.gc.cleanup_interval_seconds);
            info!("  Max leases per service: {}", config.gc.max_leases_per_service);
            
            if config.metrics.enabled {
                info!("  Metrics enabled: true");
                info!("  Metrics port: {}", config.metrics.port);
            } else {
                info!("  Metrics enabled: false");
            }
            
            config
        }
        Err(e) => {
            error!("❌ Failed to load configuration: {}", e);
            std::process::exit(1);
        }
    };

    // Validate configuration
    if let Err(e) = config.validate() {
        error!("❌ Configuration validation failed: {}", e);
        std::process::exit(1);
    }

    info!("✅ Configuration loaded and validated successfully");

    // Parse server address
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .unwrap_or_else(|e| {
            error!("❌ Invalid server address: {}", e);
            std::process::exit(1);
        });

    // Method 1: Use ApplicationStartup (recommended)
    let startup = ApplicationStartup::new(config)?;
    
    info!("🌐 Starting server on {}", addr);
    
    // This will block until shutdown
    if let Err(e) = startup.start_and_run(addr).await {
        error!("❌ Server failed: {}", e);
        std::process::exit(1);
    }

    info!("👋 GarbageTruck server shut down gracefully");
    Ok(())
}

// Alternative approach if you want direct service control:
#[allow(dead_code)]
async fn alternative_startup() -> Result<()> {
    let config = Config::from_env()?;
    config.validate()?;
    
    let config = Arc::new(config);
    let metrics = Metrics::new();
    
    // Create service directly
    let service = GCService::new(config.clone(), metrics).await?;
    
    // Start service
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port).parse().unwrap();
    service.start(addr).await?;
    
    Ok(())
}