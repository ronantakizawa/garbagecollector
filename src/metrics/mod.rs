// src/metrics/mod.rs - Complete file with missing methods

use prometheus::{
    Counter, CounterVec, Gauge, GaugeVec, Histogram, HistogramOpts, HistogramVec, IntCounter,
    IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

/// Prometheus metrics for GarbageTruck
pub struct Metrics {
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
    pub cleanup_duration_histogram: Histogram,

    // Storage metrics
    pub storage_operations_total: IntCounterVec,
    pub storage_operation_duration: HistogramVec,

    // System metrics
    pub grpc_requests_total: IntCounterVec,
    pub grpc_request_duration: HistogramVec,

    // Registry for exporting
    pub registry: Registry,
}

impl Metrics {
    pub fn new() -> Arc<Self> {
        let registry = Registry::new();

        // Lease metrics
        let leases_created_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_leases_created_total",
            "Total number of leases created",
        ))
        .expect("Failed to create leases_created_total metric");

        let leases_renewed_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_leases_renewed_total",
            "Total number of lease renewals",
        ))
        .expect("Failed to create leases_renewed_total metric");

        let leases_expired_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_leases_expired_total",
            "Total number of leases that expired",
        ))
        .expect("Failed to create leases_expired_total metric");

        let leases_released_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_leases_released_total",
            "Total number of leases explicitly released",
        ))
        .expect("Failed to create leases_released_total metric");

        let active_leases = IntGauge::with_opts(Opts::new(
            "garbagetruck_active_leases",
            "Current number of active leases",
        ))
        .expect("Failed to create active_leases metric");

        let leases_by_service = IntGaugeVec::new(
            Opts::new(
                "garbagetruck_leases_by_service",
                "Number of leases grouped by service",
            ),
            &["service_id"],
        )
        .expect("Failed to create leases_by_service metric");

        let leases_by_type = IntGaugeVec::new(
            Opts::new(
                "garbagetruck_leases_by_type",
                "Number of leases grouped by object type",
            ),
            &["object_type"],
        )
        .expect("Failed to create leases_by_type metric");

        // Cleanup metrics
        let cleanup_operations_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_cleanup_operations_total",
            "Total number of cleanup operations performed",
        ))
        .expect("Failed to create cleanup_operations_total metric");

        let cleanup_failures_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_cleanup_failures_total",
            "Total number of failed cleanup operations",
        ))
        .expect("Failed to create cleanup_failures_total metric");

        let cleanup_duration_histogram = Histogram::with_opts(HistogramOpts::new(
            "garbagetruck_cleanup_duration_seconds",
            "Duration of cleanup operations in seconds",
        ))
        .expect("Failed to create cleanup_duration_histogram metric");

        // Storage metrics
        let storage_operations_total = IntCounterVec::new(
            Opts::new(
                "garbagetruck_storage_operations_total",
                "Total number of storage operations",
            ),
            &["operation", "backend"],
        )
        .expect("Failed to create storage_operations_total metric");

        let storage_operation_duration = HistogramVec::new(
            HistogramOpts::new(
                "garbagetruck_storage_operation_duration_seconds",
                "Duration of storage operations in seconds",
            ),
            &["operation", "backend"],
        )
        .expect("Failed to create storage_operation_duration metric");

        // gRPC metrics
        let grpc_requests_total = IntCounterVec::new(
            Opts::new(
                "garbagetruck_grpc_requests_total",
                "Total number of gRPC requests",
            ),
            &["method", "status"],
        )
        .expect("Failed to create grpc_requests_total metric");

        let grpc_request_duration = HistogramVec::new(
            HistogramOpts::new(
                "garbagetruck_grpc_request_duration_seconds",
                "Duration of gRPC requests in seconds",
            ),
            &["method"],
        )
        .expect("Failed to create grpc_request_duration metric");

        // Register all metrics
        registry
            .register(Box::new(leases_created_total.clone()))
            .expect("Failed to register leases_created_total");
        registry
            .register(Box::new(leases_renewed_total.clone()))
            .expect("Failed to register leases_renewed_total");
        registry
            .register(Box::new(leases_expired_total.clone()))
            .expect("Failed to register leases_expired_total");
        registry
            .register(Box::new(leases_released_total.clone()))
            .expect("Failed to register leases_released_total");
        registry
            .register(Box::new(active_leases.clone()))
            .expect("Failed to register active_leases");
        registry
            .register(Box::new(leases_by_service.clone()))
            .expect("Failed to register leases_by_service");
        registry
            .register(Box::new(leases_by_type.clone()))
            .expect("Failed to register leases_by_type");
        registry
            .register(Box::new(cleanup_operations_total.clone()))
            .expect("Failed to register cleanup_operations_total");
        registry
            .register(Box::new(cleanup_failures_total.clone()))
            .expect("Failed to register cleanup_failures_total");
        registry
            .register(Box::new(cleanup_duration_histogram.clone()))
            .expect("Failed to register cleanup_duration_histogram");
        registry
            .register(Box::new(storage_operations_total.clone()))
            .expect("Failed to register storage_operations_total");
        registry
            .register(Box::new(storage_operation_duration.clone()))
            .expect("Failed to register storage_operation_duration");
        registry
            .register(Box::new(grpc_requests_total.clone()))
            .expect("Failed to register grpc_requests_total");
        registry
            .register(Box::new(grpc_request_duration.clone()))
            .expect("Failed to register grpc_request_duration");

        let metrics = Arc::new(Self {
            leases_created_total,
            leases_renewed_total,
            leases_expired_total,
            leases_released_total,
            active_leases,
            leases_by_service,
            leases_by_type,
            cleanup_operations_total,
            cleanup_failures_total,
            cleanup_duration_histogram,
            storage_operations_total,
            storage_operation_duration,
            grpc_requests_total,
            grpc_request_duration,
            registry,
        });

        info!("✅ Metrics system initialized");
        metrics
    }

    // Lease operation metrics
    pub fn increment_leases_created(&self) {
        self.leases_created_total.inc();
        debug!("📊 Incremented leases_created_total");
    }

    pub fn increment_leases_renewed(&self) {
        self.leases_renewed_total.inc();
        debug!("📊 Incremented leases_renewed_total");
    }

    pub fn increment_leases_expired(&self) {
        self.leases_expired_total.inc();
        debug!("📊 Incremented leases_expired_total");
    }

    pub fn increment_leases_released(&self) {
        self.leases_released_total.inc();
        debug!("📊 Incremented leases_released_total");
    }

    pub fn set_active_leases(&self, count: i64) {
        self.active_leases.set(count);
        debug!("📊 Set active_leases to {}", count);
    }

    pub fn increment_active_leases(&self) {
        self.active_leases.inc();
        debug!("📊 Incremented active_leases");
    }

    pub fn decrement_active_leases(&self) {
        self.active_leases.dec();
        debug!("📊 Decremented active_leases");
    }

    // Cleanup operation metrics
    pub fn increment_cleanup_operations(&self) {
        self.cleanup_operations_total.inc();
        debug!("📊 Incremented cleanup_operations_total");
    }

    pub fn increment_cleanup_failures(&self) {
        self.cleanup_failures_total.inc();
        debug!("📊 Incremented cleanup_failures_total");
    }

    pub fn record_cleanup_duration(&self, duration: std::time::Duration) {
        self.cleanup_duration_histogram
            .observe(duration.as_secs_f64());
        debug!("📊 Recorded cleanup duration: {:?}", duration);
    }

    // Storage operation metrics
    pub fn increment_storage_operation(&self, operation: &str, backend: &str) {
        self.storage_operations_total
            .with_label_values(&[operation, backend])
            .inc();
        debug!("📊 Incremented storage operation: {} on {}", operation, backend);
    }

    pub fn record_storage_operation_duration(&self, operation: &str, backend: &str, duration: std::time::Duration) {
        self.storage_operation_duration
            .with_label_values(&[operation, backend])
            .observe(duration.as_secs_f64());
        debug!("📊 Recorded storage operation duration: {} on {} took {:?}", operation, backend, duration);
    }

    // gRPC metrics
    pub fn increment_grpc_request(&self, method: &str, status: &str) {
        self.grpc_requests_total
            .with_label_values(&[method, status])
            .inc();
        debug!("📊 Incremented gRPC request: {} with status {}", method, status);
    }

    pub fn record_grpc_request_duration(&self, method: &str, duration: std::time::Duration) {
        self.grpc_request_duration
            .with_label_values(&[method])
            .observe(duration.as_secs_f64());
        debug!("📊 Recorded gRPC request duration: {} took {:?}", method, duration);
    }

    // Service and type tracking
    pub fn update_leases_by_service(&self, stats: &HashMap<String, usize>) {
        // Clear existing values
        self.leases_by_service.reset();
        
        // Set new values
        for (service_id, count) in stats {
            self.leases_by_service
                .with_label_values(&[service_id])
                .set(*count as i64);
        }
        debug!("📊 Updated leases_by_service metrics");
    }

    pub fn update_leases_by_type(&self, stats: &HashMap<String, usize>) {
        // Clear existing values
        self.leases_by_type.reset();
        
        // Set new values
        for (object_type, count) in stats {
            self.leases_by_type
                .with_label_values(&[object_type])
                .set(*count as i64);
        }
        debug!("📊 Updated leases_by_type metrics");
    }

    // Update all lease-related metrics from storage stats
    pub fn update_from_lease_stats(&self, stats: &crate::lease::LeaseStats) {
        self.set_active_leases(stats.active_leases as i64);
        self.update_leases_by_service(&stats.leases_by_service);
        self.update_leases_by_type(&stats.leases_by_type);
        debug!("📊 Updated all metrics from lease stats");
    }

    /// Get all current metric values as a formatted string
    pub fn get_metrics_text(&self) -> String {
        // This would typically use prometheus::Encoder, but we'll provide a simple implementation
        format!(
            "# GarbageTruck Metrics\n\
             garbagetruck_leases_created_total {}\n\
             garbagetruck_leases_renewed_total {}\n\
             garbagetruck_leases_expired_total {}\n\
             garbagetruck_leases_released_total {}\n\
             garbagetruck_active_leases {}\n\
             garbagetruck_cleanup_operations_total {}\n\
             garbagetruck_cleanup_failures_total {}\n",
            self.leases_created_total.get(),
            self.leases_renewed_total.get(),
            self.leases_expired_total.get(),
            self.leases_released_total.get(),
            self.active_leases.get(),
            self.cleanup_operations_total.get(),
            self.cleanup_failures_total.get(),
        )
    }

    /// Export metrics in Prometheus format
    pub fn export_metrics(&self) -> Result<String, Box<dyn std::error::Error>> {
        let metric_families = self.registry.gather();
        // For now, return a simple text representation
        // In a full implementation, you'd use prometheus::TextEncoder
        Ok(self.get_metrics_text())
    }
}

impl Default for Metrics {
    fn default() -> Self {
        // Create a new instance and clone it instead of dereferencing Arc
        let metrics = Self::new();
        // Since Self::new() returns Arc<Self>, we need to extract the inner value
        // We'll reconstruct the default instead
        let registry = Registry::new();

        // Recreate all metrics for default implementation
        let leases_created_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_leases_created_total",
            "Total number of leases created",
        )).expect("Failed to create leases_created_total metric");

        let leases_renewed_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_leases_renewed_total", 
            "Total number of lease renewals",
        )).expect("Failed to create leases_renewed_total metric");

        let leases_expired_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_leases_expired_total",
            "Total number of leases that expired",
        )).expect("Failed to create leases_expired_total metric");

        let leases_released_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_leases_released_total",
            "Total number of leases explicitly released",
        )).expect("Failed to create leases_released_total metric");

        let active_leases = IntGauge::with_opts(Opts::new(
            "garbagetruck_active_leases",
            "Current number of active leases",
        )).expect("Failed to create active_leases metric");

        let leases_by_service = IntGaugeVec::new(
            Opts::new("garbagetruck_leases_by_service", "Number of leases grouped by service"),
            &["service_id"],
        ).expect("Failed to create leases_by_service metric");

        let leases_by_type = IntGaugeVec::new(
            Opts::new("garbagetruck_leases_by_type", "Number of leases grouped by object type"),
            &["object_type"],
        ).expect("Failed to create leases_by_type metric");

        let cleanup_operations_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_cleanup_operations_total",
            "Total number of cleanup operations performed",
        )).expect("Failed to create cleanup_operations_total metric");

        let cleanup_failures_total = IntCounter::with_opts(Opts::new(
            "garbagetruck_cleanup_failures_total",
            "Total number of failed cleanup operations",
        )).expect("Failed to create cleanup_failures_total metric");

        let cleanup_duration_histogram = Histogram::with_opts(HistogramOpts::new(
            "garbagetruck_cleanup_duration_seconds",
            "Duration of cleanup operations in seconds",
        )).expect("Failed to create cleanup_duration_histogram metric");

        let storage_operations_total = IntCounterVec::new(
            Opts::new("garbagetruck_storage_operations_total", "Total number of storage operations"),
            &["operation", "backend"],
        ).expect("Failed to create storage_operations_total metric");

        let storage_operation_duration = HistogramVec::new(
            HistogramOpts::new(
                "garbagetruck_storage_operation_duration_seconds",
                "Duration of storage operations in seconds",
            ),
            &["operation", "backend"],
        ).expect("Failed to create storage_operation_duration metric");

        let grpc_requests_total = IntCounterVec::new(
            Opts::new("garbagetruck_grpc_requests_total", "Total number of gRPC requests"),
            &["method", "status"],
        ).expect("Failed to create grpc_requests_total metric");

        let grpc_request_duration = HistogramVec::new(
            HistogramOpts::new(
                "garbagetruck_grpc_request_duration_seconds",
                "Duration of gRPC requests in seconds",
            ),
            &["method"],
        ).expect("Failed to create grpc_request_duration metric");

        Self {
            leases_created_total,
            leases_renewed_total,
            leases_expired_total,
            leases_released_total,
            active_leases,
            leases_by_service,
            leases_by_type,
            cleanup_operations_total,
            cleanup_failures_total,
            cleanup_duration_histogram,
            storage_operations_total,
            storage_operation_duration,
            grpc_requests_total,
            grpc_request_duration,
            registry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_creation() {
        let metrics = Metrics::new();
        assert_eq!(metrics.leases_created_total.get(), 0);
        assert_eq!(metrics.active_leases.get(), 0);
    }

    #[test]
    fn test_lease_metrics() {
        let metrics = Metrics::new();
        
        metrics.increment_leases_created();
        assert_eq!(metrics.leases_created_total.get(), 1);
        
        metrics.increment_active_leases();
        assert_eq!(metrics.active_leases.get(), 1);
        
        metrics.decrement_active_leases();
        assert_eq!(metrics.active_leases.get(), 0);
    }

    #[test]
    fn test_cleanup_metrics() {
        let metrics = Metrics::new();
        
        metrics.increment_cleanup_operations();
        assert_eq!(metrics.cleanup_operations_total.get(), 1);
        
        metrics.increment_cleanup_failures();
        assert_eq!(metrics.cleanup_failures_total.get(), 1);
        
        let duration = std::time::Duration::from_millis(100);
        metrics.record_cleanup_duration(duration);
        // Can't easily test histogram values, but ensure no panic
    }

    #[test]
    fn test_service_type_metrics() {
        let metrics = Metrics::new();
        
        let mut service_stats = HashMap::new();
        service_stats.insert("service1".to_string(), 5);
        service_stats.insert("service2".to_string(), 3);
        
        let mut type_stats = HashMap::new();
        type_stats.insert("TemporaryFile".to_string(), 4);
        type_stats.insert("DatabaseRow".to_string(), 4);
        
        metrics.update_leases_by_service(&service_stats);
        metrics.update_leases_by_type(&type_stats);
        
        // Test passes if no panics occur
    }
}