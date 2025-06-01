// src/service/mod.rs - Simplified service creation

use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use crate::cleanup::CleanupExecutor;
use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{LeaseFilter, LeaseState};
use crate::metrics::Metrics;
use crate::shutdown::TaskHandle;
use crate::storage::{create_storage, Storage};

// Re-export submodules
pub mod handlers;
pub mod validation;

// Re-export the handlers trait for external use
pub use handlers::GCServiceHandlers;

/// GarbageTruck service implementation with graceful shutdown support
#[derive(Clone)]
pub struct GCService {
    config: Config,
    storage: Arc<dyn Storage + Send + Sync>,
    cleanup_executor: CleanupExecutor,
    metrics: Arc<Metrics>,
    start_time: std::time::Instant,
}

impl GCService {
    /// Create a new GarbageTruck service instance - simplified
    pub async fn new(config: Config) -> Result<Self> {
        let service_creation_start = Instant::now();

        // Validate configuration - simple validation only
        config.validate()?;
        info!("✅ Configuration validated");

        // Create metrics system - no alerting
        let metrics = Arc::new(Metrics::new().map_err(|e| GCError::Internal(e.to_string()))?);

        // Create storage backend
        let storage = create_storage(&config).await?;
        info!(
            "✅ Storage backend '{}' initialized",
            config.storage.backend
        );

        // Create cleanup executor
        let cleanup_executor = CleanupExecutor::new(
            Duration::from_secs(config.cleanup.default_timeout_seconds),
            config.cleanup.default_max_retries,
            Duration::from_secs(config.cleanup.default_retry_delay_seconds),
        );

        let total_duration = service_creation_start.elapsed();
        info!(
            "✅ GarbageTruck service initialized in {:.3}s",
            total_duration.as_secs_f64()
        );

        Ok(Self {
            config,
            storage,
            cleanup_executor,
            metrics,
            start_time: Instant::now(),
        })
    }

    /// Get a reference to the metrics for use in middleware
    pub fn get_metrics(&self) -> Arc<Metrics> {
        self.metrics.clone()
    }

    /// Get a reference to the configuration
    pub fn get_config(&self) -> &Config {
        &self.config
    }

    /// Get a reference to the storage backend
    pub fn get_storage(&self) -> &Arc<dyn Storage + Send + Sync> {
        &self.storage
    }

    /// Get a reference to the cleanup executor
    pub fn get_cleanup_executor(&self) -> &CleanupExecutor {
        &self.cleanup_executor
    }

    /// Get service start time
    pub fn get_start_time(&self) -> Instant {
        self.start_time
    }

    /// Start the automatic cleanup loop with graceful shutdown support
    pub async fn start_cleanup_loop_with_shutdown(&self, mut task_handle: TaskHandle) {
        let mut interval = tokio::time::interval(self.config.cleanup_interval());
        let grace_period = self.config.cleanup_grace_period();

        info!(
            interval_seconds = self.config.gc.cleanup_interval_seconds,
            grace_period_seconds = self.config.gc.cleanup_grace_period_seconds,
            "Starting GarbageTruck cleanup loop"
        );

        let mut cleanup_cycles = 0;

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let cycle_start = Instant::now();
                    match self.run_cleanup_cycle(grace_period).await {
                        Ok(cleaned_count) => {
                            cleanup_cycles += 1;
                            let duration = cycle_start.elapsed();
                            debug!(
                                cleaned_count = cleaned_count,
                                duration_ms = duration.as_millis(),
                                cycle = cleanup_cycles,
                                "Cleanup cycle completed"
                            );
                        }
                        Err(e) => {
                            error!(error = %e, cycle = cleanup_cycles, "Cleanup cycle failed");
                            self.metrics.record_storage_error(
                                "cleanup_cycle",
                                &self.config.storage.backend,
                                "cleanup_cycle_failure"
                            );
                        }
                    }
                }
                _ = task_handle.wait_for_shutdown() => {
                    info!("🛑 Cleanup loop received shutdown signal");

                    // Perform one final cleanup cycle
                    info!("🧹 Performing final cleanup cycle");
                    match self.run_cleanup_cycle(grace_period).await {
                        Ok(cleaned_count) => {
                            info!("✅ Final cleanup: {} items cleaned", cleaned_count);
                        }
                        Err(e) => {
                            warn!("⚠️  Final cleanup failed: {}", e);
                        }
                    }

                    info!("🏁 Cleanup loop shutting down after {} cycles", cleanup_cycles);
                    task_handle.mark_completed().await;
                    break;
                }
            }
        }
    }

    /// Run a single cleanup cycle - simplified
    pub(crate) async fn run_cleanup_cycle(&self, grace_period: Duration) -> Result<usize> {
        // Get expired leases
        let expired_leases = self.storage.get_expired_leases(grace_period).await?;

        if expired_leases.is_empty() {
            return Ok(0);
        }

        info!(
            count = expired_leases.len(),
            "Found expired leases to clean up"
        );

        // Execute cleanup operations
        let cleanup_results = self
            .cleanup_executor
            .cleanup_batch(expired_leases.clone())
            .await;

        let mut successful_cleanups = 0;
        let mut failed_cleanups = 0;

        // Update lease states based on cleanup results
        for (lease, result) in expired_leases.iter().zip(cleanup_results.iter()) {
            if result.success {
                // Mark lease as expired in storage
                let mut updated_lease = lease.clone();
                updated_lease.expire();

                match self.storage.update_lease(updated_lease).await {
                    Ok(_) => {
                        successful_cleanups += 1;
                        self.metrics
                            .lease_expired(&lease.service_id, &format!("{:?}", lease.object_type));
                        self.metrics.cleanup_succeeded();
                        self.metrics
                            .record_storage_operation("update_lease", &self.config.storage.backend);
                    }
                    Err(e) => {
                        warn!(
                            lease_id = %lease.lease_id,
                            error = %e,
                            "Failed to update lease state after cleanup"
                        );
                        self.metrics.record_storage_error(
                            "update_lease",
                            &self.config.storage.backend,
                            "update_after_cleanup_failed",
                        );
                    }
                }
            } else {
                failed_cleanups += 1;
                self.metrics.cleanup_failed();

                if let Some(ref error) = result.error {
                    warn!(
                        lease_id = %lease.lease_id,
                        object_id = %lease.object_id,
                        error = %error,
                        "Cleanup operation failed"
                    );
                }
            }
        }

        // Clean up storage
        if let Err(e) = self.storage.cleanup().await {
            warn!(error = %e, "Storage cleanup failed");
            self.metrics.record_storage_error(
                "cleanup",
                &self.config.storage.backend,
                "storage_cleanup_failed",
            );
        } else {
            self.metrics
                .record_storage_operation("cleanup", &self.config.storage.backend);
        }

        info!(
            successful = successful_cleanups,
            failed = failed_cleanups,
            "Cleanup cycle completed"
        );

        Ok(successful_cleanups)
    }

    /// Check if a service has exceeded its lease limit
    pub(crate) async fn check_service_lease_limit(&self, service_id: &str) -> Result<()> {
        let filter = LeaseFilter {
            service_id: Some(service_id.to_string()),
            state: Some(LeaseState::Active),
            ..Default::default()
        };

        let active_count = self.storage.count_leases(filter).await?;
        self.metrics
            .record_storage_operation("count_leases", &self.config.storage.backend);

        if active_count >= self.config.gc.max_leases_per_service {
            return Err(GCError::ServiceLeaseLimit {
                service_id: service_id.to_string(),
                current: active_count,
                max: self.config.gc.max_leases_per_service,
            });
        }

        Ok(())
    }
}
