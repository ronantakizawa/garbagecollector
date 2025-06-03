// src/bin/server/main.rs - Fixed version without PostgreSQL references

use garbagetruck::{ApplicationStartup, Config};
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    info!("🚀 Starting GarbageTruck server");

    // Load configuration
    let config = Config::from_env().unwrap_or_else(|e| {
        error!("Failed to load configuration: {}", e);
        std::process::exit(1);
    });

    // Validate configuration
    if let Err(e) = config.validate() {
        error!("Invalid configuration: {}", e);
        std::process::exit(1);
    }

    // Log configuration summary
    let summary = config.summary();
    info!("📋 Configuration Summary:");
    info!("  Server endpoint: {}", summary.server_endpoint);
    info!("  Storage backend: {}", summary.storage_backend);
    info!("  Default lease duration: {}s", summary.default_lease_duration);
    info!("  Cleanup interval: {}s", summary.cleanup_interval);
    info!("  Max leases per service: {}", summary.max_leases_per_service);
    info!("  Metrics enabled: {}", summary.metrics_enabled);
    if let Some(port) = summary.metrics_port {
        info!("  Metrics port: {}", port);
    }

    // Create and start application
    let startup = match ApplicationStartup::new(config) {
        Ok(startup) => startup,
        Err(e) => {
            error!("Failed to create application: {}", e);
            std::process::exit(1);
        }
    };

    info!("✅ Configuration loaded and validated successfully");

    // Start the application (this will block until shutdown)
    if let Err(e) = startup.start().await {
        error!("Application failed: {}", e);
        std::process::exit(1);
    }

    info!("👋 GarbageTruck server shut down gracefully");
    Ok(())
}