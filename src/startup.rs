// src/startup.rs - Complete fixed version

use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::dependencies::DependencyChecker;
use crate::error::{GCError, Result};
use crate::metrics::Metrics;
use crate::service::GCService;
use crate::storage::{create_storage, Storage};

/// Handles application startup and component initialization
pub struct ApplicationStartup {
    config: Arc<Config>,
    metrics: Arc<Metrics>,
}

impl ApplicationStartup {
    /// Create a new application startup handler
    pub fn new(config: Config) -> Result<Self> {
        let config = Arc::new(config);
        let metrics = Metrics::new();

        Ok(Self { config, metrics })
    }

    /// Start the complete application
    pub async fn start(&self) -> Result<()> {
        info!("🚀 Starting GarbageTruck application");

        // Check external dependencies
        self.check_dependencies().await?;

        // Initialize storage
        let storage = self.initialize_storage().await?;

        // Create and start the main service
        let gc_service = GCService::new(storage, self.config.clone(), self.metrics.clone());

        let addr: SocketAddr = format!("{}:{}", self.config.server.host, self.config.server.port)
            .parse()
            .map_err(|e| GCError::Configuration(format!("Invalid server address: {}", e)))?;

        info!("✅ Application startup completed successfully");

        // Start the service (this will block until shutdown)
        gc_service.start(addr).await
    }

    /// Check external dependencies
    async fn check_dependencies(&self) -> Result<()> {
        info!("🔍 Checking external dependencies");

        let dependency_checker = DependencyChecker::new(&self.config, &self.metrics);

        if let Err(e) = dependency_checker.check_all_dependencies().await {
            error!("❌ Dependency check failed: {}", e);
            return Err(GCError::Storage(e)); // FIXED: Proper error conversion
        }

        info!("✅ All dependencies are available");
        Ok(())
    }

    /// Initialize storage backend
    async fn initialize_storage(&self) -> Result<Arc<dyn Storage>> {
        info!(
            "💾 Initializing storage backend: {}",
            self.config.storage.backend
        );

        let storage = create_storage(&self.config).await?;

        // Run any necessary migrations or setup
        if let Err(e) = storage.cleanup().await {
            warn!("Storage cleanup during initialization failed: {}", e);
        }

        info!("✅ Storage backend initialized successfully");
        Ok(storage)
    }

    /// Get configuration
    pub fn config(&self) -> &Arc<Config> {
        &self.config
    }

    /// Get metrics
    pub fn metrics(&self) -> &Arc<Metrics> {
        &self.metrics
    }
}
