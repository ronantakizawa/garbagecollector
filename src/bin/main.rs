// src/bin/main.rs - Entry point only

use anyhow::Result;
use garbagetruck::startup::ApplicationStartup;

#[tokio::main]
async fn main() -> Result<()> {
    let startup = ApplicationStartup::new().await?;
    startup.run().await
}