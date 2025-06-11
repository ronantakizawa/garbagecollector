// src/service/mod.rs - Refactored main service module

use std::net::SocketAddr;
use std::sync::Arc;
use tonic::transport::Server;
use tracing::{info, error};

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::metrics::Metrics;
use crate::proto::distributed_gc_service_server::DistributedGcServiceServer;
use crate::storage::{create_persistent_storage, create_storage, PersistentStorage, Storage};

// Submodules
pub mod handlers;
pub mod validation;
pub mod cleanup_loop;
pub mod background_tasks;
pub mod recovery_integration;
pub mod metrics_server;

// Re-export commonly used items
pub use handlers::GCServiceHandlers;
pub use cleanup_loop::CleanupLoopManager;
pub use background_tasks::BackgroundTaskManager;
pub use metrics_server::MetricsServer;

#[cfg(feature = "persistent")]
pub use recovery_integration::RecoveryIntegration;

/// Main GarbageTruck service with persistent storage and recovery
#[derive(Clone)]
pub struct GCService {
    storage: Arc<dyn Storage>,
    persistent_storage: Option<Arc<dyn PersistentStorage>>,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
    shutdown_tx: Option<Arc<tokio::sync::broadcast::Sender<()>>>,
    cleanup_loop: Arc<CleanupLoopManager>,
    background_tasks: Arc<BackgroundTaskManager>,
    #[cfg(feature = "persistent")]
    recovery_integration: Option<Arc<RecoveryIntegration>>,
}

impl GCService {
    /// Create a new GC service with automatic storage backend selection
    pub async fn new(config: Arc<Config>, metrics: Arc<Metrics>) -> Result<Self> {
        info!("🚀 Initializing GarbageTruck service");

        // Create storage backend
        let storage = create_storage(&config).await?;
        
        // Create persistent storage if using persistent backend
        let persistent_storage = if config.storage.backend == "persistent_file" {
            let storage_config = crate::config::StorageConfig {
                backend: config.storage.backend.clone(),
                persistent_config: config.storage.persistent_config.clone(),
                enable_wal: config.storage.enable_wal,
                enable_auto_recovery: config.storage.enable_auto_recovery,
                recovery_strategy: config.storage.recovery_strategy.clone(),
                snapshot_interval_seconds: config.storage.snapshot_interval_seconds,
                wal_compaction_threshold: config.storage.wal_compaction_threshold,
            };
            Some(create_persistent_storage(&storage_config).await?)
        } else {
            None
        };

        // Create recovery integration if we have persistent storage and recovery is enabled
        #[cfg(feature = "persistent")]
        let recovery_integration = if let Some(ref persistent) = persistent_storage {
            if config.storage.enable_auto_recovery {
                let recovery_config = config.recovery.clone().unwrap_or_default();
                Some(Arc::new(RecoveryIntegration::new(
                    persistent.clone(),
                    storage.clone(),
                    recovery_config,
                    metrics.clone(),
                )))
            } else {
                None
            }
        } else {
            None
        };

        #[cfg(not(feature = "persistent"))]
        let recovery_integration = None;

        // Create background task manager
        let background_tasks = Arc::new(BackgroundTaskManager::new(
            storage.clone(),
            persistent_storage.clone(),
            config.clone(),
            metrics.clone(),
        ));

        // Create cleanup loop manager
        let cleanup_loop = Arc::new(CleanupLoopManager::new(
            storage.clone(),
            config.clone(),
            metrics.clone(),
        ));

        Ok(Self {
            storage,
            persistent_storage,
            config,
            metrics,
            shutdown_tx: None,
            cleanup_loop,
            background_tasks,
            #[cfg(feature = "persistent")]
            recovery_integration,
        })
    }

    /// Create a new service with shutdown capability for testing
    pub async fn new_with_shutdown(
        config: Arc<Config>,
        metrics: Arc<Metrics>,
    ) -> Result<(Self, tokio::sync::broadcast::Receiver<()>)> {
        let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
        let mut service = Self::new(config, metrics).await?;
        service.shutdown_tx = Some(Arc::new(shutdown_tx));
        Ok((service, shutdown_rx))
    }

    /// Perform startup recovery if enabled
    async fn startup_recovery(&self) -> Result<()> {
        #[cfg(feature = "persistent")]
        if let Some(ref recovery_integration) = self.recovery_integration {
            return recovery_integration.perform_startup_recovery().await;
        }
        
        #[cfg(not(feature = "persistent"))]
        {
            tracing::debug!("Persistent storage features disabled, skipping recovery");
        }
        
        Ok(())
    }

    /// Start the GarbageTruck service
    pub async fn start(&self, addr: SocketAddr) -> Result<()> {
        info!("🚀 Starting GarbageTruck service on {}", addr);

        // Perform startup recovery if enabled
        self.startup_recovery().await?;

        // Start background tasks
        self.background_tasks.start_all(self.shutdown_tx.clone()).await;

        // Start cleanup loop
        self.cleanup_loop.start(self.shutdown_tx.clone()).await;

        // Create the gRPC service handlers
        let handlers = GCServiceHandlers::new(
            self.storage.clone(),
            self.config.clone(),
            self.metrics.clone(),
        );

        // Start metrics server if enabled
        if self.config.metrics.enabled {
            let metrics_server = MetricsServer::new(
                self.config.clone(),
                self.metrics.clone(),
            );
            metrics_server.start().await?;
        }

        // Create and start the gRPC server
        let grpc_service = DistributedGcServiceServer::new(handlers);
        let shutdown_future = self.create_shutdown_future();

        info!("🌐 gRPC server starting on {}", addr);

        // Start the server with graceful shutdown
        Server::builder()
            .add_service(grpc_service)
            .serve_with_shutdown(addr, shutdown_future)
            .await
            .map_err(|e| GCError::Internal(format!("gRPC server error: {}", e)))?;

        info!("👋 GarbageTruck service stopped gracefully");
        Ok(())
    }

    /// Create shutdown future for graceful shutdown
    async fn create_shutdown_future(&self) {
        if let Some(ref shutdown_tx) = self.shutdown_tx {
            let mut shutdown_rx = shutdown_tx.subscribe();
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("🛑 Received Ctrl+C signal");
                }
                _ = shutdown_rx.recv() => {
                    info!("🛑 Received shutdown signal");
                }
            }
        } else {
            // If no shutdown channel, just wait for Ctrl+C
            let _ = tokio::signal::ctrl_c().await;
            info!("🛑 Received Ctrl+C signal");
        }
    }

    /// Shutdown the service gracefully
    pub async fn shutdown(&self) -> Result<()> {
        info!("🛑 Shutting down GarbageTruck service...");

        if let Some(ref shutdown_tx) = self.shutdown_tx {
            let _ = shutdown_tx.send(());
        }

        // Stop cleanup loop
        self.cleanup_loop.stop().await;

        // Stop background tasks
        self.background_tasks.stop_all().await;

        // Force sync WAL if using persistent storage
        if let Some(ref persistent_storage) = self.persistent_storage {
            info!("💾 Syncing WAL before shutdown");
            if let Err(e) = persistent_storage.sync_wal().await {
                tracing::warn!("Failed to sync WAL during shutdown: {}", e);
            }
        }

        // Give a moment for cleanup tasks to finish
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        info!("✅ GarbageTruck service shutdown complete");
        Ok(())
    }

    /// Get recovery API handle for external control
    #[cfg(feature = "persistent")]
    pub fn recovery_api(&self) -> Option<crate::recovery::RecoveryAPI> {
        self.recovery_integration.as_ref().map(|ri| ri.get_api())
    }

    /// Get service status including recovery information
    pub async fn get_status(&self) -> ServiceStatus {
        let storage_stats = self.storage.get_stats().await.unwrap_or_default();
        
        #[cfg(feature = "persistent")]
        let recovery_status = if let Some(ref recovery_integration) = self.recovery_integration {
            recovery_integration.get_status().await
        } else {
            None
        };
        
        #[cfg(not(feature = "persistent"))]
        let recovery_status = None;

        #[cfg(feature = "persistent")]
        let wal_status = if let Some(ref persistent_storage) = self.persistent_storage {
            Some(WALStatus {
                current_sequence: persistent_storage.current_sequence_number().await.unwrap_or(0),
                integrity_issues: persistent_storage.verify_wal_integrity().await.unwrap_or_default(),
            })
        } else {
            None
        };
        
        #[cfg(not(feature = "persistent"))]
        let wal_status = None;

        ServiceStatus {
            storage_backend: self.config.storage.backend.clone(),
            total_leases: storage_stats.total_leases,
            active_leases: storage_stats.active_leases,
            persistent_features_enabled: self.persistent_storage.is_some(),
            recovery_status,
            wal_status,
        }
    }

    /// Get storage interface for testing
    pub fn storage(&self) -> Arc<dyn Storage> {
        self.storage.clone()
    }

    /// Get config for testing
    pub fn config(&self) -> Arc<Config> {
        self.config.clone()
    }

    /// Get metrics for testing
    pub fn metrics(&self) -> Arc<Metrics> {
        self.metrics.clone()
    }
}

/// Service status information
#[derive(Debug, Clone, serde::Serialize)]
pub struct ServiceStatus {
    pub storage_backend: String,
    pub total_leases: usize,
    pub active_leases: usize,
    pub persistent_features_enabled: bool,
    #[cfg(feature = "persistent")]
    pub recovery_status: Option<crate::recovery::RecoveryProgress>,
    #[cfg(not(feature = "persistent"))]
    pub recovery_status: Option<()>, // Placeholder when persistent features disabled
    pub wal_status: Option<WALStatus>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct WALStatus {
    pub current_sequence: u64,
    pub integrity_issues: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[tokio::test]
    async fn test_service_creation() {
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();

        let service = GCService::new(config, metrics).await.unwrap();
        assert_eq!(service.config.storage.backend, "memory");
        assert!(service.persistent_storage.is_none());
        #[cfg(feature = "persistent")]
        assert!(service.recovery_integration.is_none());
    }

    #[tokio::test]
    async fn test_service_status() {
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();

        let service = GCService::new(config, metrics).await.unwrap();
        let status = service.get_status().await;

        assert_eq!(status.storage_backend, "memory");
        assert!(!status.persistent_features_enabled);
        assert!(status.recovery_status.is_none());
        assert!(status.wal_status.is_none());
    }

    #[tokio::test]
    async fn test_service_with_shutdown() {
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();

        let (service, _shutdown_rx) = GCService::new_with_shutdown(config, metrics).await.unwrap();
        assert!(service.shutdown_tx.is_some());
    }
}