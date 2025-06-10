// src/startup.rs - Fixed startup implementation

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

    /// Start the complete application and return the service
    pub async fn start(&self) -> Result<GCService> {
        info!("🚀 Starting GarbageTruck application");

        // Check external dependencies
        self.check_dependencies().await?;

        // Initialize storage
        let _storage = self.initialize_storage().await?;

        // Create the main service
        let gc_service = GCService::new(self.config.clone(), self.metrics.clone()).await?;

        info!("✅ Application startup completed successfully");

        // Return the service for external startup
        Ok(gc_service)
    }

    /// Start the application and run the service
    pub async fn start_and_run(&self, addr: SocketAddr) -> Result<()> {
        let gc_service = self.start().await?;
        gc_service.start(addr).await
    }

    /// Check external dependencies
    async fn check_dependencies(&self) -> Result<()> {
        info!("🔍 Checking external dependencies");

        let dependency_checker = DependencyChecker::new(&self.config, &self.metrics);

        if let Err(e) = dependency_checker.check_all_dependencies().await {
            error!("❌ Dependency check failed: {}", e);
            return Err(GCError::Storage(e));
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