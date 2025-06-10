// src/service/mod.rs - Updated GC Service with persistent storage and recovery

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::time::interval;
use tonic::transport::Server;
use tracing::{debug, error, info, warn};

use crate::cleanup::CleanupExecutor;
use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::LeaseState;
use crate::metrics::Metrics;
use crate::proto::distributed_gc_service_server::DistributedGcServiceServer;
use crate::recovery::manager::{RecoveryManager, RecoveryTrigger};
use crate::storage::{create_persistent_storage, create_storage, PersistentStorage, Storage};

pub mod handlers;
pub mod validation;

use handlers::GCServiceHandlers;

/// Main GarbageTruck service with persistent storage and recovery
#[derive(Clone)]
pub struct GCService {
    storage: Arc<dyn Storage>,
    persistent_storage: Option<Arc<dyn PersistentStorage>>,
    recovery_manager: Option<Arc<RecoveryManager>>,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
    shutdown_tx: Option<Arc<tokio::sync::broadcast::Sender<()>>>,
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

        // Create recovery manager if we have persistent storage and recovery is enabled
        #[cfg(feature = "persistent")]
        let recovery_manager = if let Some(ref persistent) = persistent_storage {
            if config.storage.enable_auto_recovery {
                let recovery_config = config.recovery.clone().unwrap_or_default();
                Some(Arc::new(RecoveryManager::new(
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
        let recovery_manager = None;

        Ok(Self {
            storage,
            persistent_storage,
            recovery_manager,
            config,
            metrics,
            shutdown_tx: None,
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
        if let Some(ref recovery_manager) = self.recovery_manager {
            info!("🔄 Performing startup recovery");
            
            let recovery_result = recovery_manager.recover(RecoveryTrigger::ServiceStart).await?;
            
            if recovery_result.success {
                info!(
                    "✅ Startup recovery completed: {} leases recovered in {:.2}s",
                    recovery_result.recovery_info.leases_recovered,
                    recovery_result.total_time.as_secs_f64()
                );
                
                // Update metrics with recovery information
                self.metrics.record_recovery_success(
                    recovery_result.recovery_info.leases_recovered,
                    recovery_result.total_time,
                );
            } else {
                warn!(
                    "⚠️ Startup recovery had issues: {}",
                    recovery_result.error.unwrap_or_else(|| "Unknown error".to_string())
                );
                
                self.metrics.record_recovery_failure();
            }
        }
        
        #[cfg(not(feature = "persistent"))]
        debug!("Persistent storage features disabled, skipping recovery");
        
        Ok(())
    }

    /// Start background monitoring and maintenance tasks
    async fn start_background_tasks(&self) {
        // Start cleanup loop
        self.start_cleanup_loop();
        
        // Start WAL compaction task if using persistent storage
        if let Some(ref persistent_storage) = self.persistent_storage {
            self.start_wal_compaction_task(persistent_storage.clone()).await;
        }
        
        // Start snapshot task if configured
        if let Some(ref persistent_storage) = self.persistent_storage {
            if self.config.storage.snapshot_interval_seconds > 0 {
                self.start_snapshot_task(persistent_storage.clone()).await;
            }
        }
        
        // Start recovery monitoring if recovery manager is available
        #[cfg(feature = "persistent")]
        if let Some(ref recovery_manager) = self.recovery_manager {
            self.start_recovery_monitoring(recovery_manager.clone()).await;
        }
    }

    /// Start WAL compaction background task
    async fn start_wal_compaction_task(&self, persistent_storage: Arc<dyn PersistentStorage>) {
        let config = self.config.clone();
        let metrics = self.metrics.clone();
        let mut shutdown_rx = self.shutdown_tx.as_ref().map(|tx| tx.subscribe());

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(3600)); // Check every hour
            
            info!("📦 Started WAL compaction task");
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Ok(current_seq) = persistent_storage.current_sequence_number().await {
                            let threshold = config.storage.wal_compaction_threshold;
                            
                            if current_seq > threshold {
                                let compact_before = current_seq - (threshold / 2);
                                
                                info!("🗜️ Starting WAL compaction before sequence {}", compact_before);
                                
                                match persistent_storage.compact_wal(compact_before).await {
                                    Ok(_) => {
                                        info!("✅ WAL compaction completed");
                                        metrics.record_wal_compaction_success();
                                    }
                                    Err(e) => {
                                        error!("❌ WAL compaction failed: {}", e);
                                        metrics.record_wal_compaction_failure();
                                    }
                                }
                            }
                        }
                    }
                    _ = async {
                        if let Some(ref mut rx) = shutdown_rx {
                            let _ = rx.recv().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {
                        info!("🛑 WAL compaction task received shutdown signal");
                        break;
                    }
                }
            }
        });
    }

    /// Start snapshot creation background task
    async fn start_snapshot_task(&self, persistent_storage: Arc<dyn PersistentStorage>) {
        let interval_seconds = self.config.storage.snapshot_interval_seconds;
        let metrics = self.metrics.clone();
        let mut shutdown_rx = self.shutdown_tx.as_ref().map(|tx| tx.subscribe());

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(interval_seconds));
            
            info!("📸 Started snapshot task (interval: {}s)", interval_seconds);
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        info!("📸 Creating periodic snapshot");
                        
                        match persistent_storage.create_snapshot().await {
                            Ok(snapshot_path) => {
                                info!("✅ Snapshot created: {}", snapshot_path);
                                metrics.record_snapshot_success();
                            }
                            Err(e) => {
                                error!("❌ Snapshot creation failed: {}", e);
                                metrics.record_snapshot_failure();
                            }
                        }
                    }
                    _ = async {
                        if let Some(ref mut rx) = shutdown_rx {
                            let _ = rx.recv().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {
                        info!("🛑 Snapshot task received shutdown signal");
                        break;
                    }
                }
            }
        });
    }

    /// Start recovery monitoring task
    #[cfg(feature = "persistent")]
    async fn start_recovery_monitoring(&self, recovery_manager: Arc<RecoveryManager>) {
        let metrics = self.metrics.clone();
        let mut shutdown_rx = self.shutdown_tx.as_ref().map(|tx| tx.subscribe());

        tokio::spawn(async move {
            let mut health_check_interval = interval(Duration::from_secs(300)); // Check every 5 minutes
            
            info!("🔍 Started recovery monitoring task");
            
            loop {
                tokio::select! {
                    _ = health_check_interval.tick() => {
                        // Check if recovery is needed
                        if let Some(progress) = recovery_manager.get_recovery_progress().await {
                            metrics.record_recovery_progress(
                                progress.progress_percent,
                                progress.errors.len(),
                                progress.warnings.len(),
                            );
                        }
                        
                        // Check for any pending recovery triggers
                        // This would typically involve checking system health,
                        // storage integrity, etc.
                    }
                    _ = async {
                        if let Some(ref mut rx) = shutdown_rx {
                            let _ = rx.recv().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {
                        info!("🛑 Recovery monitoring task received shutdown signal");
                        break;
                    }
                }
            }
        });
    }

    /// Shutdown the service gracefully
    pub async fn shutdown(&self) -> Result<()> {
        info!("🛑 Shutting down GarbageTruck service...");

        if let Some(ref shutdown_tx) = self.shutdown_tx {
            let _ = shutdown_tx.send(());
        }

        // Force sync WAL if using persistent storage
        if let Some(ref persistent_storage) = self.persistent_storage {
            info!("💾 Syncing WAL before shutdown");
            if let Err(e) = persistent_storage.sync_wal().await {
                warn!("Failed to sync WAL during shutdown: {}", e);
            }
        }

        // Give a moment for cleanup tasks to finish
        tokio::time::sleep(Duration::from_millis(500)).await;

        info!("✅ GarbageTruck service shutdown complete");
        Ok(())
    }

    /// Start the GarbageTruck service
    pub async fn start(&self, addr: SocketAddr) -> Result<()> {
        info!("🚀 Starting GarbageTruck service on {}", addr);

        // Perform startup recovery if enabled
        self.startup_recovery().await?;

        // Start background tasks
        self.start_background_tasks().await;

        // Create the gRPC service handlers
        let handlers = GCServiceHandlers::new(
            self.storage.clone(),
            self.config.clone(),
            self.metrics.clone(),
        );

        // Start metrics server if enabled
        if self.config.metrics.enabled {
            let metrics_addr = format!("0.0.0.0:{}", self.config.metrics.port);
            let metrics = self.metrics.clone();

            tokio::spawn(async move {
                if let Err(e) = start_metrics_server(metrics_addr, metrics).await {
                    error!("❌ Metrics server failed: {}", e);
                }
            });

            info!(
                "📊 Metrics server started on port {} (recovery metrics: {}, WAL metrics: {})",
                self.config.metrics.port,
                self.config.metrics.include_recovery_metrics,
                self.config.metrics.include_wal_metrics
            );
        }

        // Create and start the gRPC server
        let grpc_service = DistributedGcServiceServer::new(handlers);

        info!("🌐 gRPC server starting on {}", addr);

        // Create shutdown future
        let shutdown_future = async {
            if let Some(ref shutdown_tx) = self.shutdown_tx {
                let mut shutdown_rx = shutdown_tx.subscribe();
                tokio::select! {
                    _ = signal::ctrl_c() => {
                        info!("🛑 Received Ctrl+C signal");
                    }
                    _ = shutdown_rx.recv() => {
                        info!("🛑 Received shutdown signal");
                    }
                }
            } else {
                // If no shutdown channel, just wait for Ctrl+C
                let _ = signal::ctrl_c().await;
                info!("🛑 Received Ctrl+C signal");
            }
        };

        // Start the server with graceful shutdown
        Server::builder()
            .add_service(grpc_service)
            .serve_with_shutdown(addr, shutdown_future)
            .await
            .map_err(|e| GCError::Internal(format!("gRPC server error: {}", e)))?;

        info!("👋 GarbageTruck service stopped gracefully");
        Ok(())
    }

    /// Start the cleanup loop in the background
    fn start_cleanup_loop(&self) {
        let storage = self.storage.clone();
        let config = self.config.clone();
        let metrics = self.metrics.clone();
        let mut shutdown_rx = self.shutdown_tx.as_ref().map(|tx| tx.subscribe());

        // Create cleanup executor
        let cleanup_executor = CleanupExecutor::new(
            Duration::from_secs(config.cleanup.default_timeout_seconds),
            config.cleanup.default_max_retries,
            Duration::from_secs(config.cleanup.default_retry_delay_seconds),
        );

        // Start background cleanup task
        tokio::spawn(async move {
            let mut cleanup_interval =
                interval(Duration::from_secs(config.gc.cleanup_interval_seconds));

            info!(
                "🧹 Starting cleanup loop (interval: {}s, grace: {}s)",
                config.gc.cleanup_interval_seconds, config.gc.cleanup_grace_period_seconds
            );

            loop {
                tokio::select! {
                    _ = cleanup_interval.tick() => {
                        // Run cleanup cycle
                        match run_cleanup_cycle(&storage, &cleanup_executor, &config, &metrics).await {
                            Ok((expired, cleaned)) => {
                                if expired > 0 || cleaned > 0 {
                                    info!("🧹 Cleanup cycle completed: {} expired, {} cleaned", expired, cleaned);
                                } else {
                                    debug!("✨ Cleanup cycle completed: no action needed");
                                }
                            }
                            Err(e) => {
                                error!("❌ Cleanup cycle failed: {}", e);
                                metrics.record_cleanup_failure();
                                // Don't exit the loop, just continue with next cycle
                                tokio::time::sleep(Duration::from_secs(5)).await;
                            }
                        }
                    }
                    _ = async {
                        if let Some(ref mut rx) = shutdown_rx {
                            let _ = rx.recv().await;
                        } else {
                            // If no shutdown receiver, wait forever
                            std::future::pending::<()>().await;
                        }
                    } => {
                        info!("🛑 Cleanup loop received shutdown signal");
                        break;
                    }
                }
            }

            info!("🧹 Cleanup loop stopped");
        });

        info!("✅ Cleanup loop started in background");
    }

    /// Get recovery API handle for external control
    #[cfg(feature = "persistent")]
    pub fn recovery_api(&self) -> Option<crate::recovery::RecoveryAPI> {
        self.recovery_manager.as_ref().map(|rm| {
            crate::recovery::RecoveryAPI::new(rm.clone())
        })
    }

    /// Get service status including recovery information
    pub async fn get_status(&self) -> ServiceStatus {
        let storage_stats = self.storage.get_stats().await.unwrap_or_default();
        
        #[cfg(feature = "persistent")]
        let recovery_status = if let Some(ref recovery_manager) = self.recovery_manager {
            recovery_manager.get_recovery_progress().await
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

/// Run a single cleanup cycle (updated to work with both storage types)
async fn run_cleanup_cycle(
    storage: &Arc<dyn Storage>,
    cleanup_executor: &CleanupExecutor,
    config: &Config,
    metrics: &Arc<Metrics>,
) -> Result<(usize, usize)> {
    debug!("🔍 Starting cleanup cycle");

    // Get all leases
    let all_leases = storage.list_leases(None, Some(1000)).await?;
    let grace_period = Duration::from_secs(config.gc.cleanup_grace_period_seconds);

    let mut expired_count = 0;
    let mut cleanup_candidates = Vec::new();

    // Process each lease
    for mut lease in all_leases {
        match lease.state {
            LeaseState::Active if lease.is_expired() => {
                // Mark as expired
                lease.expire();

                // Store lease info before the storage call
                let lease_id = lease.lease_id.clone();
                let object_id = lease.object_id.clone();

                if let Err(e) = storage.update_lease(lease).await {
                    warn!("Failed to update expired lease {}: {}", lease_id, e);
                } else {
                    expired_count += 1;
                    debug!(
                        "⏰ Marked lease {} as expired (object: {})",
                        lease_id, object_id
                    );
                }
            }
            LeaseState::Expired => {
                // Check if needs cleanup
                if lease.should_cleanup(grace_period) {
                    if lease.cleanup_config.is_some() {
                        debug!(
                            "🎯 Lease {} needs cleanup (object: {}, expired: {:?} ago)",
                            lease.lease_id,
                            lease.object_id,
                            lease.time_until_expiry()
                        );
                        cleanup_candidates.push(lease);
                    } else {
                        // No cleanup config, just remove from storage
                        debug!(
                            "🗑️ Lease {} has no cleanup config, removing from storage",
                            lease.lease_id
                        );
                        if let Err(e) = storage.delete_lease(&lease.lease_id).await {
                            warn!(
                                "Failed to delete lease {} from storage: {}",
                                lease.lease_id, e
                            );
                        }
                    }
                }
            }
            LeaseState::Released => {
                // Released leases with cleanup config should be cleaned immediately
                if lease.cleanup_config.is_some() {
                    debug!(
                        "🎯 Released lease {} needs immediate cleanup (object: {})",
                        lease.lease_id, lease.object_id
                    );
                    cleanup_candidates.push(lease);
                } else {
                    // No cleanup config, just remove from storage
                    if let Err(e) = storage.delete_lease(&lease.lease_id).await {
                        warn!(
                            "Failed to delete released lease {} from storage: {}",
                            lease.lease_id, e
                        );
                    }
                }
            }
            _ => {}
        }
    }

    if cleanup_candidates.is_empty() {
        if expired_count > 0 {
            debug!(
                "⏰ Marked {} leases as expired, no cleanup needed this cycle",
                expired_count
            );
        }
        return Ok((expired_count, 0));
    }

    info!(
        "🧹 Processing {} cleanup candidates",
        cleanup_candidates.len()
    );

    // Execute cleanup
    let cleanup_results = cleanup_executor
        .cleanup_batch(cleanup_candidates.clone())
        .await;

    let mut successful_cleanups = 0;
    let mut failed_cleanups = 0;

    // Process cleanup results
    for (lease, result) in cleanup_candidates.iter().zip(cleanup_results.iter()) {
        if result.success {
            // Remove lease from storage after successful cleanup
            if let Err(e) = storage.delete_lease(&lease.lease_id).await {
                warn!("Failed to delete cleaned lease {}: {}", lease.lease_id, e);
            } else {
                successful_cleanups += 1;
                info!(
                    "✅ Successfully cleaned up lease {} (object: {})",
                    lease.lease_id, lease.object_id
                );
                metrics.record_cleanup_success();
            }
        } else {
            failed_cleanups += 1;
            error!(
                "❌ Cleanup failed for lease {} (object: {}): {:?}",
                lease.lease_id, lease.object_id, result.error
            );
            metrics.record_cleanup_failure();
        }
    }

    if failed_cleanups > 0 {
        warn!("⚠️ {} cleanups failed this cycle", failed_cleanups);
    }

    Ok((expired_count, successful_cleanups))
}

/// Start metrics server with enhanced metrics
async fn start_metrics_server(addr: String, metrics: Arc<Metrics>) -> Result<()> {
    use warp::Filter;

    let metrics_route = warp::path("metrics")
        .and(warp::get())
        .and(warp::any().map(move || metrics.clone()))
        .and_then(handle_metrics_request);

    let health_route = warp::path("health").and(warp::get()).map(|| {
        warp::reply::json(&serde_json::json!({
            "status": "ok",
            "service": "garbagetruck-metrics",
            "features": ["persistent_storage", "recovery", "wal"]
        }))
    });

    let routes = metrics_route.or(health_route);

    info!("📊 Starting enhanced metrics server on http://{}", addr);

    warp::serve(routes)
        .run(addr.parse::<SocketAddr>().unwrap())
        .await;

    Ok(())
}

async fn handle_metrics_request(
    metrics: Arc<Metrics>,
) -> std::result::Result<impl warp::Reply, warp::Rejection> {
    // Return enhanced Prometheus metrics format including recovery and WAL metrics
    let metrics_text = metrics.export_enhanced_prometheus_metrics();
    Ok(warp::reply::with_header(
        metrics_text,
        "content-type",
        "text/plain; version=0.0.4; charset=utf-8",
    ))
}

// Extended Metrics implementation for persistent storage features
impl Metrics {
    pub fn record_recovery_success(&self, leases_recovered: usize, duration: Duration) {
        // Implementation would track recovery metrics
        debug!("Recovery success: {} leases in {:.2}s", leases_recovered, duration.as_secs_f64());
    }

    pub fn record_recovery_failure(&self) {
        // Implementation would track recovery failures
        debug!("Recovery failure recorded");
    }

    pub fn record_recovery_progress(&self, progress: f64, errors: usize, warnings: usize) {
        // Implementation would track ongoing recovery progress
        debug!("Recovery progress: {:.1}%, {} errors, {} warnings", progress, errors, warnings);
    }

    pub fn record_wal_compaction_success(&self) {
        // Implementation would track WAL compaction metrics
        debug!("WAL compaction success recorded");
    }

    pub fn record_wal_compaction_failure(&self) {
        // Implementation would track WAL compaction failures
        debug!("WAL compaction failure recorded");
    }

    pub fn record_snapshot_success(&self) {
        // Implementation would track snapshot creation metrics
        debug!("Snapshot creation success recorded");
    }

    pub fn record_snapshot_failure(&self) {
        // Implementation would track snapshot creation failures
        debug!("Snapshot creation failure recorded");
    }

    pub fn record_cleanup_success(&self) {
        // Implementation would track cleanup success metrics
        debug!("Cleanup success recorded");
    }

    pub fn record_cleanup_failure(&self) {
        // Implementation would track cleanup failure metrics
        debug!("Cleanup failure recorded");
    }

    pub fn export_enhanced_prometheus_metrics(&self) -> String {
        // This would return enhanced metrics including WAL, recovery, and persistent storage metrics
        format!(
            "# HELP garbagetruck_leases_total Total number of leases created\n\
             # TYPE garbagetruck_leases_total counter\n\
             garbagetruck_leases_total {}\n\
             \n\
             # HELP garbagetruck_cleanups_total Total number of cleanup operations\n\
             # TYPE garbagetruck_cleanups_total counter\n\
             garbagetruck_cleanups_total {}\n\
             \n\
             # HELP garbagetruck_active_leases Current number of active leases\n\
             # TYPE garbagetruck_active_leases gauge\n\
             garbagetruck_active_leases {}\n\
             \n\
             # HELP garbagetruck_recovery_operations_total Total number of recovery operations\n\
             # TYPE garbagetruck_recovery_operations_total counter\n\
             garbagetruck_recovery_operations_total 0\n\
             \n\
             # HELP garbagetruck_wal_entries_total Total number of WAL entries written\n\
             # TYPE garbagetruck_wal_entries_total counter\n\
             garbagetruck_wal_entries_total 0\n\
             \n\
             # HELP garbagetruck_snapshots_total Total number of snapshots created\n\
             # TYPE garbagetruck_snapshots_total counter\n\
             garbagetruck_snapshots_total 0\n",
            0, // Replace with actual lease count
            0, // Replace with actual cleanup count
            0  // Replace with actual active lease count
        )
    }
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
        assert!(service.recovery_manager.is_none());
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
}