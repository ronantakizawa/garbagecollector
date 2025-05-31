// tests/integration/service/mod.rs - Service-level integration tests

pub mod shutdown;
pub mod lifecycle;

use std::time::Duration;
use garbagetruck::{Config, GCService};
use garbagetruck::shutdown::{ShutdownCoordinator, ShutdownConfig, TaskType, TaskPriority};

/// Create a test GC service with minimal configuration
pub async fn create_test_service() -> anyhow::Result<GCService> {
    let mut config = Config::default();
    config.gc.cleanup_interval_seconds = 1; // Very short for testing
    config.gc.cleanup_grace_period_seconds = 1;
    config.storage.backend = "memory".to_string(); // Use memory storage for tests
    
    GCService::new(config).await
        .map_err(|e| anyhow::anyhow!("Service creation failed: {}", e))
}

/// Create a test shutdown coordinator with reasonable test timeouts
pub fn create_test_shutdown_coordinator() -> ShutdownCoordinator {
    let config = ShutdownConfig {
        graceful_timeout: Duration::from_secs(5),
        phase_delay: Duration::from_millis(100),
        force_kill_on_timeout: true,
        metrics_collection_timeout: Duration::from_secs(1),
    };
    
    ShutdownCoordinator::new(config)
}

/// Helper to simulate resource cleanup in tests
pub async fn simulate_resource_cleanup(task_name: &str, duration_ms: u64) {
    println!("🧹 {} starting resource cleanup...", task_name);
    tokio::time::sleep(Duration::from_millis(duration_ms)).await;
    println!("✅ {} completed resource cleanup", task_name);
}