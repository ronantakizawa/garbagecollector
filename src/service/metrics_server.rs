// src/service/metrics_server.rs - Dedicated metrics server

use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info};

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::metrics::Metrics;

/// Manages the metrics HTTP server
pub struct MetricsServer {
    config: Arc<Config>,
    metrics: Arc<Metrics>,
}

impl MetricsServer {
    pub fn new(config: Arc<Config>, metrics: Arc<Metrics>) -> Self {
        Self { config, metrics }
    }

    /// Start the metrics server
    pub async fn start(&self) -> Result<()> {
        if !self.config.metrics.enabled {
            info!("📊 Metrics server disabled by configuration");
            return Ok(());
        }

        let metrics_addr = format!("0.0.0.0:{}", self.config.metrics.port);
        let metrics = self.metrics.clone();
        let config = self.config.clone();

        tokio::spawn(async move {
            if let Err(e) = start_metrics_server(metrics_addr, metrics, config).await {
                error!("❌ Metrics server failed: {}", e);
            }
        });

        info!(
            "📊 Metrics server started on port {} (recovery metrics: {}, WAL metrics: {})",
            self.config.metrics.port,
            self.config.metrics.include_recovery_metrics,
            self.config.metrics.include_wal_metrics
        );

        Ok(())
    }
}

/// Start metrics server with enhanced metrics
async fn start_metrics_server(
    addr: String,
    metrics: Arc<Metrics>,
    config: Arc<Config>,
) -> Result<()> {
    use warp::Filter;

    // Clone metrics and config for the closures
    let metrics_for_metrics_route = metrics.clone();
    let config_for_metrics_route = config.clone();
    let metrics_for_status_route = metrics.clone();

    let metrics_route = warp::path("metrics")
        .and(warp::get())
        .and(warp::any().map(move || (metrics_for_metrics_route.clone(), config_for_metrics_route.clone())))
        .and_then(handle_metrics_request);

    let health_route = warp::path("health").and(warp::get()).map(|| {
        warp::reply::json(&serde_json::json!({
            "status": "ok",
            "service": "garbagetruck-metrics",
            "features": get_enabled_features()
        }))
    });

    let status_route = warp::path("status")
        .and(warp::get())
        .and(warp::any().map(move || metrics_for_status_route.clone()))
        .and_then(handle_status_request);

    let routes = metrics_route.or(health_route).or(status_route);

    info!("📊 Starting enhanced metrics server on http://{}", addr);

    warp::serve(routes)
        .run(addr.parse::<SocketAddr>().unwrap())
        .await;

    Ok(())
}

async fn handle_metrics_request(
    (metrics, config): (Arc<Metrics>, Arc<Config>),
) -> std::result::Result<impl warp::Reply, warp::Rejection> {
    // Return enhanced Prometheus metrics format including recovery and WAL metrics
    let metrics_text = metrics.export_enhanced_prometheus_metrics(&config);
    Ok(warp::reply::with_header(
        metrics_text,
        "content-type",
        "text/plain; version=0.0.4; charset=utf-8",
    ))
}

async fn handle_status_request(
    metrics: Arc<Metrics>,
) -> std::result::Result<impl warp::Reply, warp::Rejection> {
    let status = MetricsStatus {
        uptime_seconds: metrics.get_uptime_seconds(),
        total_requests: metrics.get_total_requests(),
        active_leases: metrics.get_active_lease_count(),
        memory_usage_mb: get_memory_usage_mb(),
        features: get_enabled_features(),
    };
    
    Ok(warp::reply::json(&status))
}

fn get_enabled_features() -> Vec<&'static str> {
    let mut features = vec!["metrics"];
    
    #[cfg(feature = "persistent")]
    features.push("persistent_storage");
    
    #[cfg(feature = "persistent")]
    features.push("recovery");
    
    #[cfg(feature = "persistent")]
    features.push("wal");
    
    features
}

fn get_memory_usage_mb() -> u64 {
    // Simple memory usage estimate
    // In a real implementation, you'd use a proper memory tracking library
    0
}

/// Metrics server status information
#[derive(Debug, serde::Serialize)]
struct MetricsStatus {
    uptime_seconds: u64,
    total_requests: u64,
    active_leases: u64,
    memory_usage_mb: u64,
    features: Vec<&'static str>,
}

// Extended Metrics implementation for enhanced Prometheus export
impl Metrics {
    // Background task metrics
    pub fn record_wal_compaction_success(&self) {
        // Implementation would track WAL compaction metrics
        tracing::debug!("WAL compaction success recorded");
    }

    pub fn record_wal_compaction_failure(&self) {
        // Implementation would track WAL compaction failures
        tracing::debug!("WAL compaction failure recorded");
    }

    pub fn record_snapshot_success(&self) {
        // Implementation would track snapshot creation metrics
        tracing::debug!("Snapshot creation success recorded");
    }

    pub fn record_snapshot_failure(&self) {
        // Implementation would track snapshot creation failures
        tracing::debug!("Snapshot creation failure recorded");
    }

    pub fn record_health_check_success(&self, component: &str) {
        // Implementation would track health check metrics
        tracing::debug!("Health check success for {}", component);
    }

    pub fn record_health_check_failure(&self, component: &str) {
        // Implementation would track health check failures
        tracing::debug!("Health check failure for {}", component);
    }

    pub fn update_storage_stats(&self, _stats: &crate::lease::LeaseStats) {
        // Implementation would update storage metrics
        tracing::debug!("Storage stats updated");
    }

    // Cleanup metrics (if not already implemented)
    pub fn increment_successful_cleanups(&self) {
        // Implementation would track successful cleanup metrics  
        tracing::debug!("Successful cleanup recorded");
    }

    // Recovery metrics (for persistent feature)
    #[cfg(feature = "persistent")]
    pub fn record_recovery_success(&self, leases_recovered: usize, duration: std::time::Duration) {
        // Implementation would track recovery metrics
        tracing::debug!("Recovery success: {} leases in {:.2}s", leases_recovered, duration.as_secs_f64());
    }

    #[cfg(feature = "persistent")]
    pub fn record_recovery_failure(&self) {
        // Implementation would track recovery failures
        tracing::debug!("Recovery failure recorded");
    }

    #[cfg(feature = "persistent")]
    pub fn record_recovery_progress(&self, progress: f64, errors: usize, warnings: usize) {
        // Implementation would track ongoing recovery progress
        tracing::debug!("Recovery progress: {:.1}%, {} errors, {} warnings", progress, errors, warnings);
    }

    // Enhanced metrics export methods for metrics server
    pub fn export_enhanced_prometheus_metrics(&self, config: &crate::config::Config) -> String {
        let mut output = String::new();

        // Basic metrics
        output.push_str(&format!(
            "# HELP garbagetruck_leases_total Total number of leases created\n\
             # TYPE garbagetruck_leases_total counter\n\
             garbagetruck_leases_total {}\n\
             \n\
             # HELP garbagetruck_cleanups_total Total number of cleanup operations\n\
             # TYPE garbagetruck_cleanups_total counter\n\
             garbagetruck_cleanups_total {}\n\
             \n\
             # HELP garbagetruck_active_leases Current number of active leases\n\
             # TYPE garbagetruck_active_leases gauge\n\
             garbagetruck_active_leases {}\n\
             \n",
            self.get_total_leases(),
            self.get_total_cleanups(),
            self.get_active_lease_count()
        ));

        // gRPC metrics
        output.push_str(&format!(
            "# HELP garbagetruck_grpc_requests_total Total number of gRPC requests\n\
             # TYPE garbagetruck_grpc_requests_total counter\n\
             garbagetruck_grpc_requests_total {}\n\
             \n\
             # HELP garbagetruck_grpc_request_duration_seconds gRPC request duration\n\
             # TYPE garbagetruck_grpc_request_duration_seconds histogram\n\
             garbagetruck_grpc_request_duration_seconds_sum {}\n\
             garbagetruck_grpc_request_duration_seconds_count {}\n\
             \n",
            self.get_total_requests(),
            self.get_total_request_duration_seconds(),
            self.get_total_requests()
        ));

        // Recovery metrics (if enabled)
        if config.metrics.include_recovery_metrics {
            output.push_str(&format!(
                "# HELP garbagetruck_recovery_operations_total Total number of recovery operations\n\
                 # TYPE garbagetruck_recovery_operations_total counter\n\
                 garbagetruck_recovery_operations_total {}\n\
                 \n\
                 # HELP garbagetruck_recovery_success_total Successful recovery operations\n\
                 # TYPE garbagetruck_recovery_success_total counter\n\
                 garbagetruck_recovery_success_total {}\n\
                 \n",
                self.get_recovery_operations_total(),
                self.get_recovery_success_total()
            ));
        }

        // WAL metrics (if enabled)
        if config.metrics.include_wal_metrics {
            output.push_str(&format!(
                "# HELP garbagetruck_wal_entries_total Total number of WAL entries written\n\
                 # TYPE garbagetruck_wal_entries_total counter\n\
                 garbagetruck_wal_entries_total {}\n\
                 \n\
                 # HELP garbagetruck_snapshots_total Total number of snapshots created\n\
                 # TYPE garbagetruck_snapshots_total counter\n\
                 garbagetruck_snapshots_total {}\n\
                 \n\
                 # HELP garbagetruck_wal_compactions_total Total number of WAL compactions\n\
                 # TYPE garbagetruck_wal_compactions_total counter\n\
                 garbagetruck_wal_compactions_total {}\n\
                 \n",
                self.get_wal_entries_total(),
                self.get_snapshots_total(),
                self.get_wal_compactions_total()
            ));
        }

        // System metrics
        output.push_str(&format!(
            "# HELP garbagetruck_uptime_seconds Service uptime in seconds\n\
             # TYPE garbagetruck_uptime_seconds counter\n\
             garbagetruck_uptime_seconds {}\n\
             \n\
             # HELP garbagetruck_build_info Build information\n\
             # TYPE garbagetruck_build_info gauge\n\
             garbagetruck_build_info{{version=\"{}\"}} 1\n",
            self.get_uptime_seconds(),
            env!("CARGO_PKG_VERSION")
        ));

        output
    }

    // Placeholder getter methods for metrics data access
    // Replace these with your actual metric counter access
    
    pub fn get_total_leases(&self) -> u64 { 
        self.leases_created_total.get() as u64
    }
    
    pub fn get_total_cleanups(&self) -> u64 { 
        self.cleanup_operations_total.get() as u64
    }
    
    pub fn get_active_lease_count(&self) -> u64 { 
        // This should be calculated from your current lease tracking
        0
    }
    
    pub fn get_total_requests(&self) -> u64 { 
        // Sum of all gRPC request counters
        0
    }
    
    pub fn get_total_request_duration_seconds(&self) -> f64 { 
        // Sum of all request durations
        0.0
    }
    
    pub fn get_recovery_operations_total(&self) -> u64 { 0 }
    pub fn get_recovery_success_total(&self) -> u64 { 0 }
    pub fn get_wal_entries_total(&self) -> u64 { 0 }
    pub fn get_snapshots_total(&self) -> u64 { 0 }
    pub fn get_wal_compactions_total(&self) -> u64 { 0 }
    
    pub fn get_uptime_seconds(&self) -> u64 { 
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_server_creation() {
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();

        let server = MetricsServer::new(config, metrics);
        // Test passes if we can create the server without panicking
        assert!(true);
    }

    #[test]
    fn test_enabled_features() {
        let features = get_enabled_features();
        assert!(features.contains(&"metrics"));
        
        #[cfg(feature = "persistent")]
        {
            assert!(features.contains(&"persistent_storage"));
            assert!(features.contains(&"recovery"));
            assert!(features.contains(&"wal"));
        }
    }

    #[tokio::test]
    async fn test_metrics_export() {
        let config = Config::default();
        let metrics = Metrics::new();

        let output = metrics.export_enhanced_prometheus_metrics(&config);
        assert!(output.contains("garbagetruck_leases_total"));
        assert!(output.contains("garbagetruck_active_leases"));
        assert!(output.contains("garbagetruck_uptime_seconds"));
    }

    #[tokio::test]
    async fn test_metrics_server_disabled() {
        let mut config = Config::default();
        config.metrics.enabled = false;
        
        let config = Arc::new(config);
        let metrics = Metrics::new();

        let server = MetricsServer::new(config, metrics);
        let result = server.start().await;
        
        assert!(result.is_ok());
    }
}