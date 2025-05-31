// src/dependencies.rs - External dependency checking

use anyhow::Result;
use std::time::Instant;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::metrics::Metrics;

/// Handles checking external dependencies during startup
pub struct DependencyChecker<'a> {
    config: &'a Config,
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
                // Memory storage is always available
                let duration = storage_check_start.elapsed();
                self.metrics
                    .record_component_initialization("storage_memory", duration);
                info!("✅ Memory storage backend ready");
                Ok(())
            }
            "postgres" => {
                #[cfg(feature = "postgres")]
                {
                    if let Some(ref database_url) = self.config.storage.database_url {
                        match self.check_postgres_connection(database_url).await {
                            Ok(_) => {
                                let duration = storage_check_start.elapsed();
                                self.metrics
                                    .record_component_initialization("storage_postgres", duration);
                                info!("✅ PostgreSQL storage backend ready");
                                Ok(())
                            }
                            Err(e) => {
                                let duration = storage_check_start.elapsed();
                                error!(
                                    "❌ PostgreSQL connection failed after {:.3}s: {}",
                                    duration.as_secs_f64(),
                                    e
                                );
                                self.metrics.record_startup_error(
                                    "dependency_check",
                                    "postgres_connection_failed",
                                );
                                Err(anyhow::anyhow!("PostgreSQL connection failed: {}", e))
                            }
                        }
                    } else {
                        error!("❌ PostgreSQL backend selected but no database URL provided");
                        self.metrics
                            .record_startup_error("dependency_check", "postgres_no_url");
                        Err(anyhow::anyhow!("PostgreSQL database URL not configured"))
                    }
                }
                #[cfg(not(feature = "postgres"))]
                {
                    error!("❌ PostgreSQL backend selected but postgres feature not enabled");
                    self.metrics
                        .record_startup_error("dependency_check", "postgres_feature_disabled");
                    Err(anyhow::anyhow!("PostgreSQL support not compiled in"))
                }
            }
            backend => {
                error!("❌ Unknown storage backend: {}", backend);
                self.metrics
                    .record_startup_error("dependency_check", "unknown_storage_backend");
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
                self.metrics
                    .record_component_initialization("metrics_port", duration);
                info!("✅ Metrics port {} is available", self.config.metrics.port);
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
