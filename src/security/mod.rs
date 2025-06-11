// src/security/mod.rs - mTLS configuration and certificate management

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, info, warn};

pub mod certificates;
pub mod tls_config;

/// mTLS configuration for GarbageTruck
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MTLSConfig {
    /// Enable mTLS for gRPC communication
    pub enabled: bool,
    /// Path to server certificate file (PEM format)
    pub server_cert_path: PathBuf,
    /// Path to server private key file (PEM format)
    pub server_key_path: PathBuf,
    /// Path to Certificate Authority (CA) certificate file
    pub ca_cert_path: PathBuf,
    /// Path to client certificate file (for client authentication)
    pub client_cert_path: Option<PathBuf>,
    /// Path to client private key file
    pub client_key_path: Option<PathBuf>,
    /// Require client certificates (mutual authentication)
    pub require_client_auth: bool,
    /// Allowed client certificate common names (CN)
    pub allowed_client_cns: Vec<String>,
    /// TLS version constraints
    pub min_tls_version: TLSVersion,
    /// Certificate validation mode
    pub cert_validation_mode: CertValidationMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TLSVersion {
    TLS12,
    TLS13,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CertValidationMode {
    /// Strict validation (recommended for production)
    Strict,
    /// Allow self-signed certificates (development only)
    AllowSelfSigned,
    /// Skip hostname verification (not recommended)
    SkipHostnameVerification,
}

impl Default for MTLSConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_cert_path: PathBuf::from("certs/server.crt"),
            server_key_path: PathBuf::from("certs/server.key"),
            ca_cert_path: PathBuf::from("certs/ca.crt"),
            client_cert_path: Some(PathBuf::from("certs/client.crt")),
            client_key_path: Some(PathBuf::from("certs/client.key")),
            require_client_auth: true,
            allowed_client_cns: vec!["garbagetruck-client".to_string()],
            min_tls_version: TLSVersion::TLS12,
            cert_validation_mode: CertValidationMode::Strict,
        }
    }
}

impl MTLSConfig {
    /// Load mTLS configuration from environment variables
    pub fn from_env() -> Result<Self> {
        let mut config = Self::default();

        if let Ok(enabled) = std::env::var("GC_MTLS_ENABLED") {
            config.enabled = enabled.parse().unwrap_or(false);
        }

        if let Ok(server_cert) = std::env::var("GC_MTLS_SERVER_CERT") {
            config.server_cert_path = PathBuf::from(server_cert);
        }

        if let Ok(server_key) = std::env::var("GC_MTLS_SERVER_KEY") {
            config.server_key_path = PathBuf::from(server_key);
        }

        if let Ok(ca_cert) = std::env::var("GC_MTLS_CA_CERT") {
            config.ca_cert_path = PathBuf::from(ca_cert);
        }

        if let Ok(client_cert) = std::env::var("GC_MTLS_CLIENT_CERT") {
            config.client_cert_path = Some(PathBuf::from(client_cert));
        }

        if let Ok(client_key) = std::env::var("GC_MTLS_CLIENT_KEY") {
            config.client_key_path = Some(PathBuf::from(client_key));
        }

        if let Ok(require_client_auth) = std::env::var("GC_MTLS_REQUIRE_CLIENT_AUTH") {
            config.require_client_auth = require_client_auth.parse().unwrap_or(true);
        }

        if let Ok(allowed_cns) = std::env::var("GC_MTLS_ALLOWED_CLIENT_CNS") {
            config.allowed_client_cns = allowed_cns
                .split(',')
                .map(|s| s.trim().to_string())
                .collect();
        }

        if let Ok(min_tls) = std::env::var("GC_MTLS_MIN_TLS_VERSION") {
            config.min_tls_version = match min_tls.as_str() {
                "1.2" | "TLS12" => TLSVersion::TLS12,
                "1.3" | "TLS13" => TLSVersion::TLS13,
                _ => TLSVersion::TLS12,
            };
        }

        if let Ok(validation_mode) = std::env::var("GC_MTLS_CERT_VALIDATION_MODE") {
            config.cert_validation_mode = match validation_mode.as_str() {
                "strict" => CertValidationMode::Strict,
                "allow_self_signed" => CertValidationMode::AllowSelfSigned,
                "skip_hostname" => CertValidationMode::SkipHostnameVerification,
                _ => CertValidationMode::Strict,
            };
        }

        Ok(config)
    }

    /// Validate the mTLS configuration
    pub fn validate(&self) -> Result<()> {
        if !self.enabled {
            debug!("mTLS is disabled");
            return Ok(());
        }

        info!("🔒 Validating mTLS configuration");

        // Check server certificates exist
        if !self.server_cert_path.exists() {
            return Err(anyhow::anyhow!(
                "Server certificate not found: {}",
                self.server_cert_path.display()
            ));
        }

        if !self.server_key_path.exists() {
            return Err(anyhow::anyhow!(
                "Server private key not found: {}",
                self.server_key_path.display()
            ));
        }

        if !self.ca_cert_path.exists() {
            return Err(anyhow::anyhow!(
                "CA certificate not found: {}",
                self.ca_cert_path.display()
            ));
        }

        // Check client certificates if client auth is required
        if self.require_client_auth {
            if let Some(ref client_cert_path) = self.client_cert_path {
                if !client_cert_path.exists() {
                    warn!(
                        "Client certificate not found: {} (required for client authentication)",
                        client_cert_path.display()
                    );
                }
            }

            if let Some(ref client_key_path) = self.client_key_path {
                if !client_key_path.exists() {
                    warn!(
                        "Client private key not found: {} (required for client authentication)",
                        client_key_path.display()
                    );
                }
            }
        }

        // Warn about insecure configurations
        if matches!(self.cert_validation_mode, CertValidationMode::AllowSelfSigned) {
            warn!("⚠️ mTLS configured to allow self-signed certificates - not recommended for production");
        }

        if matches!(
            self.cert_validation_mode,
            CertValidationMode::SkipHostnameVerification
        ) {
            warn!("⚠️ mTLS configured to skip hostname verification - not recommended");
        }

        info!("✅ mTLS configuration validated successfully");
        Ok(())
    }

    /// Get summary of mTLS configuration for logging
    pub fn summary(&self) -> MTLSConfigSummary {
        MTLSConfigSummary {
            enabled: self.enabled,
            require_client_auth: self.require_client_auth,
            allowed_client_count: self.allowed_client_cns.len(),
            min_tls_version: self.min_tls_version.clone(),
            cert_validation_mode: self.cert_validation_mode.clone(),
            server_cert_configured: self.server_cert_path.exists(),
            ca_cert_configured: self.ca_cert_path.exists(),
            client_cert_configured: self
                .client_cert_path
                .as_ref()
                .map_or(false, |p| p.exists()),
        }
    }
}

/// Summary of mTLS configuration for logging and monitoring
#[derive(Debug, Clone, Serialize)]
pub struct MTLSConfigSummary {
    pub enabled: bool,
    pub require_client_auth: bool,
    pub allowed_client_count: usize,
    pub min_tls_version: TLSVersion,
    pub cert_validation_mode: CertValidationMode,
    pub server_cert_configured: bool,
    pub ca_cert_configured: bool,
    pub client_cert_configured: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::fs;

    #[test]
    fn test_mtls_config_default() {
        let config = MTLSConfig::default();
        assert!(!config.enabled);
        assert!(config.require_client_auth);
        assert_eq!(config.allowed_client_cns.len(), 1);
    }

    #[test]
    fn test_mtls_config_from_env() {
        std::env::set_var("GC_MTLS_ENABLED", "true");
        std::env::set_var("GC_MTLS_REQUIRE_CLIENT_AUTH", "false");
        std::env::set_var("GC_MTLS_ALLOWED_CLIENT_CNS", "client1,client2,client3");

        let config = MTLSConfig::from_env().unwrap();
        assert!(config.enabled);
        assert!(!config.require_client_auth);
        assert_eq!(config.allowed_client_cns.len(), 3);

        // Cleanup
        std::env::remove_var("GC_MTLS_ENABLED");
        std::env::remove_var("GC_MTLS_REQUIRE_CLIENT_AUTH");
        std::env::remove_var("GC_MTLS_ALLOWED_CLIENT_CNS");
    }

    #[test]
    fn test_mtls_validation_disabled() {
        let config = MTLSConfig {
            enabled: false,
            ..Default::default()
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_mtls_validation_missing_certs() {
        let config = MTLSConfig {
            enabled: true,
            server_cert_path: PathBuf::from("/nonexistent/server.crt"),
            ..Default::default()
        };

        assert!(config.validate().is_err());
    }

    #[test]
    fn test_mtls_validation_with_certs() {
        let temp_dir = tempdir().unwrap();
        
        // Create dummy certificate files
        let server_cert = temp_dir.path().join("server.crt");
        let server_key = temp_dir.path().join("server.key");
        let ca_cert = temp_dir.path().join("ca.crt");
        
        fs::write(&server_cert, "dummy cert").unwrap();
        fs::write(&server_key, "dummy key").unwrap();
        fs::write(&ca_cert, "dummy ca").unwrap();

        let config = MTLSConfig {
            enabled: true,
            server_cert_path: server_cert,
            server_key_path: server_key,
            ca_cert_path: ca_cert,
            require_client_auth: false,
            ..Default::default()
        };

        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_mtls_config_summary() {
        let config = MTLSConfig::default();
        let summary = config.summary();
        
        assert!(!summary.enabled);
        assert!(summary.require_client_auth);
        assert_eq!(summary.allowed_client_count, 1);
    }
}