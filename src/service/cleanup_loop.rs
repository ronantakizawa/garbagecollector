// src/service/cleanup_loop.rs - Real cleanup loop implementation

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time::{interval, sleep};
use tracing::{debug, error, info, warn};

use crate::cleanup::CleanupExecutor;
use crate::config::Config;
use crate::error::Result;
use crate::lease::{Lease, LeaseState};
use crate::metrics::Metrics;
use crate::storage::Storage;

/// Manages the continuous cleanup loop that processes expired leases
pub struct CleanupLoop {
    storage: Arc<dyn Storage>,
    cleanup_executor: CleanupExecutor,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
    shutdown_signal: tokio::sync::watch::Receiver<bool>,
}

impl CleanupLoop {
    /// Create a new cleanup loop
    pub fn new(
        storage: Arc<dyn Storage>,
        config: Arc<Config>,
        metrics: Arc<Metrics>,
        shutdown_signal: tokio::sync::watch::Receiver<bool>,
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
            shutdown_signal,
        }
    }

    /// Start the cleanup loop - this is the main entry point
    pub async fn start(mut self) -> Result<()> {
        info!(
            "🧹 Starting cleanup loop with {:.1}s interval, {:.1}s grace period",
            self.config.gc.cleanup_interval_seconds, self.config.gc.cleanup_grace_period_seconds
        );

        let mut cleanup_interval = interval(self.config.cleanup_interval());
        let mut stats_interval = interval(Duration::from_secs(60)); // Stats every minute

        loop {
            tokio::select! {
                // Check for shutdown signal
                _ = self.shutdown_signal.changed() => {
                    if *self.shutdown_signal.borrow() {
                        info!("🛑 Cleanup loop received shutdown signal");
                        break;
                    }
                }

                // Main cleanup tick
                _ = cleanup_interval.tick() => {
                    if let Err(e) = self.cleanup_tick().await {
                        error!("Cleanup tick failed: {}", e);
                        self.metrics.cleanup_failures_total.inc();
                    }
                }

                // Periodic stats logging
                _ = stats_interval.tick() => {
                    self.log_cleanup_stats().await;
                }
            }
        }

        info!("🧹 Cleanup loop stopped");
        Ok(())
    }

    /// Single cleanup iteration
    async fn cleanup_tick(&self) -> Result<()> {
        let start_time = Instant::now();

        debug!("🔍 Starting cleanup scan...");

        // Phase 1: Mark expired leases
        let marked_count = self.mark_expired_leases().await?;

        // Phase 2: Get leases ready for cleanup (past grace period)
        let grace_period = self.config.cleanup_grace_period();
        let expired_leases = self.storage.get_expired_leases(grace_period).await?;

        if expired_leases.is_empty() {
            debug!("✅ No leases ready for cleanup");
            return Ok(());
        }

        info!(
            "🧹 Found {} expired leases ready for cleanup (marked {} as expired this cycle)",
            expired_leases.len(),
            marked_count
        );

        // Phase 3: Execute cleanup operations
        let cleanup_results = self.execute_cleanups(expired_leases).await;

        // Phase 4: Process cleanup results and update storage
        let (success_count, failure_count) = self.process_cleanup_results(cleanup_results).await?;

        // Update metrics
        self.metrics.cleanup_operations_total.inc_by(success_count);
        self.metrics.cleanup_failures_total.inc_by(failure_count);

        let duration = start_time.elapsed();
        self.metrics.record_cleanup_duration(duration.as_secs_f64());

        info!(
            "🧹 Cleanup cycle completed in {:.2}s: {} successful, {} failed",
            duration.as_secs_f64(),
            success_count,
            failure_count
        );

        Ok(())
    }

    /// Mark leases that have expired but aren't yet in expired state
    async fn mark_expired_leases(&self) -> Result<u64> {
        // This would ideally be done with a single database query for efficiency
        // For now, we'll use the storage interface

        // Get all active leases and check which have expired
        let filter = crate::lease::LeaseFilter {
            state: Some(LeaseState::Active),
            ..Default::default()
        };

        let active_leases = self.storage.list_leases(filter, None, None).await?;
        let mut marked_count = 0;

        for mut lease in active_leases {
            if lease.is_expired() {
                debug!("⏰ Marking lease {} as expired", lease.lease_id);
                lease.expire();

                if let Err(e) = self.storage.update_lease(lease).await {
                    warn!("Failed to mark lease as expired: {}", e);
                } else {
                    marked_count += 1;
                }
            }
        }

        if marked_count > 0 {
            debug!("⏰ Marked {} leases as expired", marked_count);
        }

        Ok(marked_count)
    }

    /// Execute cleanup operations for expired leases
    async fn execute_cleanups(&self, leases: Vec<Lease>) -> Vec<CleanupResult> {
        let total_leases = leases.len();

        if total_leases == 0 {
            return Vec::new();
        }

        info!("🚀 Starting cleanup execution for {} leases", total_leases);

        // Use semaphore to limit concurrent cleanup operations
        let max_concurrent = self.config.gc.max_concurrent_cleanups;
        let semaphore = Arc::new(tokio::sync::Semaphore::new(max_concurrent));
        let mut handles = Vec::new();

        for lease in leases {
            let executor = self.cleanup_executor.clone();
            let semaphore = semaphore.clone();
            let lease_id = lease.lease_id.clone();
            let object_id = lease.object_id.clone();

            let handle = tokio::spawn(async move {
                let _permit = semaphore.acquire().await.expect("Semaphore closed");

                debug!("🧹 Executing cleanup for lease {}", lease_id);
                let start_time = Instant::now();

                let result = executor.cleanup_lease(&lease).await;
                let duration = start_time.elapsed();

                match &result {
                    Ok(_) => {
                        info!(
                            "✅ Cleanup successful for lease {} (object: {}) in {:.2}s",
                            lease_id,
                            object_id,
                            duration.as_secs_f64()
                        );
                    }
                    Err(e) => {
                        warn!(
                            "❌ Cleanup failed for lease {} (object: {}): {} (took {:.2}s)",
                            lease_id,
                            object_id,
                            e,
                            duration.as_secs_f64()
                        );
                    }
                }

                CleanupResult {
                    lease_id,
                    object_id,
                    success: result.is_ok(),
                    error: result.err().map(|e| e.to_string()),
                    duration_seconds: duration.as_secs_f64(),
                }
            });

            handles.push(handle);
        }

        // Wait for all cleanup operations to complete
        let mut results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok(result) => results.push(result),
                Err(e) => {
                    error!("Cleanup task panicked: {}", e);
                    results.push(CleanupResult {
                        lease_id: "unknown".to_string(),
                        object_id: "unknown".to_string(),
                        success: false,
                        error: Some(format!("Task panicked: {}", e)),
                        duration_seconds: 0.0,
                    });
                }
            }
        }

        results
    }

    /// Process cleanup results and remove successfully cleaned leases
    async fn process_cleanup_results(&self, results: Vec<CleanupResult>) -> Result<(u64, u64)> {
        let mut success_count = 0;
        let mut failure_count = 0;

        for result in results {
            if result.success {
                // Remove the lease from storage since cleanup was successful
                match self.storage.delete_lease(&result.lease_id).await {
                    Ok(_) => {
                        success_count += 1;
                        debug!(
                            "🗑️  Removed successfully cleaned lease: {}",
                            result.lease_id
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Failed to remove cleaned lease {} from storage: {}",
                            result.lease_id, e
                        );
                        failure_count += 1;
                    }
                }
            } else {
                failure_count += 1;
                warn!(
                    "Cleanup failed for lease {}: {}",
                    result.lease_id,
                    result.error.unwrap_or_else(|| "Unknown error".to_string())
                );

                // Optionally, we could implement a retry mechanism here
                // or move the lease to a "failed cleanup" state
            }
        }

        Ok((success_count, failure_count))
    }

    /// Log cleanup statistics periodically
    async fn log_cleanup_stats(&self) {
        match self.storage.get_stats().await {
            Ok(stats) => {
                info!(
                    "📊 Cleanup stats: {} active, {} expired, {} released (total: {})",
                    stats.active_leases,
                    stats.expired_leases,
                    stats.released_leases,
                    stats.total_leases
                );

                // Update metrics with current stats
                self.metrics.update_lease_counts(&stats);
            }
            Err(e) => {
                warn!("Failed to get storage stats: {}", e);
            }
        }
    }
}

/// Result of a cleanup operation
#[derive(Debug, Clone)]
pub struct CleanupResult {
    pub lease_id: String,
    pub object_id: String,
    pub success: bool,
    pub error: Option<String>,
    pub duration_seconds: f64,
}

impl CleanupResult {
    pub fn success(lease_id: String, object_id: String, duration_seconds: f64) -> Self {
        Self {
            lease_id,
            object_id,
            success: true,
            error: None,
            duration_seconds,
        }
    }

    pub fn failure(
        lease_id: String,
        object_id: String,
        error: String,
        duration_seconds: f64,
    ) -> Self {
        Self {
            lease_id,
            object_id,
            success: false,
            error: Some(error),
            duration_seconds,
        }
    }
}
