// tests/integration/grpc/mod.rs - gRPC test utilities

// gRPC test modules
pub mod auth;
pub mod basic;
pub mod cleanup;
pub mod concurrent;

use garbagetruck::proto::{CleanupConfig, CreateLeaseRequest, ObjectType};
use std::collections::HashMap;

/// Create a standard gRPC create lease request
pub fn create_lease_request(
    object_id: &str,
    object_type: ObjectType,
    service_id: &str,
    duration_seconds: u64,
) -> CreateLeaseRequest {
    CreateLeaseRequest {
        object_id: object_id.to_string(),
        object_type: object_type as i32,
        service_id: service_id.to_string(),
        lease_duration_seconds: duration_seconds,
        metadata: [("test_key".to_string(), "test_value".to_string())].into(),
        cleanup_config: Some(CleanupConfig {
            cleanup_endpoint: String::new(),
            cleanup_http_endpoint: "http://localhost:8080/cleanup".to_string(),
            cleanup_payload: r#"{"action":"delete"}"#.to_string(),
            max_retries: 3,
            retry_delay_seconds: 1,
        }),
    }
}

/// Create a lease request with custom metadata
pub fn create_lease_request_with_metadata(
    object_id: &str,
    object_type: ObjectType,
    service_id: &str,
    duration_seconds: u64,
    metadata: HashMap<String, String>,
) -> CreateLeaseRequest {
    CreateLeaseRequest {
        object_id: object_id.to_string(),
        object_type: object_type as i32,
        service_id: service_id.to_string(),
        lease_duration_seconds: duration_seconds,
        metadata,
        cleanup_config: Some(CleanupConfig {
            cleanup_endpoint: String::new(),
            cleanup_http_endpoint: "http://localhost:8080/cleanup".to_string(),
            cleanup_payload: r#"{"action":"delete"}"#.to_string(),
            max_retries: 3,
            retry_delay_seconds: 1,
        }),
    }
}

/// Create a lease request without cleanup configuration
pub fn create_lease_request_no_cleanup(
    object_id: &str,
    object_type: ObjectType,
    service_id: &str,
    duration_seconds: u64,
) -> CreateLeaseRequest {
    CreateLeaseRequest {
        object_id: object_id.to_string(),
        object_type: object_type as i32,
        service_id: service_id.to_string(),
        lease_duration_seconds: duration_seconds,
        metadata: HashMap::new(),
        cleanup_config: None,
    }
}
