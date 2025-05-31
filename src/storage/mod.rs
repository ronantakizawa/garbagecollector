// src/storage/mod.rs - Storage trait and factory

use async_trait::async_trait;
use std::sync::Arc;

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseFilter, LeaseStats};

// Re-export submodules
pub mod memory;
pub mod postgres;

// Re-export implementations
pub use memory::MemoryStorage;
#[cfg(feature = "postgres")]
pub use postgres::PostgresStorage;

/// Storage trait defining the interface for lease persistence
#[async_trait]
pub trait Storage: Send + Sync {
    /// Create a new lease in storage
    async fn create_lease(&self, lease: Lease) -> Result<()>;
    
    /// Retrieve a lease by ID
    async fn get_lease(&self, lease_id: &str) -> Result<Option<Lease>>;
    
    /// Update an existing lease
    async fn update_lease(&self, lease: Lease) -> Result<()>;
    
    /// Delete a lease by ID
    async fn delete_lease(&self, lease_id: &str) -> Result<()>;
    
    /// List leases with optional filtering, limit, and offset
    async fn list_leases(&self, filter: LeaseFilter, limit: Option<usize>, offset: Option<usize>) -> Result<Vec<Lease>>;
    
    /// Count leases matching the given filter
    async fn count_leases(&self, filter: LeaseFilter) -> Result<usize>;
    
    /// Get expired leases that are ready for cleanup (past grace period)
    async fn get_expired_leases(&self, grace_period: std::time::Duration) -> Result<Vec<Lease>>;
    
    /// Get storage statistics and metrics
    async fn get_stats(&self) -> Result<LeaseStats>;
    
    /// Perform storage cleanup operations (remove old expired leases, etc.)
    async fn cleanup(&self) -> Result<()>;
}

/// Factory function to create storage implementations based on configuration
pub async fn create_storage(config: &Config) -> Result<Arc<dyn Storage + Send + Sync>> {
    match config.storage.backend.as_str() {
        "memory" => {
            tracing::info!("Creating memory storage backend");
            Ok(Arc::new(MemoryStorage::new()))
        }
        #[cfg(feature = "postgres")]
        "postgres" => {
            let database_url = config.storage.database_url
                .as_ref()
                .ok_or_else(|| GCError::Configuration("Database URL required for postgres backend".to_string()))?;
            let max_connections = config.storage.max_connections.unwrap_or(10);
            
            tracing::info!("Creating PostgreSQL storage backend with {} max connections", max_connections);
            let storage = PostgresStorage::new(database_url, max_connections).await?;
            Ok(Arc::new(storage))
        }
        #[cfg(not(feature = "postgres"))]
        "postgres" => {
            Err(GCError::Configuration("Postgres support not compiled in. Enable 'postgres' feature.".to_string()))
        }
        backend => {
            Err(GCError::Configuration(format!("Unknown storage backend: {}", backend)))
        }
    }
}

/// Storage backend capabilities and metadata
#[derive(Debug, Clone)]
pub struct StorageInfo {
    pub backend_type: String,
    pub supports_transactions: bool,
    pub supports_indexes: bool,
    pub estimated_capacity: Option<usize>,
    pub connection_pool_size: Option<usize>,
}

impl StorageInfo {
    /// Get storage info for memory backend
    pub fn memory() -> Self {
        Self {
            backend_type: "memory".to_string(),
            supports_transactions: false,
            supports_indexes: false,
            estimated_capacity: Some(1_000_000), // Rough estimate
            connection_pool_size: None,
        }
    }
    
    /// Get storage info for PostgreSQL backend
    #[cfg(feature = "postgres")]
    pub fn postgres(pool_size: u32) -> Self {
        Self {
            backend_type: "postgres".to_string(),
            supports_transactions: true,
            supports_indexes: true,
            estimated_capacity: None, // Depends on database configuration
            connection_pool_size: Some(pool_size as usize),
        }
    }
}

/// Extended storage trait for implementations that support additional features
#[async_trait]
pub trait ExtendedStorage: Storage {
    /// Get storage backend information
    async fn get_info(&self) -> Result<StorageInfo>;
    
    /// Perform database migrations (for SQL backends)
    async fn migrate(&self) -> Result<()>;
    
    /// Create database indexes for better performance
    async fn create_indexes(&self) -> Result<()>;
    
    /// Get detailed storage metrics
    async fn get_detailed_stats(&self) -> Result<DetailedStorageStats>;
}

/// Detailed storage statistics
#[derive(Debug, Clone)]
pub struct DetailedStorageStats {
    pub total_storage_size_bytes: Option<u64>,
    pub index_size_bytes: Option<u64>,
    pub connection_pool_stats: Option<ConnectionPoolStats>,
    pub query_performance: Option<QueryPerformanceStats>,
}

/// Connection pool statistics
#[derive(Debug, Clone)]
pub struct ConnectionPoolStats {
    pub active_connections: usize,
    pub idle_connections: usize,
    pub max_connections: usize,
    pub total_acquired: u64,
    pub total_creation_time_ms: u64,
}

/// Query performance statistics
#[derive(Debug, Clone)]
pub struct QueryPerformanceStats {
    pub average_query_time_ms: f64,
    pub slowest_query_time_ms: u64,
    pub total_queries: u64,
    pub failed_queries: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    
    #[tokio::test]
    async fn test_create_memory_storage() {
        let mut config = Config::default();
        config.storage.backend = "memory".to_string();
        
        let storage = create_storage(&config).await.unwrap();
        
        // Verify we can use the storage
        let stats = storage.get_stats().await.unwrap();
        assert_eq!(stats.total_leases, 0);
    }
    
    #[test]
    fn test_unknown_storage_backend() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let mut config = Config::default();
            config.storage.backend = "unknown".to_string();
            
            let result = create_storage(&config).await;
            assert!(result.is_err());
            match result {
                Err(e) => {
                    assert!(e.to_string().contains("Unknown storage backend"), 
                            "Error should mention unknown backend: {}", e);
                }
                Ok(_) => panic!("Expected error for unknown storage backend"),
            }
        });
    }
    
    #[cfg(feature = "postgres")]
    #[tokio::test]
    async fn test_postgres_without_url() {
        let mut config = Config::default();
        config.storage.backend = "postgres".to_string();
        config.storage.database_url = None;
        
        let result = create_storage(&config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Database URL required"));
    }
}