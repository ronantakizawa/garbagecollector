use prometheus::{
    Gauge, Histogram, HistogramOpts, HistogramVec, IntCounter,
    IntCounterVec, IntGauge, IntGaugeVec, Opts, Registry,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{error, warn, info};

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
    
    // New alerting-related metrics
    pub lease_creation_failures_total: IntCounter,
    pub storage_availability: IntGauge,
    pub consecutive_cleanup_failures: IntGauge,
    pub consecutive_lease_creation_failures: IntGauge,
    pub storage_operation_failures_by_type: IntCounterVec,
    
    // Alerting system
    pub alerting: Arc<RwLock<AlertingSystem>>,
}

#[derive(Debug, Clone)]
pub struct AlertingSystem {
    alerts: Vec<Alert>,
    alert_history: Vec<AlertEvent>,
    thresholds: AlertThresholds,
    last_alert_times: std::collections::HashMap<AlertType, Instant>,
    consecutive_failures: std::collections::HashMap<String, u64>,
}

#[derive(Debug, Clone)]
pub struct AlertThresholds {
    pub cleanup_failure_rate_threshold: f64,      // e.g., 0.5 for 50%
    pub cleanup_failure_window_minutes: u64,      // e.g., 5 minutes
    pub consecutive_cleanup_failures_threshold: u64, // e.g., 5
    pub consecutive_lease_failures_threshold: u64,   // e.g., 10
    pub storage_unavailable_threshold_seconds: u64,  // e.g., 30 seconds
    pub alert_cooldown_minutes: u64,              // e.g., 15 minutes between same alerts
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AlertType {
    HighCleanupFailureRate,
    ConsecutiveCleanupFailures,
    LeaseCreationFailures,
    StorageBackendUnavailable,
    CriticalSystemError,
}

#[derive(Debug, Clone)]
pub struct Alert {
    pub alert_type: AlertType,
    pub severity: AlertSeverity,
    pub message: String,
    pub details: std::collections::HashMap<String, String>,
    pub created_at: Instant,
}

#[derive(Debug, Clone)]
pub enum AlertSeverity {
    Warning,
    Critical,
    Fatal,
}

#[derive(Debug, Clone)]
pub struct AlertEvent {
    pub alert: Alert,
    pub triggered_at: Instant,
    pub resolved_at: Option<Instant>,
}

#[derive(Debug, Clone)]
pub struct AlertSummary {
    pub total_active_alerts: usize,
    pub critical_alerts: usize,
    pub warning_alerts: usize,
    pub fatal_alerts: usize,
    pub recent_alerts: Vec<Alert>,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            cleanup_failure_rate_threshold: 0.5,      // 50% failure rate
            cleanup_failure_window_minutes: 5,         // in last 5 minutes
            consecutive_cleanup_failures_threshold: 5,  // 5 consecutive failures
            consecutive_lease_failures_threshold: 10,   // 10 consecutive failures
            storage_unavailable_threshold_seconds: 30,  // 30 seconds of unavailability
            alert_cooldown_minutes: 15,                // 15 minutes between same alerts
        }
    }
}

impl AlertingSystem {
    pub fn new() -> Self {
        Self {
            alerts: Vec::new(),
            alert_history: Vec::new(),
            thresholds: AlertThresholds::default(),
            last_alert_times: std::collections::HashMap::new(),
            consecutive_failures: std::collections::HashMap::new(),
        }
    }
    
    pub fn with_thresholds(mut self, thresholds: AlertThresholds) -> Self {
        self.thresholds = thresholds;
        self
    }
    
    pub fn should_alert(&self, alert_type: &AlertType) -> bool {
        if let Some(last_alert_time) = self.last_alert_times.get(alert_type) {
            let cooldown = Duration::from_secs(self.thresholds.alert_cooldown_minutes * 60);
            last_alert_time.elapsed() > cooldown
        } else {
            true
        }
    }
    
    pub fn trigger_alert(&mut self, alert: Alert) {
        let alert_type = alert.alert_type.clone();
        
        if !self.should_alert(&alert_type) {
            return; // Still in cooldown period
        }
        
        // Log the alert
        match alert.severity {
            AlertSeverity::Warning => {
                warn!(
                    alert_type = ?alert_type,
                    message = %alert.message,
                    details = ?alert.details,
                    "ALERT TRIGGERED"
                );
            }
            AlertSeverity::Critical => {
                error!(
                    alert_type = ?alert_type,
                    message = %alert.message,
                    details = ?alert.details,
                    "CRITICAL ALERT TRIGGERED"
                );
            }
            AlertSeverity::Fatal => {
                error!(
                    alert_type = ?alert_type,
                    message = %alert.message,
                    details = ?alert.details,
                    "FATAL ALERT TRIGGERED"
                );
            }
        }
        
        // Update last alert time
        self.last_alert_times.insert(alert_type, Instant::now());
        
        // Store alert
        let alert_event = AlertEvent {
            alert: alert.clone(),
            triggered_at: Instant::now(),
            resolved_at: None,
        };
        
        self.alerts.push(alert);
        self.alert_history.push(alert_event);
        
        // Keep only recent alerts in memory (last 100)
        if self.alert_history.len() > 100 {
            self.alert_history.drain(..50);
        }
    }
    
    pub fn get_active_alerts(&self) -> Vec<Alert> {
        // Return alerts from the last hour that haven't been resolved
        let one_hour_ago = Instant::now() - Duration::from_secs(3600);
        self.alerts
            .iter()
            .filter(|alert| alert.created_at > one_hour_ago)
            .cloned()
            .collect()
    }
    
    pub fn increment_consecutive_failures(&mut self, failure_type: &str) {
        let count = self.consecutive_failures
            .entry(failure_type.to_string())
            .or_insert(0);
        *count += 1;
    }
    
    pub fn reset_consecutive_failures(&mut self, failure_type: &str) {
        self.consecutive_failures.insert(failure_type.to_string(), 0);
    }
    
    pub fn get_consecutive_failures(&self, failure_type: &str) -> u64 {
        self.consecutive_failures.get(failure_type).copied().unwrap_or(0)
    }
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
        
        // New alerting-related metrics
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
            alerting: Arc::new(RwLock::new(AlertingSystem::new().with_thresholds(thresholds))),
        })
    }
    
    pub fn lease_created(&self, service_id: &str, object_type: &str) {
        self.leases_created_total.inc();
        self.active_leases.inc();
        self.leases_by_service.with_label_values(&[service_id]).inc();
        self.leases_by_type.with_label_values(&[object_type]).inc();
        
        // Reset consecutive lease creation failures on success
        tokio::spawn({
            let alerting = self.alerting.clone();
            async move {
                let mut alerting = alerting.write().await;
                alerting.reset_consecutive_failures("lease_creation");
            }
        });
        self.consecutive_lease_creation_failures.set(0);
    }
    
    pub fn lease_creation_failed(&self, error_type: &str) {
        self.lease_creation_failures_total.inc();
        
        // Track consecutive failures and check for alerts
        let alerting = self.alerting.clone();
        let consecutive_gauge = self.consecutive_lease_creation_failures.clone();
        let error_type = error_type.to_string();
        
        tokio::spawn(async move {
            let mut alerting = alerting.write().await;
            alerting.increment_consecutive_failures("lease_creation");
            let consecutive_count = alerting.get_consecutive_failures("lease_creation");
            consecutive_gauge.set(consecutive_count as i64);
            
            if consecutive_count >= alerting.thresholds.consecutive_lease_failures_threshold {
                let alert = Alert {
                    alert_type: AlertType::LeaseCreationFailures,
                    severity: AlertSeverity::Critical,
                    message: format!("High number of consecutive lease creation failures: {}", consecutive_count),
                    details: [
                        ("consecutive_failures".to_string(), consecutive_count.to_string()),
                        ("error_type".to_string(), error_type),
                        ("threshold".to_string(), alerting.thresholds.consecutive_lease_failures_threshold.to_string()),
                    ].into(),
                    created_at: Instant::now(),
                };
                alerting.trigger_alert(alert);
            }
        });
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
    
    pub fn cleanup_started(&self) -> prometheus::HistogramTimer {
        self.cleanup_operations_total.inc();
        self.cleanup_duration.start_timer()
    }
    
    pub fn cleanup_succeeded(&self) {
        // Reset consecutive cleanup failures on success
        tokio::spawn({
            let alerting = self.alerting.clone();
            let consecutive_gauge = self.consecutive_cleanup_failures.clone();
            async move {
                let mut alerting = alerting.write().await;
                alerting.reset_consecutive_failures("cleanup");
                consecutive_gauge.set(0);
            }
        });
    }
    
    pub fn cleanup_failed(&self, reason: &str) {
        self.cleanup_failures_total.inc();
        self.cleanup_retries_total.with_label_values(&[reason]).inc();
        
        // Track consecutive failures and check for alerts
        let alerting = self.alerting.clone();
        let consecutive_gauge = self.consecutive_cleanup_failures.clone();
        let reason = reason.to_string();
        
        tokio::spawn(async move {
            let mut alerting = alerting.write().await;
            alerting.increment_consecutive_failures("cleanup");
            let consecutive_count = alerting.get_consecutive_failures("cleanup");
            consecutive_gauge.set(consecutive_count as i64);
            
            if consecutive_count >= alerting.thresholds.consecutive_cleanup_failures_threshold {
                let alert = Alert {
                    alert_type: AlertType::ConsecutiveCleanupFailures,
                    severity: AlertSeverity::Critical,
                    message: format!("High number of consecutive cleanup failures: {}", consecutive_count),
                    details: [
                        ("consecutive_failures".to_string(), consecutive_count.to_string()),
                        ("reason".to_string(), reason),
                        ("threshold".to_string(), alerting.thresholds.consecutive_cleanup_failures_threshold.to_string()),
                    ].into(),
                    created_at: Instant::now(),
                };
                alerting.trigger_alert(alert);
            }
        });
    }
    
    pub fn record_request(&self, method: &str, status: &str, duration: f64) {
        self.request_total.with_label_values(&[method, status]).inc();
        self.request_duration.with_label_values(&[method, status]).observe(duration);
        self.grpc_requests_total.with_label_values(&[method]).inc();
    }
    
    pub fn record_storage_operation(&self, operation: &str, backend: &str) {
        self.storage_operations_total.with_label_values(&[operation, backend]).inc();
        
        // Mark storage as available on successful operation
        self.storage_availability.set(1);
    }
    
    pub fn record_storage_error(&self, operation: &str, backend: &str, error_type: &str) {
        self.storage_errors_total.with_label_values(&[operation, backend, error_type]).inc();
        self.storage_operation_failures_by_type.with_label_values(&[operation, backend, error_type]).inc();
        
        // Check if this indicates storage unavailability
        if error_type.contains("connection") || error_type.contains("timeout") || error_type.contains("unavailable") {
            self.storage_availability.set(0);
            
            let alerting = self.alerting.clone();
            let backend = backend.to_string();
            let error_type = error_type.to_string();
            let operation = operation.to_string();
            
            tokio::spawn(async move {
                let mut alerting = alerting.write().await;
                let alert = Alert {
                    alert_type: AlertType::StorageBackendUnavailable,
                    severity: AlertSeverity::Fatal,
                    message: format!("Storage backend '{}' appears to be unavailable", backend),
                    details: [
                        ("backend".to_string(), backend),
                        ("operation".to_string(), operation),
                        ("error_type".to_string(), error_type),
                    ].into(),
                    created_at: Instant::now(),
                };
                alerting.trigger_alert(alert);
            });
        }
    }
    
    pub fn update_memory_usage(&self, bytes: f64) {
        self.memory_usage.set(bytes);
    }
    
    pub fn gather(&self) -> Vec<prometheus::proto::MetricFamily> {
        self.registry.gather()
    }
    
    pub fn reset_service_metrics(&self, service_id: &str) {
        self.leases_by_service.remove_label_values(&[service_id]).ok();
    }
    
    pub fn update_lease_counts(&self, stats: &crate::lease::LeaseStats) {
        self.active_leases.set(stats.active_leases as i64);
        
        for (service_id, count) in &stats.leases_by_service {
            self.leases_by_service.with_label_values(&[service_id]).set(*count as i64);
        }
        
        for (object_type, count) in &stats.leases_by_type {
            self.leases_by_type.with_label_values(&[object_type]).set(*count as i64);
        }
    }
    
    pub async fn check_cleanup_failure_rate(&self) {
        let alerting = self.alerting.read().await;
        
        // This is a simplified check. In a real implementation, you'd want to
        // track operations over time windows properly
        let total_operations = self.cleanup_operations_total.get();
        let total_failures = self.cleanup_failures_total.get();
        
        if total_operations > 0 {
            let failure_rate = total_failures as f64 / total_operations as f64;
            if failure_rate > alerting.thresholds.cleanup_failure_rate_threshold {
                drop(alerting); // Release read lock
                let mut alerting = self.alerting.write().await;
                
                let alert = Alert {
                    alert_type: AlertType::HighCleanupFailureRate,
                    severity: AlertSeverity::Warning,
                    message: format!("High cleanup failure rate: {:.2}%", failure_rate * 100.0),
                    details: [
                        ("failure_rate".to_string(), format!("{:.4}", failure_rate)),
                        ("total_operations".to_string(), total_operations.to_string()),
                        ("total_failures".to_string(), total_failures.to_string()),
                        ("threshold".to_string(), format!("{:.4}", alerting.thresholds.cleanup_failure_rate_threshold)),
                    ].into(),
                    created_at: Instant::now(),
                };
                alerting.trigger_alert(alert);
            }
        }
    }
    
    pub async fn get_active_alerts(&self) -> Vec<Alert> {
        let alerting = self.alerting.read().await;
        alerting.get_active_alerts()
    }
    
    pub async fn get_alert_summary(&self) -> AlertSummary {
        let alerting = self.alerting.read().await;
        let active_alerts = alerting.get_active_alerts();
        
        AlertSummary {
            total_active_alerts: active_alerts.len(),
            critical_alerts: active_alerts.iter().filter(|a| matches!(a.severity, AlertSeverity::Critical)).count(),
            warning_alerts: active_alerts.iter().filter(|a| matches!(a.severity, AlertSeverity::Warning)).count(),
            fatal_alerts: active_alerts.iter().filter(|a| matches!(a.severity, AlertSeverity::Fatal)).count(),
            recent_alerts: active_alerts,
        }
    }
    
    /// Start the alerting monitoring task
    pub fn start_alerting_monitor(&self) -> tokio::task::JoinHandle<()> {
        let metrics = self.clone();
        
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30)); // Check every 30 seconds
            
            loop {
                interval.tick().await;
                
                // Check cleanup failure rate
                metrics.check_cleanup_failure_rate().await;
                
                // Log current alert status
                let summary = metrics.get_alert_summary().await;
                if summary.total_active_alerts > 0 {
                    info!(
                        active_alerts = summary.total_active_alerts,
                        critical = summary.critical_alerts,
                        warnings = summary.warning_alerts,
                        fatal = summary.fatal_alerts,
                        "Alert status update"
                    );
                }
            }
        })
    }
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new().expect("Failed to create metrics")
    }
}

// Tonic interceptor for metrics collection
use tonic::{Request, Status};

#[derive(Clone)]
pub struct MetricsInterceptor {
    metrics: Arc<Metrics>,
}

impl MetricsInterceptor {
    pub fn new(metrics: Arc<Metrics>) -> Self {
        Self { metrics }
    }
}

impl tonic::service::Interceptor for MetricsInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        // Get method name from request metadata or extensions
        // Since we can't access the URI directly, we'll track this differently
        // The method name will be tracked in the service methods themselves
        
        // For now, just increment a general request counter
        self.metrics.grpc_requests_total.with_label_values(&["all"]).inc();
        
        // Store start time in request extensions for duration tracking
        let start_time = Instant::now();
        request.extensions_mut().insert(RequestStartTime(start_time));
        
        Ok(request)
    }
}

// Helper struct to store request start time
#[derive(Clone, Copy)]
struct RequestStartTime(Instant);

// Memory usage monitoring function
fn get_memory_usage() -> Result<f64, Box<dyn std::error::Error>> {
    // Simple implementation - in production you might use sysinfo crate
    #[cfg(target_os = "linux")]
    {
        use std::fs::read_to_string;
        let status = read_to_string("/proc/self/status")?;
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<f64>() {
                        return Ok(kb * 1024.0); // Convert KB to bytes
                    }
                }
            }
        }
    }
    
    // Fallback for other platforms or if parsing fails
    Ok(0.0)
}

// Background monitoring task
pub fn start_system_monitoring(metrics: Arc<Metrics>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        
        loop {
            interval.tick().await;
            
            // Update memory usage
            if let Ok(memory_bytes) = get_memory_usage() {
                metrics.update_memory_usage(memory_bytes);
            }
            
            // Check for performance issues
            let request_total = metrics.request_total.clone();
            let error_requests = request_total.with_label_values(&["create_lease", "error"]).get()
                + request_total.with_label_values(&["renew_lease", "error"]).get()
                + request_total.with_label_values(&["release_lease", "error"]).get();
            
            let total_requests = request_total.with_label_values(&["create_lease", "success"]).get()
                + request_total.with_label_values(&["create_lease", "error"]).get()
                + request_total.with_label_values(&["renew_lease", "success"]).get()
                + request_total.with_label_values(&["renew_lease", "error"]).get()
                + request_total.with_label_values(&["release_lease", "success"]).get()
                + request_total.with_label_values(&["release_lease", "error"]).get();
            
            // Alert on high error rates
            if total_requests > 10 && error_requests as f64 / total_requests as f64 > 0.1 {
                let alerting = metrics.alerting.clone();
                tokio::spawn(async move {
                    let mut alerting = alerting.write().await;
                    let alert = Alert {
                        alert_type: AlertType::CriticalSystemError,
                        severity: AlertSeverity::Warning,
                        message: "High gRPC error rate detected".to_string(),
                        details: [
                            ("error_rate".to_string(), format!("{:.2}%", error_requests as f64 / total_requests as f64 * 100.0)),
                            ("total_requests".to_string(), total_requests.to_string()),
                            ("error_requests".to_string(), error_requests.to_string()),
                        ].into(),
                        created_at: Instant::now(),
                    };
                    alerting.trigger_alert(alert);
                });
            }
        }
    })
}