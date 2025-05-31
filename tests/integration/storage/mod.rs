// tests/integration/storage/mod.rs - Storage test utilities

use anyhow::Result;
use garbagetruck::config::Config;
use garbagetruck::storage::{Storage, create_storage};

// Storage test modules
pub mod memory;

#[cfg(feature = "postgres")]
pub mod postgres;

pub mod factory;

/// Test storage factory functionality
pub async fn test_storage_creation(backend: &str) -> Result<std::sync::Arc<dyn Storage + Send + Sync>> {
    let mut config = Config::default();
    config.storage.backend = backend.to_string();
    
    #[cfg(feature = "postgres")]
    if backend == "postgres" {
        if let Ok(database_url) = std::env::var("DATABASE_URL") {
            config.storage.database_url = Some(database_url);
            config.storage.max_connections = Some(5);
        } else {
            return Err(anyhow::anyhow!("DATABASE_URL not set for postgres test"));
        }
    }
    
    create_storage(&config).await
        .map_err(|e| anyhow::anyhow!("Storage creation failed: {}", e))
}

/// Common storage interface tests that can be run against any storage backend
pub async fn run_common_storage_tests(storage: &dyn Storage) -> Result<()> {
    use crate::helpers::test_data::create_test_lease_data;
    use crate::helpers::assertions::assert_leases_equivalent;
    
    // Test basic CRUD operations
    let lease = create_test_lease_data("common-test", "common-service", 300);
    let lease_id = lease.lease_id.clone();
    
    // Create
    storage.create_lease(lease.clone()).await
        .map_err(|e| anyhow::anyhow!("Create failed: {}", e))?;
    
    // Read
    let retrieved = storage.get_lease(&lease_id).await
        .map_err(|e| anyhow::anyhow!("Get failed: {}", e))?;
    assert!(retrieved.is_some(), "Should retrieve the created lease");
    assert_leases_equivalent(&retrieved.unwrap(), &lease);
    
    // Update
    let mut updated_lease = lease.clone();
    updated_lease.renew(std::time::Duration::from_secs(600))
        .map_err(|e| anyhow::anyhow!("Renew failed: {}", e))?;
    storage.update_lease(updated_lease).await
        .map_err(|e| anyhow::anyhow!("Update failed: {}", e))?;
    
    // Delete
    storage.delete_lease(&lease_id).await
        .map_err(|e| anyhow::anyhow!("Delete failed: {}", e))?;
    let deleted = storage.get_lease(&lease_id).await
        .map_err(|e| anyhow::anyhow!("Get after delete failed: {}", e))?;
    assert!(deleted.is_none(), "Lease should be deleted");
    
    Ok(())
}