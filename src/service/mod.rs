// src/service/mod.rs - Complete fixed version with cleanup integration

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::signal;
use tokio::time::interval;
use tonic::transport::Server;
use tracing::{debug, error, info, warn};

use crate::cleanup::CleanupExecutor;
use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::LeaseState;
use crate::metrics::Metrics;
use crate::proto::distributed_gc_service_server::DistributedGcServiceServer;
use crate::storage::Storage;

pub mod handlers;
pub mod validation;

use handlers::GCServiceHandlers;

/// Main GarbageTruck service
#[derive(Clone)]
pub struct GCService {
    storage: Arc<dyn Storage>,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
    shutdown_tx: Option<Arc<tokio::sync::broadcast::Sender<()>>>,
}

impl GCService {
    pub fn new(
        storage: Arc<dyn Storage>,
        config: Arc<Config>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            storage,
            config,
            metrics,
            shutdown_tx: None,
        }
    }

    /// Create a new service with shutdown capability for testing
    pub fn new_with_shutdown(
        storage: Arc<dyn Storage>,
        config: Arc<Config>,
        metrics: Arc<Metrics>,
    ) -> (Self, tokio::sync::broadcast::Receiver<()>) {
        let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
        let service = Self {
            storage,
            config,
            metrics,
            shutdown_tx: Some(Arc::new(shutdown_tx)),
        };
        (service, shutdown_rx)
    }

    /// Shutdown the service gracefully
    pub async fn shutdown(&self) -> Result<()> {
        info!("🛑 Shutting down GarbageTruck service...");
        
        if let Some(ref shutdown_tx) = self.shutdown_tx {
            let _ = shutdown_tx.send(());
        }
        
        // Give a moment for cleanup tasks to finish
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        info!("✅ GarbageTruck service shutdown complete");
        Ok(())
    }

    /// Start the GarbageTruck service
    pub async fn start(&self, addr: SocketAddr) -> Result<()> {
        info!("🚀 Starting GarbageTruck service on {}", addr);

        // CRITICAL: Start the cleanup loop BEFORE starting gRPC server
        self.start_cleanup_loop();

        // Create the gRPC service handlers
        let handlers = GCServiceHandlers::new(
            self.storage.clone(),
            self.config.clone(),
            self.metrics.clone(),
        );

        // Start metrics server if enabled
        if self.config.metrics.enabled {
            let metrics_addr = format!("0.0.0.0:{}", self.config.metrics.port);
            let metrics = self.metrics.clone();
            
            tokio::spawn(async move {
                if let Err(e) = start_metrics_server(metrics_addr, metrics).await {
                    error!("❌ Metrics server failed: {}", e);
                }
            });
            
            info!("📊 Metrics server started on port {}", self.config.metrics.port);
        }

        // Create and start the gRPC server
        let grpc_service = DistributedGcServiceServer::new(handlers);

        info!("🌐 gRPC server starting on {}", addr);

        // Create shutdown future
        let shutdown_future = async {
            if let Some(ref shutdown_tx) = self.shutdown_tx {
                let mut shutdown_rx = shutdown_tx.subscribe();
                tokio::select! {
                    _ = signal::ctrl_c() => {
                        info!("🛑 Received Ctrl+C signal");
                    }
                    _ = shutdown_rx.recv() => {
                        info!("🛑 Received shutdown signal");
                    }
                }
            } else {
                // If no shutdown channel, just wait for Ctrl+C
                let _ = signal::ctrl_c().await;
                info!("🛑 Received Ctrl+C signal");
            }
        };

        // Start the server with graceful shutdown
        Server::builder()
            .add_service(grpc_service)
            .serve_with_shutdown(addr, shutdown_future)
            .await
            .map_err(|e| GCError::Internal(format!("gRPC server error: {}", e)))?;

        info!("👋 GarbageTruck service stopped gracefully");
        Ok(())
    }

    /// Start the cleanup loop in the background
    fn start_cleanup_loop(&self) {
        let storage = self.storage.clone();
        let config = self.config.clone();
        let metrics = self.metrics.clone();
        let mut shutdown_rx = self.shutdown_tx.as_ref().map(|tx| tx.subscribe());
        
        // Create cleanup executor
        let cleanup_executor = CleanupExecutor::new(
            Duration::from_secs(config.cleanup.default_timeout_seconds),
            config.cleanup.default_max_retries,
            Duration::from_secs(config.cleanup.default_retry_delay_seconds),
        );
        
        // Start background cleanup task
        tokio::spawn(async move {
            let mut cleanup_interval = interval(Duration::from_secs(config.gc.cleanup_interval_seconds));
            
            info!(
                "🧹 Starting cleanup loop (interval: {}s, grace: {}s)",
                config.gc.cleanup_interval_seconds,
                config.gc.cleanup_grace_period_seconds
            );
            
            loop {
                tokio::select! {
                    _ = cleanup_interval.tick() => {
                        // Run cleanup cycle
                        match run_cleanup_cycle(&storage, &cleanup_executor, &config, &metrics).await {
                            Ok((expired, cleaned)) => {
                                if expired > 0 || cleaned > 0 {
                                    info!("🧹 Cleanup cycle completed: {} expired, {} cleaned", expired, cleaned);
                                } else {
                                    debug!("✨ Cleanup cycle completed: no action needed");
                                }
                            }
                            Err(e) => {
                                error!("❌ Cleanup cycle failed: {}", e);
                                // Don't exit the loop, just continue with next cycle
                                tokio::time::sleep(Duration::from_secs(5)).await;
                            }
                        }
                    }
                    _ = async {
                        if let Some(ref mut rx) = shutdown_rx {
                            let _ = rx.recv().await;
                        } else {
                            // If no shutdown receiver, wait forever
                            std::future::pending::<()>().await;
                        }
                    } => {
                        info!("🛑 Cleanup loop received shutdown signal");
                        break;
                    }
                }
            }
            
            info!("🧹 Cleanup loop stopped");
        });
        
        info!("✅ Cleanup loop started in background");
    }
}

/// Run a single cleanup cycle
async fn run_cleanup_cycle(
    storage: &Arc<dyn Storage>,
    cleanup_executor: &CleanupExecutor,
    config: &Config,
    _metrics: &Arc<Metrics>,
) -> Result<(usize, usize)> {
    debug!("🔍 Starting cleanup cycle");
    
    // Get all leases
    let all_leases = storage.list_leases(None, Some(1000)).await?;
    let grace_period = Duration::from_secs(config.gc.cleanup_grace_period_seconds);
    
    let mut expired_count = 0;
    let mut cleanup_candidates = Vec::new();
    
    // Process each lease
    for mut lease in all_leases {
        match lease.state {
            LeaseState::Active if lease.is_expired() => {
                // Mark as expired
                lease.expire();
                
                // Store lease info before the storage call (in case update_lease takes ownership)
                let lease_id = lease.lease_id.clone();
                let object_id = lease.object_id.clone();
                
                if let Err(e) = storage.update_lease(lease).await {
                    warn!("Failed to update expired lease {}: {}", lease_id, e);
                } else {
                    expired_count += 1;
                    debug!("⏰ Marked lease {} as expired (object: {})", lease_id, object_id);
                }
            }
            LeaseState::Expired => {
                // Check if needs cleanup
                if lease.should_cleanup(grace_period) {
                    if lease.cleanup_config.is_some() {
                        debug!(
                            "🎯 Lease {} needs cleanup (object: {}, expired: {:?} ago)",
                            lease.lease_id,
                            lease.object_id,
                            lease.time_until_expiry()
                        );
                        cleanup_candidates.push(lease);
                    } else {
                        // No cleanup config, just remove from storage
                        debug!("🗑️ Lease {} has no cleanup config, removing from storage", lease.lease_id);
                        if let Err(e) = storage.delete_lease(&lease.lease_id).await {
                            warn!("Failed to delete lease {} from storage: {}", lease.lease_id, e);
                        }
                    }
                }
            }
            LeaseState::Released => {
                // Released leases with cleanup config should be cleaned immediately
                if lease.cleanup_config.is_some() {
                    debug!("🎯 Released lease {} needs immediate cleanup (object: {})", lease.lease_id, lease.object_id);
                    cleanup_candidates.push(lease);
                } else {
                    // No cleanup config, just remove from storage
                    if let Err(e) = storage.delete_lease(&lease.lease_id).await {
                        warn!("Failed to delete released lease {} from storage: {}", lease.lease_id, e);
                    }
                }
            }
            _ => {}
        }
    }
    
    if cleanup_candidates.is_empty() {
        if expired_count > 0 {
            debug!("⏰ Marked {} leases as expired, no cleanup needed this cycle", expired_count);
        }
        return Ok((expired_count, 0));
    }
    
    info!("🧹 Processing {} cleanup candidates", cleanup_candidates.len());
    
    // Log cleanup candidates for debugging
    for lease in &cleanup_candidates {
        debug!(
            "📋 Cleanup candidate: {} (object: {}, config: {})",
            lease.lease_id,
            lease.object_id,
            lease.cleanup_config.as_ref()
                .map(|c| format!("endpoint={}", c.cleanup_http_endpoint))
                .unwrap_or_else(|| "none".to_string())
        );
    }
    
    // Execute cleanup
    let cleanup_results = cleanup_executor.cleanup_batch(cleanup_candidates.clone()).await;
    
    let mut successful_cleanups = 0;
    let mut failed_cleanups = 0;
    
    // Process cleanup results
    for (lease, result) in cleanup_candidates.iter().zip(cleanup_results.iter()) {
        if result.success {
            // Remove lease from storage after successful cleanup
            if let Err(e) = storage.delete_lease(&lease.lease_id).await {
                warn!("Failed to delete cleaned lease {}: {}", lease.lease_id, e);
            } else {
                successful_cleanups += 1;
                info!("✅ Successfully cleaned up lease {} (object: {})", lease.lease_id, lease.object_id);
            }
        } else {
            failed_cleanups += 1;
            error!(
                "❌ Cleanup failed for lease {} (object: {}): {:?}",
                lease.lease_id, lease.object_id, result.error
            );
        }
    }
    
    if failed_cleanups > 0 {
        warn!("⚠️ {} cleanups failed this cycle", failed_cleanups);
    }
    
    Ok((expired_count, successful_cleanups))
}

/// Start metrics server
async fn start_metrics_server(addr: String, metrics: Arc<Metrics>) -> Result<()> {
    use warp::Filter;

    let metrics_route = warp::path("metrics")
        .and(warp::get())
        .and(warp::any().map(move || metrics.clone()))
        .and_then(handle_metrics_request);

    let health_route = warp::path("health")
        .and(warp::get())
        .map(|| {
            warp::reply::json(&serde_json::json!({
                "status": "ok",
                "service": "garbagetruck-metrics"
            }))
        });

    let routes = metrics_route.or(health_route);

    info!("📊 Starting metrics server on http://{}", addr);

    warp::serve(routes)
        .run(addr.parse::<SocketAddr>().unwrap())
        .await;

    Ok(())
}

async fn handle_metrics_request(
    metrics: Arc<Metrics>,
) -> std::result::Result<impl warp::Reply, warp::Rejection> {
    // Return Prometheus metrics format
    let metrics_text = metrics.export_prometheus_metrics();
    Ok(warp::reply::with_header(
        metrics_text,
        "content-type",
        "text/plain; version=0.0.4; charset=utf-8",
    ))
}

// Extend Metrics implementation if needed
impl Metrics {
    pub fn export_prometheus_metrics(&self) -> String {
        // This is a placeholder - implement based on your actual metrics
        // For now, return basic metrics
        format!(
            "# HELP garbagetruck_leases_total Total number of leases created\n\
             # TYPE garbagetruck_leases_total counter\n\
             garbagetruck_leases_total {}\n\
             \n\
             # HELP garbagetruck_cleanups_total Total number of cleanup operations\n\
             # TYPE garbagetruck_cleanups_total counter\n\
             garbagetruck_cleanups_total {}\n\
             \n\
             # HELP garbagetruck_active_leases Current number of active leases\n\
             # TYPE garbagetruck_active_leases gauge\n\
             garbagetruck_active_leases {}\n",
            0, // Replace with actual lease count from your metrics
            0, // Replace with actual cleanup count from your metrics  
            0  // Replace with actual active lease count from your metrics
        )
    }
}