// src/bin/cert-gen/main.rs - Certificate generation CLI tool

use garbagetruck::security::certificates::{CertGenCLI};
use tracing::error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("garbagetruck=info,cert_gen=info")
        .init();

    // Check if TLS feature is enabled
    #[cfg(not(feature = "tls"))]
    {
        error!("❌ TLS feature not enabled. Please compile with --features tls");
        std::process::exit(1);
    }

    #[cfg(feature = "tls")]
    {
        // Run certificate generation CLI
        if let Err(e) = CertGenCLI::run() {
            error!("❌ Certificate generation failed: {}", e);
            std::process::exit(1);
        }
    }

    Ok(())
}