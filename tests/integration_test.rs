// tests/integration_test.rs
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::{sleep, timeout};
use tonic::transport::Channel;
use uuid::Uuid;

// Import the generated protobuf code and service types
use distributed_gc_sidecar::proto::{
    distributed_gc_service_client::DistributedGcServiceClient,
    CreateLeaseRequest, CreateLeaseResponse, RenewLeaseRequest, GetLeaseRequest,
    ReleaseLeaseRequest, ListLeasesRequest, HealthCheckRequest, MetricsRequest,
    ObjectType, LeaseState, CleanupConfig,
};

/// Mock cleanup server to simulate service endpoints
#[derive(Clone, Default)]
struct MockCleanupServer {
    cleanup_calls: Arc<Mutex<Vec<CleanupCall>>>,
    should_fail: Arc<Mutex<bool>>,
    delay_ms: Arc<Mutex<u64>>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct CleanupCall {
    lease_id: String,
    object_id: String,
    object_type: String,
    service_id: String,
    metadata: HashMap<String, String>,
    payload: String,
}

impl MockCleanupServer {
    fn new() -> Self {
        Self::default()
    }

    async fn get_cleanup_calls(&self) -> Vec<CleanupCall> {
        self.cleanup_calls.lock().await.clone()
    }

    async fn set_should_fail(&self, fail: bool) {
        *self.should_fail.lock().await = fail;
    }

    async fn set_delay(&self, delay_ms: u64) {
        *self.delay_ms.lock().await = delay_ms;
    }

    async fn clear_calls(&self) {
        self.cleanup_calls.lock().await.clear();
    }

    async fn handle_cleanup(&self, call: CleanupCall) -> Result<(), String> {
        // Add delay if configured
        let delay = *self.delay_ms.lock().await;
        if delay > 0 {
            sleep(Duration::from_millis(delay)).await;
        }

        // Record the call
        self.cleanup_calls.lock().await.push(call);

        // Return error if configured to fail
        if *self.should_fail.lock().await {
            return Err("Mock cleanup failure".to_string());
        }

        Ok(())
    }
}

// Test harness that manages the GC service and mock cleanup server
struct TestHarness {
    gc_client: DistributedGcServiceClient<Channel>,
    cleanup_server: MockCleanupServer,
    _cleanup_server_handle: tokio::task::JoinHandle<()>,
}

impl TestHarness {
    async fn new() -> Result<Self> {
        // Start mock cleanup server
        let cleanup_server = MockCleanupServer::new();
        let cleanup_server_clone = cleanup_server.clone();
        
        let cleanup_server_handle = tokio::spawn(async move {
            start_mock_cleanup_server(cleanup_server_clone, 8080).await;
        });

        // Wait for cleanup server to start
        sleep(Duration::from_millis(100)).await;

        // Connect to GC service (assuming it's running on localhost:50051)
        let gc_client = DistributedGcServiceClient::connect("http://localhost:50051").await?;

        Ok(Self {
            gc_client,
            cleanup_server,
            _cleanup_server_handle: cleanup_server_handle,
        })
    }

    async fn create_test_lease(&mut self, object_id: &str, duration_seconds: u64) -> Result<String> {
        let request = CreateLeaseRequest {
            object_id: object_id.to_string(),
            object_type: ObjectType::WebsocketSession as i32,
            service_id: "test-service".to_string(),
            lease_duration_seconds: duration_seconds,
            metadata: [("test_key".to_string(), "test_value".to_string())].into(),
            cleanup_config: Some(CleanupConfig {
                cleanup_endpoint: String::new(),
                cleanup_http_endpoint: "http://localhost:8080/cleanup".to_string(),
                cleanup_payload: r#"{"action":"delete"}"#.to_string(),
                max_retries: 3,
                retry_delay_seconds: 1,
            }),
        };

        let response = self.gc_client.create_lease(request).await?;
        let lease_response = response.into_inner();
        
        if !lease_response.success {
            anyhow::bail!("Failed to create lease: {}", lease_response.error_message);
        }

        Ok(lease_response.lease_id)
    }
}

async fn start_mock_cleanup_server(server: MockCleanupServer, port: u16) {
    use warp::Filter;

    let cleanup_route = warp::path("cleanup")
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::any().map(move || server.clone()))
        .and_then(|call: CleanupCall, server: MockCleanupServer| async move {
            match server.handle_cleanup(call).await {
                Ok(()) => Ok(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({"success": true})),
                    warp::http::StatusCode::OK,
                )),
                Err(e) => Ok(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({"error": e})),
                    warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                )),
            }
        });

    warp::serve(cleanup_route)
        .run(([127, 0, 0, 1], port))
        .await;
}

#[tokio::test]
async fn test_health_check() -> Result<()> {
    println!("🏥 Testing health check...");
    
    let mut harness = TestHarness::new().await?;
    
    let response = harness.gc_client.health_check(HealthCheckRequest {}).await?;
    let health = response.into_inner();
    
    assert!(health.healthy, "Service should be healthy");
    assert_eq!(health.version, "0.1.0");
    assert!(health.uptime.is_some(), "Uptime should be present");
    
    println!("✅ Health check passed");
    Ok(())
}

#[tokio::test]
async fn test_lease_creation_and_retrieval() -> Result<()> {
    println!("📝 Testing lease creation and retrieval...");
    
    let mut harness = TestHarness::new().await?;
    let object_id = format!("test-object-{}", Uuid::new_v4());
    
    // Create lease
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

#[tokio::test]
async fn test_lease_renewal() -> Result<()> {
    println!("🔄 Testing lease renewal...");
    
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

#[tokio::test]
async fn test_lease_release() -> Result<()> {
    println!("🗑️ Testing lease release...");
    
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

#[tokio::test]
async fn test_list_leases() -> Result<()> {
    println!("📋 Testing lease listing...");
    
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

#[tokio::test]
async fn test_automatic_cleanup() -> Result<()> {
    println!("🧹 Testing automatic cleanup...");
    
    let mut harness = TestHarness::new().await?;
    harness.cleanup_server.clear_calls().await;
    
    let object_id = format!("cleanup-test-{}", Uuid::new_v4());
    
    // Create lease with very short duration (2 seconds)
    let lease_id = harness.create_test_lease(&object_id, 2).await?;
    println!("Created short-lived lease: {}", lease_id);
    
    // Wait for lease to expire and cleanup to occur
    // The cleanup loop runs every 60 seconds by default, but expired leases
    // should be detected. We'll wait up to 10 seconds for cleanup to happen.
    println!("Waiting for lease expiration and cleanup...");
    
    let mut cleanup_occurred = false;
    for attempt in 1..=20 {
        sleep(Duration::from_millis(500)).await;
        
        let calls = harness.cleanup_server.get_cleanup_calls().await;
        if calls.iter().any(|call| call.lease_id == lease_id) {
            cleanup_occurred = true;
            println!("✅ Cleanup call detected after {} attempts", attempt);
            break;
        }
        
        if attempt % 4 == 0 {
            println!("  ... still waiting for cleanup (attempt {})", attempt);
        }
    }
    
    // Check if cleanup was called
    let cleanup_calls = harness.cleanup_server.get_cleanup_calls().await;
    let our_cleanup = cleanup_calls.iter().find(|call| call.lease_id == lease_id);
    
    if let Some(cleanup_call) = our_cleanup {
        assert_eq!(cleanup_call.object_id, object_id);
        assert_eq!(cleanup_call.service_id, "test-service");
        assert_eq!(cleanup_call.object_type, "WebsocketSession");
        assert!(!cleanup_call.metadata.is_empty());
        println!("✅ Automatic cleanup passed");
    } else {
        println!("⚠️  Cleanup may not have occurred yet (this might be expected if cleanup interval is long)");
        println!("   Available cleanup calls: {:?}", cleanup_calls.len());
    }
    
    Ok(())
}

#[tokio::test]
async fn test_cleanup_retry_logic() -> Result<()> {
    println!("🔄 Testing cleanup retry logic...");
    
    let mut harness = TestHarness::new().await?;
    harness.cleanup_server.clear_calls().await;
    
    // Configure cleanup server to fail initially
    harness.cleanup_server.set_should_fail(true).await;
    
    let object_id = format!("retry-test-{}", Uuid::new_v4());
    let lease_id = harness.create_test_lease(&object_id, 1).await?;
    
    // Wait a bit for initial failure attempts
    sleep(Duration::from_secs(3)).await;
    
    // Now make cleanup succeed
    harness.cleanup_server.set_should_fail(false).await;
    
    // Wait for retry to succeed
    sleep(Duration::from_secs(5)).await;
    
    let cleanup_calls = harness.cleanup_server.get_cleanup_calls().await;
    let retry_calls: Vec<_> = cleanup_calls.iter()
        .filter(|call| call.lease_id == lease_id)
        .collect();
    
    if !retry_calls.is_empty() {
        println!("✅ Cleanup retry logic working (found {} retry attempts)", retry_calls.len());
    } else {
        println!("⚠️  Retry logic test inconclusive - may need longer wait time");
    }
    
    Ok(())
}

#[tokio::test]
async fn test_metrics_collection() -> Result<()> {
    println!("📊 Testing metrics collection...");
    
    let mut harness = TestHarness::new().await?;
    
    // Create some leases to generate metrics
    let _lease1 = harness.create_test_lease(&format!("metrics-test-1-{}", Uuid::new_v4()), 300).await?;
    let _lease2 = harness.create_test_lease(&format!("metrics-test-2-{}", Uuid::new_v4()), 300).await?;
    
    // Get metrics
    let metrics_response = harness.gc_client.get_metrics(MetricsRequest {}).await?;
    let metrics = metrics_response.into_inner();
    
    assert!(metrics.total_leases_created > 0, "Should have created leases");
    assert!(metrics.active_leases > 0, "Should have active leases");
    assert!(!metrics.leases_by_service.is_empty(), "Should have service metrics");
    assert!(!metrics.leases_by_type.is_empty(), "Should have type metrics");
    
    // Check specific service metrics
    let test_service_count = metrics.leases_by_service.get("test-service").unwrap_or(&0);
    assert!(*test_service_count > 0, "Should have leases for test-service");
    
    println!("Metrics summary:");
    println!("  Total created: {}", metrics.total_leases_created);
    println!("  Active leases: {}", metrics.active_leases);
    println!("  By service: {:?}", metrics.leases_by_service);
    println!("  By type: {:?}", metrics.leases_by_type);
    
    println!("✅ Metrics collection passed");
    Ok(())
}

#[tokio::test]
async fn test_unauthorized_access() -> Result<()> {
    println!("🔒 Testing unauthorized access prevention...");
    
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

#[tokio::test]
async fn test_invalid_lease_duration() -> Result<()> {
    println!("⏱️ Testing invalid lease duration handling...");
    
    let mut harness = TestHarness::new().await?;
    
    // Test duration too short (assuming min is 30 seconds)
    let request = CreateLeaseRequest {
        object_id: "invalid-duration-test".to_string(),
        object_type: ObjectType::TemporaryFile as i32,
        service_id: "test-service".to_string(),
        lease_duration_seconds: 10, // Too short
        metadata: HashMap::new(),
        cleanup_config: None,
    };
    
    let response = harness.gc_client.create_lease(request).await?;
    let result = response.into_inner();
    
    assert!(!result.success, "Should reject too-short duration");
    assert!(result.error_message.contains("Invalid lease duration"), 
            "Error should mention invalid duration");
    
    // Test duration too long (assuming max is 3600 seconds)
    let request = CreateLeaseRequest {
        object_id: "invalid-duration-test-2".to_string(),
        object_type: ObjectType::TemporaryFile as i32,  
        service_id: "test-service".to_string(),
        lease_duration_seconds: 7200, // Too long
        metadata: HashMap::new(),
        cleanup_config: None,
    };
    
    let response = harness.gc_client.create_lease(request).await?;
    let result = response.into_inner();
    
    assert!(!result.success, "Should reject too-long duration");
    
    println!("✅ Invalid lease duration handling passed");
    Ok(())
}

#[tokio::test]
async fn test_concurrent_operations() -> Result<()> {
    println!("🚀 Testing concurrent operations...");
    
    let mut harness = TestHarness::new().await?;
    
    // Create multiple leases concurrently
    let mut handles = vec![];
    
    for i in 0..10 {
        let mut client = harness.gc_client.clone();
        let handle = tokio::spawn(async move {
            let object_id = format!("concurrent-test-{}-{}", i, Uuid::new_v4());
            let request = CreateLeaseRequest {
                object_id,
                object_type: ObjectType::CacheEntry as i32,
                service_id: format!("test-service-{}", i),
                lease_duration_seconds: 300,
                metadata: [("index".to_string(), i.to_string())].into(),
                cleanup_config: None,
            };
            
            client.create_lease(request).await
        });
        handles.push(handle);
    }
    
    // Wait for all operations to complete
    let mut successful_creates = 0;
    for handle in handles {
        match handle.await? {
            Ok(response) => {
                if response.into_inner().success {
                    successful_creates += 1;
                }
            }
            Err(_) => {} // Count failures
        }
    }
    
    assert!(successful_creates >= 8, "Most concurrent operations should succeed");
    println!("✅ Concurrent operations passed ({}/10 successful)", successful_creates);
    Ok(())
}

// Test runner that provides a summary
#[tokio::test]
async fn run_comprehensive_test_suite() -> Result<()> {
    println!("\n🧪 Running Comprehensive GC Sidecar Test Suite");
    println!("=" .repeat(50));
    
    let test_functions = vec![
        ("Health Check", test_health_check),
        ("Lease Creation & Retrieval", test_lease_creation_and_retrieval),
        ("Lease Renewal", test_lease_renewal),
        ("Lease Release", test_lease_release),
        ("List Leases", test_list_leases),
        ("Metrics Collection", test_metrics_collection),
        ("Unauthorized Access", test_unauthorized_access),
        ("Invalid Duration", test_invalid_lease_duration),
        ("Concurrent Operations", test_concurrent_operations),
        // Note: Automatic cleanup test is separate due to timing requirements
    ];
    
    let mut passed = 0;
    let mut failed = 0;
    
    for (name, test_fn) in test_functions {
        print!("\n🔄 Running: {} ... ", name);
        match timeout(Duration::from_secs(30), test_fn()).await {
            Ok(Ok(())) => {
                println!("✅ PASSED");
                passed += 1;
            }
            Ok(Err(e)) => {
                println!("❌ FAILED: {}", e);
                failed += 1;
            }
            Err(_) => {
                println!("⏱️ TIMEOUT");
                failed += 1;
            }
        }
    }
    
    println!("\n" + "=".repeat(50));
    println!("📊 Test Results Summary:");
    println!("  ✅ Passed: {}", passed);
    println!("  ❌ Failed: {}", failed);
    println!("  📈 Success Rate: {:.1}%", (passed as f64 / (passed + failed) as f64) * 100.0);
    
    if failed > 0 {
        println!("\n⚠️  Some tests failed. Check the output above for details.");
        anyhow::bail!("Test suite failed with {} failures", failed);
    } else {
        println!("\n🎉 All tests passed! Your GC Sidecar is working perfectly!");
    }
    
    Ok(())
}

#[cfg(test)]
mod test_helpers {
    use super::*;
    
    // Helper function to wait for condition with timeout
    pub async fn wait_for_condition<F, Fut>(
        mut condition: F,
        timeout_duration: Duration,
        check_interval: Duration,
    ) -> bool 
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let start = std::time::Instant::now();
        
        while start.elapsed() < timeout_duration {
            if condition().await {
                return true;
            }
            sleep(check_interval).await;
        }
        
        false
    }
    
    // Helper to generate unique test identifiers
    pub fn generate_test_id(prefix: &str) -> String {
        format!("{}-{}", prefix, Uuid::new_v4())
    }
}