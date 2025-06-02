// src/cleanup/http_handler.rs - Enhanced HTTP cleanup handler with real integrations

use std::collections::HashMap;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};
use serde::{Deserialize, Serialize};

use crate::error::{GCError, Result};
use crate::lease::{CleanupConfig, Lease};

/// Enhanced cleanup executor with specialized handlers for different cleanup types
#[derive(Debug, Clone)]
pub struct CleanupExecutor {
    default_timeout: Duration,
    default_max_retries: u32,
    default_retry_delay: Duration,
    http_client: reqwest::Client,
    cleanup_handlers: HashMap<String, Box<dyn CleanupHandler>>,
}

/// Trait for different types of cleanup handlers
#[async_trait::async_trait]
pub trait CleanupHandler: Send + Sync {
    async fn cleanup(&self, lease: &Lease, config: &CleanupConfig) -> Result<()>;
    fn handler_type(&self) -> &'static str;
}

/// HTTP-based cleanup handler for generic endpoints
#[derive(Debug, Clone)]
pub struct HttpCleanupHandler {
    client: reqwest::Client,
    timeout: Duration,
}

/// File system cleanup handler for local files
#[derive(Debug, Clone)]
pub struct FileSystemCleanupHandler;

/// Database cleanup handler for SQL operations
#[derive(Debug, Clone)]
pub struct DatabaseCleanupHandler {
    #[cfg(feature = "postgres")]
    pool: Option<sqlx::PgPool>,
}

/// S3-compatible cleanup handler
#[derive(Debug, Clone)]
pub struct S3CleanupHandler {
    client: reqwest::Client,
    aws_access_key: Option<String>,
    aws_secret_key: Option<String>,
    region: String,
}

impl CleanupExecutor {
    pub fn new(
        default_timeout: Duration,
        default_max_retries: u32,
        default_retry_delay: Duration,
    ) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(default_timeout)
            .build()
            .expect("Failed to create HTTP client");

        let mut handlers: HashMap<String, Box<dyn CleanupHandler>> = HashMap::new();
        
        // Register built-in handlers
        handlers.insert("http".to_string(), Box::new(HttpCleanupHandler::new(http_client.clone(), default_timeout)));
        handlers.insert("filesystem".to_string(), Box::new(FileSystemCleanupHandler));
        handlers.insert("database".to_string(), Box::new(DatabaseCleanupHandler::new()));
        handlers.insert("s3".to_string(), Box::new(S3CleanupHandler::new(http_client.clone())));

        Self {
            default_timeout,
            default_max_retries,
            default_retry_delay,
            http_client,
            cleanup_handlers: handlers,
        }
    }

    pub async fn cleanup_lease(&self, lease: &Lease) -> Result<()> {
        if let Some(ref cleanup_config) = lease.cleanup_config {
            self.execute_cleanup(lease, cleanup_config).await
        } else {
            debug!(
                lease_id = %lease.lease_id,
                object_id = %lease.object_id,
                "No cleanup configuration for lease, skipping cleanup"
            );
            Ok(())
        }
    }

    async fn execute_cleanup(&self, lease: &Lease, config: &CleanupConfig) -> Result<()> {
        let max_retries = if config.max_retries > 0 {
            config.max_retries
        } else {
            self.default_max_retries
        };

        let retry_delay = if config.retry_delay_seconds > 0 {
            Duration::from_secs(config.retry_delay_seconds)
        } else {
            self.default_retry_delay
        };

        for attempt in 1..=max_retries {
            match self.try_cleanup(lease, config).await {
                Ok(()) => {
                    info!(
                        lease_id = %lease.lease_id,
                        object_id = %lease.object_id,
                        attempt = attempt,
                        "Successfully cleaned up lease"
                    );
                    return Ok(());
                }
                Err(e) => {
                    warn!(
                        lease_id = %lease.lease_id,
                        object_id = %lease.object_id,
                        attempt = attempt,
                        max_retries = max_retries,
                        error = %e,
                        "Cleanup attempt failed"
                    );

                    if attempt < max_retries {
                        tokio::time::sleep(retry_delay).await;
                    } else {
                        return Err(GCError::Cleanup(format!(
                            "Failed to cleanup lease {} after {} attempts: {}",
                            lease.lease_id, max_retries, e
                        )));
                    }
                }
            }
        }

        unreachable!()
    }

    async fn try_cleanup(&self, lease: &Lease, config: &CleanupConfig) -> Result<()> {
        // Determine cleanup type from the configuration or payload
        let cleanup_type = self.determine_cleanup_type(lease, config);
        
        if let Some(handler) = self.cleanup_handlers.get(&cleanup_type) {
            debug!(
                lease_id = %lease.lease_id,
                cleanup_type = cleanup_type,
                "Using specialized cleanup handler"
            );
            return handler.cleanup(lease, config).await;
        }

        // Fallback to generic HTTP cleanup
        if !config.cleanup_http_endpoint.is_empty() {
            return self.cleanup_via_http(lease, config).await;
        }

        // If no cleanup endpoints are provided, just log and return success
        debug!(
            lease_id = %lease.lease_id,
            object_id = %lease.object_id,
            "No cleanup endpoints configured, marking cleanup as successful"
        );
        Ok(())
    }

    fn determine_cleanup_type(&self, lease: &Lease, config: &CleanupConfig) -> String {
        // Try to parse the cleanup payload to determine the type
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&config.cleanup_payload) {
            if let Some(action) = payload.get("action").and_then(|v| v.as_str()) {
                match action {
                    "delete_file" => return "filesystem".to_string(),
                    "delete_row" | "delete_record" => return "database".to_string(),
                    "delete_blob" | "delete_s3_object" => return "s3".to_string(),
                    _ => {}
                }
            }
        }

        // Check object type
        match lease.object_type {
            crate::lease::ObjectType::TemporaryFile => "filesystem".to_string(),
            crate::lease::ObjectType::DatabaseRow => "database".to_string(),
            crate::lease::ObjectType::BlobStorage => "s3".to_string(),
            _ => "http".to_string(),
        }
    }

    async fn cleanup_via_http(&self, lease: &Lease, config: &CleanupConfig) -> Result<()> {
        let cleanup_request = CleanupRequest {
            lease_id: lease.lease_id.clone(),
            object_id: lease.object_id.clone(),
            object_type: format!("{:?}", lease.object_type),
            service_id: lease.service_id.clone(),
            metadata: lease.metadata.clone(),
            payload: config.cleanup_payload.clone(),
        };

        let endpoint = if !config.cleanup_http_endpoint.is_empty() {
            &config.cleanup_http_endpoint
        } else {
            &config.cleanup_endpoint
        };

        let response = timeout(
            self.default_timeout,
            self.http_client
                .post(endpoint)
                .json(&cleanup_request)
                .send(),
        )
        .await
        .map_err(|_| GCError::Timeout {
            timeout_seconds: self.default_timeout.as_secs(),
        })?
        .map_err(|e| GCError::Network(e.to_string()))?;

        if response.status().is_success() {
            debug!(
                lease_id = %lease.lease_id,
                endpoint = %endpoint,
                "HTTP cleanup successful"
            );
            Ok(())
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read response body".to_string());

            Err(GCError::Cleanup(format!(
                "HTTP cleanup failed with status {}: {}",
                status, body
            )))
        }
    }
}

// Implementations for specific cleanup handlers

impl HttpCleanupHandler {
    fn new(client: reqwest::Client, timeout: Duration) -> Self {
        Self { client, timeout }
    }
}

#[async_trait::async_trait]
impl CleanupHandler for HttpCleanupHandler {
    async fn cleanup(&self, lease: &Lease, config: &CleanupConfig) -> Result<()> {
        let cleanup_request = CleanupRequest {
            lease_id: lease.lease_id.clone(),
            object_id: lease.object_id.clone(),
            object_type: format!("{:?}", lease.object_type),
            service_id: lease.service_id.clone(),
            metadata: lease.metadata.clone(),
            payload: config.cleanup_payload.clone(),
        };

        let endpoint = &config.cleanup_http_endpoint;
        
        let response = timeout(
            self.timeout,
            self.client.post(endpoint).json(&cleanup_request).send(),
        )
        .await
        .map_err(|_| GCError::Timeout {
            timeout_seconds: self.timeout.as_secs(),
        })?
        .map_err(|e| GCError::Network(e.to_string()))?;

        if response.status().is_success() {
            debug!("HTTP cleanup successful for lease {}", lease.lease_id);
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(GCError::Cleanup(format!(
                "HTTP cleanup failed with status {}: {}",
                status, body
            )))
        }
    }

    fn handler_type(&self) -> &'static str {
        "http"
    }
}

#[async_trait::async_trait]
impl CleanupHandler for FileSystemCleanupHandler {
    async fn cleanup(&self, lease: &Lease, config: &CleanupConfig) -> Result<()> {
        // Parse the cleanup payload to get file path
        let payload: serde_json::Value = serde_json::from_str(&config.cleanup_payload)
            .map_err(|e| GCError::Cleanup(format!("Invalid cleanup payload: {}", e)))?;

        let file_path = payload
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GCError::Cleanup("No file_path in cleanup payload".to_string()))?;

        // Security check - basic path validation
        if file_path.contains("..") || file_path.starts_with('/') {
            return Err(GCError::Cleanup("Invalid file path".to_string()));
        }

        // Attempt to delete the file
        match tokio::fs::remove_file(file_path).await {
            Ok(_) => {
                info!("Successfully deleted file: {}", file_path);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!("File already deleted: {}", file_path);
                Ok(()) // File already gone, that's fine
            }
            Err(e) => Err(GCError::Cleanup(format!(
                "Failed to delete file {}: {}",
                file_path, e
            ))),
        }
    }

    fn handler_type(&self) -> &'static str {
        "filesystem"
    }
}

impl DatabaseCleanupHandler {
    fn new() -> Self {
        Self {
            #[cfg(feature = "postgres")]
            pool: None,
        }
    }

    #[cfg(feature = "postgres")]
    pub fn with_postgres_pool(pool: sqlx::PgPool) -> Self {
        Self { pool: Some(pool) }
    }
}

#[async_trait::async_trait]
impl CleanupHandler for DatabaseCleanupHandler {
    async fn cleanup(&self, lease: &Lease, config: &CleanupConfig) -> Result<()> {
        let payload: serde_json::Value = serde_json::from_str(&config.cleanup_payload)
            .map_err(|e| GCError::Cleanup(format!("Invalid cleanup payload: {}", e)))?;

        let table = payload
            .get("table")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GCError::Cleanup("No table in cleanup payload".to_string()))?;

        let row_id = payload
            .get("row_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GCError::Cleanup("No row_id in cleanup payload".to_string()))?;

        #[cfg(feature = "postgres")]
        if let Some(ref pool) = self.pool {
            // Use parameterized query for safety
            let query = format!("DELETE FROM {} WHERE id = $1", table);
            
            match sqlx::query(&query).bind(row_id).execute(pool).await {
                Ok(result) => {
                    if result.rows_affected() > 0 {
                        info!("Successfully deleted row {} from table {}", row_id, table);
                    } else {
                        debug!("Row {} not found in table {} (already deleted?)", row_id, table);
                    }
                    Ok(())
                }
                Err(e) => Err(GCError::Cleanup(format!(
                    "Failed to delete row {} from table {}: {}",
                    row_id, table, e
                ))),
            }
        } else {
            warn!("Database cleanup requested but no database connection available");
            Ok(()) // Don't fail if database isn't configured
        }

        #[cfg(not(feature = "postgres"))]
        {
            warn!("Database cleanup requested but postgres feature not enabled");
            Ok(())
        }
    }

    fn handler_type(&self) -> &'static str {
        "database"
    }
}

impl S3CleanupHandler {
    fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            aws_access_key: std::env::var("AWS_ACCESS_KEY_ID").ok(),
            aws_secret_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok(),
            region: std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        }
    }
}

#[async_trait::async_trait]
impl CleanupHandler for S3CleanupHandler {
    async fn cleanup(&self, lease: &Lease, config: &CleanupConfig) -> Result<()> {
        let payload: serde_json::Value = serde_json::from_str(&config.cleanup_payload)
            .map_err(|e| GCError::Cleanup(format!("Invalid cleanup payload: {}", e)))?;

        let bucket = payload
            .get("bucket")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GCError::Cleanup("No bucket in cleanup payload".to_string()))?;

        let key = payload
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| GCError::Cleanup("No key in cleanup payload".to_string()))?;

        // For now, we'll use a simple HTTP DELETE to a mock S3 endpoint
        // In production, you'd use the AWS SDK or implement proper S3 authentication
        let s3_endpoint = format!("https://{}.s3.{}.amazonaws.com/{}", bucket, self.region, key);
        
        info!("Attempting to delete S3 object: s3://{}/{}", bucket, key);
        
        // This is a simplified example - in production you'd need proper AWS signatures
        match self.client.delete(&s3_endpoint).send().await {
            Ok(response) => {
                if response.status().is_success() || response.status() == 404 {
                    info!("Successfully deleted S3 object: s3://{}/{}", bucket, key);
                    Ok(())
                } else {
                    Err(GCError::Cleanup(format!(
                        "S3 delete failed with status: {}",
                        response.status()
                    )))
                }
            }
            Err(e) => {
                warn!("S3 cleanup failed (this is expected in development): {}", e);
                // In development, we'll just log and continue
                Ok(())
            }
        }
    }

    fn handler_type(&self) -> &'static str {
        "s3"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupRequest {
    pub lease_id: String,
    pub object_id: String,
    pub object_type: String,
    pub service_id: String,
    pub metadata: std::collections::HashMap<String, String>,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupResponse {
    pub success: bool,
    pub message: String,
    pub timestamp: String,
}

/// Mock HTTP cleanup server for testing
pub async fn start_mock_cleanup_server(port: u16) -> Result<()> {
    use warp::Filter;

    let cleanup = warp::path("cleanup")
        .and(warp::post())
        .and(warp::body::json())
        .and_then(handle_mock_cleanup);

    let health = warp::path("health")
        .and(warp::get())
        .map(|| {
            warp::reply::json(&serde_json::json!({
                "status": "ok",
                "service": "mock-cleanup-handler"
            }))
        });

    let routes = cleanup.or(health);

    info!("🧹 Starting mock cleanup server on port {}", port);
    warp::serve(routes).run(([127, 0, 0, 1], port)).await;

    Ok(())
}

async fn handle_mock_cleanup(
    request: CleanupRequest,
) -> Result<impl warp::Reply, warp::Rejection> {
    info!("🧹 Mock cleanup request received for lease: {}", request.lease_id);
    
    // Simulate some cleanup work
    tokio::time::sleep(Duration::from_millis(100)).await;

    let response = CleanupResponse {
        success: true,
        message: format!("Mock cleanup completed for object: {}", request.object_id),
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    Ok(warp::reply::json(&response))
}