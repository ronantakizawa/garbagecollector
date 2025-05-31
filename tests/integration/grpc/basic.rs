// tests/integration/grpc/basic.rs - Basic gRPC operation tests

use anyhow::Result;
use uuid::Uuid;

use garbagetruck::proto::{
    HealthCheckRequest, GetLeaseRequest, RenewLeaseRequest, 
    ReleaseLeaseRequest, ListLeasesRequest, ObjectType, LeaseState
};

use crate::integration::{TestHarness, print_test_header};

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_health_check() -> Result<()> {
    print_test_header("health check", "🏥");
    
    let mut harness = TestHarness::new().await?;
    
    let response = harness.gc_client.health_check(HealthCheckRequest {}).await?;
    let health = response.into_inner();
    
    assert!(health.healthy, "Service should be healthy");
    assert_eq!(health.version, "0.1.0");
    assert!(health.uptime.is_some(), "Uptime should be present");
    
    println!("✅ Health check passed");
    Ok(())
}

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_lease_creation_and_retrieval() -> Result<()> {
    print_test_header("lease creation and retrieval", "📝");
    
    let mut harness = TestHarness::new().await?;
    let object_id = format!("test-object-{}", Uuid::new_v4());
    
    // Create lease with valid duration (minimum is 30 seconds)
    let lease_id = harness.create_test_lease(&object_id, 300).await?;
    println!("Created lease: {}", lease_id);
    
    // Get lease
    let response = harness.gc_client.get_lease(GetLeaseRequest {
        lease_id: lease_id.clone(),
    }).await?;
    
    let get_response = response.into_inner();
    assert!(get_response.found, "Lease should be found");
    
    let lease = get_response.lease.unwrap();
    assert_eq!(lease.lease_id, lease_id);
    assert_eq!(lease.object_id, object_id);
    assert_eq!(lease.service_id, "test-service");
    assert_eq!(lease.object_type, ObjectType::WebsocketSession as i32);
    assert!(!lease.metadata.is_empty());
    
    println!("✅ Lease creation and retrieval passed");
    Ok(())
}

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_lease_renewal() -> Result<()> {
    print_test_header("lease renewal", "🔄");
    
    let mut harness = TestHarness::new().await?;
    let object_id = format!("renewable-object-{}", Uuid::new_v4());
    
    // Create lease with short duration
    let lease_id = harness.create_test_lease(&object_id, 60).await?;
    
    // Get initial expiration time
    let initial_response = harness.gc_client.get_lease(GetLeaseRequest {
        lease_id: lease_id.clone(),
    }).await?;
    let initial_lease = initial_response.into_inner().lease.unwrap();
    let initial_expires_at = initial_lease.expires_at.unwrap();
    
    // Renew lease
    let renew_response = harness.gc_client.renew_lease(RenewLeaseRequest {
        lease_id: lease_id.clone(),
        service_id: "test-service".to_string(),
        extend_duration_seconds: 120,
    }).await?;
    
    let renew_result = renew_response.into_inner();
    assert!(renew_result.success, "Renewal should succeed: {}", renew_result.error_message);
    
    let new_expires_at = renew_result.new_expires_at.unwrap();
    assert!(new_expires_at.seconds > initial_expires_at.seconds, "New expiration should be later");
    
    // Verify renewal count increased
    let updated_response = harness.gc_client.get_lease(GetLeaseRequest {
        lease_id: lease_id.clone(),
    }).await?;
    let updated_lease = updated_response.into_inner().lease.unwrap();
    assert!(updated_lease.renewal_count > 0, "Renewal count should be > 0");
    
    println!("✅ Lease renewal passed");
    Ok(())
}

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_lease_release() -> Result<()> {
    print_test_header("lease release", "🗑️");
    
    let mut harness = TestHarness::new().await?;
    let object_id = format!("releasable-object-{}", Uuid::new_v4());
    
    // Create lease
    let lease_id = harness.create_test_lease(&object_id, 300).await?;
    
    // Release lease
    let release_response = harness.gc_client.release_lease(ReleaseLeaseRequest {
        lease_id: lease_id.clone(),
        service_id: "test-service".to_string(),
    }).await?;
    
    let release_result = release_response.into_inner();
    assert!(release_result.success, "Release should succeed: {}", release_result.error_message);
    
    // Verify lease state is now released
    let get_response = harness.gc_client.get_lease(GetLeaseRequest {
        lease_id: lease_id.clone(),
    }).await?;
    
    let lease = get_response.into_inner().lease.unwrap();
    assert_eq!(lease.state, LeaseState::Released as i32);
    
    println!("✅ Lease release passed");
    Ok(())
}

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_list_leases() -> Result<()> {
    print_test_header("lease listing", "📋");
    
    let mut harness = TestHarness::new().await?;
    
    // Create multiple leases
    let lease_ids = vec![
        harness.create_test_lease(&format!("list-test-1-{}", Uuid::new_v4()), 300).await?,
        harness.create_test_lease(&format!("list-test-2-{}", Uuid::new_v4()), 300).await?,
        harness.create_test_lease(&format!("list-test-3-{}", Uuid::new_v4()), 300).await?,
    ];
    
    // List all leases
    let list_response = harness.gc_client.list_leases(ListLeasesRequest {
        service_id: "test-service".to_string(),
        object_type: 0, // All types
        state: 0, // All states
        limit: 10,
        page_token: String::new(),
    }).await?;
    
    let leases = list_response.into_inner().leases;
    assert!(leases.len() >= 3, "Should find at least 3 leases");
    
    // Verify our leases are in the list
    let found_ids: Vec<String> = leases.iter().map(|l| l.lease_id.clone()).collect();
    for lease_id in &lease_ids {
        assert!(found_ids.contains(lease_id), "Lease {} should be in list", lease_id);
    }
    
    println!("✅ Lease listing passed");
    Ok(())
}

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_invalid_lease_duration() -> Result<()> {
    print_test_header("invalid lease duration handling", "⏱️");
    
    let mut harness = TestHarness::new().await?;
    
    // Test duration too short (minimum is 30 seconds)
    let request = super::create_lease_request(
        "invalid-duration-test",
        ObjectType::TemporaryFile,
        "test-service",
        10, // Too short
    );
    
    let response = harness.gc_client.create_lease(request).await?;
    let result = response.into_inner();
    
    assert!(!result.success, "Should reject too-short duration");
    assert!(result.error_message.contains("Invalid lease duration"), 
            "Error should mention invalid duration");
    
    // Test duration too long (maximum is 3600 seconds)
    let request = super::create_lease_request(
        "invalid-duration-test-2",
        ObjectType::TemporaryFile,
        "test-service",
        7200, // Too long
    );
    
    let response = harness.gc_client.create_lease(request).await?;
    let result = response.into_inner();
    
    assert!(!result.success, "Should reject too-long duration");
    
    println!("✅ Invalid lease duration handling passed");
    Ok(())
}