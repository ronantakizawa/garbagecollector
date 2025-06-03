// src/service/mod.rs - Fixed version with corrected update_metrics call

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tonic::transport::Server;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::metrics::Metrics;
use crate::proto::distributed_gc_service_server::DistributedGcServiceServer;
use crate::shutdown::{ShutdownCoordinator, ShutdownConfig, TaskPriority, TaskType};
use crate::storage::Storage;

mod cleanup_loop;
mod handlers;

pub use cleanup_loop::CleanupLoop;
pub use handlers::GCServiceHandlers;

/// Main GarbageTruck service that coordinates all operations
pub struct GCService {
    storage: Arc<dyn Storage>,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
    shutdown_coordinator: ShutdownCoordinator,
}

impl GCService {
    /// Create a new GarbageTruck service
    pub fn new(
        storage: Arc<dyn Storage>,
        config: Arc<Config>,
        metrics: Arc<Metrics>,
    ) -> Self {
        let shutdown_config = ShutdownConfig::default();
        let shutdown_coordinator = ShutdownCoordinator::new(shutdown_config);

        Self {
            storage,
            config,
            metrics,
            shutdown_coordinator,
        }
    }

    /// Start the complete service with all components
    pub async fn start(&self, addr: SocketAddr) -> Result<()> {
        info!("🚀 Starting GarbageTruck service at {}", addr);

        // Start listening for shutdown signals
        self.shutdown_coordinator.listen_for_signals().await;

        // Start background tasks
        self.start_cleanup_loop().await?;
        self.start_metrics_updater().await?;

        // Create gRPC service handlers
        let handlers = GCServiceHandlers::new(
            self.storage.clone(),
            self.config.clone(),
            self.metrics.clone(),
        );

        // Start gRPC server
        let grpc_service = DistributedGcServiceServer::new(handlers);

        info!("✅ GarbageTruck service ready at {}", addr);

        // Start the gRPC server (this blocks until shutdown)
        match Server::builder()
            .add_service(grpc_service)
            .serve_with_shutdown(addr, async {
                let mut shutdown_rx = self.shutdown_coordinator.subscribe_to_shutdown();
                let _ = shutdown_rx.recv().await;
                info!("🛑 gRPC server received shutdown signal");
            })
            .await
        {
            Ok(_) => {
                info!("✅ gRPC server shut down gracefully");
                Ok(())
            }
            Err(e) => {
                error!("❌ gRPC server error: {}", e);
                Err(GCError::Internal(format!("gRPC server error: {}", e)))
            }
        }
    }

    /// Start the cleanup loop as a background task
    async fn start_cleanup_loop(&self) -> Result<()> {
        info!("🧹 Starting cleanup loop");

        // Register cleanup task with shutdown coordinator
        let task_handle = self.shutdown_coordinator.register_task(
            "cleanup-loop".to_string(),
            TaskType::CleanupLoop,
            TaskPriority::High,
        ).await;

        let cleanup_loop = CleanupLoop::new(
            self.storage.clone(),
            self.config.clone(),
            self.metrics.clone(),
        );

        // Spawn the cleanup loop
        let handle = tokio::spawn(async move {
            cleanup_loop.start(task_handle).await;
        });

        // Update the task handle in the shutdown coordinator
        self.shutdown_coordinator.update_task_handle("cleanup-loop", handle).await;

        info!("✅ Cleanup loop started");
        Ok(())
    }

    /// Start periodic metrics updater
    async fn start_metrics_updater(&self) -> Result<()> {
        info!("📊 Starting metrics updater");

        // Register metrics task with shutdown coordinator
        let task_handle = self.shutdown_coordinator.register_task(
            "metrics-updater".to_string(),
            TaskType::MetricsMonitor,
            TaskPriority::Normal,
        ).await;

        let storage = self.storage.clone();
        let metrics = self.metrics.clone();

        // Spawn the metrics updater
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            let mut task_handle = task_handle;

            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Err(e) = update_metrics(storage.as_ref(), &*metrics).await {
                            warn!("Failed to update metrics: {}", e);
                        }
                    }
                    _ = task_handle.wait_for_shutdown() => {
                        info!("🛑 Metrics updater received shutdown signal");
                        break;
                    }
                }
            }

            task_handle.mark_completed().await;
            info!("✅ Metrics updater shut down gracefully");
        });

        // Update the task handle in the shutdown coordinator
        self.shutdown_coordinator.update_task_handle("metrics-updater", handle).await;

        info!("✅ Metrics updater started");
        Ok(())
    }

    /// Get the shutdown coordinator for external use
    pub fn shutdown_coordinator(&self) -> &ShutdownCoordinator {
        &self.shutdown_coordinator
    }

    /// Gracefully shutdown the service
    pub async fn shutdown(&self) {
        info!("🛑 Initiating service shutdown");
        self.shutdown_coordinator
            .initiate_shutdown(crate::shutdown::ShutdownReason::Graceful)
            .await;
    }
}

/// Update metrics from storage statistics
async fn update_metrics(storage: &dyn Storage, metrics: &Metrics) -> Result<()> {
    let stats = storage.get_stats().await?;
    metrics.update_from_lease_stats(&stats);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;

    #[tokio::test]
    async fn test_service_creation() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();

        let _service = GCService::new(storage, config, metrics);
        // Test passes if we can create the service without panicking
    }

    #[tokio::test]
    async fn test_metrics_update() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let metrics = Metrics::new();

        // Test the update_metrics function - FIXED: use as_ref() instead of &**
        update_metrics(storage.as_ref(), &*metrics).await.unwrap();
        
        // Should not panic and should update metrics
    }
}