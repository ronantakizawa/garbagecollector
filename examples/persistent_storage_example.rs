// examples/persistent_storage_example.rs - Complete example of using persistent storage and recovery

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use garbagetruck::{
    config::Config,
    error::Result,
    lease::{CleanupConfig, ObjectType},
    metrics::Metrics,
    recovery::manager::{RecoveryConfig, RecoveryStrategy, RecoveryTrigger},
    service::GCService,
    storage::persistent::{PersistentStorageConfig, WALSyncPolicy},
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("garbagetruck=info,persistent_storage_example=info")
        .init();

    info!("🚀 Starting GarbageTruck Persistent Storage Example");

    // Example 1: Basic persistent storage setup
    basic_persistent_storage_example().await?;

    // Example 2: Recovery scenarios
    recovery_scenarios_example().await?;

    // Example 3: Advanced WAL and snapshot management
    advanced_wal_management_example().await?;

    // Example 4: Production deployment configuration
    production_deployment_example().await?;

    info!("✅ All examples completed successfully");
    Ok(())
}

/// Example 1: Basic persistent storage setup
async fn basic_persistent_storage_example() -> Result<()> {
    info!("📝 Example 1: Basic persistent storage with WAL");

    // Create configuration with persistent storage
    let mut config = Config::default();
    
    // Configure persistent file storage
    config.storage.backend = "persistent_file".to_string();
    config.storage.enable_wal = true;
    config.storage.enable_auto_recovery = true;
    config.storage.recovery_strategy = RecoveryStrategy::Conservative;
    
    // Configure persistent storage settings
    config.storage.persistent_config = Some(PersistentStorageConfig {
        data_directory: "./example_data".to_string(),
        wal_path: "./example_data/garbagetruck.wal".to_string(),
        snapshot_path: "./example_data/garbagetruck.snapshot".to_string(),
        max_wal_size: 10 * 1024 * 1024, // 10MB
        sync_policy: WALSyncPolicy::EveryN(5), // Sync every 5 operations
        snapshot_interval: 1800, // 30 minutes
        max_wal_files: 3,
        compress_snapshots: true,
    });

    let config = Arc::new(config);
    let metrics = Metrics::new();

    // Create service with persistent storage
    let service = GCService::new(config.clone(), metrics.clone()).await?;
    
    info!("✅ Service created with persistent storage backend");
    info!("   - WAL enabled: {}", config.storage.enable_wal);
    info!("   - Auto-recovery: {}", config.storage.enable_auto_recovery);
    info!("   - Data directory: {:?}", config.storage.persistent_config.as_ref().map(|c| &c.data_directory));

    // Get service status to verify persistent features
    let status = service.get_status().await;
    info!("📊 Service Status:");
    info!("   - Storage backend: {}", status.storage_backend);
    info!("   - Persistent features: {}", status.persistent_features_enabled);
    
    if let Some(wal_status) = &status.wal_status {
        info!("   - WAL sequence: {}", wal_status.current_sequence);
        if !wal_status.integrity_issues.is_empty() {
            warn!("   - WAL integrity issues: {:?}", wal_status.integrity_issues);
        }
    }

    // Example: Create some leases that will be persisted
    info!("📋 Creating sample leases...");
    
    // Create client to interact with service (in a real scenario, this would be over gRPC)
    // For this example, we'll simulate the operations directly with the storage layer
    
    info!("✅ Basic persistent storage example completed");
    Ok(())
}

/// Example 2: Recovery scenarios
async fn recovery_scenarios_example() -> Result<()> {
    info!("📝 Example 2: Recovery scenarios");

    // Scenario 1: Service restart with automatic recovery
    info!("🔄 Scenario 1: Service restart with recovery");
    
    let recovery_config = RecoveryConfig {
        strategy: RecoveryStrategy::Conservative,
        max_recovery_time: Duration::from_secs(120),
        create_backup_before_recovery: true,
        auto_recovery_triggers: vec![
            RecoveryTrigger::ServiceStart,
            RecoveryTrigger::CorruptionDetected,
        ],
        notification_endpoints: vec![
            "http://localhost:8080/recovery-webhook".to_string(),
        ],
        health_check_interval: Duration::from_secs(30),
    };

    let mut config = Config::default();
    config.storage.backend = "persistent_file".to_string();
    config.storage.enable_wal = true;
    config.storage.enable_auto_recovery = true;
    config.recovery = Some(recovery_config);
    
    let config = Arc::new(config);
    let metrics = Metrics::new();

    // Create service - this will trigger automatic recovery
    let service = GCService::new(config, metrics).await?;
    
    // Get recovery API for manual control
    if let Some(recovery_api) = service.recovery_api() {
        info!("🔧 Recovery API available for manual control");
        
        // Example: Trigger manual recovery with different strategy
        info!("🚨 Triggering emergency recovery...");
        
        let recovery_result = recovery_api
            .trigger_recovery(Some(RecoveryStrategy::Emergency))
            .await?;
            
        if recovery_result.success {
            info!("✅ Emergency recovery successful:");
            info!("   - Leases recovered: {}", recovery_result.recovery_info.leases_recovered);
            info!("   - Entries replayed: {}", recovery_result.recovery_info.entries_replayed);
            info!("   - Total time: {:.2}s", recovery_result.total_time.as_secs_f64());
        } else {
            warn!("❌ Emergency recovery failed: {:?}", recovery_result.error);
        }
        
        // Check recovery status
        if let Some(status) = recovery_api.get_recovery_status().await {
            info!("📊 Recovery Status: {:?} ({:.1}%)", status.phase, status.progress_percent);
        }
    }

    info!("✅ Recovery scenarios example completed");
    Ok(())
}

/// Example 3: Advanced WAL and snapshot management
async fn advanced_wal_management_example() -> Result<()> {
    info!("📝 Example 3: Advanced WAL and snapshot management");

    // Configure with different WAL sync policies
    let wal_configs = vec![
        ("Every Write", WALSyncPolicy::EveryWrite),
        ("Every 10 Operations", WALSyncPolicy::EveryN(10)),
        ("Every 30 Seconds", WALSyncPolicy::Interval(30)),
    ];

    for (name, sync_policy) in wal_configs {
        info!("🔧 Testing WAL sync policy: {}", name);
        
        let mut config = Config::default();
        config.storage.backend = "persistent_file".to_string();
        config.storage.enable_wal = true;
        config.storage.persistent_config = Some(PersistentStorageConfig {
            data_directory: format!("./example_data_{}", name.replace(" ", "_").to_lowercase()),
            wal_path: format!("./example_data_{}/test.wal", name.replace(" ", "_").to_lowercase()),
            snapshot_path: format!("./example_data_{}/test.snapshot", name.replace(" ", "_").to_lowercase()),
            max_wal_size: 1024 * 1024, // 1MB for testing
            sync_policy,
            snapshot_interval: 0, // Disable automatic snapshots for this test
            max_wal_files: 2,
            compress_snapshots: false,
        });

        let config = Arc::new(config);
        let metrics = Metrics::new();
        let service = GCService::new(config, metrics).await?;
        
        info!("   ✅ Service created with {} sync policy", name);
        
        // In a real application, you would create leases here to test the WAL behavior
        // For this example, we just verify the service starts correctly
    }

    // Snapshot management example
    info!("📸 Snapshot management example");
    
    let mut config = Config::default();
    config.storage.backend = "persistent_file".to_string();
    config.storage.enable_wal = true;
    config.storage.snapshot_interval_seconds = 60; // Snapshot every minute
    config.storage.persistent_config = Some(PersistentStorageConfig {
        data_directory: "./example_data_snapshots".to_string(),
        wal_path: "./example_data_snapshots/test.wal".to_string(),
        snapshot_path: "./example_data_snapshots/test.snapshot".to_string(),
        max_wal_size: 5 * 1024 * 1024,
        sync_policy: WALSyncPolicy::EveryN(5),
        snapshot_interval: 60, // 1 minute
        max_wal_files: 5,
        compress_snapshots: true,
    });

    let config = Arc::new(config);
    let metrics = Metrics::new();
    let service = GCService::new(config, metrics).await?;
    
    info!("   ✅ Service configured for automatic snapshots every 60 seconds");

    info!("✅ Advanced WAL management example completed");
    Ok(())
}

/// Example 4: Production deployment configuration
async fn production_deployment_example() -> Result<()> {
    info!("📝 Example 4: Production deployment configuration");

    // Production-grade configuration
    let mut config = Config::default();
    
    // Server configuration
    config.server.host = "0.0.0.0".to_string();
    config.server.port = 50051;
    
    // GC configuration
    config.gc.default_lease_duration_seconds = 3600; // 1 hour
    config.gc.max_lease_duration_seconds = 86400; // 24 hours
    config.gc.cleanup_interval_seconds = 300; // 5 minutes
    config.gc.cleanup_grace_period_seconds = 120; // 2 minutes
    config.gc.max_leases_per_service = 100000;
    config.gc.max_concurrent_cleanups = 25;
    
    // Production storage configuration
    config.storage.backend = "persistent_file".to_string();
    config.storage.enable_wal = true;
    config.storage.enable_auto_recovery = true;
    config.storage.recovery_strategy = RecoveryStrategy::Conservative;
    config.storage.snapshot_interval_seconds = 3600; // 1 hour
    config.storage.wal_compaction_threshold = 50000;
    
    config.storage.persistent_config = Some(PersistentStorageConfig {
        data_directory: "/var/lib/garbagetruck".to_string(),
        wal_path: "/var/lib/garbagetruck/garbagetruck.wal".to_string(),
        snapshot_path: "/var/lib/garbagetruck/garbagetruck.snapshot".to_string(),
        max_wal_size: 100 * 1024 * 1024, // 100MB
        sync_policy: WALSyncPolicy::EveryN(10), // Balance between safety and performance
        snapshot_interval: 3600, // 1 hour
        max_wal_files: 10, // Keep more WAL files for better recovery options
        compress_snapshots: true,
    });
    
    // Production cleanup configuration
    config.cleanup.default_timeout_seconds = 60;
    config.cleanup.default_max_retries = 5;
    config.cleanup.default_retry_delay_seconds = 10;
    config.cleanup.max_cleanup_time_seconds = 600; // 10 minutes
    
    // Production metrics configuration
    config.metrics.enabled = true;
    config.metrics.port = 9090;
    config.metrics.include_recovery_metrics = true;
    config.metrics.include_wal_metrics = true;
    
    // Production recovery configuration
    config.recovery = Some(RecoveryConfig {
        strategy: RecoveryStrategy::Conservative,
        max_recovery_time: Duration::from_secs(600), // 10 minutes
        create_backup_before_recovery: true,
        auto_recovery_triggers: vec![
            RecoveryTrigger::ServiceStart,
            RecoveryTrigger::CorruptionDetected,
        ],
        notification_endpoints: vec![
            "http://monitoring.company.com/alerts/garbagetruck-recovery".to_string(),
            "http://slack-webhook.company.com/recovery-notifications".to_string(),
        ],
        health_check_interval: Duration::from_secs(60),
    });

    let config = Arc::new(config);
    
    info!("🏭 Production Configuration Summary:");
    info!("   - Storage: Persistent file with WAL");
    info!("   - WAL sync: Every 10 operations");
    info!("   - Snapshots: Every hour, compressed");
    info!("   - Recovery: Conservative with notifications");
    info!("   - Max leases per service: {}", config.gc.max_leases_per_service);
    info!("   - Cleanup interval: {}s", config.gc.cleanup_interval_seconds);
    info!("   - WAL compaction: {} entries", config.storage.wal_compaction_threshold);

    // Validate production configuration
    match config.validate() {
        Ok(_) => info!("✅ Production configuration is valid"),
        Err(e) => {
            warn!("❌ Production configuration validation failed: {}", e);
            return Err(e.into());
        }
    }

    // Show configuration summary
    let summary = config.summary();
    info!("📊 Configuration Summary:");
    info!("   - Server: {}", summary.server_endpoint);
    info!("   - Storage backend: {}", summary.storage_backend);
    info!("   - WAL enabled: {}", summary.storage_features.wal_enabled);
    info!("   - Auto-recovery: {}", summary.storage_features.auto_recovery_enabled);
    info!("   - Recovery strategy: {:?}", summary.storage_features.recovery_strategy);
    
    if let Some(snapshot_interval) = summary.storage_features.snapshot_interval {
        info!("   - Snapshot interval: {}s", snapshot_interval);
    }

    // Create production service
    let metrics = Metrics::new();
    let service = GCService::new(config, metrics).await?;
    
    // Get comprehensive service status
    let status = service.get_status().await;
    info!("🔍 Service Status:");
    info!("   - Storage backend: {}", status.storage_backend);
    info!("   - Persistent features: {}", status.persistent_features_enabled);
    info!("   - Total leases: {}", status.total_leases);
    info!("   - Active leases: {}", status.active_leases);
    
    if let Some(wal_status) = &status.wal_status {
        info!("   - WAL sequence: {}", wal_status.current_sequence);
        if wal_status.integrity_issues.is_empty() {
            info!("   - WAL integrity: ✅ OK");
        } else {
            info!("   - WAL integrity: ⚠️ {} issues", wal_status.integrity_issues.len());
        }
    }

    info!("✅ Production deployment example completed");
    Ok(())
}

/// Example: Creating leases with different cleanup configurations
async fn lease_management_example() -> Result<()> {
    info!("📝 Lease Management with Persistence Example");

    // This would typically be done using the gRPC client
    // For this example, we'll show the configuration patterns

    // Temporary file lease with HTTP cleanup
    let temp_file_cleanup = CleanupConfig {
        cleanup_endpoint: String::new(),
        cleanup_http_endpoint: "http://localhost:8080/cleanup/file".to_string(),
        cleanup_payload: r#"{"file_path": "/tmp/processing/temp_file_123.dat"}"#.to_string(),
        max_retries: 3,
        retry_delay_seconds: 5,
    };

    info!("📄 Temp file lease cleanup config:");
    info!("   - HTTP endpoint: {}", temp_file_cleanup.cleanup_http_endpoint);
    info!("   - Max retries: {}", temp_file_cleanup.max_retries);

    // Database row lease with specific cleanup
    let db_cleanup = CleanupConfig {
        cleanup_endpoint: String::new(),
        cleanup_http_endpoint: "http://localhost:8080/cleanup/database".to_string(),
        cleanup_payload: r#"{"action": "delete_row", "table": "processing_jobs", "row_id": "job_456"}"#.to_string(),
        max_retries: 5,
        retry_delay_seconds: 10,
    };

    info!("🗄️ Database lease cleanup config:");
    info!("   - HTTP endpoint: {}", db_cleanup.cleanup_http_endpoint);
    info!("   - Max retries: {}", db_cleanup.max_retries);

    // S3 object lease with blob cleanup
    let s3_cleanup = CleanupConfig {
        cleanup_endpoint: String::new(),
        cleanup_http_endpoint: "http://localhost:8080/cleanup/s3".to_string(),
        cleanup_payload: r#"{"action": "delete_blob", "bucket": "temp-processing", "key": "uploads/batch_789.zip"}"#.to_string(),
        max_retries: 3,
        retry_delay_seconds: 15,
    };

    info!("☁️ S3 object lease cleanup config:");
    info!("   - HTTP endpoint: {}", s3_cleanup.cleanup_http_endpoint);
    info!("   - Payload: {}", s3_cleanup.cleanup_payload);

    info!("✅ Lease management examples completed");
    Ok(())
}

/// Environment variable configuration example
async fn environment_config_example() -> Result<()> {
    info!("📝 Environment Variable Configuration Example");

    // Show how to configure via environment variables
    let env_vars = vec![
        ("GC_STORAGE_BACKEND", "persistent_file"),
        ("GC_ENABLE_WAL", "true"),
        ("GC_ENABLE_AUTO_RECOVERY", "true"),
        ("GC_DATA_DIRECTORY", "/var/lib/garbagetruck"),
        ("GC_WAL_PATH", "/var/lib/garbagetruck/garbagetruck.wal"),
        ("GC_SNAPSHOT_PATH", "/var/lib/garbagetruck/garbagetruck.snapshot"),
        ("GC_MAX_WAL_SIZE", "104857600"), // 100MB
        ("GC_WAL_SYNC_POLICY", "every:10"),
        ("GC_COMPRESS_SNAPSHOTS", "true"),
        ("GC_RECOVERY_STRATEGY", "conservative"),
        ("GC_SNAPSHOT_INTERVAL", "3600"), // 1 hour
        ("GC_WAL_COMPACTION_THRESHOLD", "50000"),
        ("GC_INCLUDE_RECOVERY_METRICS", "true"),
        ("GC_INCLUDE_WAL_METRICS", "true"),
    ];

    info!("🌍 Environment Variables for Production:");
    for (key, value) in env_vars {
        info!("   export {}={}", key, value);
    }

    info!("");
    info!("💡 WAL Sync Policy Options:");
    info!("   - every_write: Sync after every operation (safest, slowest)");
    info!("   - every:N: Sync after N operations (e.g., every:10)");
    info!("   - interval:N: Sync every N seconds (e.g., interval:30)");
    info!("   - none: No explicit sync (fastest, least safe)");

    info!("");
    info!("🔄 Recovery Strategy Options:");
    info!("   - conservative: Full validation and backup (safest)");
    info!("   - fast: Quick recovery with minimal validation");
    info!("   - emergency: Try all recovery methods");

    info!("✅ Environment configuration example completed");
    Ok(())
}

/// Monitoring and alerting integration example
async fn monitoring_integration_example() -> Result<()> {
    info!("📝 Monitoring and Alerting Integration Example");

    info!("📊 Prometheus Metrics Available:");
    info!("   Core Metrics:");
    info!("   - garbagetruck_leases_total");
    info!("   - garbagetruck_cleanups_total");
    info!("   - garbagetruck_active_leases");
    info!("");
    info!("   Persistent Storage Metrics:");
    info!("   - garbagetruck_wal_entries_total");
    info!("   - garbagetruck_wal_compactions_total");
    info!("   - garbagetruck_snapshots_total");
    info!("   - garbagetruck_recovery_operations_total");
    info!("");
    info!("   Performance Metrics:");
    info!("   - garbagetruck_operation_duration_seconds");
    info!("   - garbagetruck_wal_sync_duration_seconds");
    info!("   - garbagetruck_recovery_duration_seconds");

    info!("");
    info!("🚨 Recommended Alerts:");
    info!("   1. WAL integrity issues detected");
    info!("   2. Recovery operations failing");
    info!("   3. Cleanup success rate below threshold");
    info!("   4. WAL file size approaching limit");
    info!("   5. Service restart without successful recovery");

    info!("");
    info!("📈 Health Check Endpoints:");
    info!("   - http://localhost:9090/health (basic health)");
    info!("   - http://localhost:9090/metrics (Prometheus metrics)");

    info!("✅ Monitoring integration example completed");
    Ok(())
}

// Run additional examples
async fn run_additional_examples() -> Result<()> {
    lease_management_example().await?;
    environment_config_example().await?;
    monitoring_integration_example().await?;
    Ok(())
}