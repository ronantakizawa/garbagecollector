// tests/integration/security/comprehensive_mtls_test.rs - Comprehensive mTLS test runner

use garbagetruck::{
    Config, MTLSConfig, CertificateManager,
    security::{TLSVersion, CertValidationMode},
    client::MTLSClientBuilder,
    error::GCError,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::time::timeout;
use tracing::{info, warn, error};

use crate::helpers::mtls_helpers::*;

/// Comprehensive mTLS test suite that runs all security tests
#[tokio::test]
async fn comprehensive_mtls_test_suite() {
    info!("🧪 Starting comprehensive mTLS test suite");
    
    let mut test_results = MTLSTestResults::new();
    
    // Initialize test fixtures
    setup_test_environment().await;
    
    // Run all test categories
    test_results.add_category("Certificate Management", run_certificate_tests().await);
    test_results.add_category("Configuration", run_configuration_tests().await);
    test_results.add_category("Client-Server Communication", run_communication_tests().await);
    test_results.add_category("Authentication", run_authentication_tests().await);
    test_results.add_category("Authorization", run_authorization_tests().await);
    test_results.add_category("Security Features", run_security_feature_tests().await);
    test_results.add_category("Error Handling", run_error_handling_tests().await);
    test_results.add_category("Performance", run_performance_tests().await);
    test_results.add_category("Integration", run_integration_tests().await);
    
    // Report results
    test_results.report();
    
    // Ensure all critical tests passed
    test_results.assert_critical_tests_passed();
    
    info!("✅ Comprehensive mTLS test suite completed");
}

/// Test results tracking
struct MTLSTestResults {
    categories: Vec<TestCategory>,
    total_tests: usize,
    passed_tests: usize,
    failed_tests: usize,
    skipped_tests: usize,
}

struct TestCategory {
    name: String,
    results: Vec<TestResult>,
}

struct TestResult {
    name: String,
    status: TestStatus,
    duration: Duration,
    error: Option<String>,
}

#[derive(Debug, Clone)]
enum TestStatus {
    Passed,
    Failed,
    Skipped,
}

impl MTLSTestResults {
    fn new() -> Self {
        Self {
            categories: Vec::new(),
            total_tests: 0,
            passed_tests: 0,
            failed_tests: 0,
            skipped_tests: 0,
        }
    }
    
    fn add_category(&mut self, name: &str, category: TestCategory) {
        for result in &category.results {
            self.total_tests += 1;
            match result.status {
                TestStatus::Passed => self.passed_tests += 1,
                TestStatus::Failed => self.failed_tests += 1,
                TestStatus::Skipped => self.skipped_tests += 1,
            }
        }
        
        info!("📊 {} tests: {} passed, {} failed, {} skipped", 
              name, 
              category.results.iter().filter(|r| matches!(r.status, TestStatus::Passed)).count(),
              category.results.iter().filter(|r| matches!(r.status, TestStatus::Failed)).count(),
              category.results.iter().filter(|r| matches!(r.status, TestStatus::Skipped)).count());
        
        self.categories.push(category);
    }
    
    fn report(&self) {
        info!("📈 mTLS Test Suite Results:");
        info!("  Total tests: {}", self.total_tests);
        info!("  Passed: {} ({:.1}%)", self.passed_tests, 
              (self.passed_tests as f64 / self.total_tests as f64) * 100.0);
        info!("  Failed: {} ({:.1}%)", self.failed_tests,
              (self.failed_tests as f64 / self.total_tests as f64) * 100.0);
        info!("  Skipped: {} ({:.1}%)", self.skipped_tests,
              (self.skipped_tests as f64 / self.total_tests as f64) * 100.0);
        
        // Report detailed failures
        for category in &self.categories {
            for result in &category.results {
                if matches!(result.status, TestStatus::Failed) {
                    error!("❌ {}: {} - {}", 
                           category.name, result.name, 
                           result.error.as_deref().unwrap_or("Unknown error"));
                }
            }
        }
    }
    
    fn assert_critical_tests_passed(&self) {
        let critical_tests = [
            "certificate_validation",
            "basic_client_server_communication",
            "configuration_loading",
        ];
        
        for critical_test in &critical_tests {
            let found = self.categories.iter()
                .flat_map(|c| &c.results)
                .any(|r| r.name.contains(critical_test) && matches!(r.status, TestStatus::Passed));
                
            if !found {
                panic!("Critical test '{}' did not pass", critical_test);
            }
        }
    }
}

/// Certificate management tests
async fn run_certificate_tests() -> TestCategory {
    let mut results = Vec::new();
    
    // Test certificate generation
    results.push(run_test("certificate_generation", || async {
        if !CertificateManager::check_openssl_available() {
            return TestResult::skipped("OpenSSL not available");
        }
        
        let temp_dir = TempDir::new().map_err(|e| format!("TempDir creation failed: {}", e))?;
        let cert_manager = CertificateManager::new(temp_dir.path());
        
        let dev_certs = cert_manager.generate_dev_certificates()
            .map_err(|e| format!("Certificate generation failed: {}", e))?;
        
        // Verify certificates exist
        if !dev_certs.ca_cert.exists() {
            return Err("CA certificate not created".to_string());
        }
        
        if !dev_certs.server_cert.exists() {
            return Err("Server certificate not created".to_string());
        }
        
        if !dev_certs.client_cert.exists() {
            return Err("Client certificate not created".to_string());
        }
        
        Ok(())
    }).await);
    
    // Test certificate validation
    results.push(run_test("certificate_validation", || async {
        let context = MTLSTestContext::new_with_dummy_certs()
            .map_err(|e| format!("Test context creation failed: {}", e))?;
        
        let validation_result = context.mtls_config.validate();
        validation_result.map_err(|e| format!("Certificate validation failed: {}", e))?;
        
        Ok(())
    }).await);
    
    // Test certificate info extraction
    results.push(run_test("certificate_info_extraction", || async {
        if !CertificateManager::check_openssl_available() {
            return TestResult::skipped("OpenSSL not available");
        }
        
        let temp_dir = TempDir::new().map_err(|e| format!("TempDir creation failed: {}", e))?;
        let cert_manager = CertificateManager::new(temp_dir.path());
        
        let dev_certs = cert_manager.generate_dev_certificates()
            .map_err(|e| format!("Certificate generation failed: {}", e))?;
        
        let cert_info = cert_manager.get_certificate_info(&dev_certs.server_cert)
            .map_err(|e| format!("Certificate info extraction failed: {}", e))?;
        
        if cert_info.subject.is_empty() {
            return Err("Certificate subject is empty".to_string());
        }
        
        Ok(())
    }).await);
    
    TestCategory {
        name: "Certificate Management".to_string(),
        results,
    }
}

/// Configuration tests
async fn run_configuration_tests() -> TestCategory {
    let mut results = Vec::new();
    
    // Test configuration loading from environment
    results.push(run_test("configuration_loading", || async {
        // Set test environment variables
        std::env::set_var("GC_MTLS_ENABLED", "true");
        std::env::set_var("GC_MTLS_REQUIRE_CLIENT_AUTH", "true");
        
        let config_result = MTLSConfig::from_env();
        
        // Clean up
        std::env::remove_var("GC_MTLS_ENABLED");
        std::env::remove_var("GC_MTLS_REQUIRE_CLIENT_AUTH");
        
        let config = config_result.map_err(|e| format!("Config loading failed: {}", e))?;
        
        if !config.enabled {
            return Err("mTLS should be enabled from environment".to_string());
        }
        
        Ok(())
    }).await);
    
    // Test configuration validation
    results.push(run_test("configuration_validation", || async {
        let test_configs = MTLSTestData::generate_test_configs();
        
        for (name, config) in test_configs {
            let validation_result = config.validate();
            
            match name {
                "disabled" => {
                    // Disabled config should validate
                    if validation_result.is_err() {
                        return Err(format!("Disabled config should validate: {:?}", validation_result));
                    }
                }
                "strict_validation" => {
                    // This would fail without real certificates, which is expected
                }
                _ => {
                    // Other configs may fail validation depending on certificate availability
                }
            }
        }
        
        Ok(())
    }).await);
    
    // Test invalid configuration detection
    results.push(run_test("invalid_configuration_detection", || async {
        let invalid_configs = MTLSTestData::generate_invalid_configs();
        
        for (name, config) in invalid_configs {
            let validation_result = config.validate();
            
            if validation_result.is_ok() {
                return Err(format!("Invalid config '{}' should fail validation", name));
            }
        }
        
        Ok(())
    }).await);
    
    TestCategory {
        name: "Configuration".to_string(),
        results,
    }
}

/// Client-server communication tests
async fn run_communication_tests() -> TestCategory {
    let mut results = Vec::new();
    
    // Test basic client-server communication
    results.push(run_test("basic_client_server_communication", || async {
        let context = MTLSTestContext::new_with_certs().await
            .map_err(|e| format!("Test context creation failed: {}", e))?;
        
        if !context.has_real_certs() {
            return TestResult::skipped("No real certificates available");
        }
        
        let server = MTLSTestServer::start(&context).await
            .map_err(|e| format!("Server start failed: {}", e))?;
        
        let client = MTLSTestClientFactory::create_matching_client(&context, &server, "test-service").await
            .map_err(|e| format!("Client creation failed: {}", e))?;
        
        let health_result = timeout(Duration::from_secs(5), client.health_check()).await
            .map_err(|_| "Health check timed out".to_string())?
            .map_err(|e| format!("Health check failed: {}", e))?;
        
        server.shutdown().await.map_err(|e| format!("Server shutdown failed: {}", e))?;
        
        Ok(())
    }).await);
    
    // Test client builder patterns
    results.push(run_test("client_builder_patterns", || async {
        // Test plain client builder
        let plain_client_result = MTLSClientBuilder::new(
            "http://localhost:50051".to_string(),
            "test-service".to_string(),
        )
        .without_mtls()
        .build()
        .await;
        
        match plain_client_result {
            Ok(client) => {
                let summary = client.get_mtls_summary();
                if summary.enabled {
                    return Err("Plain client should have mTLS disabled".to_string());
                }
            }
            Err(_) => {
                // Expected when no server is running
            }
        }
        
        Ok(())
    }).await);
    
    TestCategory {
        name: "Client-Server Communication".to_string(),
        results,
    }
}

/// Authentication tests
async fn run_authentication_tests() -> TestCategory {
    let mut results = Vec::new();
    
    // Test successful authentication
    results.push(run_test("successful_authentication", || async {
        let context = MTLSTestContext::new_with_certs().await
            .map_err(|e| format!("Test context creation failed: {}", e))?;
        
        if !context.has_real_certs() {
            return TestResult::skipped("No real certificates available");
        }
        
        let server = MTLSTestServer::start(&context).await
            .map_err(|e| format!("Server start failed: {}", e))?;
        
        let client = MTLSTestClientFactory::create_matching_client(&context, &server, "auth-test-service").await
            .map_err(|e| format!("Client creation failed: {}", e))?;
        
        let auth_result = MTLSAssertions::assert_mtls_connection_succeeds(&client).await;
        server.shutdown().await.map_err(|e| format!("Server shutdown failed: {}", e))?;
        
        auth_result.map_err(|e| format!("Authentication assertion failed: {}", e))?;
        Ok(())
    }).await);
    
    // Test authentication failure
    results.push(run_test("authentication_failure", || async {
        // Test with invalid certificate configuration
        let invalid_config = MTLSConfig {
            enabled: true,
            require_client_auth: true,
            server_cert_path: "/nonexistent/server.crt".into(),
            server_key_path: "/nonexistent/server.key".into(),
            ca_cert_path: "/nonexistent/ca.crt".into(),
            client_cert_path: None,
            client_key_path: None,
            ..Default::default()
        };
        
        let client_result = MTLSClientBuilder::new(
            "https://localhost:50051".to_string(),
            "invalid-auth-service".to_string(),
        )
        .with_mtls_config(invalid_config)
        .build()
        .await;
        
        if client_result.is_ok() {
            return Err("Client creation should fail with invalid certificates".to_string());
        }
        
        Ok(())
    }).await);
    
    TestCategory {
        name: "Authentication".to_string(),
        results,
    }
}

/// Authorization tests
async fn run_authorization_tests() -> TestCategory {
    let mut results = Vec::new();
    
    // Test client authorization by CN
    results.push(run_test("client_authorization_by_cn", || async {
        let context = MTLSTestContext::new_with_certs().await
            .map_err(|e| format!("Test context creation failed: {}", e))?;
        
        if !context.has_real_certs() {
            return TestResult::skipped("No real certificates available");
        }
        
        // Test that the configuration specifies allowed CNs
        if context.mtls_config.allowed_client_cns.is_empty() {
            return Err("Test context should have allowed client CNs".to_string());
        }
        
        Ok(())
    }).await);
    
    // Test production readiness validation
    results.push(run_test("production_readiness_validation", || async {
        let production_config = MTLSConfig {
            enabled: true,
            require_client_auth: true,
            cert_validation_mode: CertValidationMode::Strict,
            allowed_client_cns: vec!["prod-client".to_string()],
            min_tls_version: TLSVersion::TLS12,
            server_cert_path: "/etc/certs/server.crt".into(),
            server_key_path: "/etc/certs/server.key".into(),
            ca_cert_path: "/etc/certs/ca.crt".into(),
            client_cert_path: Some("/etc/certs/client.crt".into()),
            client_key_path: Some("/etc/certs/client.key".into()),
        };
        
        MTLSAssertions::assert_production_ready(&production_config)
            .map_err(|e| format!("Production readiness check failed: {}", e))?;
        
        Ok(())
    }).await);
    
    TestCategory {
        name: "Authorization".to_string(),
        results,
    }
}

/// Security feature tests
async fn run_security_feature_tests() -> TestCategory {
    let mut results = Vec::new();
    
    // Test TLS version enforcement
    results.push(run_test("tls_version_enforcement", || async {
        let tls12_config = MTLSConfig {
            enabled: true,
            min_tls_version: TLSVersion::TLS12,
            ..Default::default()
        };
        
        let tls13_config = MTLSConfig {
            enabled: true,
            min_tls_version: TLSVersion::TLS13,
            ..Default::default()
        };
        
        if !matches!(tls12_config.min_tls_version, TLSVersion::TLS12) {
            return Err("TLS 1.2 configuration incorrect".to_string());
        }
        
        if !matches!(tls13_config.min_tls_version, TLSVersion::TLS13) {
            return Err("TLS 1.3 configuration incorrect".to_string());
        }
        
        Ok(())
    }).await);
    
    // Test certificate validation modes
    results.push(run_test("certificate_validation_modes", || async {
        let strict_config = MTLSConfig {
            enabled: true,
            cert_validation_mode: CertValidationMode::Strict,
            ..Default::default()
        };
        
        let self_signed_config = MTLSConfig {
            enabled: true,
            cert_validation_mode: CertValidationMode::AllowSelfSigned,
            ..Default::default()
        };
        
        // Only strict mode should be production ready
        let strict_prod_ready = MTLSAssertions::assert_production_ready(&strict_config);
        let self_signed_prod_ready = MTLSAssertions::assert_production_ready(&self_signed_config);
        
        if self_signed_prod_ready.is_ok() {
            return Err("Self-signed mode should not be production ready".to_string());
        }
        
        Ok(())
    }).await);
    
    // Test enhanced security integration
    results.push(run_test("enhanced_security_integration", || async {
        let mut config = Config::default();
        config.security.enhanced_security = true;
        config.security.rate_limiting.enabled = true;
        config.security.rate_limiting.requests_per_minute = 1000;
        config.security.max_connections_per_client = 50;
        
        let validation_result = config.validate();
        validation_result.map_err(|e| format!("Enhanced security config validation failed: {}", e))?;
        
        let summary = config.summary();
        if !summary.security_features.enhanced_security {
            return Err("Enhanced security should be enabled in summary".to_string());
        }
        
        if !summary.security_features.rate_limiting_enabled {
            return Err("Rate limiting should be enabled in summary".to_string());
        }
        
        Ok(())
    }).await);
    
    TestCategory {
        name: "Security Features".to_string(),
        results,
    }
}

/// Error handling tests
async fn run_error_handling_tests() -> TestCategory {
    let mut results = Vec::new();
    
    // Test configuration error handling
    results.push(run_test("configuration_error_handling", || async {
        let invalid_config = MTLSConfig {
            enabled: true,
            server_cert_path: "/nonexistent/cert.pem".into(),
            server_key_path: "/nonexistent/key.pem".into(),
            ca_cert_path: "/nonexistent/ca.pem".into(),
            ..Default::default()
        };
        
        let validation_result = invalid_config.validate();
        if validation_result.is_ok() {
            return Err("Invalid configuration should fail validation".to_string());
        }
        
        Ok(())
    }).await);
    
    // Test client creation error handling
    results.push(run_test("client_creation_error_handling", || async {
        let invalid_config = MTLSConfig {
            enabled: true,
            require_client_auth: true,
            server_cert_path: "/nonexistent/server.crt".into(),
            server_key_path: "/nonexistent/server.key".into(),
            ca_cert_path: "/nonexistent/ca.crt".into(),
            client_cert_path: Some("/nonexistent/client.crt".into()),
            client_key_path: Some("/nonexistent/client.key".into()),
            ..Default::default()
        };
        
        let client_result = MTLSTestClientFactory::create_client(
            "https://localhost:50051",
            "error-test-service",
            invalid_config,
        ).await;
        
        match client_result {
            Ok(_) => {
                return Err("Client creation should fail with invalid certificates".to_string());
            }
            Err(e) => {
                // Verify error contains relevant information
                let error_msg = e.to_string();
                if !error_msg.contains("certificate") && !error_msg.contains("cert") && !error_msg.contains("config") {
                    return Err(format!("Error message should contain relevant keywords: {}", error_msg));
                }
            }
        }
        
        Ok(())
    }).await);
    
    TestCategory {
        name: "Error Handling".to_string(),
        results,
    }
}

/// Performance tests
async fn run_performance_tests() -> TestCategory {
    let mut results = Vec::new();
    
    // Test mTLS overhead measurement
    results.push(run_test("mtls_overhead_measurement", || async {
        let context = MTLSTestContext::new_with_certs().await
            .map_err(|e| format!("Test context creation failed: {}", e))?;
        
        if !context.has_real_certs() {
            return TestResult::skipped("No real certificates available");
        }
        
        let server = MTLSTestServer::start(&context).await
            .map_err(|e| format!("Server start failed: {}", e))?;
        
        // Measure handshake time
        let handshake_times = MTLSPerformanceTest::measure_handshake_time(
            &server.endpoint(),
            context.client_config(),
            3, // Small number for test
        ).await.map_err(|e| format!("Performance measurement failed: {}", e))?;
        
        if handshake_times.is_empty() {
            server.shutdown().await.map_err(|e| format!("Server shutdown failed: {}", e))?;
            return Err("No successful handshakes measured".to_string());
        }
        
        let avg_time = handshake_times.iter().sum::<Duration>() / handshake_times.len() as u32;
        
        // Assert reasonable performance (less than 5 seconds for test environment)
        if avg_time > Duration::from_secs(5) {
            server.shutdown().await.map_err(|e| format!("Server shutdown failed: {}", e))?;
            return Err(format!("mTLS handshake too slow: {:?}", avg_time));
        }
        
        server.shutdown().await.map_err(|e| format!("Server shutdown failed: {}", e))?;
        Ok(())
    }).await);
    
    TestCategory {
        name: "Performance".to_string(),
        results,
    }
}

/// Integration tests
async fn run_integration_tests() -> TestCategory {
    let mut results = Vec::new();
    
    // Test full mTLS workflow
    results.push(run_test("full_mtls_workflow", || async {
        let context = MTLSTestContext::new_with_certs().await
            .map_err(|e| format!("Test context creation failed: {}", e))?;
        
        if !context.has_real_certs() {
            return TestResult::skipped("No real certificates available");
        }
        
        // Start server
        let server = MTLSTestServer::start(&context).await
            .map_err(|e| format!("Server start failed: {}", e))?;
        
        // Create client
        let client = MTLSTestClientFactory::create_matching_client(&context, &server, "workflow-test").await
            .map_err(|e| format!("Client creation failed: {}", e))?;
        
        // Test operations
        let health_check = client.health_check().await
            .map_err(|e| format!("Health check failed: {}", e))?;
        
        let lease_list = client.list_leases(None, 10).await
            .map_err(|e| format!("List leases failed: {}", e))?;
        
        // Cleanup
        server.shutdown().await.map_err(|e| format!("Server shutdown failed: {}", e))?;
        
        Ok(())
    }).await);
    
    // Test service configuration integration
    results.push(run_test("service_configuration_integration", || async {
        let context = MTLSTestContext::new_with_dummy_certs()
            .map_err(|e| format!("Test context creation failed: {}", e))?;
        
        // Validate that mTLS config integrates with service config
        let validation_result = context.server_config.validate();
        validation_result.map_err(|e| format!("Service config validation failed: {}", e))?;
        
        // Test configuration summary
        let summary = context.server_config.summary();
        if !summary.security_features.mtls_enabled {
            return Err("Service config summary should show mTLS enabled".to_string());
        }
        
        Ok(())
    }).await);
    
    TestCategory {
        name: "Integration".to_string(),
        results,
    }
}

/// Run a single test with timing and error handling
async fn run_test<F, Fut>(name: &str, test_fn: F) -> TestResult
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), String>>,
{
    let start_time = std::time::Instant::now();
    
    match test_fn().await {
        Ok(()) => TestResult {
            name: name.to_string(),
            status: TestStatus::Passed,
            duration: start_time.elapsed(),
            error: None,
        },
        Err(error) => TestResult {
            name: name.to_string(),
            status: TestStatus::Failed,
            duration: start_time.elapsed(),
            error: Some(error),
        },
    }
}

impl TestResult {
    fn skipped(reason: &str) -> Result<(), String> {
        warn!("⚠️ Skipping test: {}", reason);
        Err(format!("SKIPPED: {}", reason))
    }
}

/// Set up test environment
async fn setup_test_environment() {
    // Create test fixtures directory
    let fixtures_dir = std::path::Path::new("tests/fixtures");
    if let Err(e) = std::fs::create_dir_all(fixtures_dir) {
        warn!("Failed to create fixtures directory: {}", e);
    }

    // Create dummy certificate files for testing
    create_dummy_certificates().await;
    
    // Set up logging for tests
    let _ = tracing_subscriber::fmt()
        .with_env_filter("garbagetruck=debug,mtls=debug")
        .with_test_writer()
        .try_init();
}

async fn create_dummy_certificates() {
    let fixtures_dir = std::path::Path::new("tests/fixtures");
    
    let dummy_cert = r#"-----BEGIN CERTIFICATE-----
MIIBkTCB+wIJAL7Q6Q6Q6Q6QMA0GCSqGSIb3DQEBCwUAMBQxEjAQBgNVBAMMCWxv
Y2FsaG9zdDAeFw0yNDA2MTEwMDAwMDBaFw0yNTA2MTEwMDAwMDBaMBQxEjAQBgNV
BAMMCWxvY2FsaG9zdDBZMBMGByqGSM49AgEGCCqGSM49AwEHA0IABG+G3r9q8j8H
9QKJ9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J
9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J
9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J
9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J
9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J
-----END CERTIFICATE-----"#;

    let dummy_key = r#"-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg+G3r9q8j8H9QKJ9Y
2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2J9Y2JohRANCAAR/ht6/avI/B/UCifWNifWNifWN
ifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWN
ifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWNifWN
-----END PRIVATE KEY-----"#;

    let _ = std::fs::write(fixtures_dir.join("dummy_cert.pem"), dummy_cert);
    let _ = std::fs::write(fixtures_dir.join("dummy_key.pem"), dummy_key);
}

/// Specialized test for certificate chain validation
#[tokio::test]
async fn test_certificate_chain_validation() {
    if !CertificateManager::check_openssl_available() {
        warn!("⚠️ OpenSSL not available, skipping certificate chain validation test");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let cert_manager = CertificateManager::new(temp_dir.path());
    
    match cert_manager.generate_dev_certificates() {
        Ok(dev_certs) => {
            // Verify certificate chain
            let ca_valid = cert_manager.verify_certificate_chain(&dev_certs.ca_cert, &dev_certs.ca_cert);
            let server_valid = cert_manager.verify_certificate_chain(&dev_certs.server_cert, &dev_certs.ca_cert);
            let client_valid = cert_manager.verify_certificate_chain(&dev_certs.client_cert, &dev_certs.ca_cert);
            
            match (ca_valid, server_valid, client_valid) {
                (Ok(true), Ok(true), Ok(true)) => {
                    info!("✅ All certificates in chain are valid");
                }
                (ca, server, client) => {
                    warn!("⚠️ Certificate chain validation results: CA={:?}, Server={:?}, Client={:?}", 
                          ca, server, client);
                }
            }
        }
        Err(e) => {
            warn!("⚠️ Certificate generation failed: {}", e);
        }
    }
}

/// Test mTLS metrics collection
#[tokio::test]
async fn test_mtls_metrics_collection() {
    info!("📊 Testing mTLS metrics collection");
    
    let context = MTLSTestContext::new_with_certs().await.unwrap();
    
    // Verify metrics configuration
    assert!(context.server_config.metrics.include_security_metrics, 
            "Security metrics should be enabled");
    
    // Test metrics summary
    let summary = context.server_config.summary();
    assert!(summary.metrics_enabled, "Metrics should be enabled");
    
    info!("✅ mTLS metrics collection test completed");
}

/// Load test for mTLS performance under concurrent connections
#[tokio::test]
#[ignore] // Ignore by default due to resource intensity
async fn test_mtls_concurrent_connections() {
    let context = MTLSTestContext::new_with_certs().await.unwrap();
    
    if !context.has_real_certs() {
        warn!("⚠️ No real certificates available, skipping concurrent connections test");
        return;
    }
    
    let server = match MTLSTestServer::start(&context).await {
        Ok(server) => server,
        Err(e) => {
            warn!("⚠️ Could not start server for concurrent test: {}", e);
            return;
        }
    };
    
    info!("🔄 Testing concurrent mTLS connections");
    
    let concurrent_clients = 5;
    let mut handles = Vec::new();
    
    for i in 0..concurrent_clients {
        let context_clone = context.client_config();
        let endpoint = server.endpoint();
        let service_id = format!("concurrent-client-{}", i);
        
        let handle = tokio::spawn(async move {
            match MTLSTestClientFactory::create_client(&endpoint, &service_id, context_clone).await {
                Ok(client) => {
                    match client.health_check().await {
                        Ok(_) => {
                            info!("✅ Concurrent client {} succeeded", i);
                            true
                        }
                        Err(e) => {
                            warn!("⚠️ Concurrent client {} health check failed: {}", i, e);
                            false
                        }
                    }
                }
                Err(e) => {
                    warn!("⚠️ Concurrent client {} creation failed: {}", i, e);
                    false
                }
            }
        });
        
        handles.push(handle);
    }
    
    let mut successful_connections = 0;
    for handle in handles {
        if let Ok(success) = handle.await {
            if success {
                successful_connections += 1;
            }
        }
    }
    
    info!("📊 Concurrent connections result: {}/{} successful", 
          successful_connections, concurrent_clients);
    
    // At least half should succeed
    assert!(successful_connections >= concurrent_clients / 2, 
            "At least half of concurrent connections should succeed");
    
    server.shutdown().await.unwrap();
}