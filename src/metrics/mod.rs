// src/metrics/mod.rs - Complete fixed version with Clone derive

mod interceptors;

pub use interceptors::{MetricsInterceptor, RequestStartTime};

use prometheus::{
    CounterVec, Gauge, Histogram, HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge,
    IntGaugeVec, Opts, Registry,
};
use std::sync::Arc;
use tracing::{debug, warn};

use crate::lease::LeaseStats;

/// Comprehensive metrics collection for GarbageTruck
#[derive(Debug, Clone)] // FIXED: Added Clone derive
pub struct Metrics {
    registry: Registry,

    // Lease lifecycle metrics
    pub leases_created_total: IntCounterVec,
    pub leases_renewed_total: IntCounter,
    pub leases_expired_total: IntCounter,
    pub leases_released_total: IntCounterVec,

    // Current state metrics
    pub active_leases: IntGauge,
    pub expired_leases: IntGauge,
    pub total_leases: IntGauge,

    // Cleanup metrics
    pub cleanup_operations_total: IntCounter,
    pub cleanup_failures_total: IntCounter,
    pub cleanup_duration_seconds: Histogram,
    pub cleanup_batch_size: Histogram,

    // gRPC request metrics
    pub request_total: CounterVec,
    pub request_duration_seconds: HistogramVec,

    // Storage metrics
    pub storage_operations_total: CounterVec,
    pub storage_errors_total: CounterVec,
    pub storage_operation_duration: HistogramVec,

    // System metrics
    pub memory_usage_bytes: Gauge,
    pub cpu_usage_percent: Gauge,
    pub open_file_descriptors: IntGauge,

    // Business metrics
    pub leases_by_service: IntGaugeVec,
    pub leases_by_type: IntGaugeVec,
    pub average_lease_duration_seconds: Gauge,
}

impl Metrics {
    /// Create a new metrics instance
    pub fn new() -> Arc<Self> {
        let registry = Registry::new();

        let metrics = Self {
            // Lease lifecycle metrics
            leases_created_total: IntCounterVec::new(
                Opts::new("gc_leases_created_total", "Total number of leases created"),
                &["service_id", "object_type"],
            )
            .expect("Failed to create leases_created_total metric"),

            leases_renewed_total: IntCounter::new(
                "gc_leases_renewed_total",
                "Total number of lease renewals",
            )
            .expect("Failed to create leases_renewed_total metric"),

            leases_expired_total: IntCounter::new(
                "gc_leases_expired_total",
                "Total number of leases that expired",
            )
            .expect("Failed to create leases_expired_total metric"),

            leases_released_total: IntCounterVec::new(
                Opts::new(
                    "gc_leases_released_total",
                    "Total number of leases released",
                ),
                &["service_id", "object_type"],
            )
            .expect("Failed to create leases_released_total metric"),

            // Current state metrics
            active_leases: IntGauge::new("gc_active_leases", "Current number of active leases")
                .expect("Failed to create active_leases metric"),

            expired_leases: IntGauge::new(
                "gc_expired_leases",
                "Current number of expired leases awaiting cleanup",
            )
            .expect("Failed to create expired_leases metric"),

            total_leases: IntGauge::new("gc_total_leases", "Total number of leases in the system")
                .expect("Failed to create total_leases metric"),

            // Cleanup metrics
            cleanup_operations_total: IntCounter::new(
                "gc_cleanup_operations_total",
                "Total number of cleanup operations performed",
            )
            .expect("Failed to create cleanup_operations_total metric"),

            cleanup_failures_total: IntCounter::new(
                "gc_cleanup_failures_total",
                "Total number of failed cleanup operations",
            )
            .expect("Failed to create cleanup_failures_total metric"),

            cleanup_duration_seconds: Histogram::with_opts(
                HistogramOpts::new(
                    "gc_cleanup_duration_seconds",
                    "Duration of cleanup operations in seconds",
                )
                .buckets(vec![0.1, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 120.0]),
            )
            .expect("Failed to create cleanup_duration_seconds metric"),

            cleanup_batch_size: Histogram::with_opts(
                HistogramOpts::new(
                    "gc_cleanup_batch_size",
                    "Number of leases processed in cleanup batches",
                )
                .buckets(vec![1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0, 500.0]),
            )
            .expect("Failed to create cleanup_batch_size metric"),

            // gRPC request metrics
            request_total: CounterVec::new(
                Opts::new("gc_grpc_requests_total", "Total number of gRPC requests"),
                &["method", "status"],
            )
            .expect("Failed to create request_total metric"),

            request_duration_seconds: HistogramVec::new(
                HistogramOpts::new(
                    "gc_grpc_request_duration_seconds",
                    "Duration of gRPC requests in seconds",
                )
                .buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0]),
                &["method"],
            )
            .expect("Failed to create request_duration_seconds metric"),

            // Storage metrics
            storage_operations_total: CounterVec::new(
                Opts::new(
                    "gc_storage_operations_total",
                    "Total number of storage operations",
                ),
                &["operation", "backend"],
            )
            .expect("Failed to create storage_operations_total metric"),

            storage_errors_total: CounterVec::new(
                Opts::new("gc_storage_errors_total", "Total number of storage errors"),
                &["operation", "backend", "error_type"],
            )
            .expect("Failed to create storage_errors_total metric"),

            storage_operation_duration: HistogramVec::new(
                HistogramOpts::new(
                    "gc_storage_operation_duration_seconds",
                    "Duration of storage operations in seconds",
                )
                .buckets(vec![0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5]),
                &["operation", "backend"],
            )
            .expect("Failed to create storage_operation_duration metric"),

            // System metrics
            memory_usage_bytes: Gauge::new(
                "gc_memory_usage_bytes",
                "Current memory usage in bytes",
            )
            .expect("Failed to create memory_usage_bytes metric"),

            cpu_usage_percent: Gauge::new("gc_cpu_usage_percent", "Current CPU usage percentage")
                .expect("Failed to create cpu_usage_percent metric"),

            open_file_descriptors: IntGauge::new(
                "gc_open_file_descriptors",
                "Number of open file descriptors",
            )
            .expect("Failed to create open_file_descriptors metric"),

            // Business metrics
            leases_by_service: IntGaugeVec::new(
                Opts::new(
                    "gc_leases_by_service",
                    "Number of active leases per service",
                ),
                &["service_id"],
            )
            .expect("Failed to create leases_by_service metric"),

            leases_by_type: IntGaugeVec::new(
                Opts::new(
                    "gc_leases_by_type",
                    "Number of active leases per object type",
                ),
                &["object_type"],
            )
            .expect("Failed to create leases_by_type metric"),

            average_lease_duration_seconds: Gauge::new(
                "gc_average_lease_duration_seconds",
                "Average lease duration in seconds",
            )
            .expect("Failed to create average_lease_duration_seconds metric"),

            registry,
        };

        // Register all metrics
        let metrics_arc = Arc::new(metrics);
        metrics_arc.register_all();
        metrics_arc
    }

    /// Register all metrics with the registry
    fn register_all(&self) {
        // Helper macro to register metrics with error handling
        macro_rules! register_metric {
            ($metric:expr, $name:expr) => {
                if let Err(e) = self.registry.register(Box::new($metric.clone())) {
                    warn!("Failed to register metric {}: {}", $name, e);
                }
            };
        }

        // Register all metrics
        register_metric!(self.leases_created_total, "leases_created_total");
        register_metric!(self.leases_renewed_total, "leases_renewed_total");
        register_metric!(self.leases_expired_total, "leases_expired_total");
        register_metric!(self.leases_released_total, "leases_released_total");

        register_metric!(self.active_leases, "active_leases");
        register_metric!(self.expired_leases, "expired_leases");
        register_metric!(self.total_leases, "total_leases");

        register_metric!(self.cleanup_operations_total, "cleanup_operations_total");
        register_metric!(self.cleanup_failures_total, "cleanup_failures_total");
        register_metric!(self.cleanup_duration_seconds, "cleanup_duration_seconds");
        register_metric!(self.cleanup_batch_size, "cleanup_batch_size");

        register_metric!(self.request_total, "request_total");
        register_metric!(self.request_duration_seconds, "request_duration_seconds");

        register_metric!(self.storage_operations_total, "storage_operations_total");
        register_metric!(self.storage_errors_total, "storage_errors_total");
        register_metric!(
            self.storage_operation_duration,
            "storage_operation_duration"
        );

        register_metric!(self.memory_usage_bytes, "memory_usage_bytes");
        register_metric!(self.cpu_usage_percent, "cpu_usage_percent");
        register_metric!(self.open_file_descriptors, "open_file_descriptors");

        register_metric!(self.leases_by_service, "leases_by_service");
        register_metric!(self.leases_by_type, "leases_by_type");
        register_metric!(
            self.average_lease_duration_seconds,
            "average_lease_duration_seconds"
        );

        debug!("All metrics registered successfully");
    }

    /// Get the Prometheus registry
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    // Lease lifecycle tracking methods
    pub fn lease_created(&self, service_id: &str, object_type: &str) {
        self.leases_created_total
            .with_label_values(&[service_id, object_type])
            .inc();
    }

    pub fn lease_renewed(&self) {
        self.leases_renewed_total.inc();
    }

    pub fn lease_expired(&self) {
        self.leases_expired_total.inc();
    }

    pub fn lease_released(&self, service_id: &str, object_type: &str) {
        self.leases_released_total
            .with_label_values(&[service_id, object_type])
            .inc();
    }

    // Cleanup tracking methods
    pub fn record_cleanup_duration(&self, duration_seconds: f64) {
        self.cleanup_duration_seconds.observe(duration_seconds);
    }

    pub fn record_cleanup_batch(&self, batch_size: usize) {
        self.cleanup_batch_size.observe(batch_size as f64);
    }

    // Request tracking methods
    pub fn record_request(&self, method: &str, status: &str, duration_seconds: f64) {
        self.request_total
            .with_label_values(&[method, status])
            .inc();

        self.request_duration_seconds
            .with_label_values(&[method])
            .observe(duration_seconds);
    }

    // Storage tracking methods
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

    pub fn record_storage_duration(&self, operation: &str, backend: &str, duration_seconds: f64) {
        self.storage_operation_duration
            .with_label_values(&[operation, backend])
            .observe(duration_seconds);
    }

    // System metrics updates
    pub fn update_memory_usage(&self, bytes: f64) {
        self.memory_usage_bytes.set(bytes);
    }

    pub fn update_cpu_usage(&self, percent: f64) {
        self.cpu_usage_percent.set(percent);
    }

    pub fn update_file_descriptors(&self, count: i64) {
        self.open_file_descriptors.set(count);
    }

    // Business metrics updates
    pub fn update_lease_counts(&self, stats: &LeaseStats) {
        self.active_leases.set(stats.active_leases as i64);
        self.expired_leases.set(stats.expired_leases as i64);
        self.total_leases.set(stats.total_leases as i64);

        // Reset and update per-service counts
        for (service_id, count) in &stats.leases_by_service {
            self.leases_by_service
                .with_label_values(&[service_id])
                .set(*count as i64);
        }

        // Reset and update per-type counts
        for (object_type, count) in &stats.leases_by_type {
            self.leases_by_type
                .with_label_values(&[object_type])
                .set(*count as i64);
        }

        // Update average lease duration
        if let Some(avg_duration) = stats.average_lease_duration {
            self.average_lease_duration_seconds
                .set(avg_duration.num_seconds() as f64);
        }
    }

    /// Get total leases created (sum across all labels)
    pub fn get_total_leases_created(&self) -> u64 {
        let mut total = 0;
        let metric_families = self.registry.gather();

        for family in metric_families {
            if family.get_name() == "gc_leases_created_total" {
                for metric in family.get_metric() {
                    total += metric.get_counter().get_value() as u64;
                }
            }
        }
        total
    }

    /// Get total leases released (sum across all labels)
    pub fn get_total_leases_released(&self) -> u64 {
        let mut total = 0;
        let metric_families = self.registry.gather();

        for family in metric_families {
            if family.get_name() == "gc_leases_released_total" {
                for metric in family.get_metric() {
                    total += metric.get_counter().get_value() as u64;
                }
            }
        }
        total
    }

    /// Export metrics in Prometheus format
    pub fn export(&self) -> String {
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let metric_families = self.registry.gather();
        encoder
            .encode_to_string(&metric_families)
            .unwrap_or_else(|e| {
                warn!("Failed to encode metrics: {}", e);
                String::new()
            })
    }
}

impl Default for Metrics {
    fn default() -> Self {
        // FIXED: Properly dereference Arc and clone
        (*Self::new()).clone()
    }
}
