// src/service/mod.rs - Updated with cleanup loop integration

mod cleanup_loop;
mod handlers;
mod validation;

pub use cleanup_loop::{CleanupLoop, CleanupResult};
pub use handlers::GCServiceHandlers;

use std::sync::Arc;
use std::time::Instant;
use tokio::sync::watch;
use tonic::transport::Server;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{LeaseFilter, LeaseState};
use crate::metrics::Metrics;
use crate::proto::distributed_gc_service_server::DistributedGcServiceServer;
use crate::storage::Storage;

/// Main GarbageTruck service
#[derive(Clone)]
pub struct GCService {
    pub(crate) storage: Arc<dyn Storage>,
    pub(crate) config: Arc<Config>,
    pub(crate) metrics: Arc<Metrics>,
    pub(crate) start_time: Instant,
    cleanup_shutdown_tx: Arc<watch::Sender<bool>>,
}

impl GCService {
    /// Create a new GCService instance
    pub fn new(storage: Arc<dyn Storage>, config: Arc<Config>, metrics: Arc<Metrics>) -> Self {
        let (cleanup_shutdown_tx, _) = watch::channel(false);

        Self {
            storage,
            config,
            metrics,
            start_time: Instant::now(),
            cleanup_shutdown_tx: Arc::new(cleanup_shutdown_tx),
        }
    }

    /// Start the gRPC server and cleanup loop
    pub async fn start(&self, addr: std::net::SocketAddr) -> Result<()> {
        info!("🚀 Starting GarbageTruck service on {}", addr);

        // Start the cleanup loop in a separate task
        let cleanup_handle = self.start_cleanup_loop().await?;

        // Start the gRPC server
        let server_handle = self.start_grpc_server(addr).await?;

        // Wait for either task to complete (or fail)
        tokio::select! {
            result = cleanup_handle => {
                error!("Cleanup loop exited: {:?}", result);
                result??;
            }
            result = server_handle => {
                error!("gRPC server exited: {:?}", result);
                result??;
            }
        }

        Ok(())
    }

    /// Start the cleanup loop in a background task
    async fn start_cleanup_loop(&self) -> Result<tokio::task::JoinHandle<Result<()>>> {
        let cleanup_rx = self.cleanup_shutdown_tx.subscribe();

        let cleanup_loop = CleanupLoop::new(
            self.storage.clone(),
            self.config.clone(),
            self.metrics.clone(),
            cleanup_rx,
        );

        let handle = tokio::spawn(async move { cleanup_loop.start().await });

        info!("🧹 Cleanup loop started");
        Ok(handle)
    }

    /// Start the gRPC server
    async fn start_grpc_server(
        &self,
        addr: std::net::SocketAddr,
    ) -> Result<tokio::task::JoinHandle<Result<()>>> {
        let service = DistributedGcServiceServer::new(self.clone());

        let handle = tokio::spawn(async move {
            Server::builder()
                .add_service(service)
                .serve(addr)
                .await
                .map_err(|e| GCError::Internal(e.to_string()))
        });

        info!("🌐 gRPC server started on {}", addr);
        Ok(handle)
    }

    /// Gracefully shutdown the service
    pub async fn shutdown(&self) -> Result<()> {
        info!("🛑 Initiating graceful shutdown...");

        // Signal cleanup loop to stop
        if let Err(e) = self.cleanup_shutdown_tx.send(true) {
            warn!("Failed to send shutdown signal to cleanup loop: {}", e);
        }

        // Give some time for cleanup operations to complete
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        info!("✅ Graceful shutdown completed");
        Ok(())
    }

    /// Check if a service has too many leases
    pub(crate) async fn check_service_lease_limit(&self, service_id: &str) -> Result<()> {
        let filter = LeaseFilter {
            service_id: Some(service_id.to_string()),
            state: Some(LeaseState::Active),
            ..Default::default()
        };

        let lease_count = self.storage.count_leases(filter).await?;

        if lease_count >= self.config.gc.max_leases_per_service {
            return Err(GCError::ServiceLeaseLimit {
                service_id: service_id.to_string(),
                current: lease_count,
                max: self.config.gc.max_leases_per_service,
            });
        }

        Ok(())
    }

    /// Get service configuration
    pub(crate) fn get_config(&self) -> &Config {
        &self.config
    }

    /// Get metrics instance
    pub(crate) fn get_metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    /// Get storage instance
    pub(crate) fn get_storage(&self) -> &Arc<dyn Storage> {
        &self.storage
    }

    /// Get service uptime
    pub fn uptime(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Perform manual cleanup cycle (for testing/admin purposes)
    pub async fn manual_cleanup(&self) -> Result<CleanupStats> {
        info!("🔧 Performing manual cleanup cycle...");

        let start_time = Instant::now();

        // Get expired leases
        let grace_period = self.config.cleanup_grace_period();
        let expired_leases = self.storage.get_expired_leases(grace_period).await?;
        let expired_count = expired_leases.len();

        if expired_count == 0 {
            info!("✅ Manual cleanup: No expired leases found");
            return Ok(CleanupStats {
                expired_leases_found: 0,
                cleanup_attempts: 0,
                successful_cleanups: 0,
                failed_cleanups: 0,
                duration_seconds: start_time.elapsed().as_secs_f64(),
            });
        }

        // Create cleanup executor and process leases
        let cleanup_executor = crate::cleanup::CleanupExecutor::new(
            std::time::Duration::from_secs(self.config.cleanup.default_timeout_seconds),
            self.config.cleanup.default_max_retries,
            std::time::Duration::from_secs(self.config.cleanup.default_retry_delay_seconds),
        );

        let mut successful_cleanups = 0;
        let mut failed_cleanups = 0;

        for lease in expired_leases {
            match cleanup_executor.cleanup_lease(&lease).await {
                Ok(_) => {
                    // Remove successfully cleaned lease
                    if let Err(e) = self.storage.delete_lease(&lease.lease_id).await {
                        warn!("Failed to remove cleaned lease from storage: {}", e);
                        failed_cleanups += 1;
                    } else {
                        successful_cleanups += 1;
                    }
                }
                Err(e) => {
                    warn!("Manual cleanup failed for lease {}: {}", lease.lease_id, e);
                    failed_cleanups += 1;
                }
            }
        }

        let stats = CleanupStats {
            expired_leases_found: expired_count,
            cleanup_attempts: expired_count,
            successful_cleanups,
            failed_cleanups,
            duration_seconds: start_time.elapsed().as_secs_f64(),
        };

        info!(
            "✅ Manual cleanup completed: {} successful, {} failed in {:.2}s",
            successful_cleanups, failed_cleanups, stats.duration_seconds
        );

        Ok(stats)
    }

    /// Get service health information
    pub async fn health_info(&self) -> Result<HealthInfo> {
        let storage_stats = self.storage.get_stats().await?;

        Ok(HealthInfo {
            uptime_seconds: self.uptime().as_secs(),
            active_leases: storage_stats.active_leases as u64,
            expired_leases: storage_stats.expired_leases as u64,
            total_leases: storage_stats.total_leases as u64,
            storage_backend: self.config.storage.backend.clone(),
            cleanup_interval_seconds: self.config.gc.cleanup_interval_seconds,
            version: env!("CARGO_PKG_VERSION").to_string(),
        })
    }
}

/// Statistics from a cleanup operation
#[derive(Debug, Clone)]
pub struct CleanupStats {
    pub expired_leases_found: usize,
    pub cleanup_attempts: usize,
    pub successful_cleanups: usize,
    pub failed_cleanups: usize,
    pub duration_seconds: f64,
}

/// Health information for the service
#[derive(Debug, Clone)]
pub struct HealthInfo {
    pub uptime_seconds: u64,
    pub active_leases: u64,
    pub expired_leases: u64,
    pub total_leases: u64,
    pub storage_backend: String,
    pub cleanup_interval_seconds: u64,
    pub version: String,
}
