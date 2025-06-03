use anyhow::Result;
use std::time::Instant;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::metrics::Metrics;

/// Handles checking external dependencies during startup
pub struct DependencyChecker<'a> {
    config: &'a Config,
    #[allow(dead_code)]
    metrics: &'a std::sync::Arc<Metrics>,
}

impl<'a> DependencyChecker<'a> {
    /// Create a new dependency checker
    pub fn new(config: &'a Config, metrics: &'a std::sync::Arc<Metrics>) -> Self {
        Self { config, metrics }
    }

    /// Check all external dependencies and their health
    pub async fn check_all_dependencies(&self) -> Result<()> {
        info!("🔍 Checking external dependencies...");

        // Check storage backend availability
        self.check_storage_backend().await?;

        // Check if metrics port is available (if metrics are enabled)
        if self.config.metrics.enabled {
            self.check_metrics_port().await;
        }

        Ok(())
    }

    /// Check storage backend availability
    async fn check_storage_backend(&self) -> Result<()> {
        let storage_check_start = Instant::now();

        match self.config.storage.backend.as_str() {
            "memory" => {
                let duration = storage_check_start.elapsed();
                info!(
                    "✅ Memory storage backend ready in {:.3}s",
                    duration.as_secs_f64()
                );
                Ok(())
            }
            backend => {
                error!("❌ Unsupported storage backend: {}", backend);
                Err(anyhow::anyhow!("Unsupported storage backend: {}", backend))
            }
        }
    }

    /// Check if metrics port is available
    async fn check_metrics_port(&self) {
        let metrics_check_start = Instant::now();
        match self.check_port_availability(self.config.metrics.port).await {
            Ok(_) => {
                let duration = metrics_check_start.elapsed();
                info!(
                    "✅ Metrics port {} is available (checked in {:.3}s)",
                    self.config.metrics.port,
                    duration.as_secs_f64()
                );
            }
            Err(e) => {
                let duration = metrics_check_start.elapsed();
                warn!(
                    "⚠️  Metrics port {} check failed after {:.3}s: {}",
                    self.config.metrics.port,
                    duration.as_secs_f64(),
                    e
                );
                // This is not a fatal error, just log it
            }
        }
    }

    /// Check if a port is available for binding
    async fn check_port_availability(&self, port: u16) -> Result<()> {
        use tokio::net::TcpListener;

        let addr = format!("127.0.0.1:{}", port);
        match TcpListener::bind(&addr).await {
            Ok(_) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("Port {} is not available: {}", port, e)),
        }
    }
}