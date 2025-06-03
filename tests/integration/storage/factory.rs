// tests/integration/storage/factory.rs - Complete fixed version

use garbagetruck::{
    config::Config,
    error::GCError,
    storage::{create_storage, Storage},
};

#[tokio::test]
async fn test_memory_storage_creation() {
    println!("🧠 Testing memory storage creation...");
    
    let mut config = Config::default();
    config.storage.backend = "memory".to_string();
    
    let storage = create_storage(&config).await.unwrap();
    
    // Test that we can use the storage
    let healthy = storage.health_check().await.unwrap();
    assert!(healthy, "Memory storage should be healthy");
    
    // Test that it's actually memory storage by checking stats
    let stats = storage.get_stats().await.unwrap();
    assert_eq!(stats.total_leases, 0, "New memory storage should have no leases");
}

#[tokio::test]
async fn test_default_config_creates_memory_storage() {
    println!("📋 Testing default config creates memory storage...");
    
    let config = Config::default();
    assert_eq!(config.storage.backend, "memory");
    
    let storage = create_storage(&config).await.unwrap();
    let healthy = storage.health_check().await.unwrap();
    assert!(healthy, "Default storage should be healthy");
}

#[tokio::test]
async fn test_storage_factory_interface() {
    println!("🏭 Testing storage factory interface...");
    
    let config = Config::default();
    
    // Test that create_storage returns the correct type
    let storage: std::sync::Arc<dyn Storage> = create_storage(&config).await.unwrap();
    
    // Test basic operations work through the trait
    let stats = storage.get_stats().await.unwrap();
    assert_eq!(stats.total_leases, 0);
    
    let healthy = storage.health_check().await.unwrap();
    assert!(healthy);
}

#[tokio::test]
async fn test_unknown_storage_backend() {
    println!("❌ Testing unknown storage backend handling...");
    
    let mut config = Config::default();
    config.storage.backend = "unknown-backend".to_string();
    
    let result = create_storage(&config).await;
    
    match result {
        Err(GCError::Configuration(msg)) => {
            assert!(
                msg.contains("Unsupported storage backend"),
                "Error should mention unsupported backend: {}",
                msg
            );
            assert!(
                msg.contains("unknown-backend"),
                "Error should mention the specific backend: {}",
                msg
            );
        }
        _ => panic!("Expected configuration error for unknown backend"),
    }
}

#[tokio::test]
async fn test_postgres_backend_not_supported() {
    println!("🔧 Testing postgres backend not supported...");
    
    let mut config = Config::default();
    config.storage.backend = "postgres".to_string();
    
    let result = create_storage(&config).await;
    
    match result {
        Err(GCError::Configuration(msg)) => {
            assert!(
                msg.contains("Unsupported storage backend"),
                "Error should mention unsupported backend: {}",
                msg
            );
            assert!(
                msg.contains("postgres"),
                "Error should mention postgres: {}",
                msg
            );
            assert!(
                msg.contains("Only 'memory' is supported"),
                "Error should mention only memory is supported: {}",
                msg
            );
        }
        _ => panic!("Expected configuration error for postgres backend"),
    }
}

#[tokio::test]
async fn test_case_sensitive_backend_names() {
    println!("🔤 Testing case sensitive backend names...");
    
    // Test uppercase
    let mut config = Config::default();
    config.storage.backend = "MEMORY".to_string();
    
    let result = create_storage(&config).await;
    assert!(result.is_err(), "Backend names should be case sensitive");
    
    // Test mixed case
    config.storage.backend = "Memory".to_string();
    let result = create_storage(&config).await;
    assert!(result.is_err(), "Backend names should be case sensitive");
    
    // Test correct case
    config.storage.backend = "memory".to_string();
    let result = create_storage(&config).await;
    assert!(result.is_ok(), "Correct case should work");
}

#[tokio::test]
async fn test_empty_backend_name() {
    println!("📭 Testing empty backend name...");
    
    let mut config = Config::default();
    config.storage.backend = "".to_string();
    
    let result = create_storage(&config).await;
    
    match result {
        Err(GCError::Configuration(msg)) => {
            assert!(
                msg.contains("Unsupported storage backend"),
                "Error should mention unsupported backend: {}",
                msg
            );
        }
        _ => panic!("Expected configuration error for empty backend name"),
    }
}

#[tokio::test]
async fn test_storage_creation_with_custom_config() {
    println!("⚙️ Testing storage creation with custom config...");
    
    let mut config = Config::default();
    config.storage.backend = "memory".to_string();
    config.gc.default_lease_duration_seconds = 600;
    config.gc.max_leases_per_service = 5000;
    
    // Storage creation should succeed regardless of other config settings
    let storage = create_storage(&config).await.unwrap();
    let healthy = storage.health_check().await.unwrap();
    assert!(healthy, "Storage should be healthy with custom config");
}

#[tokio::test]
async fn test_multiple_storage_instances() {
    println!("🔢 Testing multiple storage instances...");
    
    let config = Config::default();
    
    // Create multiple storage instances
    let storage1 = create_storage(&config).await.unwrap();
    let storage2 = create_storage(&config).await.unwrap();
    
    // They should be independent
    let stats1 = storage1.get_stats().await.unwrap();
    let stats2 = storage2.get_stats().await.unwrap();
    
    assert_eq!(stats1.total_leases, 0);
    assert_eq!(stats2.total_leases, 0);
    
    // Creating a lease in one shouldn't affect the other
    let lease = garbagetruck::lease::Lease::new(
        "test-object".to_string(),
        garbagetruck::lease::ObjectType::TemporaryFile,
        "test-service".to_string(),
        std::time::Duration::from_secs(300),
        std::collections::HashMap::new(),
        None,
    );
    
    storage1.create_lease(lease).await.unwrap();
    
    let stats1_after = storage1.get_stats().await.unwrap();
    let stats2_after = storage2.get_stats().await.unwrap();
    
    assert_eq!(stats1_after.total_leases, 1);
    assert_eq!(stats2_after.total_leases, 0);
}