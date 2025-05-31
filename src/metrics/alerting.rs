// src/metrics/alerting.rs - Alerting system for metrics

use std::time::{Duration, Instant};
use tracing::{error, warn};

/// Alerting system for tracking and managing alerts
#[derive(Debug, Clone)]
pub struct AlertingSystem {
    alerts: Vec<Alert>,
    alert_history: Vec<AlertEvent>,
    pub thresholds: AlertThresholds,
    last_alert_times: std::collections::HashMap<AlertType, Instant>,
    consecutive_failures: std::collections::HashMap<String, u64>,
}

/// Configuration thresholds for triggering alerts
#[derive(Debug, Clone)]
pub struct AlertThresholds {
    pub cleanup_failure_rate_threshold: f64,
    pub cleanup_failure_window_minutes: u64,
    pub consecutive_cleanup_failures_threshold: u64,
    pub consecutive_lease_failures_threshold: u64,
    pub storage_unavailable_threshold_seconds: u64,
    pub alert_cooldown_minutes: u64,
    // Startup and config thresholds
    pub startup_duration_threshold_seconds: f64,
    pub config_validation_failure_threshold: u64,
    pub consecutive_startup_failures_threshold: u64,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            cleanup_failure_rate_threshold: 0.5,
            cleanup_failure_window_minutes: 5,
            consecutive_cleanup_failures_threshold: 5,
            consecutive_lease_failures_threshold: 10,
            storage_unavailable_threshold_seconds: 30,
            alert_cooldown_minutes: 15,
            startup_duration_threshold_seconds: 30.0,
            config_validation_failure_threshold: 3,
            consecutive_startup_failures_threshold: 3,
        }
    }
}

/// Types of alerts that can be triggered
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AlertType {
    HighCleanupFailureRate,
    ConsecutiveCleanupFailures,
    LeaseCreationFailures,
    StorageBackendUnavailable,
    CriticalSystemError,
    // Startup and config alert types
    SlowStartupTime,
    ConfigurationValidationFailure,
    StartupFailure,
    DependencyCheckFailure,
}

/// Severity levels for alerts
#[derive(Debug, Clone)]
pub enum AlertSeverity {
    Warning,
    Critical,
    Fatal,
}

/// Individual alert instance
#[derive(Debug, Clone)]
pub struct Alert {
    pub alert_type: AlertType,
    pub severity: AlertSeverity,
    pub message: String,
    pub details: std::collections::HashMap<String, String>,
    pub created_at: Instant,
}

/// Alert event with resolution tracking
#[derive(Debug, Clone)]
pub struct AlertEvent {
    pub alert: Alert,
    pub triggered_at: Instant,
    pub resolved_at: Option<Instant>,
}

/// Summary of current alert status
#[derive(Debug, Clone)]
pub struct AlertSummary {
    pub total_active_alerts: usize,
    pub critical_alerts: usize,
    pub warning_alerts: usize,
    pub fatal_alerts: usize,
    pub recent_alerts: Vec<Alert>,
}

impl AlertingSystem {
    /// Create a new alerting system
    pub fn new() -> Self {
        Self {
            alerts: Vec::new(),
            alert_history: Vec::new(),
            thresholds: AlertThresholds::default(),
            last_alert_times: std::collections::HashMap::new(),
            consecutive_failures: std::collections::HashMap::new(),
        }
    }

    /// Create alerting system with custom thresholds
    pub fn with_thresholds(mut self, thresholds: AlertThresholds) -> Self {
        self.thresholds = thresholds;
        self
    }

    /// Check if an alert should be triggered (respects cooldown)
    pub fn should_alert(&self, alert_type: &AlertType) -> bool {
        if let Some(last_alert_time) = self.last_alert_times.get(alert_type) {
            let cooldown = Duration::from_secs(self.thresholds.alert_cooldown_minutes * 60);
            last_alert_time.elapsed() > cooldown
        } else {
            true
        }
    }

    /// Trigger a new alert
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

    /// Get currently active alerts
    pub fn get_active_alerts(&self) -> Vec<Alert> {
        // Return alerts from the last hour that haven't been resolved
        let one_hour_ago = Instant::now() - Duration::from_secs(3600);
        self.alerts
            .iter()
            .filter(|alert| alert.created_at > one_hour_ago)
            .cloned()
            .collect()
    }

    /// Increment consecutive failure counter
    pub fn increment_consecutive_failures(&mut self, failure_type: &str) {
        let count = self
            .consecutive_failures
            .entry(failure_type.to_string())
            .or_insert(0);
        *count += 1;
    }

    /// Reset consecutive failure counter
    pub fn reset_consecutive_failures(&mut self, failure_type: &str) {
        self.consecutive_failures
            .insert(failure_type.to_string(), 0);
    }

    /// Get consecutive failure count
    pub fn get_consecutive_failures(&self, failure_type: &str) -> u64 {
        self.consecutive_failures
            .get(failure_type)
            .copied()
            .unwrap_or(0)
    }

    /// Get alert summary for health checks
    pub fn get_summary(&self) -> AlertSummary {
        let active_alerts = self.get_active_alerts();

        AlertSummary {
            total_active_alerts: active_alerts.len(),
            critical_alerts: active_alerts
                .iter()
                .filter(|a| matches!(a.severity, AlertSeverity::Critical))
                .count(),
            warning_alerts: active_alerts
                .iter()
                .filter(|a| matches!(a.severity, AlertSeverity::Warning))
                .count(),
            fatal_alerts: active_alerts
                .iter()
                .filter(|a| matches!(a.severity, AlertSeverity::Fatal))
                .count(),
            recent_alerts: active_alerts,
        }
    }

    /// Clear old alerts to prevent memory growth
    pub fn cleanup_old_alerts(&mut self) {
        let cutoff_time = Instant::now() - Duration::from_secs(24 * 3600); // 24 hours
        self.alerts.retain(|alert| alert.created_at > cutoff_time);
        self.alert_history
            .retain(|event| event.triggered_at > cutoff_time);
    }
}

impl Default for AlertingSystem {
    fn default() -> Self {
        Self::new()
    }
}
