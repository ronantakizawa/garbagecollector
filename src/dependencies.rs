// src/dependencies.rs - Simplified external dependency checking

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
            "postgres" => {
                #[cfg(feature = "postgres")]
                {
                    if let Some(ref database_url) = self.config.storage.database_url {
                        match self.check_postgres_connection(database_url).await {
                            Ok(_) => {
                                let duration = storage_check_start.elapsed();
                                info!(
                                    "✅ PostgreSQL storage backend ready in {:.3}s",
                                    duration.as_secs_f64()
                                );
                                Ok(())
                            }
                            Err(e) => {
                                let duration = storage_check_start.elapsed();
                                error!(
                                    "❌ PostgreSQL connection failed after {:.3}s: {}",
                                    duration.as_secs_f64(),
                                    e
                                );
                                Err(anyhow::anyhow!("PostgreSQL connection failed: {}", e))
                            }
                        }
                    } else {
                        error!("❌ PostgreSQL backend selected but no database URL provided");
                        Err(anyhow::anyhow!("PostgreSQL database URL not configured"))
                    }
                }
                #[cfg(not(feature = "postgres"))]
                {
                    error!("❌ PostgreSQL backend selected but postgres feature not enabled");
                    Err(anyhow::anyhow!("PostgreSQL support not compiled in"))
                }
            }
            backend => {
                error!("❌ Unknown storage backend: {}", backend);
                Err(anyhow::anyhow!("Unknown storage backend: {}", backend))
            }
        }
    }

    /// Check PostgreSQL connection
    #[cfg(feature = "postgres")]
    async fn check_postgres_connection(&self, database_url: &str) -> Result<()> {
        use sqlx::PgPool;

        info!("🐘 Testing PostgreSQL connection...");

        let pool = PgPool::connect(database_url)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to connect to PostgreSQL: {}", e))?;

        // Test with a simple query
        sqlx::query("SELECT 1")
            .fetch_one(&pool)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to execute test query: {}", e))?;

        pool.close().await;
        info!("✅ PostgreSQL connection test successful");

        Ok(())
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
