// src/bin/server/main.rs - Fixed server binary with proper configuration

use tracing::{error, info, Level};
use tracing_subscriber;

use garbagetruck::{ApplicationStartup, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .with_target(false)
        .init();

    info!("🚀 Starting GarbageTruck Server");

    // Load configuration from environment variables
    let config = match Config::from_env() {
        Ok(config) => {
            info!("✅ Configuration loaded successfully");

            // Validate the configuration
            if let Err(e) = config.validate() {
                error!("❌ Configuration validation failed: {}", e);
                std::process::exit(1);
            }

            info!("✅ Configuration validated successfully");
            config
        }
        Err(e) => {
            error!("❌ Failed to load configuration: {}", e);
            error!("💡 Make sure to set required environment variables:");
            error!("   - GC_STORAGE_BACKEND (memory or postgres)");
            error!("   - DATABASE_URL (if using postgres)");
            error!("   - GC_SERVER_HOST (default: 0.0.0.0)");
            error!("   - GC_SERVER_PORT (default: 50051)");
            std::process::exit(1);
        }
    };

    // Log configuration summary
    let summary = config.summary();
    info!("📋 Configuration Summary:");
    info!("  Server: {}", summary.server_endpoint);
    info!("  Storage: {}", summary.storage_backend);
    info!("  Database URL configured: {}", summary.has_database_url);
    info!(
        "  Default lease duration: {}s",
        summary.default_lease_duration
    );
    info!("  Cleanup interval: {}s", summary.cleanup_interval);
    info!(
        "  Max leases per service: {}",
        summary.max_leases_per_service
    );
    info!("  Metrics enabled: {}", summary.metrics_enabled);
    if let Some(metrics_port) = summary.metrics_port {
        info!("  Metrics port: {}", metrics_port);
    }

    // Create and start the application
    let startup = ApplicationStartup::new(config)?; // No .await here - it's not async

    info!("🎯 Starting application startup sequence...");

    // Start the application (this will block until shutdown)
    match startup.start().await {
        Ok(_) => {
            info!("✅ GarbageTruck server shut down gracefully");
            Ok(())
        }
        Err(e) => {
            error!("❌ GarbageTruck server failed: {}", e);
            Err(e.into())
        }
    }
}
