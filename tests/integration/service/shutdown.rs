// tests/integration/service/shutdown.rs - Fixed shutdown coordination tests

use std::time::Duration;
use tokio::time::timeout;
use serial_test::serial;

use garbagetruck::{Config, GCClient};
use crate::integration::service::TestHarness;

#[tokio::test]
#[serial]
async fn test_graceful_shutdown() {
    let harness = TestHarness::new().await.expect("Failed to create test harness");
    
    // Start the service
    let (_addr, service_handle) = harness.start_service().await.expect("Failed to start service");
    harness.wait_for_ready().await.expect("Service not ready");
    
    // Create some leases to ensure service is active
    let client = GCClient::new(&harness.endpoint(), "shutdown-test".to_string())
        .await
        .expect("Failed to create client");
    
    for i in 0..5 {
        let lease_id = client.create_lease(
            format!("shutdown-object-{}", i),
            garbagetruck::ObjectType::TemporaryFile,
            300,
            std::collections::HashMap::new(),
            None,
        ).await.expect("Failed to create lease");
        
        assert!(!lease_id.is_empty(), "Lease ID should not be empty");
    }
    
    // Verify leases were created
    let lease_count = harness.get_active_lease_count().await.expect("Failed to get lease count");
    assert_eq!(lease_count, 5, "Should have 5 active leases");
    
    // Initiate graceful shutdown
    let shutdown_start = std::time::Instant::now();
    harness.stop_service().await.expect("Failed to stop service");
    
    // Wait for service task to complete
    let result = timeout(Duration::from_secs(10), service_handle).await;
    assert!(result.is_ok(), "Service should shutdown gracefully within 10 seconds");
    
    let shutdown_duration = shutdown_start.elapsed();
    println!("Graceful shutdown took: {:?}", shutdown_duration);
    
    // Shutdown should take some time (not immediate) to allow cleanup
    assert!(shutdown_duration >= Duration::from_millis(100), "Shutdown should not be immediate");
    assert!(shutdown_duration <= Duration::from_secs(8), "Shutdown should not take too long");
}

#[tokio::test]
#[serial]
async fn test_shutdown_with_active_operations() {
    let mut config = Config::default();
    config.gc.cleanup_interval_seconds = 1; // Fast cleanup
    config.storage.backend = "memory".to_string();
    
    let harness = TestHarness::with_config(config).await.expect("Failed to create harness");
    
    // Start service
    let (_addr, service_handle) = harness.start_service().await.expect("Failed to start service");
    harness.wait_for_ready().await.expect("Service not ready");
    
    let client = GCClient::new(&harness.endpoint(), "active-ops-test".to_string())
        .await
        .expect("Failed to create client");
    
    // Create leases continuously while shutting down
    let client_clone = client.clone();
    let create_task = tokio::spawn(async move {
        for i in 0..10 {
            let _ = client_clone.create_lease(
                format!("active-object-{}", i),
                garbagetruck::ObjectType::CacheEntry,
                60,
                std::collections::HashMap::new(),
                None,
            ).await;
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    
    // Let some operations start
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Initiate shutdown while operations are ongoing
    let shutdown_task = tokio::spawn(async move {
        harness.stop_service().await.expect("Failed to stop service");
    });
    
    // Wait for both tasks
    let (create_result, shutdown_result) = tokio::join!(create_task, shutdown_task);
    
    // Operations might fail after shutdown starts, that's expected
    println!("Create task result: {:?}", create_result);
    assert!(shutdown_result.is_ok(), "Shutdown should succeed");
    
    // Wait for service to actually stop
    let result = timeout(Duration::from_secs(5), service_handle).await;
    assert!(result.is_ok(), "Service should stop within 5 seconds");
}

#[tokio::test]
#[serial]
async fn test_shutdown_cleanup_completion() {
    let mut config = Config::default();
    config.gc.cleanup_interval_seconds = 2;
    config.gc.cleanup_grace_period_seconds = 1;
    config.storage.backend = "memory".to_string();
    
    let harness = TestHarness::with_config(config).await.expect("Failed to create harness");
    
    // Start service
    let (_addr, service_handle) = harness.start_service().await.expect("Failed to start service");
    harness.wait_for_ready().await.expect("Service not ready");
    
    let client = GCClient::new(&harness.endpoint(), "cleanup-completion-test".to_string())
        .await
        .expect("Failed to create client");
    
    // Create short-lived leases that will expire soon
    for i in 0..3 {
        let _lease_id = client.create_lease(
            format!("expiring-object-{}", i),
            garbagetruck::ObjectType::TemporaryFile,
            1, // 1 second lease
            std::collections::HashMap::new(),
            None,
        ).await.expect("Failed to create lease");
    }
    
    // Wait for leases to expire but not be cleaned up yet
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Initiate shutdown - this should allow cleanup to complete
    harness.stop_service().await.expect("Failed to stop service");
    
    // Service should wait for cleanup operations to complete
    let result = timeout(Duration::from_secs(10), service_handle).await;
    assert!(result.is_ok(), "Service should shutdown after cleanup completion");
}

#[tokio::test]
#[serial]
async fn test_shutdown_timeout_behavior() {
    let harness = TestHarness::new().await.expect("Failed to create harness");
    
    // Start service
    let (_addr, service_handle) = harness.start_service().await.expect("Failed to start service");
    harness.wait_for_ready().await.expect("Service not ready");
    
    // Initiate shutdown
    harness.stop_service().await.expect("Failed to stop service");
    
    // Service should shutdown within reasonable time even if there are issues
    let result = timeout(Duration::from_secs(15), service_handle).await;
    assert!(result.is_ok(), "Service should shutdown within 15 seconds maximum");
}

#[tokio::test]
#[serial]
async fn test_multiple_shutdown_calls() {
    let harness = TestHarness::new().await.expect("Failed to create harness");
    
    // Start service
    let (_addr, service_handle) = harness.start_service().await.expect("Failed to start service");
    harness.wait_for_ready().await.expect("Service not ready");
    
    // Call shutdown multiple times (should be idempotent)
    let shutdown1 = harness.stop_service();
    let shutdown2 = harness.stop_service();
    let shutdown3 = harness.stop_service();
    
    let (result1, result2, result3) = tokio::join!(shutdown1, shutdown2, shutdown3);
    
    // All shutdown calls should succeed (or at least not panic)
    assert!(result1.is_ok() || result2.is_ok() || result3.is_ok(), 
            "At least one shutdown call should succeed");
    
    // Service should stop
    let result = timeout(Duration::from_secs(5), service_handle).await;
    assert!(result.is_ok(), "Service should stop within 5 seconds");
}

#[tokio::test]
#[serial]
async fn test_shutdown_signal_propagation() {
    let harness = TestHarness::new().await.expect("Failed to create harness");
    
    // Start service
    let (_addr, service_handle) = harness.start_service().await.expect("Failed to start service");
    harness.wait_for_ready().await.expect("Service not ready");
    
    // Test that shutdown signal is properly propagated to all components
    let client = GCClient::new(&harness.endpoint(), "signal-test".to_string())
        .await
        .expect("Failed to create client");
    
    // Verify service is healthy before shutdown
    let is_healthy = client.health_check().await.expect("Health check failed");
    assert!(is_healthy, "Service should be healthy before shutdown");
    
    // Initiate shutdown
    harness.stop_service().await.expect("Failed to stop service");
    
    // After shutdown, health checks should fail
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    let health_result = client.health_check().await;
    // Health check should fail after shutdown starts
    assert!(health_result.is_err(), "Health check should fail after shutdown");
    
    // Wait for complete shutdown
    let result = timeout(Duration::from_secs(5), service_handle).await;
    assert!(result.is_ok(), "Service should complete shutdown");
}