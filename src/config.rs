use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;

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
    /// Storage backend type: "memory" or "postgres"
    pub backend: String,
    
    /// Database connection string (if using postgres)
    pub database_url: Option<String>,
    
    /// Maximum number of database connections
    pub max_connections: Option<u32>,
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

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 50051,
            },
            gc: GCConfig {
                default_lease_duration_seconds: 300, // 5 minutes
                max_lease_duration_seconds: 3600,   // 1 hour
                min_lease_duration_seconds: 30,     // 30 seconds
                cleanup_interval_seconds: 60,       // 1 minute
                cleanup_grace_period_seconds: 30,   // 30 seconds
                max_leases_per_service: 10000,
                max_concurrent_cleanups: 10,
            },
            storage: StorageConfig {
                backend: "memory".to_string(),
                database_url: None,
                max_connections: Some(10),
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
    pub fn from_env() -> Result<Self> {
        let mut config = Config::default();
        
        // Server configuration
        if let Ok(host) = std::env::var("GC_SERVER_HOST") {
            config.server.host = host;
        }
        if let Ok(port) = std::env::var("GC_SERVER_PORT") {
            config.server.port = port.parse()?;
        }
        
        // GC configuration
        if let Ok(duration) = std::env::var("GC_DEFAULT_LEASE_DURATION") {
            config.gc.default_lease_duration_seconds = duration.parse()?;
        }
        if let Ok(max_duration) = std::env::var("GC_MAX_LEASE_DURATION") {
            config.gc.max_lease_duration_seconds = max_duration.parse()?;
        }
        if let Ok(min_duration) = std::env::var("GC_MIN_LEASE_DURATION") {
            config.gc.min_lease_duration_seconds = min_duration.parse()?;
        }
        if let Ok(interval) = std::env::var("GC_CLEANUP_INTERVAL") {
            config.gc.cleanup_interval_seconds = interval.parse()?;
        }
        if let Ok(grace) = std::env::var("GC_CLEANUP_GRACE_PERIOD") {
            config.gc.cleanup_grace_period_seconds = grace.parse()?;
        }
        if let Ok(max_leases) = std::env::var("GC_MAX_LEASES_PER_SERVICE") {
            config.gc.max_leases_per_service = max_leases.parse()?;
        }
        
        // Storage configuration
        if let Ok(backend) = std::env::var("GC_STORAGE_BACKEND") {
            config.storage.backend = backend;
        }
        if let Ok(db_url) = std::env::var("DATABASE_URL") {
            config.storage.database_url = Some(db_url);
        }
        if let Ok(max_conn) = std::env::var("GC_MAX_DB_CONNECTIONS") {
            config.storage.max_connections = Some(max_conn.parse()?);
        }
        
        // Cleanup configuration
        if let Ok(timeout) = std::env::var("GC_CLEANUP_TIMEOUT") {
            config.cleanup.default_timeout_seconds = timeout.parse()?;
        }
        if let Ok(retries) = std::env::var("GC_CLEANUP_MAX_RETRIES") {
            config.cleanup.default_max_retries = retries.parse()?;
        }
        if let Ok(delay) = std::env::var("GC_CLEANUP_RETRY_DELAY") {
            config.cleanup.default_retry_delay_seconds = delay.parse()?;
        }
        
        // Metrics configuration
        if let Ok(enabled) = std::env::var("GC_METRICS_ENABLED") {
            config.metrics.enabled = enabled.parse().unwrap_or(true);
        }
        if let Ok(port) = std::env::var("GC_METRICS_PORT") {
            config.metrics.port = port.parse()?;
        }
        
        Ok(config)
    }
    
    pub fn validate(&self) -> Result<()> {
        if self.gc.min_lease_duration_seconds >= self.gc.max_lease_duration_seconds {
            anyhow::bail!("Min lease duration must be less than max lease duration");
        }
        
        if self.gc.default_lease_duration_seconds < self.gc.min_lease_duration_seconds
            || self.gc.default_lease_duration_seconds > self.gc.max_lease_duration_seconds
        {
            anyhow::bail!("Default lease duration must be between min and max lease duration");
        }
        
        if self.storage.backend == "postgres" && self.storage.database_url.is_none() {
            anyhow::bail!("Database URL is required when using postgres backend");
        }
        
        Ok(())
    }
    
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