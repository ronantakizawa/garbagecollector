// tests/integration/storage/memory.rs - Complete fixed version

use std::collections::HashMap;
use tokio::time::sleep;

use garbagetruck::{
    lease::{Lease, LeaseFilter, LeaseState, ObjectType},
    storage::Storage,
};

use super::create_test_storage;

#[tokio::test]
async fn test_memory_storage_basic_operations() {
    println!("🧠 Testing memory storage basic operations...");
    
    let storage = create_test_storage().await.unwrap();
    
    // Create a test lease
    let lease = Lease::new(
        "test-object-1".to_string(),
        ObjectType::TemporaryFile,
        "test-service".to_string(),
        std::time::Duration::from_secs(300),
        HashMap::new(),
        None,
    );
    
    let lease_id = lease.lease_id.clone();
    
    // Test create
    storage.create_lease(lease.clone()).await.unwrap();
    
    // Test get
    let retrieved = storage.get_lease(&lease_id).await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().object_id, "test-object-1");
    
    // Test update
    let mut updated_lease = lease.clone();
    updated_lease.renewal_count = 5;
    storage.update_lease(updated_lease).await.unwrap();
    
    let retrieved = storage.get_lease(&lease_id).await.unwrap().unwrap();
    assert_eq!(retrieved.renewal_count, 5);
    
    // Test delete
    storage.delete_lease(&lease_id).await.unwrap();
    
    let deleted = storage.get_lease(&lease_id).await.unwrap();
    assert!(deleted.is_none());
}

#[tokio::test]
async fn test_memory_storage_list_and_filter() {
    println!("📝 Testing memory storage list and filter operations...");
    
    let storage = create_test_storage().await.unwrap();
    
    // Create multiple test leases
    let lease1 = Lease::new(
        "object-1".to_string(),
        ObjectType::TemporaryFile,
        "service-1".to_string(),
        std::time::Duration::from_secs(300),
        HashMap::new(),
        None,
    );
    
    let lease2 = Lease::new(
        "object-2".to_string(),
        ObjectType::DatabaseRow,
        "service-2".to_string(),
        std::time::Duration::from_secs(300),
        HashMap::new(),
        None,
    );
    
    storage.create_lease(lease1).await.unwrap();
    storage.create_lease(lease2).await.unwrap();
    
    // Test list all
    let all_leases = storage
        .list_leases(Some(LeaseFilter::default()), None)
        .await
        .unwrap();
    assert_eq!(all_leases.len(), 2);
    
    // Test filter by service
    let service_filter = LeaseFilter {
        service_id: Some("service-1".to_string()),
        ..Default::default()
    };
    let service_leases = storage.list_leases(Some(service_filter), None).await.unwrap();
    assert_eq!(service_leases.len(), 1);
    assert_eq!(service_leases[0].service_id, "service-1");
    
    // Test limit
    let limited_leases = storage
        .list_leases(Some(LeaseFilter::default()), Some(2))
        .await
        .unwrap();
    assert!(limited_leases.len() <= 2);
}

#[tokio::test]
async fn test_memory_storage_expired_leases() {
    println!("⏰ Testing memory storage expired lease handling...");
    
    let storage = create_test_storage().await.unwrap();
    
    // Create a lease with very short duration
    let lease = Lease::new(
        "expired-object".to_string(),
        ObjectType::TemporaryFile,
        "test-service".to_string(),
        std::time::Duration::from_millis(1), // Very short duration
        HashMap::new(),
        None,
    );
    
    storage.create_lease(lease).await.unwrap();
    
    // Wait for lease to expire
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    
    // Get expired leases with NO grace period (0 seconds)
    let expired_leases = storage
        .get_expired_leases(std::time::Duration::from_secs(0))
        .await
        .unwrap();
    
    println!("Found {} expired leases", expired_leases.len());
    assert_eq!(expired_leases.len(), 1, "Should find 1 expired lease");
    assert_eq!(expired_leases[0].object_id, "expired-object");
}

#[tokio::test]
async fn test_memory_storage_service_queries() {
    println!("🔍 Testing memory storage service-specific queries...");
    
    let storage = create_test_storage().await.unwrap();
    
    // Create leases for different services
    let lease1 = Lease::new(
        "object-1".to_string(),
        ObjectType::TemporaryFile,
        "service-a".to_string(),
        std::time::Duration::from_secs(300),
        HashMap::new(),
        None,
    );
    
    let lease2 = Lease::new(
        "object-2".to_string(),
        ObjectType::DatabaseRow,
        "service-a".to_string(),
        std::time::Duration::from_secs(300),
        HashMap::new(),
        None,
    );
    
    let lease3 = Lease::new(
        "object-3".to_string(),
        ObjectType::BlobStorage,
        "service-b".to_string(),
        std::time::Duration::from_secs(300),
        HashMap::new(),
        None,
    );
    
    storage.create_lease(lease1).await.unwrap();
    storage.create_lease(lease2).await.unwrap();
    storage.create_lease(lease3).await.unwrap();
    
    // Test get by service
    let service_a_leases = storage.get_leases_by_service("service-a").await.unwrap();
    assert_eq!(service_a_leases.len(), 2);
    
    let service_b_leases = storage.get_leases_by_service("service-b").await.unwrap();
    assert_eq!(service_b_leases.len(), 1);
    
    // Test get by type
    let file_leases = storage.get_leases_by_type(ObjectType::TemporaryFile).await.unwrap();
    assert_eq!(file_leases.len(), 1);
    
    let db_leases = storage.get_leases_by_type(ObjectType::DatabaseRow).await.unwrap();
    assert_eq!(db_leases.len(), 1);
    
    // Test count active leases for service
    let active_count = storage.count_active_leases_for_service("service-a").await.unwrap();
    assert_eq!(active_count, 2);
}

#[tokio::test]
async fn test_memory_storage_lease_states() {
    println!("🔄 Testing memory storage lease state management...");
    
    let storage = create_test_storage().await.unwrap();
    
    let lease = Lease::new(
        "state-test-object".to_string(),
        ObjectType::TemporaryFile,
        "test-service".to_string(),
        std::time::Duration::from_secs(300),
        HashMap::new(),
        None,
    );
    
    let lease_id = lease.lease_id.clone();
    storage.create_lease(lease).await.unwrap();
    
    // Test mark expired
    storage.mark_lease_expired(&lease_id).await.unwrap();
    let expired_lease = storage.get_lease(&lease_id).await.unwrap().unwrap();
    assert_eq!(expired_lease.state, LeaseState::Expired);
    
    // Test mark released
    storage.mark_lease_released(&lease_id).await.unwrap();
    let released_lease = storage.get_lease(&lease_id).await.unwrap().unwrap();
    assert_eq!(released_lease.state, LeaseState::Released);
}

#[tokio::test]
async fn test_memory_storage_cleanup() {
    println!("🧹 Testing memory storage cleanup operations...");
    
    let storage = create_test_storage().await.unwrap();
    
    // Create released lease
    let mut released_lease = Lease::new(
        "cleanup-test-1".to_string(),
        ObjectType::TemporaryFile,
        "test-service".to_string(),
        std::time::Duration::from_secs(300),
        HashMap::new(),
        None,
    );
    released_lease.release();
    
    // Create expired lease (old expiration)
    let mut expired_lease = Lease::new(
        "cleanup-test-2".to_string(),
        ObjectType::DatabaseRow,
        "test-service".to_string(),
        std::time::Duration::from_millis(1),
        HashMap::new(),
        None,
    );
    
    storage.create_lease(released_lease).await.unwrap();
    storage.create_lease(expired_lease.clone()).await.unwrap();
    
    // Wait for the second lease to expire
    sleep(std::time::Duration::from_millis(10)).await;
    
    // Mark the expired lease as expired
    storage.mark_lease_expired(&expired_lease.lease_id).await.unwrap();
    
    // Verify we have 2 leases initially
    let initial_leases = storage.list_leases(None, None).await.unwrap();
    assert_eq!(initial_leases.len(), 2);
    
    // Run cleanup
    let cleaned_count = storage.cleanup().await.unwrap();
    assert!(cleaned_count > 0, "Should have cleaned up some leases");
    
    // Verify cleanup reduced the number of leases
    let remaining_leases = storage.list_leases(None, None).await.unwrap();
    assert!(remaining_leases.len() < initial_leases.len(), "Cleanup should remove some leases");
}

#[tokio::test]
async fn test_memory_storage_statistics() {
    println!("📊 Testing memory storage statistics...");
    
    let storage = create_test_storage().await.unwrap();
    
    // Create various leases
    let lease1 = Lease::new(
        "stats-object-1".to_string(),
        ObjectType::TemporaryFile,
        "service-stats-1".to_string(),
        std::time::Duration::from_secs(300),
        HashMap::new(),
        None,
    );
    
    let mut lease2 = Lease::new(
        "stats-object-2".to_string(),
        ObjectType::DatabaseRow,
        "service-stats-2".to_string(),
        std::time::Duration::from_secs(300),
        HashMap::new(),
        None,
    );
    lease2.release();
    
    storage.create_lease(lease1).await.unwrap();
    storage.create_lease(lease2).await.unwrap();
    
    // Get stats
    let stats = storage.get_stats().await.unwrap();
    assert_eq!(stats.total_leases, 2);
    assert_eq!(stats.active_leases, 1);
    assert_eq!(stats.released_leases, 1);
    assert!(stats.leases_by_service.contains_key("service-stats-1"));
    assert!(stats.leases_by_service.contains_key("service-stats-2"));
}

#[tokio::test]
async fn test_memory_storage_health_check() {
    println!("❤️ Testing memory storage health check...");
    
    let storage = create_test_storage().await.unwrap();
    
    let healthy = storage.health_check().await.unwrap();
    assert!(healthy, "Memory storage should always be healthy");
}