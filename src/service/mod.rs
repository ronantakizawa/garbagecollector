// src/service/mod.rs - Service struct and core logic

use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use crate::cleanup::CleanupExecutor;
use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseFilter, LeaseState};
use crate::metrics::{Metrics, AlertThresholds};
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
    /// Create a new GarbageTruck service instance with detailed startup tracking
    pub async fn new(config: Config) -> Result<Self> {
        let service_creation_start = Instant::now();
        
        // Phase 1: Validate configuration and record metrics
        let config_validation_start = Instant::now();
        let validation_result = config.validate_detailed();
        let config_validation_duration = config_validation_start.elapsed();
        
        if !validation_result.success {
            // Create metrics even for failed validation to record the failure
            let metrics = Arc::new(
                Metrics::with_alerting_thresholds(AlertThresholds::default())
                    .map_err(|e| GCError::Internal(e.to_string()))?
            );
            
            // Record configuration validation failure
            let error_type = validation_result.errors.first()
                .map(|e| e.error_type.as_str())
                .unwrap_or("unknown");
            
            metrics.record_config_validation(false, config_validation_duration, Some(error_type));
            
            return Err(GCError::Configuration(format!(
                "Configuration validation failed: {}",
                validation_result.errors.iter()
                    .map(|e| format!("{}: {}", e.field, e.message))
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        
        info!("✅ Configuration validation passed in {:.3}s", config_validation_duration.as_secs_f64());
        
        // Phase 2: Create metrics system
        let metrics_creation_start = Instant::now();
        let alerting_thresholds = AlertThresholds {
            cleanup_failure_rate_threshold: 0.5,
            cleanup_failure_window_minutes: 5,
            consecutive_cleanup_failures_threshold: 5,
            consecutive_lease_failures_threshold: 10,
            storage_unavailable_threshold_seconds: 30,
            alert_cooldown_minutes: 15,
            startup_duration_threshold_seconds: 30.0,
            config_validation_failure_threshold: 3,
            consecutive_startup_failures_threshold: 3,
        };
        
        let metrics = Arc::new(
            Metrics::with_alerting_thresholds(alerting_thresholds)
                .map_err(|e| GCError::Internal(e.to_string()))?
        );
        let metrics_creation_duration = metrics_creation_start.elapsed();
        
        // Record successful configuration validation
        metrics.record_config_validation(true, config_validation_duration, None);
        metrics.record_component_initialization("metrics_system", metrics_creation_duration);
        
        // Phase 3: Create storage backend
        let storage_creation_start = Instant::now();
        let storage = match create_storage(&config).await {
            Ok(storage) => {
                let storage_creation_duration = storage_creation_start.elapsed();
                metrics.record_component_initialization("storage_backend", storage_creation_duration);
                info!("✅ Storage backend '{}' initialized in {:.3}s", 
                      config.storage.backend, storage_creation_duration.as_secs_f64());
                storage
            }
            Err(e) => {
                let storage_creation_duration = storage_creation_start.elapsed();
                metrics.record_startup_error("storage_creation", "storage_creation_failed");
                error!("❌ Failed to create storage backend after {:.3}s: {}", 
                       storage_creation_duration.as_secs_f64(), e);
                return Err(e);
            }
        };
        
        // Phase 4: Create cleanup executor
        let cleanup_creation_start = Instant::now();
        let cleanup_executor = CleanupExecutor::new(
            Duration::from_secs(config.cleanup.default_timeout_seconds),
            config.cleanup.default_max_retries,
            Duration::from_secs(config.cleanup.default_retry_delay_seconds),
        );
        let cleanup_creation_duration = cleanup_creation_start.elapsed();
        metrics.record_component_initialization("cleanup_executor", cleanup_creation_duration);
        
        let total_service_creation_duration = service_creation_start.elapsed();
        
        info!("✅ GarbageTruck service components initialized in {:.3}s", 
              total_service_creation_duration.as_secs_f64());
        
        // Log component breakdown
        info!("📊 Service creation breakdown:");
        info!("   • Config validation: {:.3}s", config_validation_duration.as_secs_f64());
        info!("   • Metrics system: {:.3}s", metrics_creation_duration.as_secs_f64());
        info!("   • Storage backend: {:.3}s", storage_creation_start.elapsed().as_secs_f64());
        info!("   • Cleanup executor: {:.3}s", cleanup_creation_duration.as_secs_f64());
        
        // Log configuration summary
        let config_summary = config.summary();
        info!("📋 Service configuration:");
        info!("   • Server: {}", config_summary.server_endpoint);
        info!("   • Storage: {} (DB: {})", config_summary.storage_backend, config_summary.has_database_url);
        info!("   • Lease duration: {}s (default)", config_summary.default_lease_duration);
        info!("   • Cleanup interval: {}s", config_summary.cleanup_interval);
        info!("   • Max leases/service: {}", config_summary.max_leases_per_service);
        info!("   • Metrics: {} (port: {:?})", config_summary.metrics_enabled, config_summary.metrics_port);
        
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
    
    /// Start the automatic cleanup loop (legacy method without shutdown support)
    pub async fn start_cleanup_loop(&self) {
        let mut interval = tokio::time::interval(self.config.cleanup_interval());
        let grace_period = self.config.cleanup_grace_period();
        
        info!(
            interval_seconds = self.config.gc.cleanup_interval_seconds,
            grace_period_seconds = self.config.gc.cleanup_grace_period_seconds,
            "Starting GarbageTruck cleanup loop"
        );
        
        loop {
            interval.tick().await;
            
            let start = Instant::now();
            match self.run_cleanup_cycle(grace_period).await {
                Ok(cleaned_count) => {
                    let duration = start.elapsed();
                    debug!(
                        cleaned_count = cleaned_count,
                        duration_ms = duration.as_millis(),
                        "GarbageTruck cleanup cycle completed"
                    );
                }
                Err(e) => {
                    error!(error = %e, "GarbageTruck cleanup cycle failed");
                    self.metrics.record_storage_error(
                        "cleanup_cycle",
                        &self.config.storage.backend,
                        "cleanup_cycle_failure"
                    );
                }
            }
        }
    }
    
    /// Start the automatic cleanup loop with graceful shutdown support
    pub async fn start_cleanup_loop_with_shutdown(&self, mut task_handle: TaskHandle) {
        let mut interval = tokio::time::interval(self.config.cleanup_interval());
        let grace_period = self.config.cleanup_grace_period();
        
        info!(
            interval_seconds = self.config.gc.cleanup_interval_seconds,
            grace_period_seconds = self.config.gc.cleanup_grace_period_seconds,
            "Starting GarbageTruck cleanup loop with shutdown support"
        );
        
        let mut cleanup_cycles = 0;
        let start_time = Instant::now();
        
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
                                "GarbageTruck cleanup cycle completed"
                            );
                            
                            // Log periodic statistics
                            if cleanup_cycles % 10 == 0 {
                                info!(
                                    "Cleanup loop statistics: {} cycles completed in {:.2}s (avg: {:.2}s/cycle)",
                                    cleanup_cycles,
                                    start_time.elapsed().as_secs_f64(),
                                    start_time.elapsed().as_secs_f64() / cleanup_cycles as f64
                                );
                            }
                        }
                        Err(e) => {
                            error!(error = %e, cycle = cleanup_cycles, "GarbageTruck cleanup cycle failed");
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
                    
                    // Perform one final cleanup cycle before shutting down
                    info!("🧹 Performing final cleanup cycle before shutdown");
                    match self.run_cleanup_cycle(grace_period).await {
                        Ok(cleaned_count) => {
                            info!("✅ Final cleanup cycle completed: {} items cleaned", cleaned_count);
                        }
                        Err(e) => {
                            warn!("⚠️  Final cleanup cycle failed: {}", e);
                        }
                    }
                    
                    info!(
                        "🏁 Cleanup loop shutting down after {} cycles and {:.2}s runtime",
                        cleanup_cycles,
                        start_time.elapsed().as_secs_f64()
                    );
                    
                    task_handle.mark_completed().await;
                    break;
                }
            }
        }
    }
    
    /// Run a single cleanup cycle with enhanced error tracking
    pub(crate) async fn run_cleanup_cycle(&self, grace_period: Duration) -> Result<usize> {
        let cycle_start = Instant::now();
        
        // Get expired leases that need cleanup
        let expired_leases = match self.storage.get_expired_leases(grace_period).await {
            Ok(leases) => {
                self.metrics.record_storage_operation("get_expired_leases", &self.config.storage.backend);
                leases
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "get_expired_leases",
                    &self.config.storage.backend,
                    "storage_error"
                );
                return Err(e);
            }
        };
        
        if expired_leases.is_empty() {
            return Ok(0);
        }
        
        info!(count = expired_leases.len(), "Found expired leases to clean up");
        
        // Execute cleanup operations with timing
        let cleanup_start = Instant::now();
        let cleanup_results = self.cleanup_executor.cleanup_batch(expired_leases.clone()).await;
        let cleanup_duration = cleanup_start.elapsed();
        
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
                        self.metrics.lease_expired(
                            &lease.service_id,
                            &format!("{:?}", lease.object_type)
                        );
                        self.metrics.cleanup_succeeded();
                        self.metrics.record_storage_operation("update_lease", &self.config.storage.backend);
                    }
                    Err(e) => {
                        warn!(
                            lease_id = %lease.lease_id,
                            error = %e,
                            "Failed to update lease state after successful cleanup"
                        );
                        self.metrics.record_storage_error(
                            "update_lease",
                            &self.config.storage.backend,
                            "update_after_cleanup_failed"
                        );
                    }
                }
            } else {
                failed_cleanups += 1;
                self.metrics.cleanup_failed("cleanup_operation_failed");
                
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
        
        // Clean up storage (remove expired leases that have been processed)
        match self.storage.cleanup().await {
            Ok(_) => {
                self.metrics.record_storage_operation("cleanup", &self.config.storage.backend);
            }
            Err(e) => {
                warn!(error = %e, "Storage cleanup failed");
                self.metrics.record_storage_error(
                    "cleanup",
                    &self.config.storage.backend,
                    "storage_cleanup_failed"
                );
            }
        }
        
        let total_cycle_duration = cycle_start.elapsed();
        
        info!(
            successful = successful_cleanups,
            failed = failed_cleanups,
            total_duration_ms = total_cycle_duration.as_millis(),
            cleanup_duration_ms = cleanup_duration.as_millis(),
            "GarbageTruck cleanup cycle completed"
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
        
        let active_count = match self.storage.count_leases(filter).await {
            Ok(count) => {
                self.metrics.record_storage_operation("count_leases", &self.config.storage.backend);
                count
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "count_leases",
                    &self.config.storage.backend,
                    "count_operation_failed"
                );
                return Err(e);
            }
        };
        
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