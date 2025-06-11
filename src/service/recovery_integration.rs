// src/service/recovery_integration.rs - Recovery system integration

#[cfg(feature = "persistent")]
use std::sync::Arc;
#[cfg(feature = "persistent")]
use tracing::{info, warn};

#[cfg(feature = "persistent")]
use crate::error::Result;
#[cfg(feature = "persistent")]
use crate::metrics::Metrics;
#[cfg(feature = "persistent")]
use crate::recovery::{RecoveryConfig, RecoveryManager, RecoveryAPI, RecoveryTrigger, RecoveryProgress};
#[cfg(feature = "persistent")]
use crate::storage::{PersistentStorage, Storage};

/// Integration layer for recovery functionality with the main service
#[cfg(feature = "persistent")]
pub struct RecoveryIntegration {
    recovery_manager: Arc<RecoveryManager>,
    metrics: Arc<Metrics>,
}

#[cfg(feature = "persistent")]
impl RecoveryIntegration {
    pub fn new(
        persistent_storage: Arc<dyn PersistentStorage>,
        storage_interface: Arc<dyn Storage>,
        recovery_config: RecoveryConfig,
        metrics: Arc<Metrics>,
    ) -> Self {
        let recovery_manager = Arc::new(RecoveryManager::new(
            persistent_storage,
            storage_interface,
            recovery_config,
            metrics.clone(),
        ));

        Self {
            recovery_manager,
            metrics,
        }
    }

    /// Perform startup recovery if enabled
    pub async fn perform_startup_recovery(&self) -> Result<()> {
        info!("🔄 Performing startup recovery");
        
        let recovery_result = self.recovery_manager.recover(RecoveryTrigger::ServiceStart).await?;
        
        if recovery_result.success {
            info!(
                "✅ Startup recovery completed: {} leases recovered in {:.2}s",
                recovery_result.recovery_info.leases_recovered,
                recovery_result.total_time.as_secs_f64()
            );
            
            // Update metrics with recovery information
            self.metrics.record_recovery_success(
                recovery_result.recovery_info.leases_recovered,
                recovery_result.total_time,
            );
        } else {
            warn!(
                "⚠️ Startup recovery had issues: {}",
                recovery_result.error.unwrap_or_else(|| "Unknown error".to_string())
            );
            
            self.metrics.record_recovery_failure();
        }

        Ok(())
    }

    /// Get recovery API for external control
    pub fn get_api(&self) -> RecoveryAPI {
        RecoveryAPI::new(self.recovery_manager.clone())
    }

    /// Get current recovery status
    pub async fn get_status(&self) -> Option<RecoveryProgress> {
        self.recovery_manager.get_recovery_progress().await
    }

    /// Check if recovery is currently in progress
    pub async fn is_recovery_in_progress(&self) -> bool {
        self.recovery_manager.is_recovery_in_progress().await
    }

    /// Trigger manual recovery
    pub async fn trigger_manual_recovery(&self) -> Result<crate::recovery::RecoveryResult> {
        info!("🔄 Triggering manual recovery");
        self.recovery_manager.recover(RecoveryTrigger::Manual).await
    }

    /// Trigger emergency recovery
    pub async fn trigger_emergency_recovery(&self) -> Result<crate::recovery::RecoveryResult> {
        warn!("🚨 Triggering emergency recovery");
        self.recovery_manager.recover(RecoveryTrigger::Manual).await // Use manual trigger for emergency
    }

    /// Get recovery manager for advanced operations
    pub fn recovery_manager(&self) -> Arc<RecoveryManager> {
        self.recovery_manager.clone()
    }
}

// Extended Metrics implementation for recovery features
#[cfg(feature = "persistent")]

// Provide empty implementations when persistent features are disabled
#[cfg(not(feature = "persistent"))]
pub struct RecoveryIntegration;

#[cfg(not(feature = "persistent"))]
impl RecoveryIntegration {
    pub fn new(
        _persistent_storage: (),
        _storage_interface: (),
        _recovery_config: (),
        _metrics: (),
    ) -> Self {
        Self
    }

    pub async fn perform_startup_recovery(&self) -> Result<()> {
        tracing::debug!("Persistent storage features disabled, skipping recovery");
        Ok(())
    }

    pub fn get_api(&self) -> () {
        ()
    }

    pub async fn get_status(&self) -> Option<()> {
        None
    }

    pub async fn is_recovery_in_progress(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[cfg(feature = "persistent")]
    mod persistent_tests {
        use super::*;
        use crate::storage::MemoryStorage;
        use crate::storage::file_persistent::FilePersistentStorage;
        use tempfile::TempDir;

        async fn create_test_recovery_integration() -> (RecoveryIntegration, TempDir) {
            let temp_dir = TempDir::new().unwrap();
            let storage_interface = Arc::new(MemoryStorage::new()) as Arc<dyn Storage>;
            let persistent_storage = Arc::new(FilePersistentStorage::new()) as Arc<dyn PersistentStorage>;
            
            let config = RecoveryConfig::default();
            let metrics = Metrics::new();

            let integration = RecoveryIntegration::new(
                persistent_storage,
                storage_interface,
                config,
                metrics,
            );

            (integration, temp_dir)
        }

        #[tokio::test]
        async fn test_recovery_integration_creation() {
            let (integration, _temp_dir) = create_test_recovery_integration().await;
            assert!(!integration.is_recovery_in_progress().await);
        }

        #[tokio::test]
        async fn test_recovery_api_access() {
            let (integration, _temp_dir) = create_test_recovery_integration().await;
            let _api = integration.get_api();
            // API creation should not panic
        }

        #[tokio::test]
        async fn test_recovery_status() {
            let (integration, _temp_dir) = create_test_recovery_integration().await;
            let status = integration.get_status().await;
            // Should return None when no recovery is in progress
            assert!(status.is_none());
        }
    }

    #[cfg(not(feature = "persistent"))]
    mod non_persistent_tests {
        use super::*;

        #[tokio::test]
        async fn test_recovery_integration_disabled() {
            let integration = RecoveryIntegration::new((), (), (), ());
            assert!(!integration.is_recovery_in_progress().await);
            assert!(integration.get_status().await.is_none());
            
            let result = integration.perform_startup_recovery().await;
            assert!(result.is_ok());
        }
    }
}