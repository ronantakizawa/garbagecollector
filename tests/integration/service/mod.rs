// tests/integration/service/mod.rs - Fixed test integration

use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::sleep;

use garbagetruck::storage::create_storage;
use garbagetruck::{Config, GCService, Metrics};

/// Test harness for service integration tests
pub struct ServiceTestHarness {
    pub config: Arc<Config>,
    pub metrics: Arc<Metrics>,
    pub temp_dir: TempDir,
    pub service: Option<GCService>,
}

impl ServiceTestHarness {
    /// Create a new test harness with in-memory storage
    pub async fn new() -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let temp_dir = tempfile::tempdir()?;

        // Create test configuration
        let mut config = Config::default();
        config.server.host = "127.0.0.1".to_string();
        config.server.port = 0; // Let OS choose port
        config.storage.backend = "memory".to_string();
        config.gc.cleanup_interval_seconds = 1; // Fast cleanup for testing
        config.gc.cleanup_grace_period_seconds = 1;

        let config = Arc::new(config);
        let metrics = Metrics::new();

        Ok(Self {
            config,
            metrics,
            temp_dir,
            service: None,
        })
    }

    /// Start the service in the background
    pub async fn start_service(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let storage = create_storage(&self.config).await?;

        let service = GCService::new(storage, self.config.clone(), self.metrics.clone());

        self.service = Some(service);

        // Give service time to start
        sleep(Duration::from_millis(100)).await;

        Ok(())
    }

    /// Get the service endpoint
    pub fn endpoint(&self) -> String {
        format!(
            "http://{}:{}",
            self.config.server.host, self.config.server.port
        )
    }

    /// Create a test client
    pub async fn create_client(
        &self,
    ) -> Result<garbagetruck::GCClient, Box<dyn std::error::Error + Send + Sync>> {
        let mut client =
            garbagetruck::GCClient::new(&self.endpoint(), "test-client".to_string()).await?;

        // Test connectivity with a quick health check
        for _attempt in 0..10 {
            if client.health_check().await.is_ok() {
                return Ok(client);
            }
            sleep(Duration::from_millis(100)).await;
        }

        Err("Failed to connect to test service after 10 attempts".into())
    }

    /// Get metrics snapshot
    pub fn get_metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }

    /// Stop the service
    pub async fn stop_service(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(service) = &self.service {
            service.shutdown().await;
        }
        self.service = None;
        Ok(())
    }
}

impl Drop for ServiceTestHarness {
    fn drop(&mut self) {
        // Cleanup happens automatically with TempDir
    }
}
