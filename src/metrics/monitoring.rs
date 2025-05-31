// src/metrics/monitoring.rs - Background monitoring tasks and metric collection

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::{Alert, AlertSeverity, AlertType, Metrics};

/// Start background system monitoring task
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
            check_system_performance(&metrics).await;
        }
    })
}

/// Check system performance and trigger alerts if necessary
async fn check_system_performance(metrics: &Arc<Metrics>) {
    // Check request error rates
    let request_total = metrics.request_total.clone();
    let error_requests = request_total
        .with_label_values(&["create_lease", "error"])
        .get()
        + request_total
            .with_label_values(&["renew_lease", "error"])
            .get()
        + request_total
            .with_label_values(&["release_lease", "error"])
            .get();

    let total_requests = request_total
        .with_label_values(&["create_lease", "success"])
        .get()
        + request_total
            .with_label_values(&["create_lease", "error"])
            .get()
        + request_total
            .with_label_values(&["renew_lease", "success"])
            .get()
        + request_total
            .with_label_values(&["renew_lease", "error"])
            .get()
        + request_total
            .with_label_values(&["release_lease", "success"])
            .get()
        + request_total
            .with_label_values(&["release_lease", "error"])
            .get();

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
                    (
                        "error_rate".to_string(),
                        format!(
                            "{:.2}%",
                            error_requests as f64 / total_requests as f64 * 100.0
                        ),
                    ),
                    ("total_requests".to_string(), total_requests.to_string()),
                    ("error_requests".to_string(), error_requests.to_string()),
                ]
                .into(),
                created_at: Instant::now(),
            };
            alerting.trigger_alert(alert);
        });
    }
}

/// Get memory usage for system monitoring
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

/// Extension methods for Metrics struct for startup and operational metrics
impl Metrics {
    /// Record startup duration and check for slow startup alert
    pub fn record_startup_duration(&self, duration: Duration) {
        let duration_secs = duration.as_secs_f64();
        self.startup_duration_seconds.observe(duration_secs);

        // Check if startup was slow and trigger alert
        let alerting = self.alerting.clone();
        tokio::spawn(async move {
            let alerting_guard = alerting.read().await;
            if duration_secs > alerting_guard.thresholds.startup_duration_threshold_seconds {
                drop(alerting_guard); // Release read lock
                let mut alerting = alerting.write().await;
                let alert = Alert {
                    alert_type: AlertType::SlowStartupTime,
                    severity: AlertSeverity::Warning,
                    message: format!("Service startup took {:.2} seconds, which exceeds threshold of {:.2} seconds", 
                                   duration_secs, alerting.thresholds.startup_duration_threshold_seconds),
                    details: [
                        ("startup_duration_seconds".to_string(), duration_secs.to_string()),
                        ("threshold_seconds".to_string(), alerting.thresholds.startup_duration_threshold_seconds.to_string()),
                    ].into(),
                    created_at: Instant::now(),
                };
                alerting.trigger_alert(alert);
            }
        });
    }

    /// Record startup phase timing
    pub fn record_startup_phase(&self, phase: &str, duration: Duration) {
        self.startup_phase_duration
            .with_label_values(&[phase])
            .observe(duration.as_secs_f64());
    }

    /// Mark service as ready
    pub fn record_service_ready(&self) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as f64;
        self.service_ready_timestamp.set(timestamp);
    }

    /// Record configuration validation results
    pub fn record_config_validation(
        &self,
        success: bool,
        duration: Duration,
        error_type: Option<&str>,
    ) {
        let result = if success { "success" } else { "failure" };
        self.configuration_validation_total
            .with_label_values(&[result])
            .inc();
        self.configuration_validation_duration
            .observe(duration.as_secs_f64());

        if !success {
            if let Some(error_type) = error_type {
                self.configuration_errors_total
                    .with_label_values(&[error_type])
                    .inc();
            }

            // Track consecutive failures and trigger alerts
            let alerting = self.alerting.clone();
            let error_type = error_type.unwrap_or("unknown").to_string();
            tokio::spawn(async move {
                let mut alerting = alerting.write().await;
                alerting.increment_consecutive_failures("config_validation");
                let consecutive_count = alerting.get_consecutive_failures("config_validation");

                if consecutive_count >= alerting.thresholds.config_validation_failure_threshold {
                    let alert = Alert {
                        alert_type: AlertType::ConfigurationValidationFailure,
                        severity: AlertSeverity::Critical,
                        message: format!(
                            "Configuration validation failed {} consecutive times",
                            consecutive_count
                        ),
                        details: [
                            (
                                "consecutive_failures".to_string(),
                                consecutive_count.to_string(),
                            ),
                            ("error_type".to_string(), error_type),
                            (
                                "threshold".to_string(),
                                alerting
                                    .thresholds
                                    .config_validation_failure_threshold
                                    .to_string(),
                            ),
                        ]
                        .into(),
                        created_at: Instant::now(),
                    };
                    alerting.trigger_alert(alert);
                }
            });
        } else {
            // Reset consecutive failures on success
            let alerting = self.alerting.clone();
            tokio::spawn(async move {
                let mut alerting = alerting.write().await;
                alerting.reset_consecutive_failures("config_validation");
            });
        }
    }

    /// Record component initialization timing
    pub fn record_component_initialization(&self, component: &str, duration: Duration) {
        self.component_initialization_duration
            .with_label_values(&[component])
            .observe(duration.as_secs_f64());
    }

    /// Record dependencies check timing
    pub fn record_dependencies_check(&self, duration: Duration) {
        self.dependencies_check_duration
            .observe(duration.as_secs_f64());
    }

    /// Record service restart
    pub fn record_service_restart(&self) {
        self.service_restarts_total.inc();
    }

    /// Record config reload attempt
    pub fn record_config_reload(&self, success: bool) {
        let result = if success { "success" } else { "failure" };
        self.config_reload_total.with_label_values(&[result]).inc();
    }

    /// Record startup error
    pub fn record_startup_error(&self, phase: &str, error_type: &str) {
        self.startup_errors_total
            .with_label_values(&[phase, error_type])
            .inc();

        // Track consecutive startup failures
        let alerting = self.alerting.clone();
        let phase = phase.to_string();
        let error_type = error_type.to_string();
        tokio::spawn(async move {
            let mut alerting = alerting.write().await;
            alerting.increment_consecutive_failures("startup");
            let consecutive_count = alerting.get_consecutive_failures("startup");

            if consecutive_count >= alerting.thresholds.consecutive_startup_failures_threshold {
                let alert = Alert {
                    alert_type: AlertType::StartupFailure,
                    severity: AlertSeverity::Fatal,
                    message: format!(
                        "Service startup failed {} consecutive times",
                        consecutive_count
                    ),
                    details: [
                        (
                            "consecutive_failures".to_string(),
                            consecutive_count.to_string(),
                        ),
                        ("phase".to_string(), phase),
                        ("error_type".to_string(), error_type),
                        (
                            "threshold".to_string(),
                            alerting
                                .thresholds
                                .consecutive_startup_failures_threshold
                                .to_string(),
                        ),
                    ]
                    .into(),
                    created_at: Instant::now(),
                };
                alerting.trigger_alert(alert);
            }
        });
    }

    /// Record successful lease creation
    pub fn lease_created(&self, service_id: &str, object_type: &str) {
        self.leases_created_total.inc();
        self.active_leases.inc();
        self.leases_by_service
            .with_label_values(&[service_id])
            .inc();
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

    /// Record failed lease creation
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
                    message: format!(
                        "High number of consecutive lease creation failures: {}",
                        consecutive_count
                    ),
                    details: [
                        (
                            "consecutive_failures".to_string(),
                            consecutive_count.to_string(),
                        ),
                        ("error_type".to_string(), error_type),
                        (
                            "threshold".to_string(),
                            alerting
                                .thresholds
                                .consecutive_lease_failures_threshold
                                .to_string(),
                        ),
                    ]
                    .into(),
                    created_at: Instant::now(),
                };
                alerting.trigger_alert(alert);
            }
        });
    }

    /// Record lease renewal
    pub fn lease_renewed(&self) {
        self.leases_renewed_total.inc();
    }

    /// Record lease expiration
    pub fn lease_expired(&self, service_id: &str, object_type: &str) {
        self.leases_expired_total.inc();
        self.active_leases.dec();
        self.leases_by_service
            .with_label_values(&[service_id])
            .dec();
        self.leases_by_type.with_label_values(&[object_type]).dec();
    }

    /// Record lease release
    pub fn lease_released(&self, service_id: &str, object_type: &str) {
        self.leases_released_total.inc();
        self.active_leases.dec();
        self.leases_by_service
            .with_label_values(&[service_id])
            .dec();
        self.leases_by_type.with_label_values(&[object_type]).dec();
    }

    /// Start cleanup operation timer
    pub fn cleanup_started(&self) -> prometheus::HistogramTimer {
        self.cleanup_operations_total.inc();
        self.cleanup_duration.start_timer()
    }

    /// Record successful cleanup
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

    /// Record failed cleanup
    pub fn cleanup_failed(&self, reason: &str) {
        self.cleanup_failures_total.inc();
        self.cleanup_retries_total
            .with_label_values(&[reason])
            .inc();

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
                    message: format!(
                        "High number of consecutive cleanup failures: {}",
                        consecutive_count
                    ),
                    details: [
                        (
                            "consecutive_failures".to_string(),
                            consecutive_count.to_string(),
                        ),
                        ("reason".to_string(), reason),
                        (
                            "threshold".to_string(),
                            alerting
                                .thresholds
                                .consecutive_cleanup_failures_threshold
                                .to_string(),
                        ),
                    ]
                    .into(),
                    created_at: Instant::now(),
                };
                alerting.trigger_alert(alert);
            }
        });
    }

    /// Record request metrics
    pub fn record_request(&self, method: &str, status: &str, duration: f64) {
        self.request_total
            .with_label_values(&[method, status])
            .inc();
        self.request_duration
            .with_label_values(&[method, status])
            .observe(duration);
        self.grpc_requests_total.with_label_values(&[method]).inc();
    }

    /// Record storage operation
    pub fn record_storage_operation(&self, operation: &str, backend: &str) {
        self.storage_operations_total
            .with_label_values(&[operation, backend])
            .inc();

        // Mark storage as available on successful operation
        self.storage_availability.set(1);
    }

    /// Record storage error
    pub fn record_storage_error(&self, operation: &str, backend: &str, error_type: &str) {
        self.storage_errors_total
            .with_label_values(&[operation, backend, error_type])
            .inc();
        self.storage_operation_failures_by_type
            .with_label_values(&[operation, backend, error_type])
            .inc();

        // Check if this indicates storage unavailability
        if error_type.contains("connection")
            || error_type.contains("timeout")
            || error_type.contains("unavailable")
        {
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
                    ]
                    .into(),
                    created_at: Instant::now(),
                };
                alerting.trigger_alert(alert);
            });
        }
    }

    /// Check cleanup failure rate and trigger alerts
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
                        (
                            "threshold".to_string(),
                            format!("{:.4}", alerting.thresholds.cleanup_failure_rate_threshold),
                        ),
                    ]
                    .into(),
                    created_at: Instant::now(),
                };
                alerting.trigger_alert(alert);
            }
        }
    }

    /// Get active alerts
    pub async fn get_active_alerts(&self) -> Vec<Alert> {
        let alerting = self.alerting.read().await;
        alerting.get_active_alerts()
    }

    /// Get alert summary
    pub async fn get_alert_summary(&self) -> super::alerting::AlertSummary {
        let alerting = self.alerting.read().await;
        alerting.get_summary()
    }
}
