// src/storage/mod.rs - Updated Storage trait with count_leases method

use async_trait::async_trait;
use std::sync::Arc;

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseFilter, LeaseStats, ObjectType};

mod memory;

pub use memory::MemoryStorage;

/// Main storage trait for lease persistence
#[async_trait]
pub trait Storage: Send + Sync {
    /// Create a new lease
    async fn create_lease(&self, lease: Lease) -> Result<()>;

    /// Get a lease by ID
    async fn get_lease(&self, lease_id: &str) -> Result<Option<Lease>>;

    /// Update an existing lease
    async fn update_lease(&self, lease: Lease) -> Result<()>;

    /// Delete a lease
    async fn delete_lease(&self, lease_id: &str) -> Result<()>;

    /// List leases with optional filtering
    async fn list_leases(
        &self,
        filter: Option<LeaseFilter>,
        limit: Option<usize>,
    ) -> Result<Vec<Lease>>;

    /// Count leases matching the given filter
    async fn count_leases(&self, filter: LeaseFilter) -> Result<usize>;

    /// Get expired leases that need cleanup
    async fn get_expired_leases(&self, grace_period: std::time::Duration) -> Result<Vec<Lease>>;

    /// Get leases by service ID
    async fn get_leases_by_service(&self, service_id: &str) -> Result<Vec<Lease>>;

    /// Get leases by object type
    async fn get_leases_by_type(&self, object_type: ObjectType) -> Result<Vec<Lease>>;

    /// Get lease statistics
    async fn get_stats(&self) -> Result<LeaseStats>;

    /// Health check
    async fn health_check(&self) -> Result<bool>;

    /// Cleanup expired leases (maintenance operation)
    async fn cleanup(&self) -> Result<usize>;

    /// Count active leases for a service (for rate limiting)
    async fn count_active_leases_for_service(&self, service_id: &str) -> Result<usize>;

    /// Mark lease as expired
    async fn mark_lease_expired(&self, lease_id: &str) -> Result<()>;

    /// Mark lease as released
    async fn mark_lease_released(&self, lease_id: &str) -> Result<()>;
}

/// Create storage backend based on configuration
pub async fn create_storage(config: &Config) -> Result<Arc<dyn Storage>> {
    match config.storage.backend.as_str() {
        "memory" => {
            tracing::info!("🧠 Creating in-memory storage backend");
            Ok(Arc::new(MemoryStorage::new()))
        }
        backend => {
            tracing::error!("❌ Unsupported storage backend: {}", backend);
            Err(GCError::Configuration(format!(
                "Unsupported storage backend: {}. Only 'memory' is supported.",
                backend
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lease::{LeaseFilter, ObjectType};
    use std::collections::HashMap;

    async fn test_storage_backend(storage: Arc<dyn Storage>) {
        // Test basic CRUD operations
        let lease = crate::lease::Lease::new(
            "test-object".to_string(),
            ObjectType::TemporaryFile,
            "test-service".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        let lease_id = lease.lease_id.clone();

        // Create lease
        storage.create_lease(lease.clone()).await.unwrap();

        // Get lease
        let retrieved = storage.get_lease(&lease_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().lease_id, lease_id);

        // List leases
        let leases = storage.list_leases(None, None).await.unwrap();
        assert_eq!(leases.len(), 1);

        // Count leases
        let filter = LeaseFilter::default();
        let count = storage.count_leases(filter).await.unwrap();
        assert_eq!(count, 1);

        // Delete lease
        storage.delete_lease(&lease_id).await.unwrap();

        // Verify deletion
        let deleted = storage.get_lease(&lease_id).await.unwrap();
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_memory_storage() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        test_storage_backend(storage).await;
    }

    #[tokio::test]
    async fn test_storage_factory() {
        let config = crate::config::Config {
            storage: crate::config::StorageConfig {
                backend: "memory".to_string(),
            },
            ..Default::default()
        };

        let storage = create_storage(&config).await.unwrap();
        test_storage_backend(storage).await;
    }

    #[tokio::test]
    async fn test_unsupported_backend() {
        let config = crate::config::Config {
            storage: crate::config::StorageConfig {
                backend: "postgres".to_string(),
            },
            ..Default::default()
        };

        let result = create_storage(&config).await;
        assert!(result.is_err());

        if let Err(GCError::Configuration(msg)) = result {
            assert!(msg.contains("Unsupported storage backend"));
        } else {
            panic!("Expected configuration error");
        }
    }
}
