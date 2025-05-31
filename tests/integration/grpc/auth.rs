// tests/integration/grpc/auth.rs - Authorization and security tests

use anyhow::Result;
use uuid::Uuid;

use garbagetruck::proto::{RenewLeaseRequest, ReleaseLeaseRequest};

use crate::integration::{TestHarness, print_test_header};

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_unauthorized_access() -> Result<()> {
    print_test_header("unauthorized access prevention", "🔒");
    
    let mut harness = TestHarness::new().await?;
    let object_id = format!("auth-test-{}", Uuid::new_v4());
    
    // Create lease with service A
    let lease_id = harness.create_test_lease(&object_id, 300).await?;
    
    // Try to renew with different service (should fail)
    let renew_response = harness.gc_client.renew_lease(RenewLeaseRequest {
        lease_id: lease_id.clone(),
        service_id: "different-service".to_string(),
        extend_duration_seconds: 120,
    }).await?;
    
    let renew_result = renew_response.into_inner();
    assert!(!renew_result.success, "Renewal should fail for unauthorized service");
    assert!(renew_result.error_message.contains("Unauthorized"), 
            "Error should mention unauthorized access");
    
    // Try to release with different service (should fail)
    let release_response = harness.gc_client.release_lease(ReleaseLeaseRequest {
        lease_id: lease_id.clone(),
        service_id: "different-service".to_string(),
    }).await?;
    
    let release_result = release_response.into_inner();
    assert!(!release_result.success, "Release should fail for unauthorized service");
    
    println!("✅ Unauthorized access prevention passed");
    Ok(())
}

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_lease_not_found() -> Result<()> {
    print_test_header("lease not found handling", "❓");
    
    let mut harness = TestHarness::new().await?;
    let non_existent_lease_id = Uuid::new_v4().to_string();
    
    // Try to renew non-existent lease
    let renew_response = harness.gc_client.renew_lease(RenewLeaseRequest {
        lease_id: non_existent_lease_id.clone(),
        service_id: "test-service".to_string(),
        extend_duration_seconds: 120,
    }).await?;
    
    let renew_result = renew_response.into_inner();
    assert!(!renew_result.success, "Should fail for non-existent lease");
    
    // Try to release non-existent lease
    let release_response = harness.gc_client.release_lease(ReleaseLeaseRequest {
        lease_id: non_existent_lease_id.clone(),
        service_id: "test-service".to_string(),
    }).await?;
    
    let release_result = release_response.into_inner();
    assert!(!release_result.success, "Should fail for non-existent lease");
    
    println!("✅ Lease not found handling passed");
    Ok(())
}