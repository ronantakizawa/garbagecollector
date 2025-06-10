// tests/integration/mod.rs - Integration test harness and utilities

use anyhow::Result;
use std::collections::HashMap;
use std::time::Duration;
use tonic::transport::Channel;
use uuid::Uuid;

use garbagetruck::proto::{
    distributed_gc_service_client::DistributedGcServiceClient, CleanupConfig, CreateLeaseRequest,
    ObjectType,
};

use crate::helpers::mock_server::MockCleanupServer;

// Test modules
pub mod grpc;
pub mod service;
pub mod storage;

// Remove this line since we don't have database tests yet:
// pub mod database;

pub mod cross_backend;

/// Test harness that manages the GC service and mock cleanup server
pub struct TestHarness {
    pub gc_client: DistributedGcServiceClient<Channel>,
    pub cleanup_server: MockCleanupServer,
    _cleanup_server_handle: tokio::task::JoinHandle<()>,
}

impl TestHarness {
    /// Create a new test harness
    pub async fn new() -> Result<Self> {
        // Start mock cleanup server with unique port
        let cleanup_server = MockCleanupServer::new();
        let cleanup_server_clone = cleanup_server.clone();
        let port = cleanup_server.get_port();

        let cleanup_server_handle = cleanup_server_clone.start().await;

        // Wait for cleanup server to start
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Connect to GC service (assuming it's running on localhost:50051)
        let gc_client = DistributedGcServiceClient::connect("http://localhost:50051").await?;

        Ok(Self {
            gc_client,
            cleanup_server,
            _cleanup_server_handle: cleanup_server_handle,
        })
    }

    /// Create a test lease through the gRPC API
    pub async fn create_test_lease(
        &mut self,
        object_id: &str,
        duration_seconds: u64,
    ) -> Result<String> {
        let port = self.cleanup_server.get_port();
        let request = CreateLeaseRequest {
            object_id: object_id.to_string(),
            object_type: ObjectType::WebsocketSession as i32,
            service_id: "test-service".to_string(),
            lease_duration_seconds: duration_seconds,
            metadata: [("test_key".to_string(), "test_value".to_string())].into(),
            cleanup_config: Some(CleanupConfig {
                cleanup_endpoint: String::new(),
                cleanup_http_endpoint: format!("http://localhost:{}/cleanup", port),
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

    /// Create a test lease with custom parameters
    pub async fn create_test_lease_custom(
        &mut self,
        object_id: &str,
        object_type: ObjectType,
        service_id: &str,
        duration_seconds: u64,
        metadata: HashMap<String, String>,
    ) -> Result<String> {
        let port = self.cleanup_server.get_port();
        let request = CreateLeaseRequest {
            object_id: object_id.to_string(),
            object_type: object_type as i32,
            service_id: service_id.to_string(),
            lease_duration_seconds: duration_seconds,
            metadata,
            cleanup_config: Some(CleanupConfig {
                cleanup_endpoint: String::new(),
                cleanup_http_endpoint: format!("http://localhost:{}/cleanup", port),
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

    /// Generate a unique object ID for testing
    pub fn unique_object_id(&self, prefix: &str) -> String {
        format!("{}-{}", prefix, Uuid::new_v4())
    }

    /// Wait for a condition to be true
    pub async fn wait_for_condition<F>(&self, mut condition: F, timeout: Duration) -> bool
    where
        F: FnMut() -> bool,
    {
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            if condition() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }
}

/// Common test setup for storage tests
pub fn print_test_header(test_name: &str, emoji: &str) {
    println!("{} Testing {}...", emoji, test_name);
}

/// Common test success message
pub fn print_test_success(test_name: &str) {
    println!("✅ {} passed", test_name);
}

// =============================================================================
// Integration Test Runner - Run a comprehensive test suite summary
// =============================================================================

#[tokio::test]
#[ignore = "summary"] // Use ignore to make this optional
async fn test_suite_summary() -> anyhow::Result<()> {
    println!("📋 Test Suite Summary");
    println!("====================");
    println!("✅ Storage Layer Tests:");
    println!("   • Memory storage basic operations");
    println!("   • Memory storage filtering and listing");
    println!("   • Memory storage expired lease handling");
    println!("   • Memory storage statistics");
    println!("   • Storage factory pattern");
    println!("");

    println!("✅ gRPC Service Tests:");
    println!("   • Health check");
    println!("   • Lease CRUD operations via gRPC");
    println!("   • Authorization and security");
    println!("   • Concurrent gRPC operations");
    println!("   • Automatic cleanup processes");
    println!("   • Metrics collection");
    println!("");

    println!("✅ Service Integration Tests:");
    println!("   • Graceful shutdown coordination");
    println!("   • Task priority handling");
    println!("   • Service lifecycle management");
    println!("   • Component restart scenarios");
    println!("   • Resource cleanup simulation");
    println!("");

    println!("✅ Integration Tests:");
    println!("   • End-to-end lease lifecycle");
    println!("   • Cleanup server integration");
    println!("   • Error handling and edge cases");
    println!("   • Cross-backend consistency");
    println!("");

    println!("🎯 To run all tests:");
    println!("   cargo test                      # Memory-only tests");
    println!("   make test-integration           # Integration tests");

    Ok(())
}
