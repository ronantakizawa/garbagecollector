// src/startup.rs - Application startup coordination

use anyhow::Result;
use std::net::SocketAddr;
use std::time::Instant;
use tonic::transport::Server;
use tracing::{info, error};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Config;
use crate::service::GCService;
use crate::metrics::MetricsInterceptor;
use crate::shutdown::{ShutdownCoordinator, ShutdownConfig, TaskType, TaskPriority, ShutdownReason};
use crate::dependencies::DependencyChecker;
use crate::monitoring::SystemMonitor;

/// Application startup coordinator
pub struct ApplicationStartup {
    config: Config,
    shutdown_coordinator: ShutdownCoordinator,
    startup_start: Instant,
}

impl ApplicationStartup {
    /// Create a new application startup coordinator
    pub async fn new() -> Result<Self> {
        let startup_start = Instant::now();
        
        // Initialize tracing first
        let tracing_start = Instant::now();
        Self::initialize_tracing();
        let tracing_duration = tracing_start.elapsed();

        info!("🚛 Starting GarbageTruck - Lease-based Garbage Collection Service");

        // Initialize shutdown coordinator
        let shutdown_config = ShutdownConfig::default();
        let shutdown_coordinator = ShutdownCoordinator::new(shutdown_config);
        
        // Start listening for shutdown signals
        shutdown_coordinator.listen_for_signals().await;

        // Load and validate configuration
        let config_start = Instant::now();
        let config = Self::load_and_validate_config().await?;
        let config_duration = config_start.elapsed();

        info!("📋 Loaded configuration: {:?}", config);
        info!("📊 Initialization timing:");
        info!("   • Tracing init: {:.3}s", tracing_duration.as_secs_f64());
        info!("   • Config load/validation: {:.3}s", config_duration.as_secs_f64());

        Ok(Self {
            config,
            shutdown_coordinator,
            startup_start,
        })
    }

    /// Run the complete application lifecycle
    pub async fn run(self) -> Result<()> {
        // Phase 1: Create the service with metrics
        let service_creation_start = Instant::now();
        let gc_service = match GCService::new(self.config.clone()).await {
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
        metrics.record_startup_phase("service_creation", service_creation_duration);
        
        // Phase 2: Initialize background tasks with shutdown coordination
        let component_init_start = Instant::now();
        
        // Start system monitoring
        let system_monitor = SystemMonitor::new(metrics.clone());
        let monitoring_handle = self.shutdown_coordinator.register_task(
            "system_monitoring".to_string(),
            TaskType::SystemMonitor,
            TaskPriority::High,
        ).await;
        
        let monitoring_task = system_monitor.start_with_shutdown(monitoring_handle.clone()).await;
        self.shutdown_coordinator.update_task_handle("system_monitoring", monitoring_task).await;
        
        // Start cleanup task
        let cleanup_handle = self.shutdown_coordinator.register_task(
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
        
        self.shutdown_coordinator.update_task_handle("cleanup_loop", cleanup_task).await;
        
        // Start alerting monitor
        let alerting_handle = self.shutdown_coordinator.register_task(
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
        
        self.shutdown_coordinator.update_task_handle("alerting_monitor", alerting_task).await;
        let component_init_duration = component_init_start.elapsed();
        metrics.record_startup_phase("component_init", component_init_duration);

        // Phase 3: Check dependencies
        let deps_check_start = Instant::now();
        let dependency_checker = DependencyChecker::new(&self.config, &metrics);
        match dependency_checker.check_all_dependencies().await {
            Ok(_) => info!("✅ All dependencies are healthy"),
            Err(e) => {
                error!("❌ Dependency check failed: {}", e);
                metrics.record_startup_error("dependency_check", "dependency_unavailable");
                
                // Initiate shutdown on critical startup failure
                self.shutdown_coordinator.initiate_shutdown(ShutdownReason::Critical(
                    format!("Dependency check failed: {}", e)
                )).await;
                return Err(e);
            }
        }
        let deps_check_duration = deps_check_start.elapsed();
        metrics.record_dependencies_check(deps_check_duration);

        // Phase 4: Start gRPC server
        let server_start_time = Instant::now();
        let addr: SocketAddr = format!("{}:{}", self.config.server.host, self.config.server.port)
            .parse()
            .expect("Invalid server address");

        info!("🚀 Starting GarbageTruck gRPC server on {}", addr);

        // Create the gRPC service with metrics interceptor
        let grpc_service = crate::proto::distributed_gc_service_server::DistributedGcServiceServer::new(gc_service);
        
        // Create metrics interceptor
        let metrics_interceptor = MetricsInterceptor::new(metrics.clone());
        
        // Apply interceptor to service
        let intercepted_service = tonic::service::interceptor::InterceptedService::new(
            grpc_service,
            metrics_interceptor,
        );

        // Register gRPC server task
        let grpc_handle = self.shutdown_coordinator.register_task(
            "grpc_server".to_string(),
            TaskType::GrpcServer,
            TaskPriority::Low, // Shutdown servers last
        ).await;

        let server = Server::builder()
            .add_service(intercepted_service);

        let server_setup_duration = server_start_time.elapsed();
        metrics.record_startup_phase("server_setup", server_setup_duration);

        // Record total startup time and mark service as ready
        let total_startup_duration = self.startup_start.elapsed();
        metrics.record_startup_duration(total_startup_duration);
        metrics.record_service_ready();

        info!(
            "✅ GarbageTruck startup completed in {:.2}s",
            total_startup_duration.as_secs_f64()
        );
        
        // Print startup phase breakdown
        self.log_startup_breakdown(
            total_startup_duration,
            service_creation_duration,
            component_init_duration,
            deps_check_duration,
            server_setup_duration,
        );

        // Start the gRPC server with graceful shutdown handling
        let shutdown_rx = self.shutdown_coordinator.subscribe_to_shutdown();
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
        
        self.shutdown_coordinator.update_task_handle("grpc_server", grpc_server_task).await;

        // Main event loop - wait for shutdown
        self.wait_for_shutdown().await;

        // Get final shutdown statistics
        if let Some(stats) = self.shutdown_coordinator.get_shutdown_stats().await {
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

    /// Initialize tracing/logging
    fn initialize_tracing() {
        tracing_subscriber::registry()
            .with(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "garbagetruck=debug,tower=debug".into()),
            )
            .with(tracing_subscriber::fmt::layer())
            .init();
    }

    /// Load and validate configuration with detailed error reporting
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

    /// Wait for shutdown signals
    async fn wait_for_shutdown(&self) {
        let mut shutdown_rx = self.shutdown_coordinator.subscribe_to_shutdown();
        
        tokio::select! {
            _ = shutdown_rx.recv() => {
                info!("🏁 Shutdown signal received, coordinating graceful shutdown");
            }
            _ = tokio::signal::ctrl_c() => {
                info!("🏁 Ctrl+C received, initiating graceful shutdown");
                self.shutdown_coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
            }
        }

        // Wait for shutdown completion
        while !self.shutdown_coordinator.is_shutdown_initiated().await {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }

    /// Log startup phase breakdown
    fn log_startup_breakdown(
        &self,
        total_startup_duration: std::time::Duration,
        service_creation_duration: std::time::Duration,
        component_init_duration: std::time::Duration,
        deps_check_duration: std::time::Duration,
        server_setup_duration: std::time::Duration,
    ) {
        info!("📊 Startup phase breakdown:");
        info!("   • Service creation: {:.3}s", service_creation_duration.as_secs_f64());
        info!("   • Component init: {:.3}s", component_init_duration.as_secs_f64());
        info!("   • Dependencies check: {:.3}s", deps_check_duration.as_secs_f64());
        info!("   • Server setup: {:.3}s", server_setup_duration.as_secs_f64());
        info!("   • Total: {:.3}s", total_startup_duration.as_secs_f64());
    }
}