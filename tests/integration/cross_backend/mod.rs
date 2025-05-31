// tests/integration/cross_backend/mod.rs - Cross-backend consistency tests

pub mod consistency;

use anyhow::Result;
use garbagetruck::storage::{Storage, MemoryStorage};

#[cfg(feature = "postgres")]
use garbagetruck::storage::PostgresStorage;

use crate::helpers::{skip_if_no_env, test_data::create_test_lease_data};
use crate::integration::print_test_header;

#[cfg(feature = "postgres")]
#[tokio::test]
async fn test_storage_backend_consistency() -> Result<()> {
    print_test_header("storage backend consistency", "🔄");
    
    // Test data that should behave the same across backends
    let test_lease = create_test_lease_data("consistency-test", "consistency-service", 300);
    
    // Test memory storage
    let memory_storage = MemoryStorage::new();
    memory_storage.create_lease(test_lease.clone()).await
        .map_err(|e| anyhow::anyhow!("Memory storage error: {}", e))?;
    
    let memory_result = memory_storage.get_lease(&test_lease.lease_id).await
        .map_err(|e| anyhow::anyhow!("Memory storage error: {}", e))?;
    assert!(memory_result.is_some(), "Memory storage should store and retrieve lease");
    
    let memory_stats = memory_storage.get_stats().await
        .map_err(|e| anyhow::anyhow!("Memory storage error: {}", e))?;
    assert_eq!(memory_stats.total_leases, 1, "Memory storage should count 1 lease");
    
    println!("✅ Memory storage consistency verified");
    
    // Test PostgreSQL storage (if available)
    if !skip_if_no_env("DATABASE_URL") {
        let database_url = std::env::var("DATABASE_URL")?;
        let postgres_storage = PostgresStorage::new(&database_url, 5).await
            .map_err(|e| anyhow::anyhow!("PostgreSQL storage error: {}", e))?;
        
        let pg_test_lease = create_test_lease_data("pg-consistency-test", "pg-consistency-service", 300);
        postgres_storage.create_lease(pg_test_lease.clone()).await
            .map_err(|e| anyhow::anyhow!("PostgreSQL storage error: {}", e))?;
        
        let postgres_result = postgres_storage.get_lease(&pg_test_lease.lease_id).await
            .map_err(|e| anyhow::anyhow!("PostgreSQL storage error: {}", e))?;
        assert!(postgres_result.is_some(), "PostgreSQL storage should store and retrieve lease");
        
        let postgres_stats = postgres_storage.get_stats().await
            .map_err(|e| anyhow::anyhow!("PostgreSQL storage error: {}", e))?;
        assert!(postgres_stats.total_leases >= 1, "PostgreSQL storage should count leases");
        
        // Clean up
        let _ = postgres_storage.delete_lease(&pg_test_lease.lease_id).await;
        
        println!("✅ PostgreSQL storage consistency verified");
    }
    
    Ok(())
}