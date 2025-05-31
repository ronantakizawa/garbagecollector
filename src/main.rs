// src/main.rs - Enhanced with graceful shutdown management

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
mod shutdown;

// Client module (optional)
#[cfg(feature = "client")]
mod client;

use config::Config;
use service::GCService;
use metrics::{MetricsInterceptor, start_system_monitoring};
use shutdown::{ShutdownCoordinator, ShutdownConfig, TaskType, TaskPriority, ShutdownReason};

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

    // Initialize shutdown coordinator
    let shutdown_config = ShutdownConfig::default();
    let shutdown_coordinator = ShutdownCoordinator::new(shutdown_config);
    
    // Start listening for shutdown signals
    shutdown_coordinator.listen_for_signals().await;

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
    
    // Phase 3: Initialize background tasks with shutdown coordination
    let component_init_start = Instant::now();
    
    // Register and start system monitoring task
    let monitoring_start = Instant::now();
    let system_monitor_handle = shutdown_coordinator.register_task(
        "system_monitoring".to_string(),
        TaskType::SystemMonitor,
        TaskPriority::High,
    ).await;
    
    let system_monitor_task = {
        let metrics = metrics.clone();
        let mut handle = system_monitor_handle.clone();
        
        tokio::spawn(async move {
            info!("🔍 Starting system monitoring task");
            
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Update memory usage and other system metrics
                        if let Ok(memory_bytes) = get_memory_usage() {
                            metrics.update_memory_usage(memory_bytes);
                        }
                        
                        // Check for performance issues and trigger alerts
                        check_system_health(&metrics).await;
                    }
                    _ = handle.wait_for_shutdown() => {
                        info!("🛑 System monitoring task shutting down gracefully");
                        handle.mark_completed().await;
                        break;
                    }
                }
            }
        })
    };
    
    shutdown_coordinator.update_task_handle("system_monitoring", system_monitor_task).await;
    let monitoring_duration = monitoring_start.elapsed();
    metrics.record_component_initialization("system_monitoring", monitoring_duration);
    
    // Register and start cleanup task
    let cleanup_init_start = Instant::now();
    let cleanup_handle = shutdown_coordinator.register_task(
        "cleanup_loop".to_string(),
        TaskType::CleanupLoop,
        TaskPriority::Critical,
    ).await;
    
    let cleanup_task = {
        let gc_service = gc_service.clone();
        let mut handle = cleanup_handle.clone();
        
        tokio::spawn(async move {
            info!("🧹 Starting cleanup loop task");
            gc_service.start_cleanup_loop_with_shutdown(handle).await;
        })
    };
    
    shutdown_coordinator.update_task_handle("cleanup_loop", cleanup_task).await;
    let cleanup_init_duration = cleanup_init_start.elapsed();
    metrics.record_component_initialization("cleanup_service", cleanup_init_duration);
    
    // Register and start alerting monitor
    let alerting_start = Instant::now();
    let alerting_handle = shutdown_coordinator.register_task(
        "alerting_monitor".to_string(),
        TaskType::AlertingMonitor,
        TaskPriority::High,
    ).await;
    
    let alerting_task = {
        let metrics = metrics.clone();
        let mut handle = alerting_handle.clone();
        
        tokio::spawn(async move {
            info!("🚨 Starting alerting monitor task");
            
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Check cleanup failure rate and other metrics
                        metrics.check_cleanup_failure_rate().await;
                        
                        // Log current alert status
                        let summary = metrics.get_alert_summary().await;
                        if summary.total_active_alerts > 0 {
                            info!(
                                active_alerts = summary.total_active_alerts,
                                critical = summary.critical_alerts,
                                warnings = summary.warning_alerts,
                                fatal = summary.fatal_alerts,
                                "Alert status update"
                            );
                        }
                    }
                    _ = handle.wait_for_shutdown() => {
                        info!("🛑 Alerting monitor shutting down gracefully");
                        handle.mark_completed().await;
                        break;
                    }
                }
            }
        })
    };
    
    shutdown_coordinator.update_task_handle("alerting_monitor", alerting_task).await;
    let alerting_duration = alerting_start.elapsed();
    metrics.record_component_initialization("alerting_monitor", alerting_duration);
    
    let component_init_duration = component_init_start.elapsed();
    metrics.record_startup_phase("component_init", component_init_duration);

    // Phase 4: Check dependencies (storage, external services)
    let deps_check_start = Instant::now();
    match check_dependencies(&config, &metrics).await {
        Ok(_) => info!("✅ All dependencies are healthy"),
        Err(e) => {
            error!("❌ Dependency check failed: {}", e);
            metrics.record_startup_error("dependency_check", "dependency_unavailable");
            
            // Initiate shutdown on critical startup failure
            shutdown_coordinator.initiate_shutdown(ShutdownReason::Critical(
                format!("Dependency check failed: {}", e)
            )).await;
            return Err(e);
        }
    }
    let deps_check_duration = deps_check_start.elapsed();
    metrics.record_dependencies_check(deps_check_duration);

    // Phase 5: Start gRPC server with graceful shutdown
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

    // Register gRPC server task
    let grpc_handle = shutdown_coordinator.register_task(
        "grpc_server".to_string(),
        TaskType::GrpcServer,
        TaskPriority::Low, // Shutdown servers last
    ).await;

    let server = Server::builder()
        .add_service(intercepted_service);

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

    // Start the gRPC server with graceful shutdown handling
    let shutdown_rx = shutdown_coordinator.subscribe_to_shutdown();
    let grpc_server_task = {
        let mut handle = grpc_handle.clone();
        let mut shutdown_rx = shutdown_rx;
        
        tokio::spawn(async move {
            let server_future = server.serve(addr);
            
            tokio::select! {
                result = server_future => {
                    match result {
                        Ok(_) => {
                            info!("🏁 gRPC server completed normally");
                            handle.mark_completed().await;
                        }
                        Err(e) => {
                            error!("❌ gRPC server error: {}", e);
                            handle.mark_failed(format!("Server error: {}", e)).await;
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("🛑 gRPC server received shutdown signal");
                    // Server will be gracefully shut down by dropping
                    handle.mark_completed().await;
                }
                _ = handle.wait_for_shutdown() => {
                    info!("🛑 gRPC server shutting down gracefully");
                    handle.mark_completed().await;
                }
            }
        })
    };
    
    shutdown_coordinator.update_task_handle("grpc_server", grpc_server_task).await;

    // Main event loop - wait for shutdown
    let mut shutdown_rx = shutdown_coordinator.subscribe_to_shutdown();
    
    tokio::select! {
        _ = shutdown_rx.recv() => {
            info!("🏁 Shutdown signal received, coordinating graceful shutdown");
        }
        _ = tokio::signal::ctrl_c() => {
            info!("🏁 Ctrl+C received, initiating graceful shutdown");
            shutdown_coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
        }
    }

    // Wait for shutdown completion
    while !shutdown_coordinator.is_shutdown_initiated().await {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Get final shutdown statistics
    if let Some(stats) = shutdown_coordinator.get_shutdown_stats().await {
        info!("📊 Final shutdown statistics:");
        info!("   • Reason: {:?}", stats.reason);
        info!("   • Total tasks: {}", stats.total_tasks);
        info!("   • Completed: {}", stats.completed_tasks);
        info!("   • Failed: {}", stats.failed_tasks);
        info!("   • Force killed: {}", stats.forced_kills);
        info!("   • Duration: {:.2}s", stats.total_duration.as_secs_f64());
    }

    info!("🏁 GarbageTruck service shutdown complete");
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
            
            Ok(config)
        }
        Err(e) => {
            let validation_duration = validation_start.elapsed();
            error!(
                "❌ Configuration validation failed after {:.3}s: {}",
                validation_duration.as_secs_f64(),
                e
            );
            
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

/// Get memory usage for system monitoring
fn get_memory_usage() -> Result<f64, Box<dyn std::error::Error>> {
    // Simple implementation - in production you might use sysinfo crate
    #[cfg(target_os = "linux")]
    {
        use std::fs::read_to_string;
        let status = read_to_string("/proc/self/status")?;
        for line in status.lines() {
            if line.starts_with("VmRSS:") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 {
                    if let Ok(kb) = parts[1].parse::<f64>() {
                        return Ok(kb * 1024.0); // Convert KB to bytes
                    }
                }
            }
        }
    }
    
    // Fallback for other platforms or if parsing fails
    Ok(0.0)
}

/// Check system health and trigger alerts if necessary
async fn check_system_health(metrics: &std::sync::Arc<metrics::Metrics>) {
    // Check memory usage
    if let Ok(memory_bytes) = get_memory_usage() {
        const GB: f64 = 1024.0 * 1024.0 * 1024.0;
        if memory_bytes > 2.0 * GB {
            warn!("High memory usage detected: {:.2} GB", memory_bytes / GB);
        }
    }
    
    // Check request error rates
    let request_total = metrics.request_total.clone();
    let error_requests = request_total.with_label_values(&["create_lease", "error"]).get()
        + request_total.with_label_values(&["renew_lease", "error"]).get()
        + request_total.with_label_values(&["release_lease", "error"]).get();
    
    let total_requests = request_total.with_label_values(&["create_lease", "success"]).get()
        + request_total.with_label_values(&["create_lease", "error"]).get()
        + request_total.with_label_values(&["renew_lease", "success"]).get()
        + request_total.with_label_values(&["renew_lease", "error"]).get()
        + request_total.with_label_values(&["release_lease", "success"]).get()
        + request_total.with_label_values(&["release_lease", "error"]).get();
    
    // Alert on high error rates
    if total_requests > 10 && error_requests as f64 / total_requests as f64 > 0.1 {
        warn!("High gRPC error rate detected: {:.2}%", 
              error_requests as f64 / total_requests as f64 * 100.0);
    }
}