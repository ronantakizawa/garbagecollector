use anyhow::Result;
use garbagetruck::{ApplicationStartup, Config};

#[tokio::main]
async fn main() -> Result<()> {
    println!("🚛 GarbageTruck Server Setup Example");
    println!("=====================================");

    // You can also create and configure the service manually
    println!("1. Loading configuration...");
    let config = Config::from_env()?;
    config.validate()?;

    println!("2. Configuration loaded:");
    println!("   - Server: {}:{}", config.server.host, config.server.port);
    println!("   - Storage: {}", config.storage.backend);
    println!(
        "   - Default lease duration: {}s",
        config.gc.default_lease_duration_seconds
    );

    println!("3. Starting application...");
    let startup = ApplicationStartup::new().await?;

    // This will run the server until shutdown
    startup.run().await?;

    Ok(())
}
