// src/service/grpc_server.rs - Enhanced gRPC server with mTLS support

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::metrics::Metrics;
use crate::proto::distributed_gc_service_server::DistributedGcServiceServer;
use crate::security::{MTLSConfig, ServerTLSConfigBuilder, TLSInterceptor};
use crate::service::handlers::GCServiceHandlers;
use crate::storage::Storage;

use std::net::SocketAddr;
use std::sync::Arc;
use tonic::transport::{Server, ServerTlsConfig};
use tonic::{Request, Status};
use tower::ServiceBuilder;
use tracing::{error, info, warn};

/// Enhanced gRPC server with mTLS support
pub struct GRPCServer {
    storage: Arc<dyn Storage>,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
    mtls_config: MTLSConfig,
    tls_interceptor: Option<TLSInterceptor>,
}

impl GRPCServer {
    /// Create a new gRPC server with mTLS configuration
    pub fn new(
        storage: Arc<dyn Storage>,
        config: Arc<Config>,
        metrics: Arc<Metrics>,
        mtls_config: MTLSConfig,
    ) -> Result<Self> {
        // Validate mTLS configuration
        mtls_config.validate()?;

        // Create TLS interceptor if mTLS is enabled
        let tls_interceptor = if mtls_config.enabled {
            Some(TLSInterceptor::new(mtls_config.clone()))
        } else {
            None
        };

        Ok(Self {
            storage,
            config,
            metrics,
            mtls_config,
            tls_interceptor,
        })
    }

    /// Start the gRPC server with mTLS support
    pub async fn serve(&self, addr: SocketAddr) -> Result<()> {
        info!("🚀 Starting gRPC server on {}", addr);

        // Log mTLS configuration
        let mtls_summary = self.mtls_config.summary();
        if mtls_summary.enabled {
            info!("🔒 mTLS enabled:");
            info!("  - Client authentication required: {}", mtls_summary.require_client_auth);
            info!("  - Allowed client CNs: {}", mtls_summary.allowed_client_count);
            info!("  - Min TLS version: {:?}", mtls_summary.min_tls_version);
            info!("  - Cert validation mode: {:?}", mtls_summary.cert_validation_mode);
        } else {
            warn!("⚠️ mTLS disabled - using plain gRPC (not recommended for production)");
        }

        // Create service handlers
        let handlers = GCServiceHandlers::new(
            self.storage.clone(),
            self.config.clone(),
            self.metrics.clone(),
        );

        // Wrap handlers with mTLS interceptor if enabled
        let service = if let Some(ref interceptor) = self.tls_interceptor {
            info!("🛡️ Adding mTLS interceptor for client certificate validation");
            DistributedGcServiceServer::with_interceptor(handlers, MTLSInterceptorFn::new(interceptor.clone()))
        } else {
            DistributedGcServiceServer::new(handlers)
        };

        // Build TLS configuration
        let tls_config = self.build_tls_config().await?;

        // Create server builder
        let mut server_builder = Server::builder();

        // Configure TLS if enabled
        if let Some(tls_config) = tls_config {
            info!("🔐 Configuring TLS for gRPC server");
            server_builder = server_builder.tls_config(tls_config)?;
        }

        // Add service and start server
        let server = server_builder.add_service(service);

        info!("✅ gRPC server starting with mTLS: {}", self.mtls_config.enabled);

        // Create shutdown signal
        let shutdown_signal = self.create_shutdown_signal();

        // Start the server
        server
            .serve_with_shutdown(addr, shutdown_signal)
            .await
            .map_err(|e| GCError::Internal(format!("gRPC server error: {}", e)))?;

        info!("👋 gRPC server stopped gracefully");
        Ok(())
    }

    /// Build TLS configuration for the server
    async fn build_tls_config(&self) -> Result<Option<ServerTlsConfig>> {
        if !self.mtls_config.enabled {
            return Ok(None);
        }

        let builder = ServerTLSConfigBuilder::new(self.mtls_config.clone());
        builder.build()
    }

    /// Create shutdown signal handler
    async fn create_shutdown_signal(&self) {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("🛑 Received Ctrl+C signal, shutting down gRPC server");
            }
            _ = self.wait_for_shutdown_signal() => {
                info!("🛑 Received shutdown signal, shutting down gRPC server");
            }
        }
    }

    /// Wait for external shutdown signal
    async fn wait_for_shutdown_signal(&self) {
        // This could be connected to your existing shutdown coordinator
        // For now, we'll just wait indefinitely
        std::future::pending::<()>().await;
    }

    /// Get mTLS configuration summary
    pub fn get_mtls_summary(&self) -> crate::security::MTLSConfigSummary {
        self.mtls_config.summary()
    }
}

/// mTLS interceptor function for validating client certificates
#[derive(Clone)]
struct MTLSInterceptorFn {
    interceptor: TLSInterceptor,
}

impl MTLSInterceptorFn {
    fn new(interceptor: TLSInterceptor) -> Self {
        Self { interceptor }
    }
}

impl<T> tower::Service<Request<T>> for MTLSInterceptorFn
where
    T: Send + 'static,
{
    type Response = Request<T>;
    type Error = Status;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<T>) -> Self::Future {
        let interceptor = self.interceptor.clone();
        
        Box::pin(async move {
            // Extract client certificate information from the request
            // Note: In a real implementation, you would extract the peer certificate
            // from the TLS connection context. Tonic doesn't directly expose this,
            // so you might need to use a custom connector or middleware.
            
            // For now, we'll validate based on metadata or connection info
            let peer_cert = None; // Would extract from TLS context in real implementation
            
            match interceptor.validate_connection(peer_cert) {
                Ok(true) => {
                    tracing::debug!("✅ Client certificate validation passed");
                    Ok(request)
                }
                Ok(false) => {
                    tracing::warn!("❌ Client certificate validation failed");
                    Err(Status::unauthenticated("Client certificate validation failed"))
                }
                Err(e) => {
                    tracing::error!("❌ Error during certificate validation: {}", e);
                    Err(Status::internal("Certificate validation error"))
                }
            }
        })
    }
}

/// mTLS metrics collector
pub struct MTLSMetrics {
    successful_tls_connections: prometheus::Counter,
    failed_tls_connections: prometheus::Counter,
    client_cert_validation_attempts: prometheus::Counter,
    client_cert_validation_failures: prometheus::Counter,
}

impl MTLSMetrics {
    pub fn new() -> Self {
        Self {
            successful_tls_connections: prometheus::Counter::new(
                "garbagetruck_mtls_successful_connections_total",
                "Total number of successful mTLS connections"
            ).unwrap(),
            failed_tls_connections: prometheus::Counter::new(
                "garbagetruck_mtls_failed_connections_total", 
                "Total number of failed mTLS connections"
            ).unwrap(),
            client_cert_validation_attempts: prometheus::Counter::new(
                "garbagetruck_mtls_cert_validation_attempts_total",
                "Total number of client certificate validation attempts"
            ).unwrap(),
            client_cert_validation_failures: prometheus::Counter::new(
                "garbagetruck_mtls_cert_validation_failures_total",
                "Total number of client certificate validation failures"
            ).unwrap(),
        }
    }

    pub fn record_successful_connection(&self) {
        self.successful_tls_connections.inc();
    }

    pub fn record_failed_connection(&self) {
        self.failed_tls_connections.inc();
    }

    pub fn record_cert_validation_attempt(&self) {
        self.client_cert_validation_attempts.inc();
    }

    pub fn record_cert_validation_failure(&self) {
        self.client_cert_validation_failures.inc();
    }

    /// Register metrics with Prometheus registry
    pub fn register(&self, registry: &prometheus::Registry) -> Result<()> {
        registry.register(Box::new(self.successful_tls_connections.clone()))?;
        registry.register(Box::new(self.failed_tls_connections.clone()))?;
        registry.register(Box::new(self.client_cert_validation_attempts.clone()))?;
        registry.register(Box::new(self.client_cert_validation_failures.clone()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;
    use crate::security::CertValidationMode;
    use tempfile::tempdir;
    use std::fs;

    fn create_test_mtls_config() -> (tempfile::TempDir, MTLSConfig) {
        let temp_dir = tempdir().unwrap();
        
        // Create dummy certificate files
        let server_cert = temp_dir.path().join("server.crt");
        let server_key = temp_dir.path().join("server.key");
        let ca_cert = temp_dir.path().join("ca.crt");
        
        let dummy_cert = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----";
        let dummy_key = "-----BEGIN PRIVATE KEY-----\ntest\n-----END PRIVATE KEY-----";
        
        fs::write(&server_cert, dummy_cert).unwrap();
        fs::write(&server_key, dummy_key).unwrap();
        fs::write(&ca_cert, dummy_cert).unwrap();

        let mtls_config = MTLSConfig {
            enabled: true,
            server_cert_path: server_cert,
            server_key_path: server_key,
            ca_cert_path: ca_cert,
            require_client_auth: true,
            allowed_client_cns: vec!["test-client".to_string()],
            min_tls_version: crate::security::TLSVersion::TLS12,
            cert_validation_mode: CertValidationMode::Strict,
            ..Default::default()
        };

        (temp_dir, mtls_config)
    }

    #[tokio::test]
    async fn test_grpc_server_creation() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();
        let (_temp_dir, mtls_config) = create_test_mtls_config();

        let server = GRPCServer::new(storage, config, metrics, mtls_config);
        assert!(server.is_ok());
    }

    #[tokio::test]
    async fn test_grpc_server_mtls_disabled() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();
        let mtls_config = MTLSConfig {
            enabled: false,
            ..Default::default()
        };

        let server = GRPCServer::new(storage, config, metrics, mtls_config).unwrap();
        let summary = server.get_mtls_summary();
        assert!(!summary.enabled);
        assert!(server.tls_interceptor.is_none());
    }

    #[tokio::test]
    async fn test_grpc_server_mtls_enabled() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();
        let (_temp_dir, mtls_config) = create_test_mtls_config();

        let server = GRPCServer::new(storage, config, metrics, mtls_config).unwrap();
        let summary = server.get_mtls_summary();
        assert!(summary.enabled);
        assert!(server.tls_interceptor.is_some());
    }

    #[tokio::test]
    async fn test_build_tls_config_disabled() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();
        let mtls_config = MTLSConfig {
            enabled: false,
            ..Default::default()
        };

        let server = GRPCServer::new(storage, config, metrics, mtls_config).unwrap();
        let tls_config = server.build_tls_config().await.unwrap();
        assert!(tls_config.is_none());
    }

    #[test]
    fn test_mtls_metrics() {
        let metrics = MTLSMetrics::new();
        
        metrics.record_successful_connection();
        metrics.record_failed_connection();
        metrics.record_cert_validation_attempt();
        metrics.record_cert_validation_failure();
        
        // Test that metrics can be registered
        let registry = prometheus::Registry::new();
        let result = metrics.register(&registry);
        assert!(result.is_ok());
    }
}