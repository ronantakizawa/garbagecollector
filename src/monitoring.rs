// src/monitoring.rs - System monitoring tasks (moved from metrics)

use std::time::Duration;
use tracing::{info, warn};

use crate::metrics::{Metrics, Alert, AlertType, AlertSeverity};
use crate::shutdown::TaskHandle;

/// System monitoring coordinator
pub struct SystemMonitor {
    metrics: std::sync::Arc<Metrics>,
}

impl SystemMonitor {
    /// Create a new system monitor
    pub fn new(metrics: std::sync::Arc<Metrics>) -> Self {
        Self { metrics }
    }

    /// Start system monitoring with shutdown support
    pub async fn start_with_shutdown(&self, mut task_handle: TaskHandle) -> tokio::task::JoinHandle<()> {
        let metrics = self.metrics.clone();
        
        tokio::spawn(async move {
            info!("🔍 Starting system monitoring task");
            
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            let mut monitoring_cycles = 0;
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        monitoring_cycles += 1;
                        
                        // Update memory usage and other system metrics
                        if let Ok(memory_bytes) = get_memory_usage() {
                            metrics.update_memory_usage(memory_bytes);
                        }
                        
                        // Check for performance issues and trigger alerts
                        check_system_health(&metrics).await;
                        
                        // Log periodic status
                        if monitoring_cycles % 10 == 0 {
                            info!("System monitoring: {} cycles completed", monitoring_cycles);
                        }
                    }
                    _ = task_handle.wait_for_shutdown() => {
                        info!("🛑 System monitoring task shutting down gracefully after {} cycles", monitoring_cycles);
                        task_handle.mark_completed().await;
                        break;
                    }
                }
            }
        })
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

/// Check system health and trigger alerts if necessary
async fn check_system_health(metrics: &std::sync::Arc<Metrics>) {
    // Check memory usage
    if let Ok(memory_bytes) = get_memory_usage() {
        const GB: f64 = 1024.0 * 1024.0 * 1024.0;
        if memory_bytes > 2.0 * GB {
            warn!("High memory usage detected: {:.2} GB", memory_bytes / GB);
        }
    }
    
    // Check request error rates
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
        warn!("High gRPC error rate detected: {:.2}%", 
              error_requests as f64 / total_requests as f64 * 100.0);
        
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
                created_at: std::time::Instant::now(),
            };
            alerting.trigger_alert(alert);
        });
    }
}

/// Check for performance bottlenecks
pub async fn check_performance_metrics(metrics: &std::sync::Arc<Metrics>) {
    // Check active lease counts
    let active_leases = metrics.active_leases.get();
    if active_leases > 50000 {
        warn!("High active lease count: {}", active_leases);
    }
    
    // Check cleanup performance
    let cleanup_failures = metrics.cleanup_failures_total.get();
    let cleanup_operations = metrics.cleanup_operations_total.get();
    
    if cleanup_operations > 0 {
        let failure_rate = cleanup_failures as f64 / cleanup_operations as f64;
        if failure_rate > 0.1 {
            warn!("High cleanup failure rate: {:.2}%", failure_rate * 100.0);
        }
    }
}