// tests/helpers/mod.rs - Common test utilities

pub mod mock_server;
pub mod test_data;
pub mod assertions;

use std::sync::atomic::{AtomicU16, Ordering};

// Global port counter to avoid conflicts
static PORT_COUNTER: AtomicU16 = AtomicU16::new(8080);

/// Get next available port for testing
pub fn get_next_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::SeqCst)
}

/// Wait for a service to be ready
pub async fn wait_for_service(addr: &str, timeout_ms: u64) -> bool {
    use tokio::time::{sleep, Duration};
    
    let timeout = Duration::from_millis(timeout_ms);
    let check_interval = Duration::from_millis(50);
    let start = std::time::Instant::now();
    
    while start.elapsed() < timeout {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return true;
        }
        sleep(check_interval).await;
    }
    
    false
}

/// Skip test if environment variable is not set
pub fn skip_if_no_env(var: &str) -> bool {
    if std::env::var(var).is_err() {
        println!("⚠️  Skipping test - {} not set", var);
        return true;
    }
    false
}