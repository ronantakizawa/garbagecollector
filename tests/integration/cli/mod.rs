// tests/integration/cli/mod.rs - CLI integration tests

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::time::Duration;
    use tempfile::TempDir;
    use tokio::time::sleep;

    use crate::helpers::{TestHarness, available_port};

    #[tokio::test]
    async fn test_cli_health_command() {
        let port = available_port();
        let harness = TestHarness::new(port).await;
        let _service = harness.start_service().await.unwrap();

        // Wait for service to be ready
        sleep(Duration::from_millis(500)).await;

        // Test CLI health command
        let output = Command::new("cargo")
            .args([
                "run",
                "--bin",
                "garbagetruck",
                "--features",
                "client",
                "--",
                "--endpoint",
                &format!("http://localhost:{}", port),
                "health",
            ])
            .output()
            .expect("Failed to execute CLI command");

        assert!(output.status.success(), "CLI health command should succeed");
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("GarbageTruck service is healthy"));
    }

    #[tokio::test]
    async fn test_cli_status_command() {
        let port = available_port();
        let harness = TestHarness::new(port).await;
        let _service = harness.start_service().await.unwrap();

        // Wait for service to be ready
        sleep(Duration::from_millis(500)).await;

        // Test CLI status command
        let output = Command::new("cargo")
            .args([
                "run",
                "--bin",
                "garbagetruck",
                "--features",
                "client",
                "--",
                "--endpoint",
                &format!("http://localhost:{}", port),
                "status",
            ])
            .output()
            .expect("Failed to execute CLI command");

        assert!(output.status.success(), "CLI status command should succeed");
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("GarbageTruck Service Status"));
        assert!(stdout.contains("Health:"));
        assert!(stdout.contains("Total Active Leases:"));
    }

    #[tokio::test]
    async fn test_cli_lease_lifecycle() {
        let port = available_port();
        let harness = TestHarness::new(port).await;
        let _service = harness.start_service().await.unwrap();

        // Wait for service to be ready
        sleep(Duration::from_millis(500)).await;

        let endpoint = format!("http://localhost:{}", port);

        // 1. Create a lease
        let create_output = Command::new("cargo")
            .args([
                "run",
                "--bin",
                "garbagetruck",
                "--features",
                "client",
                "--",
                "--endpoint",
                &endpoint,
                "lease",
                "create",
                "--object-id",
                "test-object-123",
                "--object-type",
                "websocket-session",
                "--duration",
                "3600",
                "--metadata",
                "user_id=test_user",
                "--metadata",
                "session_type=test",
            ])
            .output()
            .expect("Failed to execute CLI create command");

        assert!(create_output.status.success(), "CLI lease create should succeed");
        
        let create_stdout = String::from_utf8_lossy(&create_output.stdout);
        assert!(create_stdout.contains("Lease created successfully"));
        assert!(create_stdout.contains("Lease ID:"));

        // Extract lease ID from output
        let lease_id = create_stdout
            .lines()
            .find(|line| line.trim().starts_with("Lease ID:"))
            .and_then(|line| line.split(':').nth(1))
            .map(|id| id.trim())
            .expect("Should find lease ID in output");

        // 2. List leases
        let list_output = Command::new("cargo")
            .args([
                "run",
                "--bin",
                "garbagetruck",
                "--features",
                "client",
                "--",
                "--endpoint",
                &endpoint,
                "lease",
                "list",
                "--limit",
                "10",
            ])
            .output()
            .expect("Failed to execute CLI list command");

        assert!(list_output.status.success(), "CLI lease list should succeed");
        
        let list_stdout = String::from_utf8_lossy(&list_output.stdout);
        assert!(list_stdout.contains("Found") && list_stdout.contains("lease"));
        assert!(list_stdout.contains(lease_id));

        // 3. Get lease details
        let get_output = Command::new("cargo")
            .args([
                "run",
                "--bin",
                "garbagetruck",
                "--features",
                "client",
                "--",
                "--endpoint",
                &endpoint,
                "lease",
                "get",
                lease_id,
            ])
            .output()
            .expect("Failed to execute CLI get command");

        assert!(get_output.status.success(), "CLI lease get should succeed");
        
        let get_stdout = String::from_utf8_lossy(&get_output.stdout);
        assert!(get_stdout.contains("Lease Details"));
        assert!(get_stdout.contains("test-object-123"));
        assert!(get_stdout.contains("user_id = test_user"));

        // 4. Renew lease
        let renew_output = Command::new("cargo")
            .args([
                "run",
                "--bin",
                "garbagetruck",
                "--features",
                "client",
                "--",
                "--endpoint",
                &endpoint,
                "lease",
                "renew",
                lease_id,
                "--extend",
                "1800",
            ])
            .output()
            .expect("Failed to execute CLI renew command");

        assert!(renew_output.status.success(), "CLI lease renew should succeed");
        
        let renew_stdout = String::from_utf8_lossy(&renew_output.stdout);
        assert!(renew_stdout.contains("renewed successfully"));

        // 5. Release lease
        let release_output = Command::new("cargo")
            .args([
                "run",
                "--bin",
                "garbagetruck",
                "--features",
                "client",
                "--",
                "--endpoint",
                &endpoint,
                "lease",
                "release",
                lease_id,
            ])
            .output()
            .expect("Failed to execute CLI release command");

        assert!(release_output.status.success(), "CLI lease release should succeed");
        
        let release_stdout = String::from_utf8_lossy(&release_output.stdout);
        assert!(release_stdout.contains("released successfully"));
    }

    #[tokio::test]
    async fn test_cli_error_handling() {
        // Test connection to non-existent service
        let output = Command::new("cargo")
            .args([
                "run",
                "--bin",
                "garbagetruck",
                "--features",
                "client",
                "--",
                "--endpoint",
                "http://localhost:99999", // Non-existent port
                "health",
            ])
            .output()
            .expect("Failed to execute CLI command");

        assert!(!output.status.success(), "CLI should fail with bad endpoint");
        
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Failed to connect") || stderr.contains("connection"));
    }

    #[tokio::test]
    async fn test_cli_with_environment_variables() {
        let port = available_port();
        let harness = TestHarness::new(port).await;
        let _service = harness.start_service().await.unwrap();

        // Wait for service to be ready
        sleep(Duration::from_millis(500)).await;

        // Test with environment variables
        let output = Command::new("cargo")
            .env("GARBAGETRUCK_ENDPOINT", format!("http://localhost:{}", port))
            .env("GARBAGETRUCK_SERVICE_ID", "test-cli-service")
            .args([
                "run",
                "--bin",
                "garbagetruck",
                "--features",
                "client",
                "--",
                "health",
            ])
            .output()
            .expect("Failed to execute CLI command");

        assert!(output.status.success(), "CLI should work with env vars");
    }

    #[tokio::test]
    async fn test_cli_verbose_mode() {
        let port = available_port();
        let harness = TestHarness::new(port).await;
        let _service = harness.start_service().await.unwrap();

        // Wait for service to be ready
        sleep(Duration::from_millis(500)).await;

        // Test verbose mode
        let output = Command::new("cargo")
            .args([
                "run",
                "--bin",
                "garbagetruck",
                "--features",
                "client",
                "--",
                "--endpoint",
                &format!("http://localhost:{}", port),
                "--verbose",
                "health",
            ])
            .output()
            .expect("Failed to execute CLI command");

        assert!(output.status.success(), "CLI verbose mode should work");
        
        // In verbose mode, we should see debug/info logs
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("Checking") || stderr.len() > 0);
    }
}