// src/startup.rs - Simplified application startup coordination

use anyhow::Result;
use std::net::SocketAddr;
use std::time::Instant;
use tonic::transport::Server;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Config;
use crate::dependencies::DependencyChecker;
use crate::metrics::MetricsInterceptor;
use crate::service::GCService;
use crate::shutdown::{
    ShutdownConfig, ShutdownCoordinator, ShutdownReason, TaskPriority, TaskType,
};

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
        Self::initialize_tracing();

        info!("🚛 Starting GarbageTruck - Lease-based Garbage Collection Service");

        // Initialize shutdown coordinator
        let shutdown_config = ShutdownConfig::default();
        let shutdown_coordinator = ShutdownCoordinator::new(shutdown_config);

        // Start listening for shutdown signals
        shutdown_coordinator.listen_for_signals().await;

        // Load and validate configuration
        let config = Self::load_and_validate_config().await?;
        info!("📋 Configuration loaded and validated");

        Ok(Self {
            config,
            shutdown_coordinator,
            startup_start,
        })
    }

    /// Run the complete application lifecycle
    pub async fn run(self) -> Result<()> {
        // Create the service
        let gc_service = match GCService::new(self.config.clone()).await {
            Ok(service) => service,
            Err(e) => {
                error!("❌ Failed to create GC service: {}", e);
                return Err(anyhow::anyhow!("Service creation failed: {}", e));
            }
        };

        // Get metrics reference
        let metrics = gc_service.get_metrics();

        // Check dependencies
        let dependency_checker = DependencyChecker::new(&self.config, &metrics);
        match dependency_checker.check_all_dependencies().await {
            Ok(_) => info!("✅ All dependencies are healthy"),
            Err(e) => {
                error!("❌ Dependency check failed: {}", e);
                self.shutdown_coordinator
                    .initiate_shutdown(ShutdownReason::Critical(format!(
                        "Dependency check failed: {}",
                        e
                    )))
                    .await;
                return Err(e);
            }
        }

        // Start cleanup task
        let cleanup_handle = self
            .shutdown_coordinator
            .register_task(
                "cleanup_loop".to_string(),
                TaskType::CleanupLoop,
                TaskPriority::Critical,
            )
            .await;

        let cleanup_task = {
            let gc_service = gc_service.clone();
            let handle = cleanup_handle.clone();

            tokio::spawn(async move {
                info!("🧹 Starting cleanup loop task");
                gc_service.start_cleanup_loop_with_shutdown(handle).await;
            })
        };

        self.shutdown_coordinator
            .update_task_handle("cleanup_loop", cleanup_task)
            .await;

        // Start gRPC server
        let addr: SocketAddr = format!("{}:{}", self.config.server.host, self.config.server.port)
            .parse()
            .expect("Invalid server address");

        info!("🚀 Starting GarbageTruck gRPC server on {}", addr);

        // Create the gRPC service with metrics interceptor
        let grpc_service =
            crate::proto::distributed_gc_service_server::DistributedGcServiceServer::new(
                gc_service,
            );

        // Create metrics interceptor
        let metrics_interceptor = MetricsInterceptor::new(metrics.clone());

        // Apply interceptor to service
        let intercepted_service =
            tonic::service::interceptor::InterceptedService::new(grpc_service, metrics_interceptor);

        // Register gRPC server task
        let grpc_handle = self
            .shutdown_coordinator
            .register_task(
                "grpc_server".to_string(),
                TaskType::GrpcServer,
                TaskPriority::Low, // Shutdown servers last
            )
            .await;

        let server = Server::builder().add_service(intercepted_service);

        // Record total startup time
        let total_startup_duration = self.startup_start.elapsed();
        info!(
            "✅ GarbageTruck startup completed in {:.2}s",
            total_startup_duration.as_secs_f64()
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
                        handle.mark_completed().await;
                    }
                    _ = handle.wait_for_shutdown() => {
                        info!("🛑 gRPC server shutting down gracefully");
                        handle.mark_completed().await;
                    }
                }
            })
        };

        self.shutdown_coordinator
            .update_task_handle("grpc_server", grpc_server_task)
            .await;

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

    /// Load and validate configuration
    async fn load_and_validate_config() -> Result<Config> {
        // Load configuration from environment
        let config = Config::from_env()?;

        // Validate configuration
        config.validate()?;

        info!("📋 Configuration summary:");
        info!("   • Server: {}:{}", config.server.host, config.server.port);
        info!("   • Storage backend: {}", config.storage.backend);
        info!(
            "   • Default lease duration: {}s",
            config.gc.default_lease_duration_seconds
        );
        info!(
            "   • Cleanup interval: {}s",
            config.gc.cleanup_interval_seconds
        );
        info!(
            "   • Max leases per service: {}",
            config.gc.max_leases_per_service
        );

        Ok(config)
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
}
