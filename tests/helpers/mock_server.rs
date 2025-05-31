// tests/helpers/mock_server.rs - Mock cleanup server for testing

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

use super::get_next_port;

/// Mock cleanup server to simulate service endpoints
#[derive(Clone, Default)]
pub struct MockCleanupServer {
    cleanup_calls: Arc<Mutex<Vec<CleanupCall>>>,
    should_fail: Arc<Mutex<bool>>,
    delay_ms: Arc<Mutex<u64>>,
    port: u16,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CleanupCall {
    pub lease_id: String,
    pub object_id: String,
    pub object_type: String,
    pub service_id: String,
    pub metadata: HashMap<String, String>,
    pub payload: String,
}

impl MockCleanupServer {
    pub fn new() -> Self {
        Self {
            cleanup_calls: Arc::new(Mutex::new(Vec::new())),
            should_fail: Arc::new(Mutex::new(false)),
            delay_ms: Arc::new(Mutex::new(0)),
            port: get_next_port(),
        }
    }

    pub async fn get_cleanup_calls(&self) -> Vec<CleanupCall> {
        self.cleanup_calls.lock().await.clone()
    }

    pub async fn set_should_fail(&self, fail: bool) {
        *self.should_fail.lock().await = fail;
    }

    pub async fn set_delay(&self, delay_ms: u64) {
        *self.delay_ms.lock().await = delay_ms;
    }

    pub async fn clear_calls(&self) {
        self.cleanup_calls.lock().await.clear();
    }

    pub fn get_port(&self) -> u16 {
        self.port
    }

    pub async fn handle_cleanup(&self, call: CleanupCall) -> Result<(), String> {
        // Add delay if configured
        let delay = *self.delay_ms.lock().await;
        if delay > 0 {
            sleep(Duration::from_millis(delay)).await;
        }

        // Record the call
        self.cleanup_calls.lock().await.push(call);

        // Return error if configured to fail
        if *self.should_fail.lock().await {
            return Err("Mock cleanup failure".to_string());
        }

        Ok(())
    }

    /// Start the mock HTTP server
    pub async fn start(&self) -> tokio::task::JoinHandle<()> {
        let server = self.clone();
        let port = self.port;

        tokio::spawn(async move {
            start_mock_cleanup_server(server, port).await;
        })
    }
}

async fn start_mock_cleanup_server(server: MockCleanupServer, port: u16) {
    use warp::Filter;

    let cleanup_route = warp::path("cleanup")
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::any().map(move || server.clone()))
        .and_then(|call: CleanupCall, server: MockCleanupServer| async move {
            match server.handle_cleanup(call).await {
                Ok(()) => Ok::<_, warp::Rejection>(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({"success": true})),
                    warp::http::StatusCode::OK,
                )),
                Err(e) => Ok::<_, warp::Rejection>(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({"error": e})),
                    warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                )),
            }
        });

    warp::serve(cleanup_route).run(([127, 0, 0, 1], port)).await;
}
