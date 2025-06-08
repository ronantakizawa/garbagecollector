// src/config.rs - Simplified configuration without PostgreSQL

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub gc: GCConfig,
    pub storage: StorageConfig,
    pub cleanup: CleanupConfig,
    pub metrics: MetricsConfig,
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
    /// Storage backend type: only "memory" is supported
    pub backend: String,
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
            },
        }
    }
}

impl Config {
    /// Load configuration from environment variables
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

        // GC configuration
        if let Ok(duration) = std::env::var("GC_DEFAULT_LEASE_DURATION") {
            match duration.parse() {
                Ok(d) => {
                    debug!("Found GC_DEFAULT_LEASE_DURATION: {}", d);
                    config.gc.default_lease_duration_seconds = d;
                }
                Err(e) => {
                    parse_errors.push(format!(
                        "Invalid GC_DEFAULT_LEASE_DURATION '{}': {}",
                        duration, e
                    ));
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
                    parse_errors.push(format!(
                        "Invalid GC_MAX_LEASE_DURATION '{}': {}",
                        max_duration, e
                    ));
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
                    parse_errors.push(format!(
                        "Invalid GC_MIN_LEASE_DURATION '{}': {}",
                        min_duration, e
                    ));
                }
            }
        }

        if let Ok(interval) = std::env::var("GC_CLEANUP_INTERVAL") {
            match interval.parse() {
                Ok(i) => {
                    debug!("Found GC_CLEANUP_INTERVAL: {}", i);
                    config.gc.cleanup_interval_seconds = i;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_CLEANUP_INTERVAL '{}': {}", interval, e));
                }
            }
        }

        if let Ok(grace) = std::env::var("GC_CLEANUP_GRACE_PERIOD") {
            match grace.parse() {
                Ok(g) => {
                    debug!("Found GC_CLEANUP_GRACE_PERIOD: {}", g);
                    config.gc.cleanup_grace_period_seconds = g;
                }
                Err(e) => {
                    parse_errors.push(format!(
                        "Invalid GC_CLEANUP_GRACE_PERIOD '{}': {}",
                        grace, e
                    ));
                }
            }
        }

        if let Ok(max_leases) = std::env::var("GC_MAX_LEASES_PER_SERVICE") {
            match max_leases.parse() {
                Ok(m) => {
                    debug!("Found GC_MAX_LEASES_PER_SERVICE: {}", m);
                    config.gc.max_leases_per_service = m;
                }
                Err(e) => {
                    parse_errors.push(format!(
                        "Invalid GC_MAX_LEASES_PER_SERVICE '{}': {}",
                        max_leases, e
                    ));
                }
            }
        }

        // Storage configuration - only memory backend supported
        if let Ok(backend) = std::env::var("GC_STORAGE_BACKEND") {
            if backend != "memory" {
                parse_errors.push(format!(
                    "Unsupported storage backend '{}': only 'memory' is supported",
                    backend
                ));
            } else {
                debug!("Found GC_STORAGE_BACKEND: {}", backend);
                config.storage.backend = backend;
            }
        }

        // Cleanup configuration
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

        if let Ok(retries) = std::env::var("GC_CLEANUP_MAX_RETRIES") {
            match retries.parse() {
                Ok(r) => {
                    debug!("Found GC_CLEANUP_MAX_RETRIES: {}", r);
                    config.cleanup.default_max_retries = r;
                }
                Err(e) => {
                    parse_errors.push(format!(
                        "Invalid GC_CLEANUP_MAX_RETRIES '{}': {}",
                        retries, e
                    ));
                }
            }
        }

        if let Ok(delay) = std::env::var("GC_CLEANUP_RETRY_DELAY") {
            match delay.parse() {
                Ok(d) => {
                    debug!("Found GC_CLEANUP_RETRY_DELAY: {}", d);
                    config.cleanup.default_retry_delay_seconds = d;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_CLEANUP_RETRY_DELAY '{}': {}", delay, e));
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

        if let Ok(port) = std::env::var("GC_METRICS_PORT") {
            match port.parse() {
                Ok(p) => {
                    debug!("Found GC_METRICS_PORT: {}", p);
                    config.metrics.port = p;
                }
                Err(e) => {
                    parse_errors.push(format!("Invalid GC_METRICS_PORT '{}': {}", port, e));
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
            "✅ Configuration loaded from environment in {:.3}s",
            load_duration.as_secs_f64()
        );

        Ok(config)
    }

    /// Validate configuration with detailed error reporting
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

    /// Perform detailed validation with structured error reporting
    pub fn validate_detailed(&self) -> ValidationResult {
        let start_time = std::time::Instant::now();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Validate lease duration relationships
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

        if self.gc.default_lease_duration_seconds < self.gc.min_lease_duration_seconds
            || self.gc.default_lease_duration_seconds > self.gc.max_lease_duration_seconds
        {
            errors.push(ValidationError {
                field: "gc.default_lease_duration_seconds".to_string(),
                error_type: "out_of_range".to_string(),
                message: format!(
                    "Default lease duration ({}) must be between min ({}) and max ({})",
                    self.gc.default_lease_duration_seconds,
                    self.gc.min_lease_duration_seconds,
                    self.gc.max_lease_duration_seconds
                ),
                suggested_fix: Some(format!(
                    "Set default_lease_duration between {} and {}",
                    self.gc.min_lease_duration_seconds, self.gc.max_lease_duration_seconds
                )),
            });
        }

        // Validate storage configuration
        if self.storage.backend != "memory" {
            errors.push(ValidationError {
                field: "storage.backend".to_string(),
                error_type: "unsupported_backend".to_string(),
                message: format!("Unsupported storage backend: {}", self.storage.backend),
                suggested_fix: Some("Set storage backend to 'memory'".to_string()),
            });
        }

        // Validate server configuration
        if self.server.port < 1024 {
            warnings.push(ValidationWarning {
                field: "server.port".to_string(),
                message: format!("Port {} is a privileged port", self.server.port),
                recommendation: Some(
                    "Consider using a port >= 1024 for non-root execution".to_string(),
                ),
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

        // Validate cleanup configuration
        if self.gc.cleanup_interval_seconds < 10 {
            warnings.push(ValidationWarning {
                field: "gc.cleanup_interval_seconds".to_string(),
                message: "Very short cleanup interval may impact performance".to_string(),
                recommendation: Some(
                    "Consider using an interval of at least 30 seconds".to_string(),
                ),
            });
        }

        if self.gc.cleanup_grace_period_seconds > self.gc.cleanup_interval_seconds {
            warnings.push(ValidationWarning {
                field: "gc.cleanup_grace_period_seconds".to_string(),
                message: "Grace period is longer than cleanup interval".to_string(),
                recommendation: Some(
                    "Consider setting grace period <= cleanup interval".to_string(),
                ),
            });
        }

        // Validate resource limits
        if self.gc.max_leases_per_service > 100000 {
            warnings.push(ValidationWarning {
                field: "gc.max_leases_per_service".to_string(),
                message: "Very high lease limit may impact memory usage".to_string(),
                recommendation: Some("Monitor memory usage with high lease counts".to_string()),
            });
        }

        // Validate cleanup timeouts
        if self.cleanup.default_timeout_seconds > 300 {
            warnings.push(ValidationWarning {
                field: "cleanup.default_timeout_seconds".to_string(),
                message: "Long cleanup timeout may delay lease processing".to_string(),
                recommendation: Some(
                    "Consider shorter timeouts for better responsiveness".to_string(),
                ),
            });
        }

        let duration = start_time.elapsed();

        ValidationResult {
            success: errors.is_empty(),
            errors,
            warnings,
            duration,
        }
    }

    /// Get configuration as a summary for logging
    pub fn summary(&self) -> ConfigSummary {
        ConfigSummary {
            server_endpoint: format!("{}:{}", self.server.host, self.server.port),
            storage_backend: self.storage.backend.clone(),
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

    // Helper methods for duration conversion
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

/// Configuration summary for logging and metrics
#[derive(Debug, Clone, Serialize)]
pub struct ConfigSummary {
    pub server_endpoint: String,
    pub storage_backend: String,
    pub default_lease_duration: u64,
    pub cleanup_interval: u64,
    pub max_leases_per_service: usize,
    pub metrics_enabled: bool,
    pub metrics_port: Option<u16>,
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
    fn test_invalid_lease_duration_ranges() {
        let mut config = Config::default();
        config.gc.min_lease_duration_seconds = 100;
        config.gc.max_lease_duration_seconds = 50; // Invalid: min > max

        let result = config.validate_detailed();
        assert!(!result.success);
        assert!(!result.errors.is_empty());
        assert!(result.errors[0].error_type == "invalid_range");
    }

    #[test]
    fn test_unsupported_backend() {
        let mut config = Config::default();
        config.storage.backend = "postgres".to_string();

        let result = config.validate_detailed();
        assert!(!result.success);
        assert!(result
            .errors
            .iter()
            .any(|e| e.error_type == "unsupported_backend"));
    }

    #[test]
    fn test_port_conflict() {
        let mut config = Config::default();
        config.server.port = 9090;
        config.metrics.port = 9090; // Same port

        let result = config.validate_detailed();
        assert!(!result.success);
        assert!(result
            .errors
            .iter()
            .any(|e| e.error_type == "port_conflict"));
    }

    #[test]
    fn test_config_warnings() {
        let mut config = Config::default();
        config.server.port = 80; // Privileged port
        config.gc.cleanup_interval_seconds = 5; // Very short interval

        let result = config.validate_detailed();
        assert!(result.success); // Should still be valid
        assert!(!result.warnings.is_empty()); // But should have warnings
    }
}
