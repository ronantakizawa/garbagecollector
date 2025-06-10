// src/config.rs - Updated configuration with persistent storage and recovery options

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::storage::{PersistentStorageConfig, WALSyncPolicy};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub gc: GCConfig,
    pub storage: StorageConfig,
    pub cleanup: CleanupConfig,
    pub metrics: MetricsConfig,
    /// Recovery configuration (optional)
    #[cfg(feature = "persistent")]
    pub recovery: Option<crate::recovery::RecoveryConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GCConfig {
    /// Default lease duration in seconds
    pub default_lease_duration_seconds: u64,
    /// Maximum lease duration in seconds
    pub max_lease_duration_seconds: u64,
    /// Minimum lease duration in seconds
    pub min_lease_duration_seconds: u64,
    /// How often to run the cleanup loop (in seconds)
    pub cleanup_interval_seconds: u64,
    /// Grace period before actually cleaning up expired leases (in seconds)
    pub cleanup_grace_period_seconds: u64,
    /// Maximum number of leases per service
    pub max_leases_per_service: usize,
    /// Maximum number of concurrent cleanup operations
    pub max_concurrent_cleanups: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    /// Storage backend type: "memory", "persistent_file"
    pub backend: String,
    /// Persistent storage configuration (when using persistent backends)
    pub persistent_config: Option<PersistentStorageConfig>,
    /// Enable write-ahead logging
    pub enable_wal: bool,
    /// Enable automatic recovery on startup
    pub enable_auto_recovery: bool,
    /// Recovery strategy
    #[cfg(feature = "persistent")]
    pub recovery_strategy: crate::recovery::RecoveryStrategy,
    #[cfg(not(feature = "persistent"))]
    pub recovery_strategy: String, // Fallback string for when persistent feature is disabled
    /// Automatic snapshot interval (seconds, 0 to disable)
    pub snapshot_interval_seconds: u64,
    /// WAL compaction threshold (entries)
    pub wal_compaction_threshold: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            backend: "memory".to_string(),
            persistent_config: None,
            enable_wal: false,
            enable_auto_recovery: false,
            #[cfg(feature = "persistent")]
            recovery_strategy: crate::recovery::RecoveryStrategy::Conservative,
            #[cfg(not(feature = "persistent"))]
            recovery_strategy: "conservative".to_string(),
            snapshot_interval_seconds: 3600, // 1 hour
            wal_compaction_threshold: 10000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupConfig {
    /// Default timeout for cleanup operations (in seconds)
    pub default_timeout_seconds: u64,
    /// Default number of retries for failed cleanups
    pub default_max_retries: u32,
    /// Default delay between retries (in seconds)
    pub default_retry_delay_seconds: u64,
    /// Maximum time to wait for cleanup operations (in seconds)
    pub max_cleanup_time_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    /// Whether to enable Prometheus metrics
    pub enabled: bool,
    /// Metrics server port
    pub port: u16,
    /// Include recovery metrics
    pub include_recovery_metrics: bool,
    /// Include WAL metrics
    pub include_wal_metrics: bool,
}

/// Detailed validation result with specific error information
#[derive(Debug, Clone)]
pub struct ValidationResult {
    pub success: bool,
    pub errors: Vec<ValidationError>,
    pub warnings: Vec<ValidationWarning>,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub struct ValidationError {
    pub field: String,
    pub error_type: String,
    pub message: String,
    pub suggested_fix: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ValidationWarning {
    pub field: String,
    pub message: String,
    pub recommendation: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 50051,
            },
            gc: GCConfig {
                default_lease_duration_seconds: 300, // 5 minutes
                max_lease_duration_seconds: 3600,    // 1 hour
                min_lease_duration_seconds: 30,      // 30 seconds
                cleanup_interval_seconds: 60,        // 1 minute
                cleanup_grace_period_seconds: 30,    // 30 seconds
                max_leases_per_service: 10000,
                max_concurrent_cleanups: 10,
            },
            storage: StorageConfig {
                backend: "memory".to_string(),
                persistent_config: None,
                enable_wal: false,
                enable_auto_recovery: false,
                #[cfg(feature = "persistent")]
                recovery_strategy: crate::recovery::RecoveryStrategy::Conservative,
                #[cfg(not(feature = "persistent"))]
                recovery_strategy: "conservative".to_string(),
                snapshot_interval_seconds: 3600, // 1 hour
                wal_compaction_threshold: 10000,
            },
            cleanup: CleanupConfig {
                default_timeout_seconds: 30,
                default_max_retries: 3,
                default_retry_delay_seconds: 5,
                max_cleanup_time_seconds: 300, // 5 minutes
            },
            metrics: MetricsConfig {
                enabled: true,
                port: 9090,
                include_recovery_metrics: true,
                include_wal_metrics: true,
            },
            #[cfg(feature = "persistent")]
            recovery: Some(crate::recovery::RecoveryConfig::default()),
            #[cfg(not(feature = "persistent"))]
            recovery: None,
        }
    }
}

impl Config {
    /// Load configuration from environment variables with persistent storage support
    pub fn from_env() -> Result<Self> {
        let start_time = std::time::Instant::now();
        let mut config = Config::default();
        let mut parse_errors = Vec::new();

        debug!("🔧 Loading configuration from environment variables");

        // Server configuration
        if let Ok(host) = std::env::var("GC_SERVER_HOST") {
            debug!("Found GC_SERVER_HOST: {}", host);
            config.server.host = host;
        }

        if let Ok(port) = std::env::var("GC_SERVER_PORT") {
            match port.parse() {
                Ok(p) => {
                    debug!("Found GC_SERVER_PORT: {}", p);
                    config.server.port = p;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_SERVER_PORT '{}': {}", port, e));
                }
            }
        }

        // Storage configuration
        if let Ok(backend) = std::env::var("GC_STORAGE_BACKEND") {
            debug!("Found GC_STORAGE_BACKEND: {}", backend);
            config.storage.backend = backend;
        }

        if let Ok(enable_wal) = std::env::var("GC_ENABLE_WAL") {
            match enable_wal.parse() {
                Ok(enabled) => {
                    debug!("Found GC_ENABLE_WAL: {}", enabled);
                    config.storage.enable_wal = enabled;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_ENABLE_WAL '{}': {}", enable_wal, e));
                }
            }
        }

        if let Ok(auto_recovery) = std::env::var("GC_ENABLE_AUTO_RECOVERY") {
            match auto_recovery.parse() {
                Ok(enabled) => {
                    debug!("Found GC_ENABLE_AUTO_RECOVERY: {}", enabled);
                    config.storage.enable_auto_recovery = enabled;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_ENABLE_AUTO_RECOVERY '{}': {}", auto_recovery, e));
                }
            }
        }

        // Persistent storage configuration
        if config.storage.backend == "persistent_file" || config.storage.enable_wal {
            let mut persistent_config = PersistentStorageConfig::default();

            if let Ok(data_dir) = std::env::var("GC_DATA_DIRECTORY") {
                debug!("Found GC_DATA_DIRECTORY: {}", data_dir);
                persistent_config.data_directory = data_dir;
            }

            if let Ok(wal_path) = std::env::var("GC_WAL_PATH") {
                debug!("Found GC_WAL_PATH: {}", wal_path);
                persistent_config.wal_path = wal_path;
            }

            if let Ok(snapshot_path) = std::env::var("GC_SNAPSHOT_PATH") {
                debug!("Found GC_SNAPSHOT_PATH: {}", snapshot_path);
                persistent_config.snapshot_path = snapshot_path;
            }

            if let Ok(max_wal_size) = std::env::var("GC_MAX_WAL_SIZE") {
                match max_wal_size.parse() {
                    Ok(size) => {
                        debug!("Found GC_MAX_WAL_SIZE: {}", size);
                        persistent_config.max_wal_size = size;
                    }
                    Err(e) => {
                        parse_errors.push(format!("Invalid GC_MAX_WAL_SIZE '{}': {}", max_wal_size, e));
                    }
                }
            }

            if let Ok(sync_policy) = std::env::var("GC_WAL_SYNC_POLICY") {
                debug!("Found GC_WAL_SYNC_POLICY: {}", sync_policy);
                persistent_config.sync_policy = match sync_policy.as_str() {
                    "every_write" => WALSyncPolicy::EveryWrite,
                    "none" => WALSyncPolicy::None,
                    interval if interval.starts_with("interval:") => {
                        if let Ok(seconds) = interval.strip_prefix("interval:").unwrap().parse::<u64>() {
                            WALSyncPolicy::Interval(seconds)
                        } else {
                            parse_errors.push(format!("Invalid interval in WAL sync policy: {}", interval));
                            WALSyncPolicy::EveryN(10)
                        }
                    }
                    every_n if every_n.starts_with("every:") => {
                        if let Ok(n) = every_n.strip_prefix("every:").unwrap().parse::<u32>() {
                            WALSyncPolicy::EveryN(n)
                        } else {
                            parse_errors.push(format!("Invalid count in WAL sync policy: {}", every_n));
                            WALSyncPolicy::EveryN(10)
                        }
                    }
                    _ => {
                        parse_errors.push(format!("Unknown WAL sync policy: {}", sync_policy));
                        WALSyncPolicy::EveryN(10)
                    }
                };
            }

            if let Ok(compress) = std::env::var("GC_COMPRESS_SNAPSHOTS") {
                match compress.parse() {
                    Ok(enabled) => {
                        debug!("Found GC_COMPRESS_SNAPSHOTS: {}", enabled);
                        persistent_config.compress_snapshots = enabled;
                    }
                    Err(e) => {
                        parse_errors.push(format!("Invalid GC_COMPRESS_SNAPSHOTS '{}': {}", compress, e));
                    }
                }
            }

            config.storage.persistent_config = Some(persistent_config);
        }

        // Recovery strategy
        #[cfg(feature = "persistent")]
        if let Ok(strategy) = std::env::var("GC_RECOVERY_STRATEGY") {
            debug!("Found GC_RECOVERY_STRATEGY: {}", strategy);
            config.storage.recovery_strategy = match strategy.as_str() {
                "conservative" => crate::recovery::RecoveryStrategy::Conservative,
                "fast" => crate::recovery::RecoveryStrategy::Fast,
                "emergency" => crate::recovery::RecoveryStrategy::Emergency,
                custom if custom.starts_with("custom:") => {
                    // Parse custom recovery parameters
                    crate::recovery::RecoveryStrategy::Custom {
                        validate_checksums: true,
                        skip_corrupted_entries: false,
                        force_snapshot_load: false,
                        max_recovery_time_seconds: 300,
                    }
                }
                _ => {
                    parse_errors.push(format!("Unknown recovery strategy: {}", strategy));
                    crate::recovery::RecoveryStrategy::Conservative
                }
            };
        }

        #[cfg(not(feature = "persistent"))]
        if let Ok(strategy) = std::env::var("GC_RECOVERY_STRATEGY") {
            debug!("Found GC_RECOVERY_STRATEGY: {} (persistent features disabled)", strategy);
            config.storage.recovery_strategy = strategy;
        }

        // Snapshot interval
        if let Ok(interval) = std::env::var("GC_SNAPSHOT_INTERVAL") {
            match interval.parse() {
                Ok(seconds) => {
                    debug!("Found GC_SNAPSHOT_INTERVAL: {}", seconds);
                    config.storage.snapshot_interval_seconds = seconds;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_SNAPSHOT_INTERVAL '{}': {}", interval, e));
                }
            }
        }

        // WAL compaction threshold
        if let Ok(threshold) = std::env::var("GC_WAL_COMPACTION_THRESHOLD") {
            match threshold.parse() {
                Ok(count) => {
                    debug!("Found GC_WAL_COMPACTION_THRESHOLD: {}", count);
                    config.storage.wal_compaction_threshold = count;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_WAL_COMPACTION_THRESHOLD '{}': {}", threshold, e));
                }
            }
        }

        // GC configuration (existing code)
        if let Ok(duration) = std::env::var("GC_DEFAULT_LEASE_DURATION") {
            match duration.parse() {
                Ok(d) => {
                    debug!("Found GC_DEFAULT_LEASE_DURATION: {}", d);
                    config.gc.default_lease_duration_seconds = d;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_DEFAULT_LEASE_DURATION '{}': {}", duration, e));
                }
            }
        }

        if let Ok(max_duration) = std::env::var("GC_MAX_LEASE_DURATION") {
            match max_duration.parse() {
                Ok(d) => {
                    debug!("Found GC_MAX_LEASE_DURATION: {}", d);
                    config.gc.max_lease_duration_seconds = d;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_MAX_LEASE_DURATION '{}': {}", max_duration, e));
                }
            }
        }

        if let Ok(min_duration) = std::env::var("GC_MIN_LEASE_DURATION") {
            match min_duration.parse() {
                Ok(d) => {
                    debug!("Found GC_MIN_LEASE_DURATION: {}", d);
                    config.gc.min_lease_duration_seconds = d;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_MIN_LEASE_DURATION '{}': {}", min_duration, e));
                }
            }
        }

        // Cleanup configuration (existing code)
        if let Ok(timeout) = std::env::var("GC_CLEANUP_TIMEOUT") {
            match timeout.parse() {
                Ok(t) => {
                    debug!("Found GC_CLEANUP_TIMEOUT: {}", t);
                    config.cleanup.default_timeout_seconds = t;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_CLEANUP_TIMEOUT '{}': {}", timeout, e));
                }
            }
        }

        // Metrics configuration
        if let Ok(enabled) = std::env::var("GC_METRICS_ENABLED") {
            match enabled.parse() {
                Ok(e) => {
                    debug!("Found GC_METRICS_ENABLED: {}", e);
                    config.metrics.enabled = e;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_METRICS_ENABLED '{}': {}", enabled, e));
                }
            }
        }

        if let Ok(recovery_metrics) = std::env::var("GC_INCLUDE_RECOVERY_METRICS") {
            match recovery_metrics.parse() {
                Ok(enabled) => {
                    debug!("Found GC_INCLUDE_RECOVERY_METRICS: {}", enabled);
                    config.metrics.include_recovery_metrics = enabled;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_INCLUDE_RECOVERY_METRICS '{}': {}", recovery_metrics, e));
                }
            }
        }

        if let Ok(wal_metrics) = std::env::var("GC_INCLUDE_WAL_METRICS") {
            match wal_metrics.parse() {
                Ok(enabled) => {
                    debug!("Found GC_INCLUDE_WAL_METRICS: {}", enabled);
                    config.metrics.include_wal_metrics = enabled;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_INCLUDE_WAL_METRICS '{}': {}", wal_metrics, e));
                }
            }
        }

        let load_duration = start_time.elapsed();

        if !parse_errors.is_empty() {
            let error_msg = format!(
                "Configuration parsing failed after {:.3}s with {} errors: {}",
                load_duration.as_secs_f64(),
                parse_errors.len(),
                parse_errors.join(", ")
            );
            return Err(anyhow::anyhow!(error_msg));
        }

        info!(
            "✅ Configuration loaded from environment in {:.3}s (backend: {}, WAL: {}, auto-recovery: {})",
            load_duration.as_secs_f64(),
            config.storage.backend,
            config.storage.enable_wal,
            config.storage.enable_auto_recovery
        );

        Ok(config)
    }

    /// Validate configuration with persistent storage checks
    pub fn validate(&self) -> Result<()> {
        let validation_result = self.validate_detailed();

        if !validation_result.success {
            let error_messages: Vec<String> = validation_result
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect();

            return Err(anyhow::anyhow!(
                "Configuration validation failed: {}",
                error_messages.join(", ")
            ));
        }

        // Log warnings if any
        for warning in &validation_result.warnings {
            warn!(
                "Configuration warning for {}: {}{}",
                warning.field,
                warning.message,
                warning
                    .recommendation
                    .as_ref()
                    .map(|r| format!(" (Recommendation: {})", r))
                    .unwrap_or_default()
            );
        }

        Ok(())
    }

    /// Perform detailed validation with persistent storage support
    pub fn validate_detailed(&self) -> ValidationResult {
        let start_time = std::time::Instant::now();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Validate lease duration relationships (existing validation)
        if self.gc.min_lease_duration_seconds >= self.gc.max_lease_duration_seconds {
            errors.push(ValidationError {
                field: "gc.min_lease_duration_seconds".to_string(),
                error_type: "invalid_range".to_string(),
                message: format!(
                    "Min lease duration ({}) must be less than max lease duration ({})",
                    self.gc.min_lease_duration_seconds, self.gc.max_lease_duration_seconds
                ),
                suggested_fix: Some(format!(
                    "Set min_lease_duration to a value less than {}",
                    self.gc.max_lease_duration_seconds
                )),
            });
        }

        // Validate storage backend
        match self.storage.backend.as_str() {
            "memory" => {
                if self.storage.enable_wal {
                    warnings.push(ValidationWarning {
                        field: "storage.enable_wal".to_string(),
                        message: "WAL is enabled but memory backend doesn't persist data".to_string(),
                        recommendation: Some("Consider using 'persistent_file' backend for WAL".to_string()),
                    });
                }
                if self.storage.enable_auto_recovery {
                    warnings.push(ValidationWarning {
                        field: "storage.enable_auto_recovery".to_string(),
                        message: "Auto-recovery is enabled but memory backend has no persistent state".to_string(),
                        recommendation: Some("Use 'persistent_file' backend for recovery features".to_string()),
                    });
                }
            }
            "persistent_file" => {
                if self.storage.persistent_config.is_none() {
                    errors.push(ValidationError {
                        field: "storage.persistent_config".to_string(),
                        error_type: "missing_config".to_string(),
                        message: "Persistent file backend requires persistent_config".to_string(),
                        suggested_fix: Some("Add persistent_config section to storage configuration".to_string()),
                    });
                } else if let Some(ref config) = self.storage.persistent_config {
                    // Validate persistent storage configuration
                    if config.data_directory.is_empty() {
                        errors.push(ValidationError {
                            field: "storage.persistent_config.data_directory".to_string(),
                            error_type: "empty_path".to_string(),
                            message: "Data directory cannot be empty".to_string(),
                            suggested_fix: Some("Set a valid path for data_directory".to_string()),
                        });
                    }

                    if config.max_wal_size < 1024 * 1024 {
                        warnings.push(ValidationWarning {
                            field: "storage.persistent_config.max_wal_size".to_string(),
                            message: "Very small WAL size may cause frequent rotations".to_string(),
                            recommendation: Some("Consider using at least 10MB for max_wal_size".to_string()),
                        });
                    }

                    if config.max_wal_files < 2 {
                        warnings.push(ValidationWarning {
                            field: "storage.persistent_config.max_wal_files".to_string(),
                            message: "Very few WAL files may not provide sufficient recovery history".to_string(),
                            recommendation: Some("Consider keeping at least 3 WAL files".to_string()),
                        });
                    }
                }
            }
            backend => {
                errors.push(ValidationError {
                    field: "storage.backend".to_string(),
                    error_type: "unsupported_backend".to_string(),
                    message: format!("Unsupported storage backend: {}", backend),
                    suggested_fix: Some("Use 'memory' or 'persistent_file'".to_string()),
                });
            }
        }

        // Validate server configuration (existing validation)
        if self.server.port < 1024 {
            warnings.push(ValidationWarning {
                field: "server.port".to_string(),
                message: format!("Port {} is a privileged port", self.server.port),
                recommendation: Some("Consider using a port >= 1024 for non-root execution".to_string()),
            });
        }

        if self.server.port == self.metrics.port {
            errors.push(ValidationError {
                field: "server.port".to_string(),
                error_type: "port_conflict".to_string(),
                message: "Server port and metrics port cannot be the same".to_string(),
                suggested_fix: Some("Use different ports for server and metrics".to_string()),
            });
        }

        // Validate WAL compaction threshold
        if self.storage.wal_compaction_threshold < 1000 {
            warnings.push(ValidationWarning {
                field: "storage.wal_compaction_threshold".to_string(),
                message: "Low WAL compaction threshold may cause frequent compactions".to_string(),
                recommendation: Some("Consider using at least 1000 entries".to_string()),
            });
        }

        // Validate snapshot interval
        if self.storage.snapshot_interval_seconds > 0 && self.storage.snapshot_interval_seconds < 300 {
            warnings.push(ValidationWarning {
                field: "storage.snapshot_interval_seconds".to_string(),
                message: "Very frequent snapshots may impact performance".to_string(),
                recommendation: Some("Consider using at least 5 minutes (300 seconds)".to_string()),
            });
        }

        // Validate recovery configuration
        if let Some(ref recovery_config) = self.recovery {
            if recovery_config.max_recovery_time.as_secs() < 10 {
                warnings.push(ValidationWarning {
                    field: "recovery.max_recovery_time".to_string(),
                    message: "Very short recovery timeout may cause recovery failures".to_string(),
                    recommendation: Some("Consider using at least 30 seconds".to_string()),
                });
            }
        }

        let duration = start_time.elapsed();

        ValidationResult {
            success: errors.is_empty(),
            errors,
            warnings,
            duration,
        }
    }

    /// Get configuration summary with persistent storage info
    pub fn summary(&self) -> ConfigSummary {
        ConfigSummary {
            server_endpoint: format!("{}:{}", self.server.host, self.server.port),
            storage_backend: self.storage.backend.clone(),
            storage_features: StorageFeatures {
                wal_enabled: self.storage.enable_wal,
                auto_recovery_enabled: self.storage.enable_auto_recovery,
                snapshot_interval: if self.storage.snapshot_interval_seconds > 0 {
                    Some(self.storage.snapshot_interval_seconds)
                } else {
                    None
                },
                #[cfg(feature = "persistent")]
                recovery_strategy: self.storage.recovery_strategy.clone(),
                #[cfg(not(feature = "persistent"))]
                recovery_strategy: self.storage.recovery_strategy.clone(),
            },
            default_lease_duration: self.gc.default_lease_duration_seconds,
            cleanup_interval: self.gc.cleanup_interval_seconds,
            max_leases_per_service: self.gc.max_leases_per_service,
            metrics_enabled: self.metrics.enabled,
            metrics_port: if self.metrics.enabled {
                Some(self.metrics.port)
            } else {
                None
            },
        }
    }

    // Helper methods for duration conversion (existing methods)
    pub fn cleanup_interval(&self) -> Duration {
        Duration::from_secs(self.gc.cleanup_interval_seconds)
    }

    pub fn cleanup_grace_period(&self) -> Duration {
        Duration::from_secs(self.gc.cleanup_grace_period_seconds)
    }

    pub fn default_lease_duration(&self) -> Duration {
        Duration::from_secs(self.gc.default_lease_duration_seconds)
    }

    pub fn max_lease_duration(&self) -> Duration {
        Duration::from_secs(self.gc.max_lease_duration_seconds)
    }

    pub fn min_lease_duration(&self) -> Duration {
        Duration::from_secs(self.gc.min_lease_duration_seconds)
    }
}

/// Configuration summary for logging and metrics with storage features
#[derive(Debug, Clone, Serialize)]
pub struct ConfigSummary {
    pub server_endpoint: String,
    pub storage_backend: String,
    pub storage_features: StorageFeatures,
    pub default_lease_duration: u64,
    pub cleanup_interval: u64,
    pub max_leases_per_service: usize,
    pub metrics_enabled: bool,
    pub metrics_port: Option<u16>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageFeatures {
    pub wal_enabled: bool,
    pub auto_recovery_enabled: bool,
    pub snapshot_interval: Option<u64>,
    #[cfg(feature = "persistent")]
    pub recovery_strategy: crate::recovery::RecoveryStrategy,
    #[cfg(not(feature = "persistent"))]
    pub recovery_strategy: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_is_valid() {
        let config = Config::default();
        let result = config.validate_detailed();
        assert!(
            result.success,
            "Default config should be valid: {:?}",
            result.errors
        );
    }

    #[test]
    fn test_persistent_storage_validation() {
        let mut config = Config::default();
        config.storage.backend = "persistent_file".to_string();
        // Missing persistent_config should cause validation error

        let result = config.validate_detailed();
        assert!(!result.success);
        assert!(result.errors.iter().any(|e| e.error_type == "missing_config"));
    }

    #[test]
    fn test_wal_with_memory_backend_warning() {
        let mut config = Config::default();
        config.storage.backend = "memory".to_string();
        config.storage.enable_wal = true;

        let result = config.validate_detailed();
        assert!(result.success); // Should still be valid
        assert!(!result.warnings.is_empty()); // But should have warnings
        assert!(result.warnings.iter().any(|w| w.field.contains("enable_wal")));
    }

    #[test]
    fn test_unsupported_backend() {
        let mut config = Config::default();
        config.storage.backend = "nonexistent".to_string();

        let result = config.validate_detailed();
        assert!(!result.success);
        assert!(result.errors.iter().any(|e| e.error_type == "unsupported_backend"));
    }

    #[test]
    fn test_config_summary() {
        let config = Config::default();
        let summary = config.summary();
        
        assert_eq!(summary.storage_backend, "memory");
        assert!(!summary.storage_features.wal_enabled);
        assert!(!summary.storage_features.auto_recovery_enabled);
    }

    #[test]
    fn test_persistent_config_summary() {
        let mut config = Config::default();
        config.storage.backend = "persistent_file".to_string();
        config.storage.enable_wal = true;
        config.storage.enable_auto_recovery = true;
        config.storage.persistent_config = Some(PersistentStorageConfig::default());

        let summary = config.summary();
        assert_eq!(summary.storage_backend, "persistent_file");
        assert!(summary.storage_features.wal_enabled);
        assert!(summary.storage_features.auto_recovery_enabled);
    }
}