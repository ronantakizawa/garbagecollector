// tests/integration/storage/mod.rs - Complete fixed version

use anyhow::Result;
use garbagetruck::{
    config::Config,
    storage::{create_storage, Storage},
};

pub mod factory;
pub mod memory;

/// Helper function to create a test storage instance with memory backend
pub async fn create_test_storage() -> Result<std::sync::Arc<dyn Storage>> {
    let mut config = Config::default();
    config.storage.backend = "memory".to_string();
    
    create_storage(&config)
        .await
        .map_err(|e| anyhow::anyhow!("Storage creation failed: {}", e))
}

/// Helper function to create test storage with custom config
pub async fn create_test_storage_with_config(
    config: Config,
) -> Result<std::sync::Arc<dyn Storage>> {
    create_storage(&config)
        .await
        .map_err(|e| anyhow::anyhow!("Storage creation failed: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_test_storage() {
        let storage = create_test_storage().await.unwrap();
        let healthy = storage.health_check().await.unwrap();
        assert!(healthy);
    }

    #[tokio::test]
    async fn test_create_test_storage_with_config() {
        let mut config = Config::default();
        config.storage.backend = "memory".to_string();
        
        let storage = create_test_storage_with_config(config).await.unwrap();
        let healthy = storage.health_check().await.unwrap();
        assert!(healthy);
    }
}