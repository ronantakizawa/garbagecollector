// src/metrics/mod.rs - Simplified core metrics only

use prometheus::{
    Gauge, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    IntGaugeVec, Opts, Registry,
};
use std::sync::Arc;

// Re-export only the interceptor
pub mod interceptors;
pub use interceptors::MetricsInterceptor;

/// Core metrics collection - simplified and focused
#[derive(Clone)]
pub struct Metrics {
    pub registry: Arc<Registry>,

    // Core lease metrics
    pub leases_created_total: IntCounter,
    pub leases_renewed_total: IntCounter,
    pub leases_expired_total: IntCounter,
    pub leases_released_total: IntCounter,
    pub active_leases: IntGauge,
    pub leases_by_service: IntGaugeVec,
    pub leases_by_type: IntGaugeVec,

    // Core cleanup metrics
    pub cleanup_operations_total: IntCounter,
    pub cleanup_failures_total: IntCounter,
    pub cleanup_duration: Histogram,

    // Core performance metrics
    pub request_duration: HistogramVec,
    pub request_total: IntCounterVec,

    // Basic system metrics
    pub memory_usage: Gauge,
    pub storage_operations_total: IntCounterVec,
    pub storage_errors_total: IntCounterVec,
}

impl Metrics {
    pub fn new() -> prometheus::Result<Self> {
        let registry = Arc::new(Registry::new());

        // Lease metrics
        let leases_created_total =
            IntCounter::new("gc_leases_created_total", "Total number of leases created")?;
        registry.register(Box::new(leases_created_total.clone()))?;

        let leases_renewed_total =
            IntCounter::new("gc_leases_renewed_total", "Total number of lease renewals")?;
        registry.register(Box::new(leases_renewed_total.clone()))?;

        let leases_expired_total = IntCounter::new(
            "gc_leases_expired_total",
            "Total number of leases that expired",
        )?;
        registry.register(Box::new(leases_expired_total.clone()))?;

        let leases_released_total = IntCounter::new(
            "gc_leases_released_total",
            "Total number of leases explicitly released",
        )?;
        registry.register(Box::new(leases_released_total.clone()))?;

        let active_leases = IntGauge::new("gc_active_leases", "Current number of active leases")?;
        registry.register(Box::new(active_leases.clone()))?;

        let leases_by_service = IntGaugeVec::new(
            Opts::new("gc_leases_by_service", "Number of leases by service"),
            &["service_id"],
        )?;
        registry.register(Box::new(leases_by_service.clone()))?;

        let leases_by_type = IntGaugeVec::new(
            Opts::new("gc_leases_by_type", "Number of leases by object type"),
            &["object_type"],
        )?;
        registry.register(Box::new(leases_by_type.clone()))?;

        // Cleanup metrics
        let cleanup_operations_total = IntCounter::new(
            "gc_cleanup_operations_total",
            "Total number of cleanup operations performed",
        )?;
        registry.register(Box::new(cleanup_operations_total.clone()))?;

        let cleanup_failures_total = IntCounter::new(
            "gc_cleanup_failures_total",
            "Total number of failed cleanup operations",
        )?;
        registry.register(Box::new(cleanup_failures_total.clone()))?;

        let cleanup_duration = Histogram::with_opts(
            HistogramOpts::new(
                "gc_cleanup_duration_seconds",
                "Duration of cleanup operations in seconds",
            )
            .buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 25.0, 60.0]),
        )?;
        registry.register(Box::new(cleanup_duration.clone()))?;

        // Performance metrics
        let request_duration = HistogramVec::new(
            HistogramOpts::new(
                "gc_request_duration_seconds",
                "Duration of gRPC requests in seconds",
            )
            .buckets(vec![
                0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
            ]),
            &["method", "status"],
        )?;
        registry.register(Box::new(request_duration.clone()))?;

        let request_total = IntCounterVec::new(
            Opts::new("gc_requests_total", "Total number of gRPC requests"),
            &["method", "status"],
        )?;
        registry.register(Box::new(request_total.clone()))?;

        // System metrics
        let memory_usage = Gauge::new("gc_memory_usage_bytes", "Current memory usage in bytes")?;
        registry.register(Box::new(memory_usage.clone()))?;

        let storage_operations_total = IntCounterVec::new(
            Opts::new("gc_storage_operations_total", "Total storage operations"),
            &["operation", "backend"],
        )?;
        registry.register(Box::new(storage_operations_total.clone()))?;

        let storage_errors_total = IntCounterVec::new(
            Opts::new("gc_storage_errors_total", "Total storage errors"),
            &["operation", "backend", "error_type"],
        )?;
        registry.register(Box::new(storage_errors_total.clone()))?;

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
            request_duration,
            request_total,
            memory_usage,
            storage_operations_total,
            storage_errors_total,
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

    /// Update lease counts from storage stats
    pub fn update_lease_counts(&self, stats: &crate::lease::LeaseStats) {
        self.active_leases.set(stats.active_leases as i64);

        for (service_id, count) in &stats.leases_by_service {
            self.leases_by_service
                .with_label_values(&[service_id])
                .set(*count as i64);
        }

        for (object_type, count) in &stats.leases_by_type {
            self.leases_by_type
                .with_label_values(&[object_type])
                .set(*count as i64);
        }
    }

    // Simple metric recording methods
    pub fn lease_created(&self, service_id: &str, object_type: &str) {
        self.leases_created_total.inc();
        self.active_leases.inc();
        self.leases_by_service.with_label_values(&[service_id]).inc();
        self.leases_by_type.with_label_values(&[object_type]).inc();
    }

    pub fn lease_renewed(&self) {
        self.leases_renewed_total.inc();
    }

    pub fn lease_expired(&self, service_id: &str, object_type: &str) {
        self.leases_expired_total.inc();
        self.active_leases.dec();
        self.leases_by_service.with_label_values(&[service_id]).dec();
        self.leases_by_type.with_label_values(&[object_type]).dec();
    }

    pub fn lease_released(&self, service_id: &str, object_type: &str) {
        self.leases_released_total.inc();
        self.active_leases.dec();
        self.leases_by_service.with_label_values(&[service_id]).dec();
        self.leases_by_type.with_label_values(&[object_type]).dec();
    }

    pub fn cleanup_succeeded(&self) {
        self.cleanup_operations_total.inc();
    }

    pub fn cleanup_failed(&self) {
        self.cleanup_operations_total.inc();
        self.cleanup_failures_total.inc();
    }

    pub fn record_request(&self, method: &str, status: &str, duration: f64) {
        self.request_total
            .with_label_values(&[method, status])
            .inc();
        self.request_duration
            .with_label_values(&[method, status])
            .observe(duration);
    }

    pub fn record_storage_operation(&self, operation: &str, backend: &str) {
        self.storage_operations_total
            .with_label_values(&[operation, backend])
            .inc();
    }

    pub fn record_storage_error(&self, operation: &str, backend: &str, error_type: &str) {
        self.storage_errors_total
            .with_label_values(&[operation, backend, error_type])
            .inc();
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new().expect("Failed to create metrics")
    }
}