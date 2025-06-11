// src/config.rs - Updated configuration with mTLS support

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

use crate::storage::{PersistentStorageConfig, WALSyncPolicy};
use crate::security::MTLSConfig; // Add this import

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub gc: GCConfig,
    pub storage: StorageConfig,
    pub cleanup: CleanupConfig,
    pub metrics: MetricsConfig,
    /// mTLS security configuration
    pub security: SecurityConfig,
    /// Recovery configuration (optional)
    #[cfg(feature = "persistent")]
    pub recovery: Option<crate::recovery::RecoveryConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// mTLS configuration for gRPC communications
    pub mtls: MTLSConfig,
    /// Enable security headers and additional protections
    pub enhanced_security: bool,
    /// Maximum number of concurrent connections per client
    pub max_connections_per_client: usize,
    /// Rate limiting configuration
    pub rate_limiting: RateLimitConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Enable rate limiting
    pub enabled: bool,
    /// Maximum requests per minute per client
    pub requests_per_minute: u32,
    /// Burst capacity
    pub burst_capacity: u32,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            mtls: MTLSConfig::default(),
            enhanced_security: true,
            max_connections_per_client: 100,
            rate_limiting: RateLimitConfig {
                enabled: true,
                requests_per_minute: 1000,
                burst_capacity: 100,
            },
        }
    }
}

// Keep existing config structs...
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GCConfig {
    pub default_lease_duration_seconds: u64,
    pub max_lease_duration_seconds: u64,
    pub min_lease_duration_seconds: u64,
    pub cleanup_interval_seconds: u64,
    pub cleanup_grace_period_seconds: u64,
    pub max_leases_per_service: usize,
    pub max_concurrent_cleanups: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub backend: String,
    pub persistent_config: Option<PersistentStorageConfig>,
    pub enable_wal: bool,
    pub enable_auto_recovery: bool,
    #[cfg(feature = "persistent")]
    pub recovery_strategy: crate::recovery::RecoveryStrategy,
    #[cfg(not(feature = "persistent"))]
    pub recovery_strategy: String,
    pub snapshot_interval_seconds: u64,
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
            snapshot_interval_seconds: 3600,
            wal_compaction_threshold: 10000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupConfig {
    pub default_timeout_seconds: u64,
    pub default_max_retries: u32,
    pub default_retry_delay_seconds: u64,
    pub max_cleanup_time_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub port: u16,
    pub include_recovery_metrics: bool,
    pub include_wal_metrics: bool,
    /// Include security/mTLS metrics
    pub include_security_metrics: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 50051,
            },
            gc: GCConfig {
                default_lease_duration_seconds: 300,
                max_lease_duration_seconds: 3600,
                min_lease_duration_seconds: 30,
                cleanup_interval_seconds: 60,
                cleanup_grace_period_seconds: 30,
                max_leases_per_service: 10000,
                max_concurrent_cleanups: 10,
            },
            storage: StorageConfig::default(),
            cleanup: CleanupConfig {
                default_timeout_seconds: 30,
                default_max_retries: 3,
                default_retry_delay_seconds: 5,
                max_cleanup_time_seconds: 300,
            },
            metrics: MetricsConfig {
                enabled: true,
                port: 9090,
                include_recovery_metrics: true,
                include_wal_metrics: true,
                include_security_metrics: true,
            },
            security: SecurityConfig::default(),
            #[cfg(feature = "persistent")]
            recovery: Some(crate::recovery::RecoveryConfig::default()),
        }
    }
}

impl Config {
    /// Load configuration from environment variables with mTLS support
    pub fn from_env() -> Result<Self> {
        let start_time = std::time::Instant::now();
        let mut config = Config::default();
        let mut parse_errors = Vec::new();

        debug!("🔧 Loading configuration from environment variables (including mTLS)");

        // Load existing configuration (server, gc, storage, etc.)
        // ... [Keep existing environment loading code] ...

        // Load mTLS configuration from environment
        match MTLSConfig::from_env() {
            Ok(mtls_config) => {
                config.security.mtls = mtls_config;
                debug!("✅ mTLS configuration loaded from environment");
            }
            Err(e) => {
                parse_errors.push(format!("Failed to load mTLS config: {}", e));
            }
        }

        // Load additional security settings
        if let Ok(enhanced_security) = std::env::var("GC_ENHANCED_SECURITY") {
            match enhanced_security.parse() {
                Ok(enabled) => {
                    debug!("Found GC_ENHANCED_SECURITY: {}", enabled);
                    config.security.enhanced_security = enabled;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_ENHANCED_SECURITY '{}': {}", enhanced_security, e));
                }
            }
        }

        if let Ok(max_connections) = std::env::var("GC_MAX_CONNECTIONS_PER_CLIENT") {
            match max_connections.parse() {
                Ok(max) => {
                    debug!("Found GC_MAX_CONNECTIONS_PER_CLIENT: {}", max);
                    config.security.max_connections_per_client = max;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_MAX_CONNECTIONS_PER_CLIENT '{}': {}", max_connections, e));
                }
            }
        }

        // Rate limiting configuration
        if let Ok(rate_limit_enabled) = std::env::var("GC_RATE_LIMITING_ENABLED") {
            match rate_limit_enabled.parse() {
                Ok(enabled) => {
                    debug!("Found GC_RATE_LIMITING_ENABLED: {}", enabled);
                    config.security.rate_limiting.enabled = enabled;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_RATE_LIMITING_ENABLED '{}': {}", rate_limit_enabled, e));
                }
            }
        }

        if let Ok(requests_per_minute) = std::env::var("GC_RATE_LIMIT_REQUESTS_PER_MINUTE") {
            match requests_per_minute.parse() {
                Ok(rpm) => {
                    debug!("Found GC_RATE_LIMIT_REQUESTS_PER_MINUTE: {}", rpm);
                    config.security.rate_limiting.requests_per_minute = rpm;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_RATE_LIMIT_REQUESTS_PER_MINUTE '{}': {}", requests_per_minute, e));
                }
            }
        }

        if let Ok(burst_capacity) = std::env::var("GC_RATE_LIMIT_BURST_CAPACITY") {
            match burst_capacity.parse() {
                Ok(burst) => {
                    debug!("Found GC_RATE_LIMIT_BURST_CAPACITY: {}", burst);
                    config.security.rate_limiting.burst_capacity = burst;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_RATE_LIMIT_BURST_CAPACITY '{}': {}", burst_capacity, e));
                }
            }
        }

        // Security metrics
        if let Ok(security_metrics) = std::env::var("GC_INCLUDE_SECURITY_METRICS") {
            match security_metrics.parse() {
                Ok(enabled) => {
                    debug!("Found GC_INCLUDE_SECURITY_METRICS: {}", enabled);
                    config.metrics.include_security_metrics = enabled;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_INCLUDE_SECURITY_METRICS '{}': {}", security_metrics, e));
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
            "✅ Configuration loaded from environment in {:.3}s (mTLS: {}, enhanced security: {})",
            load_duration.as_secs_f64(),
            config.security.mtls.enabled,
            config.security.enhanced_security
        );

        Ok(config)
    }

    /// Validate configuration with mTLS security checks
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

    /// Perform detailed validation with security checks
    pub fn validate_detailed(&self) -> ValidationResult {
        let start_time = std::time::Instant::now();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Validate mTLS configuration
        if let Err(e) = self.security.mtls.validate() {
            errors.push(ValidationError {
                field: "security.mtls".to_string(),
                error_type: "mtls_validation_failed".to_string(),
                message: e.to_string(),
                suggested_fix: Some("Check certificate paths and mTLS configuration".to_string()),
            });
        }

        // Security configuration warnings
        if self.security.mtls.enabled && !self.security.enhanced_security {
            warnings.push(ValidationWarning {
                field: "security.enhanced_security".to_string(),
                message: "mTLS is enabled but enhanced security features are disabled".to_string(),
                recommendation: Some("Enable enhanced_security for better protection".to_string()),
            });
        }

        if !self.security.mtls.enabled {
            warnings.push(ValidationWarning {
                field: "security.mtls.enabled".to_string(),
                message: "mTLS is disabled - connections will use plain gRPC".to_string(),
                recommendation: Some("Enable mTLS for production deployments".to_string()),
            });
        }

        // Rate limiting validation
        if self.security.rate_limiting.enabled {
            if self.security.rate_limiting.requests_per_minute == 0 {
                errors.push(ValidationError {
                    field: "security.rate_limiting.requests_per_minute".to_string(),
                    error_type: "invalid_rate_limit".to_string(),
                    message: "Rate limiting enabled but requests_per_minute is 0".to_string(),
                    suggested_fix: Some("Set a positive value for requests_per_minute".to_string()),
                });
            }

            if self.security.rate_limiting.burst_capacity > self.security.rate_limiting.requests_per_minute {
                warnings.push(ValidationWarning {
                    field: "security.rate_limiting.burst_capacity".to_string(),
                    message: "Burst capacity exceeds requests per minute".to_string(),
                    recommendation: Some("Consider setting burst capacity <= requests_per_minute".to_string()),
                });
            }
        }

        // Connection limits validation
        if self.security.max_connections_per_client == 0 {
            errors.push(ValidationError {
                field: "security.max_connections_per_client".to_string(),
                error_type: "invalid_connection_limit".to_string(),
                message: "Maximum connections per client cannot be 0".to_string(),
                suggested_fix: Some("Set a positive value for max_connections_per_client".to_string()),
            });
        }

        // Validate port conflicts (including security considerations)
        if self.server.port == self.metrics.port {
            errors.push(ValidationError {
                field: "server.port".to_string(),
                error_type: "port_conflict".to_string(),
                message: "Server port and metrics port cannot be the same".to_string(),
                suggested_fix: Some("Use different ports for server and metrics".to_string()),
            });
        }

        // Add other existing validation logic...

        let duration = start_time.elapsed();

        ValidationResult {
            success: errors.is_empty(),
            errors,
            warnings,
            duration,
        }
    }

    /// Get configuration summary with security information
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
            security_features: SecurityFeatures {
                mtls_enabled: self.security.mtls.enabled,
                client_auth_required: self.security.mtls.require_client_auth,
                enhanced_security: self.security.enhanced_security,
                rate_limiting_enabled: self.security.rate_limiting.enabled,
                max_connections_per_client: self.security.max_connections_per_client,
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

    // Keep existing helper methods...
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

/// Configuration summary for logging and metrics with security features
#[derive(Debug, Clone, Serialize)]
pub struct ConfigSummary {
    pub server_endpoint: String,
    pub storage_backend: String,
    pub storage_features: StorageFeatures,
    pub security_features: SecurityFeatures,
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

#[derive(Debug, Clone, Serialize)]
pub struct SecurityFeatures {
    pub mtls_enabled: bool,
    pub client_auth_required: bool,
    pub enhanced_security: bool,
    pub rate_limiting_enabled: bool,
    pub max_connections_per_client: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_with_security() {
        let config = Config::default();
        let result = config.validate_detailed();
        
        // Should pass validation but may have warnings about disabled mTLS
        assert!(result.success, "Default config should be valid: {:?}", result.errors);
        
        // Check security defaults
        assert!(!config.security.mtls.enabled); // mTLS disabled by default
        assert!(config.security.enhanced_security);
        assert!(config.security.rate_limiting.enabled);
    }

    #[test]
    fn test_config_summary_with_security() {
        let config = Config::default();
        let summary = config.summary();
        
        assert_eq!(summary.storage_backend, "memory");
        assert!(!summary.security_features.mtls_enabled);
        assert!(summary.security_features.enhanced_security);
        assert!(summary.security_features.rate_limiting_enabled);
    }

    #[test]
    fn test_security_validation() {
        let mut config = Config::default();
        
        // Test invalid rate limiting configuration
        config.security.rate_limiting.enabled = true;
        config.security.rate_limiting.requests_per_minute = 0;
        
        let result = config.validate_detailed();
        assert!(!result.success);
        assert!(result.errors.iter().any(|e| e.error_type == "invalid_rate_limit"));
    }

    #[test]
    fn test_security_warnings() {
        let mut config = Config::default();
        
        // Enable mTLS but disable enhanced security
        config.security.mtls.enabled = true;
        config.security.enhanced_security = false;
        
        let result = config.validate_detailed();
        // Should have warnings about security configuration
        assert!(!result.warnings.is_empty());
    }
}