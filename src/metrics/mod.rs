// src/metrics/mod.rs - Core metrics definitions and main Metrics struct

use prometheus::{
    Gauge, Histogram, HistogramOpts, HistogramVec, IntCounter,
    IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

// Re-export submodules
pub mod alerting;
pub mod interceptors;
pub mod monitoring;

// Re-export commonly used types from submodules
pub use alerting::{AlertingSystem, AlertThresholds, Alert, AlertType, AlertSeverity, AlertSummary};
pub use interceptors::MetricsInterceptor;
pub use monitoring::start_system_monitoring;

/// Core metrics collection and management
#[derive(Clone)]
pub struct Metrics {
    pub registry: Arc<Registry>,
    
    // Lease metrics
    pub leases_created_total: IntCounter,
    pub leases_renewed_total: IntCounter,
    pub leases_expired_total: IntCounter,
    pub leases_released_total: IntCounter,
    pub active_leases: IntGauge,
    pub leases_by_service: IntGaugeVec,
    pub leases_by_type: IntGaugeVec,
    
    // Cleanup metrics
    pub cleanup_operations_total: IntCounter,
    pub cleanup_failures_total: IntCounter,
    pub cleanup_duration: Histogram,
    pub cleanup_retries_total: IntCounterVec,
    
    // Performance metrics
    pub request_duration: HistogramVec,
    pub request_total: IntCounterVec,
    pub grpc_requests_total: IntCounterVec,
    
    // System metrics
    pub memory_usage: Gauge,
    pub storage_operations_total: IntCounterVec,
    pub storage_errors_total: IntCounterVec,
    
    // Alerting-related metrics
    pub lease_creation_failures_total: IntCounter,
    pub storage_availability: IntGauge,
    pub consecutive_cleanup_failures: IntGauge,
    pub consecutive_lease_creation_failures: IntGauge,
    pub storage_operation_failures_by_type: IntCounterVec,
    
    // Startup and Configuration metrics
    pub startup_duration_seconds: Histogram,
    pub startup_phase_duration: HistogramVec,
    pub service_ready_timestamp: Gauge,
    pub configuration_validation_total: IntCounterVec,
    pub configuration_validation_duration: Histogram,
    pub configuration_errors_total: IntCounterVec,
    pub component_initialization_duration: HistogramVec,
    pub dependencies_check_duration: Histogram,
    pub service_restarts_total: IntCounter,
    pub config_reload_total: IntCounterVec,
    pub startup_errors_total: IntCounterVec,
    
    // Alerting system
    pub alerting: Arc<RwLock<AlertingSystem>>,
}

impl Metrics {
    pub fn new() -> prometheus::Result<Self> {
        Self::with_alerting_thresholds(AlertThresholds::default())
    }
    
    pub fn with_alerting_thresholds(thresholds: AlertThresholds) -> prometheus::Result<Self> {
        let registry = Arc::new(Registry::new());
        
        // Lease metrics
        let leases_created_total = IntCounter::new(
            "gc_leases_created_total",
            "Total number of leases created"
        )?;
        registry.register(Box::new(leases_created_total.clone()))?;
        
        let leases_renewed_total = IntCounter::new(
            "gc_leases_renewed_total",
            "Total number of lease renewals"
        )?;
        registry.register(Box::new(leases_renewed_total.clone()))?;
        
        let leases_expired_total = IntCounter::new(
            "gc_leases_expired_total",
            "Total number of leases that expired"
        )?;
        registry.register(Box::new(leases_expired_total.clone()))?;
        
        let leases_released_total = IntCounter::new(
            "gc_leases_released_total",
            "Total number of leases explicitly released"
        )?;
        registry.register(Box::new(leases_released_total.clone()))?;
        
        let active_leases = IntGauge::new(
            "gc_active_leases",
            "Current number of active leases"
        )?;
        registry.register(Box::new(active_leases.clone()))?;
        
        let leases_by_service = IntGaugeVec::new(
            Opts::new("gc_leases_by_service", "Number of leases by service"),
            &["service_id"]
        )?;
        registry.register(Box::new(leases_by_service.clone()))?;
        
        let leases_by_type = IntGaugeVec::new(
            Opts::new("gc_leases_by_type", "Number of leases by object type"),
            &["object_type"]
        )?;
        registry.register(Box::new(leases_by_type.clone()))?;
        
        // Cleanup metrics
        let cleanup_operations_total = IntCounter::new(
            "gc_cleanup_operations_total",
            "Total number of cleanup operations performed"
        )?;
        registry.register(Box::new(cleanup_operations_total.clone()))?;
        
        let cleanup_failures_total = IntCounter::new(
            "gc_cleanup_failures_total",
            "Total number of failed cleanup operations"
        )?;
        registry.register(Box::new(cleanup_failures_total.clone()))?;
        
        let cleanup_duration = Histogram::with_opts(
            HistogramOpts::new(
                "gc_cleanup_duration_seconds",
                "Duration of cleanup operations in seconds"
            ).buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 25.0, 60.0])
        )?;
        registry.register(Box::new(cleanup_duration.clone()))?;
        
        let cleanup_retries_total = IntCounterVec::new(
            Opts::new("gc_cleanup_retries_total", "Total number of cleanup retries by reason"),
            &["reason"]
        )?;
        registry.register(Box::new(cleanup_retries_total.clone()))?;
        
        // Performance metrics
        let request_duration = HistogramVec::new(
            HistogramOpts::new(
                "gc_request_duration_seconds",
                "Duration of gRPC requests in seconds"
            ).buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]),
            &["method", "status"]
        )?;
        registry.register(Box::new(request_duration.clone()))?;
        
        let request_total = IntCounterVec::new(
            Opts::new("gc_requests_total", "Total number of gRPC requests"),
            &["method", "status"]
        )?;
        registry.register(Box::new(request_total.clone()))?;
        
        let grpc_requests_total = IntCounterVec::new(
            Opts::new("gc_grpc_requests_total", "Total number of gRPC requests by method"),
            &["method"]
        )?;
        registry.register(Box::new(grpc_requests_total.clone()))?;
        
        // System metrics
        let memory_usage = Gauge::new(
            "gc_memory_usage_bytes",
            "Current memory usage in bytes"
        )?;
        registry.register(Box::new(memory_usage.clone()))?;
        
        let storage_operations_total = IntCounterVec::new(
            Opts::new("gc_storage_operations_total", "Total storage operations"),
            &["operation", "backend"]
        )?;
        registry.register(Box::new(storage_operations_total.clone()))?;
        
        let storage_errors_total = IntCounterVec::new(
            Opts::new("gc_storage_errors_total", "Total storage errors"),
            &["operation", "backend", "error_type"]
        )?;
        registry.register(Box::new(storage_errors_total.clone()))?;
        
        // Alerting-related metrics
        let lease_creation_failures_total = IntCounter::new(
            "gc_lease_creation_failures_total",
            "Total number of lease creation failures"
        )?;
        registry.register(Box::new(lease_creation_failures_total.clone()))?;
        
        let storage_availability = IntGauge::new(
            "gc_storage_availability",
            "Storage backend availability (1 = available, 0 = unavailable)"
        )?;
        registry.register(Box::new(storage_availability.clone()))?;
        
        let consecutive_cleanup_failures = IntGauge::new(
            "gc_consecutive_cleanup_failures",
            "Number of consecutive cleanup failures"
        )?;
        registry.register(Box::new(consecutive_cleanup_failures.clone()))?;
        
        let consecutive_lease_creation_failures = IntGauge::new(
            "gc_consecutive_lease_creation_failures",
            "Number of consecutive lease creation failures"
        )?;
        registry.register(Box::new(consecutive_lease_creation_failures.clone()))?;
        
        let storage_operation_failures_by_type = IntCounterVec::new(
            Opts::new("gc_storage_operation_failures_by_type", "Storage operation failures by type"),
            &["operation", "backend", "error_type"]
        )?;
        registry.register(Box::new(storage_operation_failures_by_type.clone()))?;
        
        // Startup and Configuration metrics
        let startup_duration_seconds = Histogram::with_opts(
            HistogramOpts::new(
                "gc_startup_duration_seconds",
                "Time taken for complete service startup in seconds"
            ).buckets(vec![0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0])
        )?;
        registry.register(Box::new(startup_duration_seconds.clone()))?;
        
        let startup_phase_duration = HistogramVec::new(
            HistogramOpts::new(
                "gc_startup_phase_duration_seconds",
                "Duration of individual startup phases in seconds"
            ).buckets(vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0]),
            &["phase"]
        )?;
        registry.register(Box::new(startup_phase_duration.clone()))?;
        
        let service_ready_timestamp = Gauge::new(
            "gc_service_ready_timestamp",
            "Unix timestamp when service became ready"
        )?;
        registry.register(Box::new(service_ready_timestamp.clone()))?;
        
        let configuration_validation_total = IntCounterVec::new(
            Opts::new("gc_configuration_validation_total", "Total configuration validation attempts"),
            &["result"] // "success" or "failure"
        )?;
        registry.register(Box::new(configuration_validation_total.clone()))?;
        
        let configuration_validation_duration = Histogram::with_opts(
            HistogramOpts::new(
                "gc_configuration_validation_duration_seconds",
                "Time taken to validate configuration in seconds"
            ).buckets(vec![0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0])
        )?;
        registry.register(Box::new(configuration_validation_duration.clone()))?;
        
        let configuration_errors_total = IntCounterVec::new(
            Opts::new("gc_configuration_errors_total", "Total configuration errors by type"),
            &["error_type"]
        )?;
        registry.register(Box::new(configuration_errors_total.clone()))?;
        
        let component_initialization_duration = HistogramVec::new(
            HistogramOpts::new(
                "gc_component_initialization_duration_seconds",
                "Time taken to initialize individual components in seconds"
            ).buckets(vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0]),
            &["component"]
        )?;
        registry.register(Box::new(component_initialization_duration.clone()))?;
        
        let dependencies_check_duration = Histogram::with_opts(
            HistogramOpts::new(
                "gc_dependencies_check_duration_seconds",
                "Time taken to check external dependencies in seconds"
            ).buckets(vec![0.1, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0])
        )?;
        registry.register(Box::new(dependencies_check_duration.clone()))?;
        
        let service_restarts_total = IntCounter::new(
            "gc_service_restarts_total",
            "Total number of service restarts"
        )?;
        registry.register(Box::new(service_restarts_total.clone()))?;
        
        let config_reload_total = IntCounterVec::new(
            Opts::new("gc_config_reload_total", "Total configuration reload attempts"),
            &["result"] // "success" or "failure"
        )?;
        registry.register(Box::new(config_reload_total.clone()))?;
        
        let startup_errors_total = IntCounterVec::new(
            Opts::new("gc_startup_errors_total", "Total startup errors by phase"),
            &["phase", "error_type"]
        )?;
        registry.register(Box::new(startup_errors_total.clone()))?;
        
        // Initialize storage as available
        storage_availability.set(1);
        
        Ok(Self {
            registry,
            leases_created_total,
            leases_renewed_total,
            leases_expired_total,
            leases_released_total,
            active_leases,
            leases_by_service,
            leases_by_type,
            cleanup_operations_total,
            cleanup_failures_total,
            cleanup_duration,
            cleanup_retries_total,
            request_duration,
            request_total,
            grpc_requests_total,
            memory_usage,
            storage_operations_total,
            storage_errors_total,
            lease_creation_failures_total,
            storage_availability,
            consecutive_cleanup_failures,
            consecutive_lease_creation_failures,
            storage_operation_failures_by_type,
            startup_duration_seconds,
            startup_phase_duration,
            service_ready_timestamp,
            configuration_validation_total,
            configuration_validation_duration,
            configuration_errors_total,
            component_initialization_duration,
            dependencies_check_duration,
            service_restarts_total,
            config_reload_total,
            startup_errors_total,
            alerting: Arc::new(RwLock::new(AlertingSystem::new().with_thresholds(thresholds))),
        })
    }
    
    /// Get Prometheus metrics for export
    pub fn gather(&self) -> Vec<prometheus::proto::MetricFamily> {
        self.registry.gather()
    }
    
    /// Update memory usage metric
    pub fn update_memory_usage(&self, bytes: f64) {
        self.memory_usage.set(bytes);
    }
    
    /// Reset service-specific metrics
    pub fn reset_service_metrics(&self, service_id: &str) {
        self.leases_by_service.remove_label_values(&[service_id]).ok();
    }
    
    /// Update lease counts from storage stats
    pub fn update_lease_counts(&self, stats: &crate::lease::LeaseStats) {
        self.active_leases.set(stats.active_leases as i64);
        
        for (service_id, count) in &stats.leases_by_service {
            self.leases_by_service.with_label_values(&[service_id]).set(*count as i64);
        }
        
        for (object_type, count) in &stats.leases_by_type {
            self.leases_by_type.with_label_values(&[object_type]).set(*count as i64);
        }
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new().expect("Failed to create metrics")
    }
}