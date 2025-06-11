// src/security/certificates.rs - Certificate generation and management utilities

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

/// Certificate generation and management utilities
pub struct CertificateManager {
    cert_dir: PathBuf,
}

impl CertificateManager {
    /// Create a new certificate manager
    pub fn new<P: AsRef<Path>>(cert_dir: P) -> Self {
        Self {
            cert_dir: cert_dir.as_ref().to_path_buf(),
        }
    }

    /// Generate a complete mTLS certificate chain for development/testing
    pub fn generate_dev_certificates(&self) -> Result<DevCertificates> {
        info!("🔐 Generating development mTLS certificates");

        // Create certificate directory
        fs::create_dir_all(&self.cert_dir)
            .with_context(|| format!("Failed to create cert directory: {}", self.cert_dir.display()))?;

        // Generate CA certificate
        let ca_cert_path = self.cert_dir.join("ca.crt");
        let ca_key_path = self.cert_dir.join("ca.key");
        self.generate_ca_certificate(&ca_cert_path, &ca_key_path)?;

        // Generate server certificate
        let server_cert_path = self.cert_dir.join("server.crt");
        let server_key_path = self.cert_dir.join("server.key");
        self.generate_server_certificate(&server_cert_path, &server_key_path, &ca_cert_path, &ca_key_path)?;

        // Generate client certificate
        let client_cert_path = self.cert_dir.join("client.crt");
        let client_key_path = self.cert_dir.join("client.key");
        self.generate_client_certificate(&client_cert_path, &client_key_path, &ca_cert_path, &ca_key_path)?;

        info!("✅ Development certificates generated successfully");

        Ok(DevCertificates {
            ca_cert: ca_cert_path,
            ca_key: ca_key_path,
            server_cert: server_cert_path,
            server_key: server_key_path,
            client_cert: client_cert_path,
            client_key: client_key_path,
        })
    }

    /// Generate CA (Certificate Authority) certificate
    fn generate_ca_certificate(&self, cert_path: &Path, key_path: &Path) -> Result<()> {
        info!("📜 Generating CA certificate");

        // Generate CA private key
        let ca_key_output = Command::new("openssl")
            .args([
                "genrsa",
                "-out", &key_path.to_string_lossy(),
                "2048",
            ])
            .output()
            .context("Failed to execute openssl for CA key generation")?;

        if !ca_key_output.status.success() {
            return Err(anyhow::anyhow!(
                "OpenSSL CA key generation failed: {}",
                String::from_utf8_lossy(&ca_key_output.stderr)
            ));
        }

        // Generate CA certificate
        let ca_cert_output = Command::new("openssl")
            .args([
                "req",
                "-new",
                "-x509",
                "-key", &key_path.to_string_lossy(),
                "-out", &cert_path.to_string_lossy(),
                "-days", "365",
                "-subj", "/C=US/ST=Dev/L=Dev/O=GarbageTruck/OU=Dev/CN=GarbageTruck-CA",
            ])
            .output()
            .context("Failed to execute openssl for CA certificate generation")?;

        if !ca_cert_output.status.success() {
            return Err(anyhow::anyhow!(
                "OpenSSL CA certificate generation failed: {}",
                String::from_utf8_lossy(&ca_cert_output.stderr)
            ));
        }

        info!("✅ CA certificate generated: {}", cert_path.display());
        Ok(())
    }

    /// Generate server certificate signed by CA
    fn generate_server_certificate(
        &self,
        cert_path: &Path,
        key_path: &Path,
        ca_cert_path: &Path,
        ca_key_path: &Path,
    ) -> Result<()> {
        info!("🖥️ Generating server certificate");

        // Generate server private key
        let server_key_output = Command::new("openssl")
            .args([
                "genrsa",
                "-out", &key_path.to_string_lossy(),
                "2048",
            ])
            .output()
            .context("Failed to execute openssl for server key generation")?;

        if !server_key_output.status.success() {
            return Err(anyhow::anyhow!(
                "OpenSSL server key generation failed: {}",
                String::from_utf8_lossy(&server_key_output.stderr)
            ));
        }

        // Generate server certificate signing request (CSR)
        let csr_path = self.cert_dir.join("server.csr");
        let server_csr_output = Command::new("openssl")
            .args([
                "req",
                "-new",
                "-key", &key_path.to_string_lossy(),
                "-out", &csr_path.to_string_lossy(),
                "-subj", "/C=US/ST=Dev/L=Dev/O=GarbageTruck/OU=Server/CN=localhost",
            ])
            .output()
            .context("Failed to execute openssl for server CSR generation")?;

        if !server_csr_output.status.success() {
            return Err(anyhow::anyhow!(
                "OpenSSL server CSR generation failed: {}",
                String::from_utf8_lossy(&server_csr_output.stderr)
            ));
        }

        // Create server certificate extensions file
        let ext_path = self.cert_dir.join("server.ext");
        let server_extensions = r#"authorityKeyIdentifier=keyid,issuer
basicConstraints=CA:FALSE
keyUsage = digitalSignature, nonRepudiation, keyEncipherment, dataEncipherment
extendedKeyUsage = serverAuth
subjectAltName = @alt_names

[alt_names]
DNS.1 = localhost
DNS.2 = garbagetruck
DNS.3 = garbagetruck-server
DNS.4 = *.garbagetruck.local
IP.1 = 127.0.0.1
IP.2 = ::1
"#;
        fs::write(&ext_path, server_extensions)
            .context("Failed to write server extensions file")?;

        // Sign server certificate with CA
        let server_cert_output = Command::new("openssl")
            .args([
                "x509",
                "-req",
                "-in", &csr_path.to_string_lossy(),
                "-CA", &ca_cert_path.to_string_lossy(),
                "-CAkey", &ca_key_path.to_string_lossy(),
                "-CAcreateserial",
                "-out", &cert_path.to_string_lossy(),
                "-days", "365",
                "-extfile", &ext_path.to_string_lossy(),
            ])
            .output()
            .context("Failed to execute openssl for server certificate signing")?;

        if !server_cert_output.status.success() {
            return Err(anyhow::anyhow!(
                "OpenSSL server certificate signing failed: {}",
                String::from_utf8_lossy(&server_cert_output.stderr)
            ));
        }

        // Cleanup temporary files
        let _ = fs::remove_file(csr_path);
        let _ = fs::remove_file(ext_path);

        info!("✅ Server certificate generated: {}", cert_path.display());
        Ok(())
    }

    /// Generate client certificate signed by CA
    fn generate_client_certificate(
        &self,
        cert_path: &Path,
        key_path: &Path,
        ca_cert_path: &Path,
        ca_key_path: &Path,
    ) -> Result<()> {
        info!("👤 Generating client certificate");

        // Generate client private key
        let client_key_output = Command::new("openssl")
            .args([
                "genrsa",
                "-out", &key_path.to_string_lossy(),
                "2048",
            ])
            .output()
            .context("Failed to execute openssl for client key generation")?;

        if !client_key_output.status.success() {
            return Err(anyhow::anyhow!(
                "OpenSSL client key generation failed: {}",
                String::from_utf8_lossy(&client_key_output.stderr)
            ));
        }

        // Generate client certificate signing request (CSR)
        let csr_path = self.cert_dir.join("client.csr");
        let client_csr_output = Command::new("openssl")
            .args([
                "req",
                "-new",
                "-key", &key_path.to_string_lossy(),
                "-out", &csr_path.to_string_lossy(),
                "-subj", "/C=US/ST=Dev/L=Dev/O=GarbageTruck/OU=Client/CN=garbagetruck-client",
            ])
            .output()
            .context("Failed to execute openssl for client CSR generation")?;

        if !client_csr_output.status.success() {
            return Err(anyhow::anyhow!(
                "OpenSSL client CSR generation failed: {}",
                String::from_utf8_lossy(&client_csr_output.stderr)
            ));
        }

        // Create client certificate extensions file
        let ext_path = self.cert_dir.join("client.ext");
        let client_extensions = r#"authorityKeyIdentifier=keyid,issuer
basicConstraints=CA:FALSE
keyUsage = digitalSignature, nonRepudiation, keyEncipherment, dataEncipherment
extendedKeyUsage = clientAuth
"#;
        fs::write(&ext_path, client_extensions)
            .context("Failed to write client extensions file")?;

        // Sign client certificate with CA
        let client_cert_output = Command::new("openssl")
            .args([
                "x509",
                "-req",
                "-in", &csr_path.to_string_lossy(),
                "-CA", &ca_cert_path.to_string_lossy(),
                "-CAkey", &ca_key_path.to_string_lossy(),
                "-CAcreateserial",
                "-out", &cert_path.to_string_lossy(),
                "-days", "365",
                "-extfile", &ext_path.to_string_lossy(),
            ])
            .output()
            .context("Failed to execute openssl for client certificate signing")?;

        if !client_cert_output.status.success() {
            return Err(anyhow::anyhow!(
                "OpenSSL client certificate signing failed: {}",
                String::from_utf8_lossy(&client_cert_output.stderr)
            ));
        }

        // Cleanup temporary files
        let _ = fs::remove_file(csr_path);
        let _ = fs::remove_file(ext_path);

        info!("✅ Client certificate generated: {}", cert_path.display());
        Ok(())
    }

    /// Verify certificate chain validity
    pub fn verify_certificate_chain(&self, cert_path: &Path, ca_cert_path: &Path) -> Result<bool> {
        info!("🔍 Verifying certificate chain: {}", cert_path.display());

        let verify_output = Command::new("openssl")
            .args([
                "verify",
                "-CAfile", &ca_cert_path.to_string_lossy(),
                &cert_path.to_string_lossy(),
            ])
            .output()
            .context("Failed to execute openssl verify")?;

        let is_valid = verify_output.status.success();
        
        if is_valid {
            info!("✅ Certificate chain verification successful");
        } else {
            warn!("❌ Certificate chain verification failed: {}",
                String::from_utf8_lossy(&verify_output.stderr));
        }

        Ok(is_valid)
    }

    /// Get certificate information
    pub fn get_certificate_info(&self, cert_path: &Path) -> Result<CertificateInfo> {
        let cert_info_output = Command::new("openssl")
            .args([
                "x509",
                "-in", &cert_path.to_string_lossy(),
                "-text",
                "-noout",
            ])
            .output()
            .context("Failed to execute openssl x509 info")?;

        if !cert_info_output.status.success() {
            return Err(anyhow::anyhow!(
                "OpenSSL certificate info failed: {}",
                String::from_utf8_lossy(&cert_info_output.stderr)
            ));
        }

        let output_text = String::from_utf8_lossy(&cert_info_output.stdout);
        
        // Parse certificate information (simplified)
        let subject = self.extract_field(&output_text, "Subject:")?;
        let issuer = self.extract_field(&output_text, "Issuer:")?;
        let not_before = self.extract_field(&output_text, "Not Before:")?;
        let not_after = self.extract_field(&output_text, "Not After :")?;

        Ok(CertificateInfo {
            subject,
            issuer,
            not_before,
            not_after,
            path: cert_path.to_path_buf(),
        })
    }

    /// Extract field from OpenSSL certificate output
    fn extract_field(&self, text: &str, field_name: &str) -> Result<String> {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(field_name) {
                if let Some(value) = trimmed.strip_prefix(field_name) {
                    return Ok(value.trim().to_string());
                }
            }
        }
        
        Err(anyhow::anyhow!("Field '{}' not found in certificate", field_name))
    }

    /// Check if OpenSSL is available
    pub fn check_openssl_available() -> bool {
        Command::new("openssl")
            .arg("version")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    }

    /// Generate certificates using built-in Rust crypto (fallback)
    pub fn generate_rust_certificates(&self) -> Result<DevCertificates> {
        warn!("⚠️ OpenSSL not available, generating certificates using Rust crypto libraries");
        
        // This would use pure Rust libraries like `rcgen` for certificate generation
        // For now, we'll return an error suggesting OpenSSL installation
        
        Err(anyhow::anyhow!(
            "Certificate generation requires OpenSSL. Please install OpenSSL or use pre-generated certificates."
        ))
    }
}

/// Container for development certificate paths
#[derive(Debug, Clone)]
pub struct DevCertificates {
    pub ca_cert: PathBuf,
    pub ca_key: PathBuf,
    pub server_cert: PathBuf,
    pub server_key: PathBuf,
    pub client_cert: PathBuf,
    pub client_key: PathBuf,
}

impl DevCertificates {
    /// Create mTLS configuration from generated certificates
    pub fn to_mtls_config(&self) -> crate::security::MTLSConfig {
        crate::security::MTLSConfig {
            enabled: true,
            server_cert_path: self.server_cert.clone(),
            server_key_path: self.server_key.clone(),
            ca_cert_path: self.ca_cert.clone(),
            client_cert_path: Some(self.client_cert.clone()),
            client_key_path: Some(self.client_key.clone()),
            require_client_auth: true,
            allowed_client_cns: vec!["garbagetruck-client".to_string()],
            min_tls_version: crate::security::TLSVersion::TLS12,
            cert_validation_mode: crate::security::CertValidationMode::Strict,
        }
    }

    /// Print certificate paths for easy configuration
    pub fn print_config_instructions(&self) {
        info!("🔧 mTLS Configuration Instructions:");
        info!("Set the following environment variables:");
        info!("  export GC_MTLS_ENABLED=true");
        info!("  export GC_MTLS_SERVER_CERT={}", self.server_cert.display());
        info!("  export GC_MTLS_SERVER_KEY={}", self.server_key.display());
        info!("  export GC_MTLS_CA_CERT={}", self.ca_cert.display());
        info!("  export GC_MTLS_CLIENT_CERT={}", self.client_cert.display());
        info!("  export GC_MTLS_CLIENT_KEY={}", self.client_key.display());
        info!("  export GC_MTLS_REQUIRE_CLIENT_AUTH=true");
        info!("  export GC_MTLS_ALLOWED_CLIENT_CNS=garbagetruck-client");
    }
}

/// Information about a certificate
#[derive(Debug, Clone)]
pub struct CertificateInfo {
    pub subject: String,
    pub issuer: String,
    pub not_before: String,
    pub not_after: String,
    pub path: PathBuf,
}

impl CertificateInfo {
    /// Check if certificate is expired or will expire soon
    pub fn is_expiring_soon(&self, days_threshold: u32) -> bool {
        // This would parse the dates and check expiration
        // For simplicity, returning false for now
        false
    }

    /// Extract common name from subject
    pub fn extract_common_name(&self) -> Option<String> {
        // Parse subject string to extract CN
        if let Some(cn_start) = self.subject.find("CN=") {
            let cn_part = &self.subject[cn_start + 3..];
            if let Some(cn_end) = cn_part.find(',') {
                Some(cn_part[..cn_end].trim().to_string())
            } else {
                Some(cn_part.trim().to_string())
            }
        } else {
            None
        }
    }
}

/// Certificate generation CLI utility
pub struct CertGenCLI;

impl CertGenCLI {
    /// Run certificate generation CLI
    pub fn run() -> Result<()> {
        use std::io::{self, Write};

        println!("🔐 GarbageTruck mTLS Certificate Generator");
        println!("==========================================");

        // Check OpenSSL availability
        if !CertificateManager::check_openssl_available() {
            println!("❌ OpenSSL not found. Please install OpenSSL to generate certificates.");
            println!("Installation instructions:");
            println!("  - macOS: brew install openssl");
            println!("  - Ubuntu/Debian: sudo apt-get install openssl");
            println!("  - CentOS/RHEL: sudo yum install openssl or sudo dnf install openssl");
            println!("  - Windows: Download from https://slproweb.com/products/Win32OpenSSL.html");
            return Err(anyhow::anyhow!("OpenSSL required for certificate generation"));
        }

        // Get certificate directory
        print!("Enter certificate directory [./certs]: ");
        io::stdout().flush()?;
        
        let mut cert_dir = String::new();
        io::stdin().read_line(&mut cert_dir)?;
        let cert_dir = cert_dir.trim();
        let cert_dir = if cert_dir.is_empty() { "./certs" } else { cert_dir };

        // Generate certificates
        let cert_manager = CertificateManager::new(cert_dir);
        let dev_certs = cert_manager.generate_dev_certificates()?;

        println!("\n✅ Certificates generated successfully!");
        println!("📁 Certificate directory: {}", cert_dir);
        
        // Verify certificates
        println!("\n🔍 Verifying certificate chain...");
        let server_valid = cert_manager.verify_certificate_chain(&dev_certs.server_cert, &dev_certs.ca_cert)?;
        let client_valid = cert_manager.verify_certificate_chain(&dev_certs.client_cert, &dev_certs.ca_cert)?;

        if server_valid && client_valid {
            println!("✅ All certificates verified successfully!");
        } else {
            println!("⚠️ Certificate verification issues detected");
        }

        // Print configuration instructions
        println!("\n{}", "=".repeat(50));
        dev_certs.print_config_instructions();
        println!("{}", "=".repeat(50));

        Ok(())
    }
}

/// Validate production mTLS configuration
pub fn validate_production_mtls_config(config: &crate::security::MTLSConfig) -> Vec<String> {
    let mut warnings = Vec::new();

    if !config.enabled {
        warnings.push("mTLS is disabled - not recommended for production".to_string());
    }

    if !config.require_client_auth {
        warnings.push("Client authentication is disabled - security risk".to_string());
    }

    if matches!(config.cert_validation_mode, crate::security::CertValidationMode::AllowSelfSigned) {
        warnings.push("Self-signed certificates allowed - not recommended for production".to_string());
    }

    if config.allowed_client_cns.is_empty() {
        warnings.push("No client CN restrictions - any client certificate will be accepted".to_string());
    }

    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_certificate_manager_creation() {
        let temp_dir = tempdir().unwrap();
        let cert_manager = CertificateManager::new(temp_dir.path());
        assert_eq!(cert_manager.cert_dir, temp_dir.path());
    }

    #[test]
    fn test_openssl_availability() {
        // This test will pass or fail depending on whether OpenSSL is installed
        let available = CertificateManager::check_openssl_available();
        println!("OpenSSL available: {}", available);
        // Don't assert, just report status
    }

    #[test]
    fn test_dev_certificates_config() {
        let temp_dir = tempdir().unwrap();
        let dev_certs = DevCertificates {
            ca_cert: temp_dir.path().join("ca.crt"),
            ca_key: temp_dir.path().join("ca.key"),
            server_cert: temp_dir.path().join("server.crt"),
            server_key: temp_dir.path().join("server.key"),
            client_cert: temp_dir.path().join("client.crt"),
            client_key: temp_dir.path().join("client.key"),
        };

        let mtls_config = dev_certs.to_mtls_config();
        assert!(mtls_config.enabled);
        assert!(mtls_config.require_client_auth);
        assert_eq!(mtls_config.allowed_client_cns, vec!["garbagetruck-client"]);
    }

    #[test]
    fn test_certificate_info_cn_extraction() {
        let cert_info = CertificateInfo {
            subject: "C=US, ST=Dev, L=Dev, O=GarbageTruck, OU=Server, CN=localhost".to_string(),
            issuer: "C=US, ST=Dev, L=Dev, O=GarbageTruck, OU=Dev, CN=GarbageTruck-CA".to_string(),
            not_before: "Jan 1 00:00:00 2024 GMT".to_string(),
            not_after: "Jan 1 00:00:00 2025 GMT".to_string(),
            path: PathBuf::from("/tmp/test.crt"),
        };

        let cn = cert_info.extract_common_name();
        assert_eq!(cn, Some("localhost".to_string()));
    }

    #[test]
    fn test_extract_field() {
        let cert_manager = CertificateManager::new("/tmp");
        let text = r#"
        Certificate:
            Data:
                Version: 3 (0x2)
                Serial Number: 1234567890
            Signature Algorithm: sha256WithRSAEncryption
                Issuer: C=US, ST=Dev, L=Dev, O=GarbageTruck, CN=Test-CA
                Validity
                    Not Before: Jan  1 00:00:00 2024 GMT
                    Not After : Dec 31 23:59:59 2024 GMT
                Subject: C=US, ST=Dev, L=Dev, O=GarbageTruck, CN=localhost
        "#;

        let issuer = cert_manager.extract_field(text, "Issuer:").unwrap();
        assert!(issuer.contains("Test-CA"));

        let subject = cert_manager.extract_field(text, "Subject:").unwrap();
        assert!(subject.contains("localhost"));
    }

    #[test]
    fn test_production_config_validation() {
        let mut config = crate::security::MTLSConfig::default();
        config.enabled = false;
        
        let warnings = validate_production_mtls_config(&config);
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("mTLS is disabled")));
        
        // Test production-ready config
        config.enabled = true;
        config.require_client_auth = true;
        config.cert_validation_mode = crate::security::CertValidationMode::Strict;
        config.allowed_client_cns = vec!["test-client".to_string()];
        
        let warnings = validate_production_mtls_config(&config);
        assert!(warnings.is_empty());
    }
}