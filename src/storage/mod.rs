// src/storage/mod.rs - Storage module with proper exports and syntax

use async_trait::async_trait;
use std::sync::Arc;

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseFilter, LeaseStats};

// Import storage implementations
pub mod memory;

#[cfg(feature = "postgres")]
pub mod postgres;

// Re-export the implementations
pub use memory::MemoryStorage;

#[cfg(feature = "postgres")]
pub use postgres::PostgresStorage;

/// Core storage trait that all storage backends must implement
#[async_trait]
pub trait Storage: Send + Sync {
    async fn create_lease(&self, lease: Lease) -> Result<()>;
    async fn get_lease(&self, lease_id: &str) -> Result<Option<Lease>>;
    async fn update_lease(&self, lease: Lease) -> Result<()>;
    async fn delete_lease(&self, lease_id: &str) -> Result<()>;
    async fn list_leases(
        &self,
        filter: LeaseFilter,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<Lease>>;
    async fn count_leases(&self, filter: LeaseFilter) -> Result<usize>;
    async fn get_expired_leases(&self, grace_period: std::time::Duration) -> Result<Vec<Lease>>;
    async fn get_stats(&self) -> Result<LeaseStats>;
    async fn cleanup(&self) -> Result<()>;
}

/// Extended storage trait for additional features (optional)
#[async_trait]
pub trait ExtendedStorage: Storage {
    async fn get_info(&self) -> Result<StorageInfo>;
    async fn migrate(&self) -> Result<()>;
    async fn create_indexes(&self) -> Result<()>;
    async fn get_detailed_stats(&self) -> Result<DetailedStorageStats>;
}

/// Storage backend information
#[derive(Debug, Clone)]
pub struct StorageInfo {
    pub backend_type: String,
    pub version: String,
    pub supports_transactions: bool,
    pub supports_indexes: bool,
    pub estimated_size_bytes: Option<u64>,
}

impl StorageInfo {
    pub fn memory() -> Self {
        Self {
            backend_type: "memory".to_string(),
            version: "1.0".to_string(),
            supports_transactions: false,
            supports_indexes: false,
            estimated_size_bytes: None,
        }
    }

    #[cfg(feature = "postgres")]
    pub fn postgres(version: String) -> Self {
        Self {
            backend_type: "postgres".to_string(),
            version,
            supports_transactions: true,
            supports_indexes: true,
            estimated_size_bytes: None,
        }
    }
}

/// Detailed storage statistics
#[derive(Debug, Clone)]
pub struct DetailedStorageStats {
    pub total_storage_size_bytes: Option<u64>,
    pub index_size_bytes: Option<u64>,
    pub connection_pool_stats: Option<ConnectionPoolStats>,
    pub query_performance: Option<QueryPerformanceStats>,
}

#[derive(Debug, Clone)]
pub struct ConnectionPoolStats {
    pub total_connections: u32,
    pub active_connections: u32,
    pub idle_connections: u32,
    pub max_connections: u32,
}

#[derive(Debug, Clone)]
pub struct QueryPerformanceStats {
    pub average_query_time_ms: f64,
    pub slowest_query_time_ms: u64,
    pub total_queries: u64,
    pub failed_queries: u64,
}

/// Factory function to create storage backends based on configuration
pub async fn create_storage(config: &Config) -> Result<Arc<dyn Storage + Send + Sync>> {
    match config.storage.backend.as_str() {
        "memory" => {
            let storage = MemoryStorage::new();
            Ok(Arc::new(storage))
        }
        #[cfg(feature = "postgres")]
        "postgres" => {
            let database_url = config.storage.database_url.as_ref().ok_or_else(|| {
                GCError::Configuration("Database URL required for postgres backend".to_string())
            })?;
            let max_connections = config.storage.max_connections.unwrap_or(10);

            let storage = PostgresStorage::new(database_url, max_connections)
                .await
                .map_err(|e| {
                    GCError::Configuration(format!("Failed to create PostgreSQL storage: {}", e))
                })?;
            Ok(Arc::new(storage))
        }
        #[cfg(not(feature = "postgres"))]
        "postgres" => Err(GCError::Configuration(
            "Postgres support not compiled in. Enable 'postgres' feature.".to_string(),
        )),
        backend => Err(GCError::Configuration(format!(
            "Unknown storage backend: {}",
            backend
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_memory_storage() {
        let mut config = Config::default();
        config.storage.backend = "memory".to_string();

        let result = create_storage(&config).await;
        assert!(result.is_ok(), "Should create memory storage successfully");

        let storage = result.unwrap();

        // Test basic functionality
        let lease = Lease::new(
            "test".to_string(),
            crate::lease::ObjectType::DatabaseRow,
            "service".to_string(),
            std::time::Duration::from_secs(300),
            std::collections::HashMap::new(),
            None,
        );
        let lease_id = lease.lease_id.clone();

        storage.create_lease(lease).await.unwrap();
        let retrieved = storage.get_lease(&lease_id).await.unwrap();
        assert!(retrieved.is_some(), "Should retrieve created lease");
    }

    #[tokio::test]
    async fn test_unknown_storage_backend() {
        let mut config = Config::default();
        config.storage.backend = "unknown-backend".to_string();

        let result = create_storage(&config).await;
        assert!(result.is_err(), "Should fail for unknown backend");

        match result {
            Err(e) => {
                assert!(
                    e.to_string().contains("Unknown storage backend"),
                    "Error should mention unknown backend: {}",
                    e
                );
            }
            Ok(_) => panic!("Expected error for unknown backend"),
        }
    }

    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn test_postgres_without_url() {
        let mut config = Config::default();
        config.storage.backend = "postgres".to_string();
        config.storage.database_url = None; // Missing URL

        let result = create_storage(&config).await;
        assert!(
            result.is_err(),
            "Should fail when postgres backend selected but no URL provided"
        );

        match result {
            Err(e) => {
                let error_msg = e.to_string();
                assert!(
                    error_msg.contains("Database URL required"),
                    "Error should mention missing URL: {}",
                    error_msg
                );
            }
            Ok(_) => panic!("Expected error for missing postgres URL"),
        }
    }

    #[cfg(not(feature = "postgres"))]
    #[tokio::test]
    async fn test_postgres_feature_disabled() {
        let mut config = Config::default();
        config.storage.backend = "postgres".to_string();

        let result = create_storage(&config).await;
        assert!(
            result.is_err(),
            "Should fail when postgres feature is disabled"
        );

        match result {
            Err(e) => {
                assert!(
                    e.to_string().contains("Postgres support not compiled in"),
                    "Error should mention missing feature: {}",
                    e
                );
            }
            Ok(_) => panic!("Expected error for disabled postgres feature"),
        }
    }
}
