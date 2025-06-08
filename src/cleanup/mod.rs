// src/cleanup/mod.rs - Fixed to actually send HTTP cleanup requests

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use crate::error::{GCError, Result};
use crate::lease::{CleanupConfig, Lease};

/// Enhanced cleanup executor with specialized handlers for different cleanup types
#[derive(Debug, Clone)]
pub struct CleanupExecutor {
    default_timeout: Duration,
    default_max_retries: u32,
    default_retry_delay: Duration,
    http_client: reqwest::Client,
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

        Self {
            default_timeout,
            default_max_retries,
            default_retry_delay,
            http_client,
        }
    }

    pub async fn cleanup_lease(&self, lease: &Lease) -> Result<()> {
        if let Some(ref cleanup_config) = lease.cleanup_config {
            info!(
                "🧹 Starting cleanup for lease {} (object: {})",
                lease.lease_id, lease.object_id
            );
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
                        "✅ Successfully cleaned up lease"
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
                        "❌ Cleanup attempt failed"
                    );

                    if attempt < max_retries {
                        debug!("⏳ Waiting {}s before retry...", retry_delay.as_secs());
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
        // FIXED: Prioritize HTTP cleanup endpoint
        if !config.cleanup_http_endpoint.is_empty() {
            debug!(
                "🌐 Using HTTP cleanup endpoint: {}",
                config.cleanup_http_endpoint
            );
            return self.cleanup_via_http(lease, config).await;
        }

        // Fallback to gRPC cleanup if endpoint is provided
        if !config.cleanup_endpoint.is_empty() {
            debug!(
                "📡 Using gRPC cleanup endpoint: {}",
                config.cleanup_endpoint
            );
            return self.cleanup_via_grpc(lease, config).await;
        }

        // If no cleanup endpoints are provided, just log and return success
        warn!(
            lease_id = %lease.lease_id,
            object_id = %lease.object_id,
            "⚠️ No cleanup endpoints configured, marking cleanup as successful"
        );
        Ok(())
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

        info!(
            "📤 Sending HTTP cleanup request to {} for lease {} (object: {})",
            config.cleanup_http_endpoint, lease.lease_id, lease.object_id
        );

        debug!(
            "📝 Cleanup request payload: {}",
            serde_json::to_string(&cleanup_request).unwrap_or_default()
        );

        let response = timeout(
            self.default_timeout,
            self.http_client
                .post(&config.cleanup_http_endpoint)
                .json(&cleanup_request)
                .send(),
        )
        .await
        .map_err(|_| {
            error!(
                "⏰ HTTP cleanup request timed out after {}s",
                self.default_timeout.as_secs()
            );
            GCError::Timeout {
                timeout_seconds: self.default_timeout.as_secs(),
            }
        })?
        .map_err(|e| {
            error!("🌐 HTTP cleanup request failed: {}", e);
            GCError::Network(e.to_string())
        })?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Failed to read response body".to_string());

        if status.is_success() {
            info!(
                "✅ HTTP cleanup successful for lease {} (status: {}, response: {})",
                lease.lease_id, status, response_text
            );
            Ok(())
        } else {
            error!(
                "❌ HTTP cleanup failed for lease {} with status {}: {}",
                lease.lease_id, status, response_text
            );
            Err(GCError::Cleanup(format!(
                "HTTP cleanup failed with status {}: {}",
                status, response_text
            )))
        }
    }

    async fn cleanup_via_grpc(&self, lease: &Lease, config: &CleanupConfig) -> Result<()> {
        // For simplicity, we'll use HTTP to call the gRPC cleanup endpoint
        // In a production system, you might want to use actual gRPC clients
        let cleanup_request = CleanupRequest {
            lease_id: lease.lease_id.clone(),
            object_id: lease.object_id.clone(),
            object_type: format!("{:?}", lease.object_type),
            service_id: lease.service_id.clone(),
            metadata: lease.metadata.clone(),
            payload: config.cleanup_payload.clone(),
        };

        info!(
            "📡 Sending gRPC cleanup request to {} for lease {}",
            config.cleanup_endpoint, lease.lease_id
        );

        let response = timeout(
            self.default_timeout,
            self.http_client
                .post(&config.cleanup_endpoint)
                .json(&cleanup_request)
                .send(),
        )
        .await
        .map_err(|_| GCError::Timeout {
            timeout_seconds: self.default_timeout.as_secs(),
        })?
        .map_err(|e| GCError::Network(e.to_string()))?;

        if response.status().is_success() {
            info!("✅ gRPC cleanup successful for lease {}", lease.lease_id);
            Ok(())
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read response body".to_string());

            error!(
                "❌ gRPC cleanup failed for lease {} with status {}: {}",
                lease.lease_id, status, body
            );
            Err(GCError::Cleanup(format!(
                "gRPC cleanup failed with status {}: {}",
                status, body
            )))
        }
    }

    pub async fn cleanup_batch(&self, leases: Vec<Lease>) -> Vec<CleanupResult> {
        let mut results = Vec::new();
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(10)); // Max 10 concurrent cleanups

        info!("🧹 Starting batch cleanup of {} leases", leases.len());

        let mut handles = Vec::new();
        for lease in leases {
            let executor = self.clone();
            let semaphore = semaphore.clone();

            let handle = tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();
                let lease_id = lease.lease_id.clone();
                let object_id = lease.object_id.clone();

                let result = executor.cleanup_lease(&lease).await;
                CleanupResult {
                    lease_id,
                    object_id,
                    success: result.is_ok(),
                    error: result.err().map(|e| e.to_string()),
                }
            });

            handles.push(handle);
        }

        for handle in handles {
            match handle.await {
                Ok(result) => {
                    if result.success {
                        info!("✅ Batch cleanup success: {}", result.object_id);
                    } else {
                        warn!(
                            "❌ Batch cleanup failed: {} - {:?}",
                            result.object_id, result.error
                        );
                    }
                    results.push(result);
                }
                Err(e) => {
                    error!("Cleanup task panicked: {}", e);
                    results.push(CleanupResult {
                        lease_id: "unknown".to_string(),
                        object_id: "unknown".to_string(),
                        success: false,
                        error: Some(format!("Task panicked: {}", e)),
                    });
                }
            }
        }

        let successful = results.iter().filter(|r| r.success).count();
        let failed = results.len() - successful;

        info!(
            "📊 Batch cleanup completed: {} successful, {} failed",
            successful, failed
        );

        results
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

#[derive(Debug, Clone)]
pub struct CleanupResult {
    pub lease_id: String,
    pub object_id: String,
    pub success: bool,
    pub error: Option<String>,
}

impl CleanupResult {
    pub fn success(lease_id: String, object_id: String) -> Self {
        Self {
            lease_id,
            object_id,
            success: true,
            error: None,
        }
    }

    pub fn failure(lease_id: String, object_id: String, error: String) -> Self {
        Self {
            lease_id,
            object_id,
            success: false,
            error: Some(error),
        }
    }
}
