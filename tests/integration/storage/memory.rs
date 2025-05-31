// tests/integration/storage/memory.rs - Memory storage tests

use anyhow::Result;
use std::time::Duration;

use garbagetruck::storage::{Storage, MemoryStorage};
use garbagetruck::lease::{LeaseFilter, LeaseState};

use crate::helpers::test_data::*;
use crate::helpers::assertions::*;
use crate::integration::print_test_header;

#[tokio::test]
async fn test_memory_storage_basic_operations() -> Result<()> {
    print_test_header("memory storage basic operations", "💾");
    
    let storage = MemoryStorage::new();
    let lease = create_test_lease_data("test-object-1", "test-service", 300);
    let lease_id = lease.lease_id.clone();
    
    // Test create
    storage.create_lease(lease.clone()).await?;
    println!("✅ Created lease in memory storage");
    
    // Test get
    let retrieved = storage.get_lease(&lease_id).await?;
    assert!(retrieved.is_some(), "Should retrieve the created lease");
    let retrieved_lease = retrieved.unwrap();
    assert_leases_equivalent(&retrieved_lease, &lease);
    println!("✅ Retrieved lease from memory storage");
    
    // Test update
    let mut updated_lease = retrieved_lease.clone();
    updated_lease.renew(Duration::from_secs(600))?;
    storage.update_lease(updated_lease.clone()).await?;
    
    let updated_retrieved = storage.get_lease(&lease_id).await?.unwrap();
    assert!(updated_retrieved.renewal_count > 0, "Renewal count should increase");
    println!("✅ Updated lease in memory storage");
    
    // Test delete
    storage.delete_lease(&lease_id).await?;
    let deleted_check = storage.get_lease(&lease_id).await?;
    assert!(deleted_check.is_none(), "Lease should be deleted");
    println!("✅ Deleted lease from memory storage");
    
    Ok(())
}

#[tokio::test]
async fn test_memory_storage_filtering_and_listing() -> Result<()> {
    print_test_header("memory storage filtering and listing", "🔍");
    
    let storage = MemoryStorage::new();
    
    // Create multiple leases with different properties
    let leases = vec![
        create_test_lease_data("obj-1", "service-a", 300),
        create_test_lease_data("obj-2", "service-a", 300),
        create_test_lease_data("obj-3", "service-b", 300),
    ];
    
    for lease in &leases {
        storage.create_lease(lease.clone()).await?;
    }
    
    // Test listing all leases
    let all_leases = storage.list_leases(LeaseFilter::default(), None, None).await?;
    assert_eq!(all_leases.len(), 3, "Should have 3 total leases");
    println!("✅ Listed all leases");
    
    // Test filtering by service
    let service_filter = LeaseFilter {
        service_id: Some("service-a".to_string()),
        ..Default::default()
    };
    let service_leases = storage.list_leases(service_filter, None, None).await?;
    assert_eq!(service_leases.len(), 2, "Should have 2 leases for service-a");
    println!("✅ Filtered leases by service");
    
    // Test counting
    let count = storage.count_leases(LeaseFilter::default()).await?;
    assert_eq!(count, 3, "Should count 3 total leases");
    println!("✅ Counted leases");
    
    // Test pagination
    let paginated = storage.list_leases(LeaseFilter::default(), Some(2), Some(1)).await?;
    assert_eq!(paginated.len(), 2, "Should have 2 leases with limit");
    println!("✅ Paginated lease listing");
    
    Ok(())
}

#[tokio::test]
async fn test_memory_storage_expired_leases() -> Result<()> {
    print_test_header("memory storage expired lease handling", "⏰");
    
    let storage = MemoryStorage::new();
    
    // Create a lease that's already expired (negative duration simulation)
    let mut expired_lease = create_test_lease_data("expired-obj", "test-service", 300);
    // Manually set expiration time to the past
    expired_lease.expires_at = chrono::Utc::now() - chrono::Duration::seconds(60);
    expired_lease.state = LeaseState::Expired;

    storage.create_lease(expired_lease.clone()).await?;
    
    // Test getting expired leases
    let grace_period = Duration::from_secs(30);
    let expired_leases = storage.get_expired_leases(grace_period).await?;
    
    assert_eq!(expired_leases.len(), 1, "Should find 1 expired lease");
    assert_eq!(expired_leases[0].lease_id, expired_lease.lease_id);
    println!("✅ Found expired leases");
    
    // Test cleanup
    storage.cleanup().await?;
    println!("✅ Ran storage cleanup");
    
    Ok(())
}

#[tokio::test]
async fn test_memory_storage_statistics() -> Result<()> {
    print_test_header("memory storage statistics", "📊");
    
    let storage = MemoryStorage::new();
    
    // Create leases with different states
    let active_lease = create_test_lease_data("active-obj", "stats-service", 300);
    let mut released_lease = create_test_lease_data("released-obj", "stats-service", 300);
    released_lease.release();
    
    storage.create_lease(active_lease).await?;
    storage.create_lease(released_lease).await?;
    
    // Get statistics
    let stats = storage.get_stats().await?;
    
    assert_lease_stats(&stats, 2);
    assert_eq!(stats.active_leases, 1, "Should have 1 active lease");
    assert_eq!(stats.released_leases, 1, "Should have 1 released lease");
    
    // Check service breakdown
    assert_stats_contains_service(&stats, "stats-service", 2);
    
    // Check type breakdown
    assert!(stats.leases_by_type.contains_key("DatabaseRow"));
    
    println!("✅ Generated storage statistics");
    println!("   Total: {}, Active: {}, Released: {}", 
             stats.total_leases, stats.active_leases, stats.released_leases);
    
    Ok(())
}