// examples/server_setup.rs - Basic server setup example

use garbagetruck::{ApplicationStartup, Config};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    println!("🚀 Setting up GarbageTruck server");

    // Create configuration
    let mut config = Config::default();

    // Configure for example
    config.server.host = "127.0.0.1".to_string();
    config.server.port = 50051;
    config.storage.backend = "memory".to_string();
    config.gc.cleanup_interval_seconds = 30;
    config.gc.cleanup_grace_period_seconds = 10;

    println!("📋 Configuration:");
    println!("  Server: {}:{}", config.server.host, config.server.port);
    println!("  Storage: {}", config.storage.backend);
    println!(
        "  Cleanup interval: {}s",
        config.gc.cleanup_interval_seconds
    );

    // Validate configuration
    config.validate()?;
    println!("✅ Configuration validated");

    // Create and start application
    let startup = ApplicationStartup::new(config)?;
    println!("🎯 Starting GarbageTruck server...");

    // This will run until interrupted
    startup.start().await?;

    Ok(())
}
