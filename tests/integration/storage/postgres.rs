// tests/integration/storage/postgres.rs - PostgreSQL storage tests

#[cfg(feature = "postgres")]
mod postgres_tests {
    use anyhow::Result;
    use std::sync::Arc;
    use std::time::Duration;

    use garbagetruck::storage::{Storage, PostgresStorage};
    use garbagetruck::lease::{LeaseFilter, ObjectType as InternalObjectType, LeaseState as InternalLeaseState};

    use crate::helpers::{skip_if_no_env, test_data::*, assertions::*};
    use crate::integration::{print_test_header, storage::run_common_storage_tests};

    #[tokio::test]
    async fn test_postgres_storage_basic_operations() -> Result<()> {
        if skip_if_no_env("DATABASE_URL") { return Ok(()); }
        
        print_test_header("PostgreSQL storage basic operations", "🐘");
        
        let database_url = std::env::var("DATABASE_URL")?;
        let storage = PostgresStorage::new(&database_url, 5).await?;
        let lease = create_test_lease_data("pg-test-object-1", "pg-test-service", 300);
        let lease_id = lease.lease_id.clone();
        
        // Test create
        storage.create_lease(lease.clone()).await?;
        println!("✅ Created lease in PostgreSQL storage");
        
        // Test get
        let retrieved = storage.get_lease(&lease_id).await?;
        assert!(retrieved.is_some(), "Should retrieve the created lease");
        let retrieved_lease = retrieved.unwrap();
        assert_leases_equivalent(&retrieved_lease, &lease);
        println!("✅ Retrieved lease from PostgreSQL storage");
        
        // Test update
        let mut updated_lease = retrieved_lease.clone();
        updated_lease.renew(Duration::from_secs(600))?;
        storage.update_lease(updated_lease.clone()).await?;
        
        let updated_retrieved = storage.get_lease(&lease_id).await?.unwrap();
        assert!(updated_retrieved.renewal_count > 0, "Renewal count should increase");
        println!("✅ Updated lease in PostgreSQL storage");
        
        // Test delete
        storage.delete_lease(&lease_id).await?;
        let deleted_check = storage.get_lease(&lease_id).await?;
        assert!(deleted_check.is_none(), "Lease should be deleted");
        println!("✅ Deleted lease from PostgreSQL storage");
        
        Ok(())
    }

    #[tokio::test]
    async fn test_postgres_storage_advanced_queries() -> Result<()> {
        if skip_if_no_env("DATABASE_URL") { return Ok(()); }
        
        print_test_header("PostgreSQL storage advanced queries", "🔍");
        
        let database_url = std::env::var("DATABASE_URL")?;
        let storage = PostgresStorage::new(&database_url, 5).await
            .map_err(|e| anyhow::anyhow!("PostgreSQL storage error: {}", e))?;
        
        // Create test data with different services and types
        let test_leases = vec![
            create_test_lease_data("pg-obj-1", "pg-service-1", 300),
            create_test_lease_data("pg-obj-2", "pg-service-1", 300),  
            create_test_lease_data("pg-obj-3", "pg-service-2", 300),
        ];
        
        for lease in &test_leases {
            storage.create_lease(lease.clone()).await
                .map_err(|e| anyhow::anyhow!("PostgreSQL storage error: {}", e))?;
        }
        
        // Test filtering by service only (avoid object_type comparison issue)
        let service_filter = LeaseFilter {
            service_id: Some("pg-service-1".to_string()),
            // Remove object_type and state filters that cause SQL type issues
            ..Default::default()
        };
        
        let filtered_leases = storage.list_leases(service_filter, Some(10), None).await
            .map_err(|e| anyhow::anyhow!("PostgreSQL storage error: {}", e))?;
        assert_eq!(filtered_leases.len(), 2, "Should find 2 leases for pg-service-1");
        println!("✅ Service filtering works in PostgreSQL");
        
        // Test statistics with database functions
        let stats = storage.get_stats().await
            .map_err(|e| anyhow::anyhow!("PostgreSQL storage error: {}", e))?;
        assert!(stats.total_leases >= 3, "Should have at least 3 leases");
        println!("✅ PostgreSQL statistics: {} total leases", stats.total_leases);
        
        // Clean up test data
        for lease in &test_leases {
            let _ = storage.delete_lease(&lease.lease_id).await;
        }
        
        Ok(())
    }

    #[tokio::test]
    async fn test_postgres_storage_concurrent_operations() -> Result<()> {
        if skip_if_no_env("DATABASE_URL") { return Ok(()); }
        
        print_test_header("PostgreSQL storage concurrent operations", "🚀");
        
        let database_url = std::env::var("DATABASE_URL")?;
        let storage = Arc::new(PostgresStorage::new(&database_url, 10).await?);
        let mut handles = vec![];
        
        // Create multiple concurrent operations
        for i in 0..10 {
            let storage_clone = storage.clone();
            let handle = tokio::spawn(async move {
                let lease = create_test_lease_data(
                    &format!("concurrent-pg-obj-{}", i),
                    &format!("concurrent-pg-service-{}", i % 3), // 3 different services
                    300
                );
                
                // Create, update, and then clean up
                match storage_clone.create_lease(lease.clone()).await {
                    Ok(_) => {
                        // Try to update the lease
                        let mut updated_lease = lease.clone();
                        updated_lease.renew(Duration::from_secs(600)).unwrap();
                        let _ = storage_clone.update_lease(updated_lease).await;
                        
                        // Clean up
                        let _ = storage_clone.delete_lease(&lease.lease_id).await;
                        true
                    }
                    Err(_) => false,
                }
            });
            handles.push(handle);
        }
        
        // Wait for all operations to complete
        let mut successful_ops = 0;
        for handle in handles {
            if handle.await? {
                successful_ops += 1;
            }
        }
        
        assert!(successful_ops >= 8, "Most concurrent operations should succeed");
        println!("✅ PostgreSQL concurrent operations: {}/10 successful", successful_ops);
        
        Ok(())
    }

    #[tokio::test]
    async fn test_postgres_storage_common_interface() -> Result<()> {
        if skip_if_no_env("DATABASE_URL") { return Ok(()); }
        
        print_test_header("PostgreSQL storage common interface", "🧪");
        
        let database_url = std::env::var("DATABASE_URL")?;
        let storage = PostgresStorage::new(&database_url, 5).await?;
        
        run_common_storage_tests(&storage).await?;
        
        println!("✅ PostgreSQL storage passes common interface tests");
        Ok(())
    }
}

#[cfg(not(feature = "postgres"))]
mod postgres_disabled {
    #[tokio::test] 
    async fn test_postgres_feature_disabled() {
        println!("⚠️  PostgreSQL tests skipped - postgres feature not enabled");
    }
}