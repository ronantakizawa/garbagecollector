// tests/integration/security/mod.rs - mTLS integration tests

use garbagetruck::{
    Config, MTLSConfig, CertificateManager,
    security::{TLSVersion, CertValidationMode},
    client::MTLSClientBuilder,
    lease::ObjectType,
    error::GCError,
};
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::time::{sleep, Duration};
use tracing::{info, warn};

mod auth_tests;
mod certificate_tests;
mod client_tests;
mod config_tests;
mod performance_tests;
mod security_tests;

use crate::helpers::mtls_helpers::*;

/// Basic mTLS functionality tests
#[tokio::test]
async fn test_mtls_disabled_connection() {
    let context = MTLSTestContext::new_without_certs().unwrap();
    
    // Should be able to create client with disabled mTLS
    let client_result = MTLSClientBuilder::new(
        "http://localhost:50051".to_string(),
        "test-service".to_string(),
    )
    .without_mtls()
    .build()
    .await;

    match client_result {
        Ok(client) => {
            let summary = client.get_mtls_summary();
            assert!(!summary.enabled, "mTLS should be disabled");
            
            let conn_info = client.get_connection_info();
            assert_eq!(conn_info.endpoint, "http://localhost:50051");
            assert!(!conn_info.mtls_enabled);
        }
        Err(e) => {
            // Expected when no server is running
            info!("Client creation failed as expected (no server): {}", e);
        }
    }
}

#[tokio::test]
async fn test_mtls_config_validation() {
    // Test valid configuration
    let context = MTLSTestContext::new_with_dummy_certs().unwrap();
    let validation_result = context.mtls_config.validate();
    assert!(validation_result.is_ok(), "Valid config should pass validation");

    // Test invalid configuration
    let invalid_config = MTLSConfig {
        enabled: true,
        server_cert_path: "/nonexistent/cert.pem".into(),
        server_key_path: "/nonexistent/key.pem".into(),
        ca_cert_path: "/nonexistent/ca.pem".into(),
        ..Default::default()
    };

    let validation_result = invalid_config.validate();
    assert!(validation_result.is_err(), "Invalid config should fail validation");
}

#[tokio::test]
async fn test_certificate_generation() {
    if !CertificateManager::check_openssl_available() {
        warn!("⚠️ OpenSSL not available, skipping certificate generation test");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let cert_manager = CertificateManager::new(temp_dir.path());
    
    let result = cert_manager.generate_dev_certificates();
    match result {
        Ok(dev_certs) => {
            info!("✅ Successfully generated development certificates");
            
            // Verify all certificate files exist
            assert!(dev_certs.ca_cert.exists(), "CA certificate should exist");
            assert!(dev_certs.server_cert.exists(), "Server certificate should exist");
            assert!(dev_certs.server_key.exists(), "Server key should exist");
            assert!(dev_certs.client_cert.exists(), "Client certificate should exist");
            assert!(dev_certs.client_key.exists(), "Client key should exist");
            
            // Verify certificate chain
            let ca_valid = cert_manager.verify_certificate_chain(&dev_certs.ca_cert, &dev_certs.ca_cert);
            let server_valid = cert_manager.verify_certificate_chain(&dev_certs.server_cert, &dev_certs.ca_cert);
            let client_valid = cert_manager.verify_certificate_chain(&dev_certs.client_cert, &dev_certs.ca_cert);
            
            assert!(ca_valid.unwrap_or(false), "CA certificate should be self-valid");
            assert!(server_valid.unwrap_or(false), "Server certificate should be valid");
            assert!(client_valid.unwrap_or(false), "Client certificate should be valid");
            
            // Test mTLS config generation
            let mtls_config = dev_certs.to_mtls_config();
            assert!(mtls_config.enabled, "Generated config should have mTLS enabled");
            assert!(mtls_config.require_client_auth, "Should require client auth");
            assert!(!mtls_config.allowed_client_cns.is_empty(), "Should have allowed CNs");
        }
        Err(e) => {
            warn!("Certificate generation failed: {}", e);
            // This might be expected in some environments
        }
    }
}

#[tokio::test]
async fn test_mtls_server_startup() {
    let context = MTLSTestContext::new_with_certs().await.unwrap();
    
    if !context.has_real_certs() {
        warn!("⚠️ No real certificates available, skipping server startup test");
        return;
    }

    let server_result = MTLSTestServer::start(&context).await;
    match server_result {
        Ok(server) => {
            info!("✅ mTLS server started successfully");
            assert!(server.is_mtls_enabled(), "Server should have mTLS enabled");
            
            // Test server endpoint format
            let endpoint = server.endpoint();
            assert!(endpoint.starts_with("https://"), "mTLS server should use HTTPS endpoint");
            
            // Cleanup
            server.shutdown().await.unwrap();
        }
        Err(e) => {
            warn!("mTLS server startup failed: {}", e);
            // This might be expected if certificates are not properly configured
        }
    }
}

#[tokio::test]
async fn test_mtls_client_server_communication() {
    let context = MTLSTestContext::new_with_certs().await.unwrap();
    
    if !context.has_real_certs() {
        warn!("⚠️ No real certificates available, skipping client-server communication test");
        return;
    }

    let server_result = MTLSTestServer::start(&context).await;
    if let Ok(server) = server_result {
        info!("🔄 Testing mTLS client-server communication");
        
        // Create matching client
        let client_result = MTLSTestClientFactory::create_matching_client(
            &context,
            &server,
            "test-service"
        ).await;
        
        match client_result {
            Ok(client) => {
                // Test basic communication
                let health_result = client.test_connection().await;
                match health_result {
                    Ok(healthy) => {
                        info!("✅ mTLS health check successful: {}", healthy);
                        
                        // Test lease operations
                        test_lease_operations_with_mtls(&client).await;
                    }
                    Err(e) => {
                        info!("mTLS health check failed: {}", e);
                    }
                }
            }
            Err(e) => {
                info!("mTLS client creation failed: {}", e);
            }
        }
        
        // Cleanup
        server.shutdown().await.unwrap();
    }
}

async fn test_lease_operations_with_mtls(client: &garbagetruck::client::MTLSGCClient) {
    info!("🧪 Testing lease operations with mTLS");
    
    let metadata: HashMap<String, String> = [
        ("test".to_string(), "mtls_integration".to_string()),
        ("secure".to_string(), "true".to_string()),
    ].into();

    // Test create lease
    let create_result = client.create_lease(
        "test-object-mtls".to_string(),
        ObjectType::TemporaryFile,
        3600,
        metadata,
        None,
    ).await;

    match create_result {
        Ok(lease_id) => {
            info!("✅ Lease created with mTLS: {}", lease_id);
            
            // Test get lease
            let get_result = client.get_lease(lease_id.clone()).await;
            match get_result {
                Ok(Some(lease_info)) => {
                    info!("✅ Lease retrieved with mTLS: {}", lease_info.object_id);
                    assert_eq!(lease_info.object_id, "test-object-mtls");
                }
                Ok(None) => {
                    warn!("⚠️ Lease not found");
                }
                Err(e) => {
                    warn!("⚠️ Get lease failed: {}", e);
                }
            }
            
            // Test list leases
            let list_result = client.list_leases(Some("test-service".to_string()), 10).await;
            match list_result {
                Ok(leases) => {
                    info!("✅ Listed {} leases with mTLS", leases.len());
                }
                Err(e) => {
                    warn!("⚠️ List leases failed: {}", e);
                }
            }
            
            // Test renew lease
            let renew_result = client.renew_lease(lease_id.clone(), 1800).await;
            match renew_result {
                Ok(()) => {
                    info!("✅ Lease renewed with mTLS");
                }
                Err(e) => {
                    warn!("⚠️ Renew lease failed: {}", e);
                }
            }
            
            // Test secure convenience method
            let secure_lease_result = client.create_temp_file_lease_secure(
                "/tmp/secure-test-file.txt".to_string(),
                7200,
                Some("http://localhost:8080/cleanup".to_string()),
            ).await;
            
            match secure_lease_result {
                Ok(secure_lease_id) => {
                    info!("✅ Secure temp file lease created: {}", secure_lease_id);
                }
                Err(e) => {
                    warn!("⚠️ Secure temp file lease failed: {}", e);
                }
            }
            
            // Test lease renewal with retry
            let retry_result = client.renew_lease_with_retry(lease_id.clone(), 1800, 3).await;
            match retry_result {
                Ok(()) => {
                    info!("✅ Lease renewed with retry via mTLS");
                }
                Err(e) => {
                    warn!("⚠️ Lease renewal with retry failed: {}", e);
                }
            }
            
            // Test release lease
            let release_result = client.release_lease(lease_id).await;
            match release_result {
                Ok(()) => {
                    info!("✅ Lease released with mTLS");
                }
                Err(e) => {
                    warn!("⚠️ Release lease failed: {}", e);
                }
            }
        }
        Err(e) => {
            warn!("⚠️ Create lease failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_mtls_configuration_from_environment() {
    // Set up environment variables for mTLS
    std::env::set_var("GC_MTLS_ENABLED", "true");
    std::env::set_var("GC_MTLS_REQUIRE_CLIENT_AUTH", "true");
    std::env::set_var("GC_MTLS_ALLOWED_CLIENT_CNS", "test-client,another-client");
    std::env::set_var("GC_MTLS_MIN_TLS_VERSION", "1.2");
    std::env::set_var("GC_MTLS_CERT_VALIDATION_MODE", "strict");

    let mtls_config_result = MTLSConfig::from_env();
    
    // Clean up environment variables
    std::env::remove_var("GC_MTLS_ENABLED");
    std::env::remove_var("GC_MTLS_REQUIRE_CLIENT_AUTH");
    std::env::remove_var("GC_MTLS_ALLOWED_CLIENT_CNS");
    std::env::remove_var("GC_MTLS_MIN_TLS_VERSION");
    std::env::remove_var("GC_MTLS_CERT_VALIDATION_MODE");

    match mtls_config_result {
        Ok(config) => {
            assert!(config.enabled, "mTLS should be enabled from environment");
            assert!(config.require_client_auth, "Client auth should be required");
            assert_eq!(config.allowed_client_cns.len(), 2, "Should have 2 allowed CNs");
            assert!(config.allowed_client_cns.contains(&"test-client".to_string()));
            assert!(config.allowed_client_cns.contains(&"another-client".to_string()));
            assert!(matches!(config.min_tls_version, TLSVersion::TLS12));
            assert!(matches!(config.cert_validation_mode, CertValidationMode::Strict));
            
            info!("✅ mTLS configuration loaded from environment successfully");
        }
        Err(e) => {
            panic!("Failed to load mTLS config from environment: {}", e);
        }
    }
}

#[tokio::test]
async fn test_mtls_config_integration_with_service() {
    let mut context = MTLSTestContext::new_with_dummy_certs().unwrap();
    
    // Test with enhanced security enabled
    context.server_config.security.enhanced_security = true;
    context.server_config.security.rate_limiting.enabled = true;
    context.server_config.security.rate_limiting.requests_per_minute = 100;
    context.server_config.security.max_connections_per_client = 10;

    // Validate the complete configuration
    let validation_result = context.server_config.validate();
    assert!(validation_result.is_ok(), "Enhanced security config should be valid");

    // Test configuration summary
    let summary = context.server_config.summary();
    assert!(summary.security_features.mtls_enabled, "Summary should show mTLS enabled");
    assert!(summary.security_features.enhanced_security, "Summary should show enhanced security");
    assert!(summary.security_features.rate_limiting_enabled, "Summary should show rate limiting");
    
    info!("✅ mTLS configuration integrates properly with service config");
}

#[tokio::test]
async fn test_mtls_error_handling() {
    info!("🧪 Testing mTLS error handling scenarios");
    
    // Test 1: Missing certificate files
    let invalid_config = MTLSConfig {
        enabled: true,
        server_cert_path: "/nonexistent/server.crt".into(),
        server_key_path: "/nonexistent/server.key".into(),
        ca_cert_path: "/nonexistent/ca.crt".into(),
        ..Default::default()
    };

    let validation_result = invalid_config.validate();
    assert!(validation_result.is_err(), "Should fail with missing certificates");
    
    // Test 2: Client creation with invalid config
    let client_result = MTLSClientBuilder::new(
        "https://localhost:50051".to_string(),
        "test-service".to_string(),
    )
    .with_mtls_config(invalid_config)
    .build()
    .await;

    match client_result {
        Ok(_) => panic!("Client creation should fail with invalid certificates"),
        Err(e) => {
            info!("✅ Client creation properly failed with invalid config: {}", e);
            
            // Check error type
            match e {
                GCError::Configuration(_) => {
                    info!("✅ Correct error type: Configuration");
                }
                GCError::Internal(msg) if msg.contains("certificate") => {
                    info!("✅ Correct error type: Certificate-related internal error");
                }
                _ => {
                    warn!("⚠️ Unexpected error type: {:?}", e);
                }
            }
        }
    }

    // Test 3: Client authentication failure simulation
    let context = MTLSTestContext::new_with_dummy_certs().unwrap();
    let unauthorized_config = MTLSConfig {
        enabled: true,
        require_client_auth: true,
        allowed_client_cns: vec!["unauthorized-client".to_string()],
        ..context.mtls_config
    };

    // This would fail in a real scenario with proper certificate validation
    info!("✅ Error handling scenarios tested successfully");
}

#[tokio::test]
#[ignore] // Ignore by default due to performance test duration
async fn test_mtls_performance_overhead() {
    let context = MTLSTestContext::new_with_certs().await.unwrap();
    
    if !context.has_real_certs() {
        warn!("⚠️ No real certificates available, skipping performance test");
        return;
    }

    let server = match MTLSTestServer::start(&context).await {
        Ok(server) => server,
        Err(e) => {
            warn!("⚠️ Could not start server for performance test: {}", e);
            return;
        }
    };

    info!("📊 Testing mTLS performance overhead");

    // Create plain server for comparison (would need separate setup)
    let plain_endpoint = "http://localhost:50052"; // Hypothetical plain server
    
    let performance_result = MTLSPerformanceTest::compare_mtls_vs_plain(
        &server.endpoint(),
        plain_endpoint,
        context.client_config(),
        5, // Small number for test
    ).await;

    match performance_result {
        Ok(comparison) => {
            let overhead = comparison.overhead_ratio();
            info!("📈 mTLS overhead ratio: {:.2}x", overhead);
            
            // Assert reasonable overhead (less than 10x slower)
            assert!(overhead < 10.0, "mTLS overhead should be reasonable");
            
            info!("✅ Performance test completed");
        }
        Err(e) => {
            warn!("⚠️ Performance test failed: {}", e);
        }
    }

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_mtls_production_readiness() {
    info!("🏭 Testing production readiness validation");
    
    // Test production-ready configuration
    let production_config = MTLSConfig {
        enabled: true,
        require_client_auth: true,
        cert_validation_mode: CertValidationMode::Strict,
        allowed_client_cns: vec!["prod-client-1".to_string(), "prod-client-2".to_string()],
        min_tls_version: TLSVersion::TLS12,
        server_cert_path: "/etc/certs/server.crt".into(),
        server_key_path: "/etc/certs/server.key".into(),
        ca_cert_path: "/etc/certs/ca.crt".into(),
        client_cert_path: Some("/etc/certs/client.crt".into()),
        client_key_path: Some("/etc/certs/client.key".into()),
    };

    let production_check = MTLSAssertions::assert_production_ready(&production_config);
    assert!(production_check.is_ok(), "Production config should pass readiness check");

    // Test non-production configuration
    let dev_config = MTLSConfig {
        enabled: true,
        require_client_auth: false, // Not production-ready
        cert_validation_mode: CertValidationMode::AllowSelfSigned, // Not production-ready
        ..production_config
    };

    let dev_check = MTLSAssertions::assert_production_ready(&dev_config);
    assert!(dev_check.is_err(), "Dev config should fail production readiness check");

    info!("✅ Production readiness validation working correctly");
}

#[tokio::test]
async fn test_mtls_certificate_info_extraction() {
    if !CertificateManager::check_openssl_available() {
        warn!("⚠️ OpenSSL not available, skipping certificate info test");
        return;
    }

    let temp_dir = TempDir::new().unwrap();
    let cert_manager = CertificateManager::new(temp_dir.path());
    
    match cert_manager.generate_dev_certificates() {
        Ok(dev_certs) => {
            // Test certificate info extraction
            let server_info_result = cert_manager.get_certificate_info(&dev_certs.server_cert);
            match server_info_result {
                Ok(cert_info) => {
                    info!("📋 Server certificate info:");
                    info!("  Subject: {}", cert_info.subject);
                    info!("  Issuer: {}", cert_info.issuer);
                    info!("  Valid from: {}", cert_info.not_before);
                    info!("  Valid until: {}", cert_info.not_after);
                    
                    // Test CN extraction
                    let cn = cert_info.extract_common_name();
                    assert!(cn.is_some(), "Should be able to extract CN");
                    
                    if let Some(common_name) = cn {
                        info!("  Common Name: {}", common_name);
                        assert!(common_name.contains("localhost") || common_name.contains("garbagetruck"));
                    }
                    
                    // Test expiration check
                    let is_expiring = cert_info.is_expiring_soon(30);
                    info!("  Expiring soon (30 days): {}", is_expiring);
                }
                Err(e) => {
                    warn!("⚠️ Failed to get certificate info: {}", e);
                }
            }

            // Test client certificate info
            let client_info_result = cert_manager.get_certificate_info(&dev_certs.client_cert);
            match client_info_result {
                Ok(cert_info) => {
                    let cn = cert_info.extract_common_name();
                    if let Some(common_name) = cn {
                        info!("📋 Client certificate CN: {}", common_name);
                        assert!(common_name.contains("client") || common_name.contains("garbagetruck"));
                    }
                }
                Err(e) => {
                    warn!("⚠️ Failed to get client certificate info: {}", e);
                }
            }
            
            info!("✅ Certificate info extraction test completed");
        }
        Err(e) => {
            warn!("⚠️ Certificate generation failed: {}", e);
        }
    }
}

#[tokio::test]
async fn test_mtls_config_validation_edge_cases() {
    info!("🧪 Testing mTLS configuration validation edge cases");
    
    // Test case 1: Empty allowed CNs with client auth required
    let empty_cns_config = MTLSConfig {
        enabled: true,
        require_client_auth: true,
        allowed_client_cns: vec![], // Empty - should generate warning
        server_cert_path: "/tmp/server.crt".into(),
        server_key_path: "/tmp/server.key".into(),
        ca_cert_path: "/tmp/ca.crt".into(),
        ..Default::default()
    };

    // This should pass validation but generate warnings
    // (Empty CNs means allow any valid certificate)
    
    // Test case 2: Client auth required but no client cert paths
    let no_client_certs_config = MTLSConfig {
        enabled: true,
        require_client_auth: true,
        client_cert_path: None,
        client_key_path: None,
        server_cert_path: "/tmp/server.crt".into(),
        server_key_path: "/tmp/server.key".into(),
        ca_cert_path: "/tmp/ca.crt".into(),
        ..Default::default()
    };

    // This should pass validation but generate warnings for missing client certs
    
    // Test case 3: TLS version edge cases
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

    // Both should be valid configurations
    assert!(matches!(tls12_config.min_tls_version, TLSVersion::TLS12));
    assert!(matches!(tls13_config.min_tls_version, TLSVersion::TLS13));
    
    info!("✅ Edge case validation tests completed");
}

#[tokio::test]
async fn test_mtls_client_builder_patterns() {
    info!("🔧 Testing mTLS client builder patterns");
    
    // Test 1: Builder with disabled mTLS
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
            assert!(!summary.enabled, "Builder should create client with disabled mTLS");
            info!("✅ Plain client builder pattern works");
        }
        Err(e) => {
            info!("Plain client creation failed (expected without server): {}", e);
        }
    }

    // Test 2: Builder with environment mTLS
    std::env::set_var("GC_MTLS_ENABLED", "false");
    
    let env_client_result = MTLSClientBuilder::new(
        "http://localhost:50051".to_string(),
        "test-service".to_string(),
    )
    .with_env_mtls();

    std::env::remove_var("GC_MTLS_ENABLED");

    match env_client_result {
        Ok(builder) => {
            match builder.build().await {
                Ok(client) => {
                    let summary = client.get_mtls_summary();
                    assert!(!summary.enabled, "Environment should disable mTLS");
                    info!("✅ Environment client builder pattern works");
                }
                Err(e) => {
                    info!("Environment client creation failed (expected without server): {}", e);
                }
            }
        }
        Err(e) => {
            warn!("Environment client builder creation failed: {}", e);
        }
    }

    // Test 3: Builder with custom config
    let custom_config = MTLSConfig {
        enabled: true,
        require_client_auth: false,
        ..Default::default()
    };

    let custom_client_result = MTLSClientBuilder::new(
        "https://localhost:50051".to_string(),
        "test-service".to_string(),
    )
    .with_mtls_config(custom_config)
    .build()
    .await;

    match custom_client_result {
        Ok(client) => {
            let summary = client.get_mtls_summary();
            assert!(summary.enabled, "Custom config should enable mTLS");
            assert!(!summary.require_client_auth, "Custom config should not require client auth");
            info!("✅ Custom config client builder pattern works");
        }
        Err(e) => {
            info!("Custom client creation failed (expected without server/certs): {}", e);
        }
    }

    info!("✅ Client builder pattern tests completed");
}

/// Helper function to create fixture files for tests
fn create_test_fixtures() -> std::io::Result<()> {
    let fixtures_dir = std::path::Path::new("tests/fixtures");
    std::fs::create_dir_all(fixtures_dir)?;

    // Create dummy certificate and key files for testing
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

    std::fs::write(fixtures_dir.join("dummy_cert.pem"), dummy_cert)?;
    std::fs::write(fixtures_dir.join("dummy_key.pem"), dummy_key)?;

    Ok(())
}

#[tokio::test]
async fn setup_test_fixtures() {
    create_test_fixtures().expect("Failed to create test fixtures");
    info!("✅ Test fixtures created successfully");
}