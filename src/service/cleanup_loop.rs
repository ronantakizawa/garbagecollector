// src/service/cleanup_loop.rs - Dedicated cleanup loop management

use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::cleanup::CleanupExecutor;
use crate::config::Config;
use crate::error::Result;
use crate::lease::{Lease, LeaseState};
use crate::metrics::Metrics;
use crate::storage::Storage;

/// Manages the background cleanup loop
pub struct CleanupLoopManager {
    storage: Arc<dyn Storage>,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
    cleanup_executor: CleanupExecutor,
    task_handle: Arc<Mutex<Option<tokio::task::JoinHandle<()>>>>,
}

impl CleanupLoopManager {
    pub fn new(
        storage: Arc<dyn Storage>,
        config: Arc<Config>,
        metrics: Arc<Metrics>,
    ) -> Self {
        let cleanup_executor = CleanupExecutor::new(
            Duration::from_secs(config.cleanup.default_timeout_seconds),
            config.cleanup.default_max_retries,
            Duration::from_secs(config.cleanup.default_retry_delay_seconds),
        );

        Self {
            storage,
            config,
            metrics,
            cleanup_executor,
            task_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the cleanup loop in the background
    pub async fn start(&self, shutdown_tx: Option<Arc<tokio::sync::broadcast::Sender<()>>>) {
        let storage = self.storage.clone();
        let config = self.config.clone();
        let metrics = self.metrics.clone();
        let cleanup_executor = self.cleanup_executor.clone();
        let mut shutdown_rx = shutdown_tx.as_ref().map(|tx| tx.subscribe());

        let task_handle = tokio::spawn(async move {
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
                                metrics.increment_cleanup_failures();
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

        *self.task_handle.lock().await = Some(task_handle);
        info!("✅ Cleanup loop started in background");
    }

    /// Stop the cleanup loop
    pub async fn stop(&self) {
        let mut handle_guard = self.task_handle.lock().await;
        if let Some(handle) = handle_guard.take() {
            handle.abort();
            info!("🛑 Cleanup loop stopped");
        }
    }

    /// Check if the cleanup loop is running
    pub async fn is_running(&self) -> bool {
        let handle_guard = self.task_handle.lock().await;
        handle_guard.as_ref().map_or(false, |h| !h.is_finished())
    }

    /// Get cleanup statistics
    pub async fn get_stats(&self) -> CleanupStats {
        // This would be populated from metrics in a real implementation
        CleanupStats {
            total_cycles: 0,
            successful_cycles: 0,
            failed_cycles: 0,
            total_leases_cleaned: 0,
            last_cycle_duration: Duration::ZERO,
        }
    }
}

/// Run a single cleanup cycle
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
                metrics.increment_successful_cleanups();
            }
        } else {
            failed_cleanups += 1;
            error!(
                "❌ Cleanup failed for lease {} (object: {}): {:?}",
                lease.lease_id, lease.object_id, result.error
            );
            metrics.increment_cleanup_failures();
        }
    }

    if failed_cleanups > 0 {
        warn!("⚠️ {} cleanups failed this cycle", failed_cleanups);
    }

    Ok((expired_count, successful_cleanups))
}

/// Cleanup statistics
#[derive(Debug, Clone)]
pub struct CleanupStats {
    pub total_cycles: u64,
    pub successful_cycles: u64,
    pub failed_cycles: u64,
    pub total_leases_cleaned: u64,
    pub last_cycle_duration: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;
    use crate::lease::{ObjectType, Lease};
    use std::collections::HashMap;

    #[tokio::test]
    async fn test_cleanup_loop_manager_creation() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();

        let manager = CleanupLoopManager::new(storage, config, metrics);
        assert!(!manager.is_running().await);
    }

    #[tokio::test]
    async fn test_cleanup_cycle_with_no_leases() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();
        let cleanup_executor = CleanupExecutor::new(
            Duration::from_secs(30),
            3,
            Duration::from_secs(5),
        );

        let result = run_cleanup_cycle(&storage, &cleanup_executor, &config, &metrics).await;
        assert!(result.is_ok());
        let (expired, cleaned) = result.unwrap();
        assert_eq!(expired, 0);
        assert_eq!(cleaned, 0);
    }

    #[tokio::test]
    async fn test_cleanup_cycle_with_expired_lease() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();
        let cleanup_executor = CleanupExecutor::new(
            Duration::from_secs(30),
            3,
            Duration::from_secs(5),
        );

        // Create an expired lease
        let mut lease = Lease::new(
            "test-object".to_string(),
            ObjectType::TemporaryFile,
            "test-service".to_string(),
            Duration::from_millis(1), // Very short duration
            HashMap::new(),
            None,
        );
        
        storage.create_lease(lease.clone()).await.unwrap();
        
        // Wait for it to expire
        tokio::time::sleep(Duration::from_millis(10)).await;

        let result = run_cleanup_cycle(&storage, &cleanup_executor, &config, &metrics).await;
        assert!(result.is_ok());
        let (expired, _cleaned) = result.unwrap();
        assert_eq!(expired, 1);
    }
}