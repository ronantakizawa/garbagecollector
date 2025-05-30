use anyhow::Result;
use std::net::SocketAddr;
use tonic::transport::Server;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod config;
mod error;
mod lease;
mod service;
mod storage;
mod cleanup;
mod metrics;

use config::Config;
use service::GCService;
use metrics::{MetricsInterceptor, start_system_monitoring};

// Include the generated protobuf code
pub mod proto {
    tonic::include_proto!("distributed_gc");
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "distributed_gc_sidecar=debug,tower=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Distributed GC Sidecar Service");

    // Load configuration
    let config = Config::from_env()?;
    info!("Loaded configuration: {:?}", config);

    // Create the service
    let gc_service = GCService::new(config.clone()).await?;
    
    // Get metrics reference for middleware
    let metrics = gc_service.get_metrics();
    
    // Start system monitoring background task
    let _monitoring_handle = start_system_monitoring(metrics.clone());
    
    // Clone for the cleanup task
    let cleanup_gc_service = gc_service.clone();
    
    // Start the cleanup background task
    tokio::spawn(async move {
        cleanup_gc_service.start_cleanup_loop().await;
    });

    // Start gRPC server with interceptor
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .expect("Invalid server address");

    info!("Starting gRPC server on {}", addr);

    // Create the gRPC service with metrics interceptor
    let grpc_service = proto::distributed_gc_service_server::DistributedGcServiceServer::new(gc_service);
    
    // Create metrics interceptor
    let metrics_interceptor = MetricsInterceptor::new(metrics);
    
    // Apply interceptor to service
    let intercepted_service = tonic::service::interceptor::InterceptedService::new(
        grpc_service,
        metrics_interceptor,
    );

    let server = Server::builder()
        .add_service(intercepted_service)
        .serve(addr);

    // Graceful shutdown
    tokio::select! {
        result = server => {
            if let Err(e) = result {
                warn!("gRPC server error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            info!("Received shutdown signal");
        }
    }

    info!("Shutting down Distributed GC Sidecar Service");
    Ok(())
}