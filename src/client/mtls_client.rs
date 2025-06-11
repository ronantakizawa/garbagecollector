// src/client/mtls_client.rs - Enhanced gRPC client with mTLS support

use crate::error::{GCError, Result};
use crate::lease::{CleanupConfig, ObjectType};
use crate::proto::{
    distributed_gc_service_client::DistributedGcServiceClient, CreateLeaseRequest, GetLeaseRequest,
    HealthCheckRequest, ListLeasesRequest, ReleaseLeaseRequest, RenewLeaseRequest,
};
use crate::security::{ClientTLSConfigBuilder, MTLSConfig};

use std::collections::HashMap;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;
use tracing::{debug, info, warn};

/// Enhanced GarbageTruck client with mTLS support
#[derive(Clone)]
pub struct MTLSGCClient {
    client: DistributedGcServiceClient<Channel>,
    service_id: String,
    mtls_config: MTLSConfig,
    endpoint: String,
}

impl MTLSGCClient {
    /// Create a new GarbageTruck client with mTLS configuration
    pub async fn new(
        endpoint: &str,
        service_id: String,
        mtls_config: MTLSConfig,
    ) -> Result<Self> {
        info!("🔗 Creating mTLS-enabled GarbageTruck client for endpoint: {}", endpoint);

        // Validate mTLS configuration
        mtls_config.validate()?;

        // Log mTLS configuration
        let mtls_summary = mtls_config.summary();
        if mtls_summary.enabled {
            info!("🔒 Client mTLS configuration:");
            info!("  - Server endpoint: {}", endpoint);
            info!("  - Client authentication: {}", mtls_summary.require_client_auth);
            info!("  - Certificate validation: {:?}", mtls_summary.cert_validation_mode);
        } else {
            warn!("⚠️ mTLS disabled for client - using plain gRPC");
        }

        // Parse endpoint to extract domain for TLS configuration
        let uri = endpoint.parse::<tonic::transport::Uri>()
            .map_err(|e| GCError::Internal(format!("Invalid endpoint URI: {}", e)))?;
        
        let domain = uri.host().unwrap_or("localhost").to_string();
        debug!("Extracted domain for TLS: {}", domain);

        // Create endpoint builder
        let mut endpoint_builder = Endpoint::from_shared(endpoint.to_string())
            .map_err(|e| GCError::Internal(format!("Failed to create endpoint: {}", e)))?;

        // Configure TLS if enabled
        if mtls_config.enabled {
            let tls_builder = ClientTLSConfigBuilder::new(mtls_config.clone(), domain);
            if let Some(tls_config) = tls_builder.build()? {
                info!("🔐 Configuring client TLS");
                endpoint_builder = endpoint_builder.tls_config(tls_config)?;
            }
        }

        // Connect to the service
        let channel = endpoint_builder
            .connect()
            .await
            .map_err(|e| GCError::Network(format!("Failed to connect: {}", e)))?;

        let client = DistributedGcServiceClient::new(channel);

        info!("✅ mTLS GarbageTruck client connected successfully");

        Ok(Self {
            client,
            service_id,
            mtls_config,
            endpoint: endpoint.to_string(),
        })
    }

    /// Create a client with default mTLS configuration from environment
    pub async fn with_env_mtls(endpoint: &str, service_id: String) -> Result<Self> {
        let mtls_config = MTLSConfig::from_env()
            .map_err(|e| GCError::Configuration(format!("Failed to load mTLS config: {}", e)))?;
        
        Self::new(endpoint, service_id, mtls_config).await
    }

    /// Test the mTLS connection with a health check
    pub async fn test_connection(&self) -> Result<bool> {
        debug!("🔍 Testing mTLS connection with health check");
        
        match self.health_check().await {
            Ok(healthy) => {
                if healthy {
                    info!("✅ mTLS connection test successful");
                } else {
                    warn!("⚠️ mTLS connection established but service unhealthy");
                }
                Ok(healthy)
            }
            Err(e) => {
                warn!("❌ mTLS connection test failed: {}", e);
                Err(e)
            }
        }
    }

    /// Get mTLS configuration summary
    pub fn get_mtls_summary(&self) -> crate::security::MTLSConfigSummary {
        self.mtls_config.summary()
    }

    /// Get connection info
    pub fn get_connection_info(&self) -> ClientConnectionInfo {
        ClientConnectionInfo {
            endpoint: self.endpoint.clone(),
            service_id: self.service_id.clone(),
            mtls_enabled: self.mtls_config.enabled,
            client_auth_configured: self.mtls_config.client_cert_path.is_some(),
        }
    }

    // All the standard GarbageTruck client methods with mTLS support

    /// Create a lease for an object
    pub async fn create_lease(
        &self,
        object_id: String,
        object_type: ObjectType,
        lease_duration_seconds: u64,
        metadata: HashMap<String, String>,
        cleanup_config: Option<CleanupConfig>,
    ) -> Result<String> {
        let mut client = self.client.clone();

        let request = CreateLeaseRequest {
            object_id,
            object_type: crate::proto::ObjectType::from(object_type) as i32,
            service_id: self.service_id.clone(),
            lease_duration_seconds,
            metadata,
            cleanup_config: cleanup_config.map(|c| c.into()),
        };

        debug!("📝 Creating lease via mTLS connection");

        let response = client
            .create_lease(Request::new(request))
            .await
            .map_err(|e| {
                if e.code() == tonic::Code::Unauthenticated {
                    GCError::Internal(format!("mTLS authentication failed: {}", e))
                } else {
                    GCError::Internal(e.to_string())
                }
            })?;

        let create_response = response.into_inner();
        if create_response.success {
            debug!("✅ Lease created successfully via mTLS");
            Ok(create_response.lease_id)
        } else {
            Err(GCError::Internal(create_response.error_message))
        }
    }

    /// Renew an existing lease
    pub async fn renew_lease(&self, lease_id: String, extend_duration_seconds: u64) -> Result<()> {
        let mut client = self.client.clone();

        let request = RenewLeaseRequest {
            lease_id,
            service_id: self.service_id.clone(),
            extend_duration_seconds,
        };

        debug!("🔄 Renewing lease via mTLS connection");

        let response = client
            .renew_lease(Request::new(request))
            .await
            .map_err(|e| {
                if e.code() == tonic::Code::Unauthenticated {
                    GCError::Internal(format!("mTLS authentication failed: {}", e))
                } else {
                    GCError::Internal(e.to_string())
                }
            })?;

        let renew_response = response.into_inner();
        if renew_response.success {
            debug!("✅ Lease renewed successfully via mTLS");
            Ok(())
        } else {
            Err(GCError::Internal(renew_response.error_message))
        }
    }

    /// Release a lease
    pub async fn release_lease(&self, lease_id: String) -> Result<()> {
        let mut client = self.client.clone();

        let request = ReleaseLeaseRequest {
            lease_id,
            service_id: self.service_id.clone(),
        };

        debug!("🗑️ Releasing lease via mTLS connection");

        let response = client
            .release_lease(Request::new(request))
            .await
            .map_err(|e| {
                if e.code() == tonic::Code::Unauthenticated {
                    GCError::Internal(format!("mTLS authentication failed: {}", e))
                } else {
                    GCError::Internal(e.to_string())
                }
            })?;

        let release_response = response.into_inner();
        if release_response.success {
            debug!("✅ Lease released successfully via mTLS");
            Ok(())
        } else {
            Err(GCError::Internal(release_response.error_message))
        }
    }

    /// Get information about a specific lease
    pub async fn get_lease(&self, lease_id: String) -> Result<Option<crate::proto::LeaseInfo>> {
        let mut client = self.client.clone();

        let request = GetLeaseRequest { lease_id };

        debug!("📋 Getting lease info via mTLS connection");

        let response = client
            .get_lease(Request::new(request))
            .await
            .map_err(|e| {
                if e.code() == tonic::Code::Unauthenticated {
                    GCError::Internal(format!("mTLS authentication failed: {}", e))
                } else {
                    GCError::Internal(e.to_string())
                }
            })?;

        let get_response = response.into_inner();
        Ok(if get_response.found {
            get_response.lease
        } else {
            None
        })
    }

    /// List leases for this service or all services
    pub async fn list_leases(
        &self,
        service_id: Option<String>,
        limit: u32,
    ) -> Result<Vec<crate::proto::LeaseInfo>> {
        let mut client = self.client.clone();

        let request = ListLeasesRequest {
            service_id: service_id.unwrap_or_default(),
            object_type: 0, // All types
            state: 0,       // All states
            limit,
            page_token: String::new(),
        };

        debug!("📜 Listing leases via mTLS connection");

        let response = client
            .list_leases(Request::new(request))
            .await
            .map_err(|e| {
                if e.code() == tonic::Code::Unauthenticated {
                    GCError::Internal(format!("mTLS authentication failed: {}", e))
                } else {
                    GCError::Internal(e.to_string())
                }
            })?;

        let list_response = response.into_inner();
        Ok(list_response.leases)
    }

    /// Check if the GarbageTruck service is healthy
    pub async fn health_check(&self) -> Result<bool> {
        let mut client = self.client.clone();

        let request = HealthCheckRequest {};

        debug!("🏥 Performing health check via mTLS connection");

        let response = client
            .health_check(Request::new(request))
            .await
            .map_err(|e| {
                if e.code() == tonic::Code::Unauthenticated {
                    GCError::Internal(format!("mTLS authentication failed: {}", e))
                } else {
                    GCError::Internal(e.to_string())
                }
            })?;

        let health_response = response.into_inner();
        Ok(health_response.healthy)
    }

    // Enhanced convenience methods with mTLS support

    /// Create a temporary file lease with mTLS
    pub async fn create_temp_file_lease_secure(
        &self,
        file_path: String,
        duration_seconds: u64,
        cleanup_endpoint: Option<String>,
    ) -> Result<String> {
        info!("🔒 Creating secure temporary file lease: {}", file_path);
        
        let cleanup_config = cleanup_endpoint.map(|endpoint| CleanupConfig {
            cleanup_endpoint: String::new(),
            cleanup_http_endpoint: endpoint,
            cleanup_payload: format!(r#"{{"file_path": "{}"}}"#, file_path),
            max_retries: 3,
            retry_delay_seconds: 2,
        });

        let metadata = [
            ("file_path".to_string(), file_path.clone()),
            ("type".to_string(), "temp_file".to_string()),
            ("mtls_secured".to_string(), "true".to_string()),
        ]
        .into();

        self.create_lease(
            file_path,
            ObjectType::TemporaryFile,
            duration_seconds,
            metadata,
            cleanup_config,
        )
        .await
    }

    /// Perform a secure lease renewal with retry logic
    pub async fn renew_lease_with_retry(
        &self,
        lease_id: String,
        extend_duration_seconds: u64,
        max_retries: u32,
    ) -> Result<()> {
        let mut attempts = 0;
        let mut last_error = None;

        while attempts < max_retries {
            match self.renew_lease(lease_id.clone(), extend_duration_seconds).await {
                Ok(()) => {
                    if attempts > 0 {
                        info!("✅ Lease renewal succeeded after {} retries", attempts);
                    }
                    return Ok(());
                }
                Err(e) => {
                    attempts += 1;
                    last_error = Some(e);
                    
                    if attempts < max_retries {
                        warn!("⚠️ Lease renewal attempt {} failed, retrying...", attempts);
                        tokio::time::sleep(tokio::time::Duration::from_secs(1 << attempts)).await;
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            GCError::Internal("Lease renewal failed after all retries".to_string())
        }))
    }
}

/// Information about the client connection
#[derive(Debug, Clone, serde::Serialize)]
pub struct ClientConnectionInfo {
    pub endpoint: String,
    pub service_id: String,
    pub mtls_enabled: bool,
    pub client_auth_configured: bool,
}

/// Client builder for easier mTLS client creation
pub struct MTLSClientBuilder {
    endpoint: String,
    service_id: String,
    mtls_config: Option<MTLSConfig>,
}

impl MTLSClientBuilder {
    pub fn new(endpoint: String, service_id: String) -> Self {
        Self {
            endpoint,
            service_id,
            mtls_config: None,
        }
    }

    /// Set custom mTLS configuration
    pub fn with_mtls_config(mut self, config: MTLSConfig) -> Self {
        self.mtls_config = Some(config);
        self
    }

    /// Load mTLS configuration from environment
    pub fn with_env_mtls(mut self) -> Result<Self> {
        let config = MTLSConfig::from_env()
            .map_err(|e| GCError::Configuration(format!("Failed to load mTLS config: {}", e)))?;
        self.mtls_config = Some(config);
        Ok(self)
    }

    /// Disable mTLS (use plain gRPC)
    pub fn without_mtls(mut self) -> Self {
        self.mtls_config = Some(MTLSConfig {
            enabled: false,
            ..Default::default()
        });
        self
    }

    /// Build the mTLS client
    pub async fn build(self) -> Result<MTLSGCClient> {
        let mtls_config = self.mtls_config.unwrap_or_else(|| {
            MTLSConfig {
                enabled: false,
                ..Default::default()
            }
        });

        MTLSGCClient::new(&self.endpoint, self.service_id, mtls_config).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::{CertValidationMode, TLSVersion};
    use tempfile::tempdir;
    use std::fs;

    fn create_test_mtls_config() -> (tempfile::TempDir, MTLSConfig) {
        let temp_dir = tempdir().unwrap();
        
        let ca_cert = temp_dir.path().join("ca.crt");
        let client_cert = temp_dir.path().join("client.crt");
        let client_key = temp_dir.path().join("client.key");
        
        let dummy_cert = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----";
        let dummy_key = "-----BEGIN PRIVATE KEY-----\ntest\n-----END PRIVATE KEY-----";
        
        fs::write(&ca_cert, dummy_cert).unwrap();
        fs::write(&client_cert, dummy_cert).unwrap();
        fs::write(&client_key, dummy_key).unwrap();

        let mtls_config = MTLSConfig {
            enabled: true,
            server_cert_path: temp_dir.path().join("server.crt"),
            server_key_path: temp_dir.path().join("server.key"),
            ca_cert_path: ca_cert,
            client_cert_path: Some(client_cert),
            client_key_path: Some(client_key),
            require_client_auth: true,
            allowed_client_cns: vec!["test-client".to_string()],
            min_tls_version: TLSVersion::TLS12,
            cert_validation_mode: CertValidationMode::Strict,
        };

        (temp_dir, mtls_config)
    }

    #[tokio::test]
    async fn test_client_builder() {
        let builder = MTLSClientBuilder::new(
            "https://localhost:50051".to_string(),
            "test-service".to_string(),
        );

        let builder_with_disabled_mtls = builder.without_mtls();
        // Can't actually build without a running server, but we can test the builder
        assert!(builder_with_disabled_mtls.mtls_config.is_some());
        assert!(!builder_with_disabled_mtls.mtls_config.unwrap().enabled);
    }

    #[tokio::test]
    async fn test_client_builder_with_config() {
        let (_temp_dir, mtls_config) = create_test_mtls_config();
        
        let builder = MTLSClientBuilder::new(
            "https://localhost:50051".to_string(),
            "test-service".to_string(),
        ).with_mtls_config(mtls_config.clone());

        assert!(builder.mtls_config.is_some());
        assert!(builder.mtls_config.unwrap().enabled);
    }

    #[test]
    fn test_connection_info() {
        let info = ClientConnectionInfo {
            endpoint: "https://localhost:50051".to_string(),
            service_id: "test-service".to_string(),
            mtls_enabled: true,
            client_auth_configured: true,
        };

        assert_eq!(info.endpoint, "https://localhost:50051");
        assert!(info.mtls_enabled);
        assert!(info.client_auth_configured);
    }

    #[test]
    fn test_client_builder_methods() {
        let builder = MTLSClientBuilder::new(
            "https://localhost:50051".to_string(),
            "test-service".to_string(),
        );

        // Test that methods return the builder for chaining
        let _builder_chained = builder.without_mtls();
        
        // Create a new builder for environment test
        let builder2 = MTLSClientBuilder::new(
            "https://localhost:50051".to_string(),
            "test-service".to_string(),
        );

        // Test environment loading (will use defaults if no env vars set)
        std::env::set_var("GC_MTLS_ENABLED", "false");
        let result = builder2.with_env_mtls();
        std::env::remove_var("GC_MTLS_ENABLED");
        
        assert!(result.is_ok());
    }
}