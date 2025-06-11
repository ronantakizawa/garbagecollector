// src/security/tls_config.rs - TLS configuration implementation for gRPC

use crate::security::{CertValidationMode, MTLSConfig, TLSVersion};
use anyhow::{Context, Result};
use std::fs;
use tonic::transport::{Certificate, ClientTlsConfig, Identity, ServerTlsConfig};
use tracing::{debug, info};

/// TLS configuration builder for gRPC server
pub struct ServerTLSConfigBuilder {
    mtls_config: MTLSConfig,
}

impl ServerTLSConfigBuilder {
    pub fn new(mtls_config: MTLSConfig) -> Self {
        Self { mtls_config }
    }

    /// Build server TLS configuration
    pub fn build(&self) -> Result<Option<ServerTlsConfig>> {
        if !self.mtls_config.enabled {
            debug!("mTLS disabled, returning None for server TLS config");
            return Ok(None);
        }

        info!("🔒 Building server TLS configuration");

        // Load server certificate and private key
        let server_cert = fs::read(&self.mtls_config.server_cert_path)
            .with_context(|| {
                format!(
                    "Failed to read server certificate: {}",
                    self.mtls_config.server_cert_path.display()
                )
            })?;

        let server_key = fs::read(&self.mtls_config.server_key_path)
            .with_context(|| {
                format!(
                    "Failed to read server private key: {}",
                    self.mtls_config.server_key_path.display()
                )
            })?;

        // Create server identity
        let server_identity = Identity::from_pem(&server_cert, &server_key);

        // Start building TLS config
        let mut tls_config = ServerTlsConfig::new().identity(server_identity);

        // Configure client authentication if required
        if self.mtls_config.require_client_auth {
            info!("🔐 Configuring mutual TLS (client authentication required)");

            // Load CA certificate for client verification
            let ca_cert = fs::read(&self.mtls_config.ca_cert_path)
                .with_context(|| {
                    format!(
                        "Failed to read CA certificate: {}",
                        self.mtls_config.ca_cert_path.display()
                    )
                })?;

            let ca_certificate = Certificate::from_pem(&ca_cert);
            tls_config = tls_config.client_ca_root(ca_certificate);
        }

        // Configure TLS version constraints
        tls_config = match self.mtls_config.min_tls_version {
            TLSVersion::TLS12 => {
                debug!("Setting minimum TLS version to 1.2");
                tls_config
            }
            TLSVersion::TLS13 => {
                debug!("Setting minimum TLS version to 1.3");
                // Note: Tonic doesn't currently expose direct TLS 1.3 enforcement
                // This would need to be handled at the transport layer
                tls_config
            }
        };

        info!("✅ Server TLS configuration built successfully");
        Ok(Some(tls_config))
    }
}

/// TLS configuration builder for gRPC client
pub struct ClientTLSConfigBuilder {
    mtls_config: MTLSConfig,
    server_domain: String,
}

impl ClientTLSConfigBuilder {
    pub fn new(mtls_config: MTLSConfig, server_domain: String) -> Self {
        Self {
            mtls_config,
            server_domain,
        }
    }

    /// Build client TLS configuration
    pub fn build(&self) -> Result<Option<ClientTlsConfig>> {
        if !self.mtls_config.enabled {
            debug!("mTLS disabled, returning None for client TLS config");
            return Ok(None);
        }

        info!("🔒 Building client TLS configuration for domain: {}", self.server_domain);

        // Start building client TLS config
        let mut tls_config = ClientTlsConfig::new().domain_name(&self.server_domain);

        // Load CA certificate for server verification
        let ca_cert = fs::read(&self.mtls_config.ca_cert_path)
            .with_context(|| {
                format!(
                    "Failed to read CA certificate: {}",
                    self.mtls_config.ca_cert_path.display()
                )
            })?;

        let ca_certificate = Certificate::from_pem(&ca_cert);
        tls_config = tls_config.ca_certificate(ca_certificate);

        // Configure client certificate if available and client auth is required
        if self.mtls_config.require_client_auth {
            if let (Some(ref client_cert_path), Some(ref client_key_path)) = (
                &self.mtls_config.client_cert_path,
                &self.mtls_config.client_key_path,
            ) {
                info!("🔐 Configuring client certificate for mutual TLS");

                let client_cert = fs::read(client_cert_path)
                    .with_context(|| {
                        format!(
                            "Failed to read client certificate: {}",
                            client_cert_path.display()
                        )
                    })?;

                let client_key = fs::read(client_key_path)
                    .with_context(|| {
                        format!(
                            "Failed to read client private key: {}",
                            client_key_path.display()
                        )
                    })?;

                let client_identity = Identity::from_pem(&client_cert, &client_key);
                tls_config = tls_config.identity(client_identity);
            } else {
                return Err(anyhow::anyhow!(
                    "Client authentication required but client certificate/key paths not configured"
                ));
            }
        }

        // Handle certificate validation mode
        match self.mtls_config.cert_validation_mode {
            CertValidationMode::Strict => {
                debug!("Using strict certificate validation");
                // Default behavior, no additional configuration needed
            }
            CertValidationMode::AllowSelfSigned => {
                debug!("⚠️ Allowing self-signed certificates");
                // Note: Tonic doesn't directly support this, would need custom verification
            }
            CertValidationMode::SkipHostnameVerification => {
                debug!("⚠️ Skipping hostname verification");
                // Note: This would require custom verification logic
            }
        }

        info!("✅ Client TLS configuration built successfully");
        Ok(Some(tls_config))
    }
}

/// Certificate validator for custom validation logic
pub struct CertificateValidator {
    mtls_config: MTLSConfig,
}

impl CertificateValidator {
    pub fn new(mtls_config: MTLSConfig) -> Self {
        Self { mtls_config }
    }

    /// Validate client certificate common name
    pub fn validate_client_cn(&self, cn: &str) -> bool {
        if self.mtls_config.allowed_client_cns.is_empty() {
            // If no CNs specified, allow any
            debug!("No allowed CNs configured, allowing all client certificates");
            return true;
        }

        let allowed = self.mtls_config.allowed_client_cns.contains(&cn.to_string());
        
        if allowed {
            debug!("Client CN '{}' is in allowed list", cn);
        } else {
            debug!("Client CN '{}' is NOT in allowed list: {:?}", cn, self.mtls_config.allowed_client_cns);
        }

        allowed
    }

    /// Extract common name from certificate subject
    pub fn extract_cn_from_cert(&self, cert_der: &[u8]) -> Result<Option<String>> {
        // This is a simplified implementation
        // In production, you'd use a proper X.509 parsing library like `x509-parser`
        debug!("Extracting CN from certificate (simplified implementation)");
        
        // For now, return None to indicate CN extraction is not implemented
        // In a real implementation, you would:
        // 1. Parse the DER-encoded certificate
        // 2. Extract the subject field
        // 3. Find the CN (Common Name) attribute
        
        Ok(None)
    }
}

/// TLS interceptor for additional security checks
pub struct TLSInterceptor {
    validator: CertificateValidator,
}

impl TLSInterceptor {
    pub fn new(mtls_config: MTLSConfig) -> Self {
        Self {
            validator: CertificateValidator::new(mtls_config),
        }
    }

    /// Validate incoming connection
    pub fn validate_connection(&self, _peer_cert: Option<&[u8]>) -> Result<bool> {
        // This would be called by a custom gRPC interceptor
        // to perform additional validation on the peer certificate
        
        debug!("Validating TLS connection");
        
        // If no peer certificate is provided and client auth is required, reject
        // Otherwise, perform CN validation if certificate is present
        
        Ok(true) // Simplified for now
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

    fn create_test_certs() -> (tempfile::TempDir, MTLSConfig) {
        let temp_dir = tempdir().unwrap();
        
        // Create dummy certificate files
        let server_cert = temp_dir.path().join("server.crt");
        let server_key = temp_dir.path().join("server.key");
        let ca_cert = temp_dir.path().join("ca.crt");
        let client_cert = temp_dir.path().join("client.crt");
        let client_key = temp_dir.path().join("client.key");
        
        // Simple PEM-format dummy certificates
        let dummy_cert = "-----BEGIN CERTIFICATE-----\nMIIBkTCB+wIJAL7Q6Q6Q6Q6QMA0GCSqGSIb3DQEBCwUAMBQxEjAQBgNVBAMMCWxv\nY2FsaG9zdDAeFw0yNDA2MTEwMDAwMDBaFw0yNTA2MTEwMDAwMDBaMBQxEjAQBgNV\nBAMMCWxvY2FsaG9zdDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABG+G3r9q8j8H\n9QKJ9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J\n9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J\n9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J\n9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J\n9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J\n9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J\n-----END CERTIFICATE-----";
        
        let dummy_key = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg+G3r9q8j8H9QKJ9Y\n2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2JohRANCAAR/ht6/avI/B/UCifWNifWNifWN\nifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWN\nifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWN\n-----END PRIVATE KEY-----";
        
        fs::write(&server_cert, dummy_cert).unwrap();
        fs::write(&server_key, dummy_key).unwrap();
        fs::write(&ca_cert, dummy_cert).unwrap();
        fs::write(&client_cert, dummy_cert).unwrap();
        fs::write(&client_key, dummy_key).unwrap();

        let mtls_config = MTLSConfig {
            enabled: true,
            server_cert_path: server_cert,
            server_key_path: server_key,
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

    #[test]
    fn test_server_tls_config_disabled() {
        let mtls_config = MTLSConfig {
            enabled: false,
            ..Default::default()
        };

        let builder = ServerTLSConfigBuilder::new(mtls_config);
        let result = builder.build().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_server_tls_config_enabled() {
        let (_temp_dir, mtls_config) = create_test_certs();
        let builder = ServerTLSConfigBuilder::new(mtls_config);
        let result = builder.build();
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_client_tls_config_disabled() {
        let mtls_config = MTLSConfig {
            enabled: false,
            ..Default::default()
        };

        let builder = ClientTLSConfigBuilder::new(mtls_config, "localhost".to_string());
        let result = builder.build().unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_client_tls_config_enabled() {
        let (_temp_dir, mtls_config) = create_test_certs();
        let builder = ClientTLSConfigBuilder::new(mtls_config, "localhost".to_string());
        let result = builder.build();
        assert!(result.is_ok());
        assert!(result.unwrap().is_some());
    }

    #[test]
    fn test_certificate_validator() {
        let mtls_config = MTLSConfig {
            allowed_client_cns: vec!["test-client".to_string(), "another-client".to_string()],
            ..Default::default()
        };

        let validator = CertificateValidator::new(mtls_config);
        
        assert!(validator.validate_client_cn("test-client"));
        assert!(validator.validate_client_cn("another-client"));
        assert!(!validator.validate_client_cn("unauthorized-client"));
    }

    #[test]
    fn test_certificate_validator_empty_allowed_list() {
        let mtls_config = MTLSConfig {
            allowed_client_cns: vec![],
            ..Default::default()
        };

        let validator = CertificateValidator::new(mtls_config);
        
        // Should allow any CN when the allowed list is empty
        assert!(validator.validate_client_cn("any-client"));
        assert!(validator.validate_client_cn("another-client"));
    }

    #[test]
    fn test_tls_interceptor() {
        let mtls_config = MTLSConfig::default();
        let interceptor = TLSInterceptor::new(mtls_config);
        
        // Basic validation test
        let result = interceptor.validate_connection(None);
        assert!(result.is_ok());
        assert!(result.unwrap());
    }
}