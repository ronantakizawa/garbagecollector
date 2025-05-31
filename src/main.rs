// src/main.rs - Enhanced with startup time and configuration validation metrics

use anyhow::Result;
use std::net::SocketAddr;
use std::time::Instant;
use tonic::transport::Server;
use tracing::{info, warn, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod error;
mod lease;
mod service;
mod storage;
mod cleanup;
mod metrics;

// Client module (optional)
#[cfg(feature = "client")]
mod client;

use config::Config;
use service::GCService;
use metrics::{MetricsInterceptor, start_system_monitoring};

// Include the generated protobuf code
pub mod proto {
    tonic::include_proto!("distributed_gc");
}

#[tokio::main]
async fn main() -> Result<()> {
    // Record the very start of startup
    let startup_start = Instant::now();
    
    // Initialize tracing first (before any metrics)
    let tracing_start = Instant::now();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "garbagetruck=debug,tower=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();
    let tracing_duration = tracing_start.elapsed();

    info!("🚛 Starting GarbageTruck - Lease-based Garbage Collection Service");

    // Phase 1: Load and validate configuration
    let config_start = Instant::now();
    let config = match load_and_validate_config().await {
        Ok(config) => config,
        Err(e) => {
            error!("❌ Failed to load configuration: {}", e);
            return Err(e);
        }
    };
    let config_duration = config_start.elapsed();

    info!("📋 Loaded configuration: {:?}", config);

    // Phase 2: Create the service with metrics
    let service_creation_start = Instant::now();
    let gc_service = match GCService::new(config.clone()).await {
        Ok(service) => service,
        Err(e) => {
            error!("❌ Failed to create GC service: {}", e);
            return Err(anyhow::anyhow!("Service creation failed: {}", e));
        }
    };
    let service_creation_duration = service_creation_start.elapsed();

    // Get metrics reference for recording startup metrics
    let metrics = gc_service.get_metrics();
    
    // Record initialization phases
    metrics.record_startup_phase("tracing_init", tracing_duration);
    metrics.record_startup_phase("config_load", config_duration);
    metrics.record_startup_phase("service_creation", service_creation_duration);
    
    // Phase 3: Initialize components
    let component_init_start = Instant::now();
    
    // Start system monitoring background task
    let monitoring_start = Instant::now();
    let _monitoring_handle = start_system_monitoring(metrics.clone());
    let monitoring_duration = monitoring_start.elapsed();
    metrics.record_component_initialization("system_monitoring", monitoring_duration);
    
    // Clone for the cleanup task
    let cleanup_gc_service = gc_service.clone();
    
    // Start the cleanup background task
    let cleanup_init_start = Instant::now();
    tokio::spawn(async move {
        cleanup_gc_service.start_cleanup_loop().await;
    });
    let cleanup_init_duration = cleanup_init_start.elapsed();
    metrics.record_component_initialization("cleanup_service", cleanup_init_duration);
    
    let component_init_duration = component_init_start.elapsed();
    metrics.record_startup_phase("component_init", component_init_duration);

    // Phase 4: Check dependencies (storage, external services)
    let deps_check_start = Instant::now();
    match check_dependencies(&config, &metrics).await {
        Ok(_) => info!("✅ All dependencies are healthy"),
        Err(e) => {
            error!("❌ Dependency check failed: {}", e);
            metrics.record_startup_error("dependency_check", "dependency_unavailable");
            return Err(e);
        }
    }
    let deps_check_duration = deps_check_start.elapsed();
    metrics.record_dependencies_check(deps_check_duration);

    // Phase 5: Start gRPC server
    let server_start_time = Instant::now();
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .expect("Invalid server address");

    info!("🚀 Starting GarbageTruck gRPC server on {}", addr);

    // Create the gRPC service with metrics interceptor
    let grpc_service = proto::distributed_gc_service_server::DistributedGcServiceServer::new(gc_service);
    
    // Create metrics interceptor
    let metrics_interceptor = MetricsInterceptor::new(metrics.clone());
    
    // Apply interceptor to service
    let intercepted_service = tonic::service::interceptor::InterceptedService::new(
        grpc_service,
        metrics_interceptor,
    );

    let server = Server::builder()
        .add_service(intercepted_service)
        .serve(addr);

    let server_setup_duration = server_start_time.elapsed();
    metrics.record_startup_phase("server_setup", server_setup_duration);

    // Record total startup time and mark service as ready
    let total_startup_duration = startup_start.elapsed();
    metrics.record_startup_duration(total_startup_duration);
    metrics.record_service_ready();

    info!(
        "✅ GarbageTruck startup completed in {:.2}s",
        total_startup_duration.as_secs_f64()
    );
    
    // Print startup phase breakdown
    info!("📊 Startup phase breakdown:");
    info!("   • Tracing init: {:.3}s", tracing_duration.as_secs_f64());
    info!("   • Config load/validation: {:.3}s", config_duration.as_secs_f64());
    info!("   • Service creation: {:.3}s", service_creation_duration.as_secs_f64());
    info!("   • Component init: {:.3}s", component_init_duration.as_secs_f64());
    info!("   • Dependencies check: {:.3}s", deps_check_duration.as_secs_f64());
    info!("   • Server setup: {:.3}s", server_setup_duration.as_secs_f64());

    // Graceful shutdown
    tokio::select! {
        result = server => {
            if let Err(e) = result {
                warn!("❌ gRPC server error: {}", e);
                metrics.record_startup_error("server_runtime", "server_error");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("🛑 Received shutdown signal");
        }
    }

    info!("🏁 Shutting down GarbageTruck service");
    Ok(())
}

/// Load and validate configuration with detailed metrics
async fn load_and_validate_config() -> Result<Config> {
    let validation_start = Instant::now();
    
    // Try to load configuration from environment
    let config_load_start = Instant::now();
    let config = match Config::from_env() {
        Ok(config) => {
            let load_duration = config_load_start.elapsed();
            info!("✅ Configuration loaded from environment in {:.3}s", load_duration.as_secs_f64());
            config
        }
        Err(e) => {
            let load_duration = config_load_start.elapsed();
            error!("❌ Failed to load configuration from environment: {}", e);
            
            // We don't have metrics yet, so we'll return the error
            // The metrics will be recorded later if we have a fallback
            return Err(anyhow::anyhow!("Configuration load failed: {}", e));
        }
    };
    
    // Validate configuration
    let validation_check_start = Instant::now();
    match config.validate() {
        Ok(_) => {
            let validation_duration = validation_start.elapsed();
            let check_duration = validation_check_start.elapsed();
            
            info!("✅ Configuration validation passed in {:.3}s", check_duration.as_secs_f64());
            info!("📋 Configuration details:");
            info!("   • Server: {}:{}", config.server.host, config.server.port);
            info!("   • Storage backend: {}", config.storage.backend);
            info!("   • Default lease duration: {}s", config.gc.default_lease_duration_seconds);
            info!("   • Cleanup interval: {}s", config.gc.cleanup_interval_seconds);
            info!("   • Max leases per service: {}", config.gc.max_leases_per_service);
            
            // Note: We can't record metrics here because we don't have the metrics instance yet
            // The service creation will record these validation metrics retrospectively
            
            Ok(config)
        }
        Err(e) => {
            let validation_duration = validation_start.elapsed();
            error!(
                "❌ Configuration validation failed after {:.3}s: {}",
                validation_duration.as_secs_f64(),
                e
            );
            
            // Categorize the validation error for better metrics
            let error_type = categorize_config_error(&e.to_string());
            error!("Configuration error type: {}", error_type);
            
            Err(anyhow::anyhow!("Configuration validation failed: {}", e))
        }
    }
}

/// Check external dependencies and their health
async fn check_dependencies(config: &Config, metrics: &std::sync::Arc<metrics::Metrics>) -> Result<()> {
    info!("🔍 Checking external dependencies...");
    
    // Check storage backend availability
    let storage_check_start = Instant::now();
    match &config.storage.backend.as_str() {
        &"memory" => {
            // Memory storage is always available
            let duration = storage_check_start.elapsed();
            metrics.record_component_initialization("storage_memory", duration);
            info!("✅ Memory storage backend ready");
        }
        &"postgres" => {
            #[cfg(feature = "postgres")]
            {
                if let Some(ref database_url) = config.storage.database_url {
                    match check_postgres_connection(database_url).await {
                        Ok(_) => {
                            let duration = storage_check_start.elapsed();
                            metrics.record_component_initialization("storage_postgres", duration);
                            info!("✅ PostgreSQL storage backend ready");
                        }
                        Err(e) => {
                            let duration = storage_check_start.elapsed();
                            error!("❌ PostgreSQL connection failed after {:.3}s: {}", duration.as_secs_f64(), e);
                            metrics.record_startup_error("dependency_check", "postgres_connection_failed");
                            return Err(anyhow::anyhow!("PostgreSQL connection failed: {}", e));
                        }
                    }
                } else {
                    error!("❌ PostgreSQL backend selected but no database URL provided");
                    metrics.record_startup_error("dependency_check", "postgres_no_url");
                    return Err(anyhow::anyhow!("PostgreSQL database URL not configured"));
                }
            }
            #[cfg(not(feature = "postgres"))]
            {
                error!("❌ PostgreSQL backend selected but postgres feature not enabled");
                metrics.record_startup_error("dependency_check", "postgres_feature_disabled");
                return Err(anyhow::anyhow!("PostgreSQL support not compiled in"));
            }
        }
        backend => {
            error!("❌ Unknown storage backend: {}", backend);
            metrics.record_startup_error("dependency_check", "unknown_storage_backend");
            return Err(anyhow::anyhow!("Unknown storage backend: {}", backend));
        }
    }
    
    // Check if metrics port is available (if metrics are enabled)
    if config.metrics.enabled {
        let metrics_check_start = Instant::now();
        match check_port_availability(config.metrics.port).await {
            Ok(_) => {
                let duration = metrics_check_start.elapsed();
                metrics.record_component_initialization("metrics_port", duration);
                info!("✅ Metrics port {} is available", config.metrics.port);
            }
            Err(e) => {
                let duration = metrics_check_start.elapsed();
                warn!("⚠️  Metrics port {} check failed after {:.3}s: {}", 
                      config.metrics.port, duration.as_secs_f64(), e);
                // This is not a fatal error, just log it
            }
        }
    }
    
    Ok(())
}

/// Check PostgreSQL connection
#[cfg(feature = "postgres")]
async fn check_postgres_connection(database_url: &str) -> Result<()> {
    use sqlx::PgPool;
    
    info!("🐘 Testing PostgreSQL connection...");
    
    let pool = PgPool::connect(database_url)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to PostgreSQL: {}", e))?;
    
    // Test with a simple query
    sqlx::query("SELECT 1")
        .fetch_one(&pool)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to execute test query: {}", e))?;
    
    pool.close().await;
    info!("✅ PostgreSQL connection test successful");
    
    Ok(())
}

/// Check if a port is available for binding
async fn check_port_availability(port: u16) -> Result<()> {
    use tokio::net::TcpListener;
    
    let addr = format!("127.0.0.1:{}", port);
    match TcpListener::bind(&addr).await {
        Ok(_) => Ok(()),
        Err(e) => Err(anyhow::anyhow!("Port {} is not available: {}", port, e)),
    }
}

/// Categorize configuration errors for better metrics
fn categorize_config_error(error_message: &str) -> &'static str {
    let error_lower = error_message.to_lowercase();
    
    if error_lower.contains("duration") {
        "invalid_duration"
    } else if error_lower.contains("database") || error_lower.contains("url") {
        "database_config"
    } else if error_lower.contains("port") || error_lower.contains("host") {
        "network_config"
    } else if error_lower.contains("storage") {
        "storage_config"
    } else if error_lower.contains("missing") || error_lower.contains("required") {
        "missing_required"
    } else if error_lower.contains("invalid") {
        "invalid_value"
    } else {
        "unknown"
    }
}

/// Record configuration validation metrics retrospectively
/// This is called after the service is created and metrics are available
pub fn record_config_validation_metrics(
    metrics: &std::sync::Arc<metrics::Metrics>,
    config: &Config,
    validation_duration: std::time::Duration,
    success: bool,
    error_type: Option<&str>,
) {
    metrics.record_config_validation(success, validation_duration, error_type);
    
    if success {
        // Record successful configuration details as metrics
        info!("📊 Recording configuration metrics");
        
        // You could add custom metrics for specific config values here
        // For example:
        metrics.record_component_initialization("config_parsing", validation_duration);
        
        // Log configuration fingerprint for debugging
        let config_fingerprint = format!(
            "backend:{},lease_dur:{},cleanup_int:{},max_leases:{}",
            config.storage.backend,
            config.gc.default_lease_duration_seconds,
            config.gc.cleanup_interval_seconds,
            config.gc.max_leases_per_service
        );
        info!("🔍 Configuration fingerprint: {}", config_fingerprint);
    }
}