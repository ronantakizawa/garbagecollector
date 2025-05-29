use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

use crate::error::{GCError, Result};
use crate::lease::{Lease, CleanupConfig};

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
        // Try gRPC cleanup first if endpoint is provided
        if !config.cleanup_endpoint.is_empty() {
            return self.cleanup_via_grpc(lease, config).await;
        }

        // Try HTTP cleanup if endpoint is provided
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
            debug!(
                lease_id = %lease.lease_id,
                endpoint = %config.cleanup_endpoint,
                "gRPC cleanup successful"
            );
            Ok(())
        } else {
            let status = response.status();
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "Failed to read response body".to_string());
            
            Err(GCError::Cleanup(format!(
                "gRPC cleanup failed with status {}: {}",
                status, body
            )))
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

        let response = timeout(
            self.default_timeout,
            self.http_client
                .post(&config.cleanup_http_endpoint)
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
                endpoint = %config.cleanup_http_endpoint,
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

    pub async fn cleanup_batch(&self, leases: Vec<Lease>) -> Vec<CleanupResult> {
        let mut results = Vec::new();
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(10)); // Max 10 concurrent cleanups

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
                Ok(result) => results.push(result),
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

        results
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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