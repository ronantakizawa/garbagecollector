// tests/helpers/test_data.rs - Test data generators

use std::collections::HashMap;
use std::time::Duration;
use garbagetruck::lease::{Lease, ObjectType as InternalObjectType, CleanupConfig as InternalCleanupConfig};

/// Create a test lease with standard parameters
pub fn create_test_lease_data(object_id: &str, service_id: &str, duration_secs: u64) -> Lease {
    let metadata = [("test".to_string(), "value".to_string())].into();
    let cleanup_config = Some(InternalCleanupConfig {
        cleanup_endpoint: String::new(),
        cleanup_http_endpoint: "http://localhost:8080/cleanup".to_string(),
        cleanup_payload: r#"{"action":"delete"}"#.to_string(),
        max_retries: 3,
        retry_delay_seconds: 1,
    });

    Lease::new(
        object_id.to_string(),
        InternalObjectType::DatabaseRow,
        service_id.to_string(),
        Duration::from_secs(duration_secs),
        metadata,
        cleanup_config,
    )
}

/// Create a test lease with custom object type
pub fn create_test_lease_with_type(
    object_id: &str, 
    service_id: &str, 
    duration_secs: u64,
    object_type: InternalObjectType
) -> Lease {
    let metadata = [("test".to_string(), "value".to_string())].into();
    let cleanup_config = Some(InternalCleanupConfig {
        cleanup_endpoint: String::new(),
        cleanup_http_endpoint: "http://localhost:8080/cleanup".to_string(),
        cleanup_payload: r#"{"action":"delete"}"#.to_string(),
        max_retries: 3,
        retry_delay_seconds: 1,
    });

    Lease::new(
        object_id.to_string(),
        object_type,
        service_id.to_string(),
        Duration::from_secs(duration_secs),
        metadata,
        cleanup_config,
    )
}

/// Create a test lease with custom metadata
pub fn create_test_lease_with_metadata(
    object_id: &str, 
    service_id: &str, 
    duration_secs: u64,
    metadata: HashMap<String, String>
) -> Lease {
    let cleanup_config = Some(InternalCleanupConfig {
        cleanup_endpoint: String::new(),
        cleanup_http_endpoint: "http://localhost:8080/cleanup".to_string(),
        cleanup_payload: r#"{"action":"delete"}"#.to_string(),
        max_retries: 3,
        retry_delay_seconds: 1,
    });

    Lease::new(
        object_id.to_string(),
        InternalObjectType::DatabaseRow,
        service_id.to_string(),
        Duration::from_secs(duration_secs),
        metadata,
        cleanup_config,
    )
}

/// Create a test lease without cleanup configuration
pub fn create_test_lease_no_cleanup(object_id: &str, service_id: &str, duration_secs: u64) -> Lease {
    let metadata = [("test".to_string(), "value".to_string())].into();

    Lease::new(
        object_id.to_string(),
        InternalObjectType::DatabaseRow,
        service_id.to_string(),
        Duration::from_secs(duration_secs),
        metadata,
        None,
    )
}

/// Create multiple test leases with different properties
pub fn create_test_lease_batch(count: usize, service_prefix: &str) -> Vec<Lease> {
    (0..count)
        .map(|i| {
            create_test_lease_data(
                &format!("obj-{}", i),
                &format!("{}-{}", service_prefix, i % 3), // Rotate between 3 services
                300 + (i as u64 * 60), // Different durations
            )
        })
        .collect()
}