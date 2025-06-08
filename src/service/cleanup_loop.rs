// src/service/cleanup_loop.rs - Fixed cleanup loop that actually executes cleanup

use std::sync::Arc;
use std::time::Duration;
use tokio::time::{interval, sleep};
use tracing::{debug, error, info, warn};

use crate::cleanup::CleanupExecutor;
use crate::config::Config;
use crate::error::Result;
use crate::lease::{Lease, LeaseState};
use crate::metrics::Metrics;
use crate::storage::Storage;

/// Background cleanup loop that processes expired leases
pub struct CleanupLoop {
    storage: Arc<dyn Storage>,
    cleanup_executor: CleanupExecutor,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
}

impl CleanupLoop {
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
            cleanup_executor,
            config,
            metrics,
        }
    }

    /// Start the cleanup loop
    pub async fn start(&self) -> Result<()> {
        let mut cleanup_interval = interval(self.config.cleanup_interval());
        let grace_period = self.config.cleanup_grace_period();

        info!(
            "🧹 Starting cleanup loop (interval: {}s, grace: {}s)",
            self.config.gc.cleanup_interval_seconds,
            self.config.gc.cleanup_grace_period_seconds
        );

        loop {
            cleanup_interval.tick().await;

            if let Err(e) = self.run_cleanup_cycle(grace_period).await {
                error!("Cleanup cycle failed: {}", e);
                self.metrics.increment_cleanup_errors();
                
                // Don't exit the loop, just log and continue
                warn!("Cleanup cycle failed, continuing with next cycle...");
                sleep(Duration::from_secs(5)).await; // Brief pause before retry
            }
        }
    }

    async fn run_cleanup_cycle(&self, grace_period: Duration) -> Result<()> {
        debug!("🔍 Starting cleanup cycle");

        // Step 1: Mark expired leases
        let expired_count = self.mark_expired_leases().await?;
        if expired_count > 0 {
            info!("⏰ Marked {} leases as expired", expired_count);
        }

        // Step 2: Get leases that need cleanup (expired + past grace period)
        let leases_to_cleanup = self.get_leases_needing_cleanup(grace_period).await?;
        
        if leases_to_cleanup.is_empty() {
            debug!("✨ No leases need cleanup this cycle");
            return Ok(());
        }

        info!("🧹 Found {} leases needing cleanup", leases_to_cleanup.len());

        // Step 3: Execute cleanup for each lease
        let cleanup_results = self.cleanup_executor.cleanup_batch(leases_to_cleanup.clone()).await;

        // Step 4: Process cleanup results
        let mut successful_cleanups = 0;
        let mut failed_cleanups = 0;

        for (lease, result) in leases_to_cleanup.iter().zip(cleanup_results.iter()) {
            if result.success {
                // Mark lease as cleaned up in storage
                if let Err(e) = self.storage.delete_lease(&lease.lease_id).await {
                    warn!("Failed to delete lease {} from storage: {}", lease.lease_id, e);
                } else {
                    debug!("✅ Deleted lease {} from storage", lease.lease_id);
                }
                successful_cleanups += 1;
                self.metrics.increment_successful_cleanups();
            } else {
                warn!(
                    "❌ Cleanup failed for lease {} ({}): {:?}",
                    lease.lease_id, lease.object_id, result.error
                );
                failed_cleanups += 1;
                self.metrics.increment_failed_cleanups();
            }
        }

        if successful_cleanups > 0 || failed_cleanups > 0 {
            info!(
                "📊 Cleanup cycle completed: {} successful, {} failed",
                successful_cleanups, failed_cleanups
            );
        }

        // Step 5: Update metrics
        self.metrics.record_cleanup_cycle(successful_cleanups, failed_cleanups);

        Ok(())
    }

    async fn mark_expired_leases(&self) -> Result<usize> {
        // Get all leases and filter for active ones that are expired
        let all_leases = self.storage.list_leases(None, Some(1000)).await?;
        let mut expired_count = 0;

        for lease in all_leases {
            if lease.state == LeaseState::Active && lease.is_expired() {
                // Update the lease to expired state
                let mut expired_lease = lease.clone();
                expired_lease.expire();
                
                if let Err(e) = self.storage.update_lease(&expired_lease).await {
                    warn!("Failed to mark lease {} as expired: {}", lease.lease_id, e);
                } else {
                    debug!("⏰ Marked lease {} as expired", lease.lease_id);
                    expired_count += 1;
                }
            }
        }

        Ok(expired_count)
    }

    async fn get_leases_needing_cleanup(&self, grace_period: Duration) -> Result<Vec<Lease>> {
        // Get all leases and filter for expired ones that need cleanup
        let all_leases = self.storage.list_leases(None, Some(1000)).await?;
        let mut leases_to_cleanup = Vec::new();

        for lease in all_leases {
            if lease.state == LeaseState::Expired {
                // Check if the lease has passed the grace period and has cleanup config
                if lease.should_cleanup(grace_period) && lease.cleanup_config.is_some() {
                    debug!(
                        "🎯 Lease {} needs cleanup (expired: {}, grace passed: {})",
                        lease.lease_id,
                        lease.is_expired(),
                        lease.should_cleanup(grace_period)
                    );
                    leases_to_cleanup.push(lease);
                } else if lease.should_cleanup(grace_period) && lease.cleanup_config.is_none() {
                    // No cleanup config, just delete from storage
                    debug!(
                        "🗑️ Lease {} has no cleanup config, removing from storage",
                        lease.lease_id
                    );
                    if let Err(e) = self.storage.delete_lease(&lease.lease_id).await {
                        warn!("Failed to delete lease {} from storage: {}", lease.lease_id, e);
                    }
                }
            }
        }

        Ok(leases_to_cleanup)
    }
}

// Add these methods to the Metrics struct if they don't exist
impl crate::metrics::Metrics {
    pub fn increment_cleanup_errors(&self) {
        // Implementation depends on your metrics structure
        // This is a placeholder - adjust based on your actual Metrics implementation
    }

    pub fn increment_successful_cleanups(&self) {
        // Implementation depends on your metrics structure
    }

    pub fn increment_failed_cleanups(&self) {
        // Implementation depends on your metrics structure
    }

    pub fn record_cleanup_cycle(&self, _successful: usize, _failed: usize) {
        // Implementation depends on your metrics structure
    }
}