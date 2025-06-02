// Update for src/client.rs - Fix mutability issues

use crate::error::{GCError, Result};
use crate::lease::{CleanupConfig, ObjectType};
use crate::proto::{
    distributed_gc_service_client::DistributedGcServiceClient, CreateLeaseRequest, GetLeaseRequest,
    HealthCheckRequest, ListLeasesRequest, ReleaseLeaseRequest, RenewLeaseRequest,
};
use std::collections::HashMap;
use tonic::transport::{Channel, Endpoint};
use tonic::Request;

/// GarbageTruck client for basic operations
#[derive(Clone)]
pub struct GCClient {
    client: DistributedGcServiceClient<Channel>,
    service_id: String,
}

impl GCClient {
    /// Create a new GarbageTruck client
    pub async fn new(endpoint: &str, service_id: String) -> Result<Self> {
        let channel = Endpoint::from_shared(endpoint.to_string())
            .map_err(|e| GCError::Internal(e.to_string()))?
            .connect()
            .await
            .map_err(|e| GCError::Internal(e.to_string()))?;

        let client = DistributedGcServiceClient::new(channel);

        Ok(Self { client, service_id })
    }

    /// Create a lease for an object
    pub async fn create_lease(
        &self, // FIXED: Remove mut
        object_id: String,
        object_type: ObjectType,
        lease_duration_seconds: u64,
        metadata: HashMap<String, String>,
        cleanup_config: Option<CleanupConfig>,
    ) -> Result<String> {
        let mut client = self.client.clone(); // Clone the client instead

        let request = CreateLeaseRequest {
            object_id,
            object_type: crate::proto::ObjectType::from(object_type) as i32,
            service_id: self.service_id.clone(),
            lease_duration_seconds,
            metadata,
            cleanup_config: cleanup_config.map(|c| c.into()),
        };

        let response = client
            .create_lease(Request::new(request))
            .await
            .map_err(|e| GCError::Internal(e.to_string()))?;

        let create_response = response.into_inner();
        if create_response.success {
            Ok(create_response.lease_id)
        } else {
            Err(GCError::Internal(create_response.error_message))
        }
    }

    /// Renew an existing lease
    pub async fn renew_lease(
        &self, // FIXED: Remove mut
        lease_id: String,
        extend_duration_seconds: u64,
    ) -> Result<()> {
        let mut client = self.client.clone();

        let request = RenewLeaseRequest {
            lease_id,
            service_id: self.service_id.clone(),
            extend_duration_seconds,
        };

        let response = client
            .renew_lease(Request::new(request))
            .await
            .map_err(|e| GCError::Internal(e.to_string()))?;

        let renew_response = response.into_inner();
        if renew_response.success {
            Ok(())
        } else {
            Err(GCError::Internal(renew_response.error_message))
        }
    }

    /// Release a lease (cleanup will happen immediately if configured)
    pub async fn release_lease(&self, lease_id: String) -> Result<()> {
        let mut client = self.client.clone();

        let request = ReleaseLeaseRequest {
            lease_id,
            service_id: self.service_id.clone(),
        };

        let response = client
            .release_lease(Request::new(request))
            .await
            .map_err(|e| GCError::Internal(e.to_string()))?;

        let release_response = response.into_inner();
        if release_response.success {
            Ok(())
        } else {
            Err(GCError::Internal(release_response.error_message))
        }
    }

    /// Get information about a specific lease
    pub async fn get_lease(&self, lease_id: String) -> Result<Option<crate::proto::LeaseInfo>> {
        let mut client = self.client.clone();

        let request = GetLeaseRequest { lease_id };

        let response = client
            .get_lease(Request::new(request))
            .await
            .map_err(|e| GCError::Internal(e.to_string()))?;

        let get_response = response.into_inner();
        if get_response.found {
            Ok(get_response.lease)
        } else {
            Ok(None)
        }
    }

    /// List leases for this service or all services
    pub async fn list_leases(
        &self,
        service_id: Option<String>,
        limit: u32,
    ) -> Result<Vec<crate::proto::LeaseInfo>> {
        let mut client = self.client.clone();

        let request = ListLeasesRequest {
            service_id: service_id.unwrap_or_default(),
            object_type: 0, // All types
            state: 0,       // All states
            limit,
            page_token: String::new(),
        };

        let response = client
            .list_leases(Request::new(request))
            .await
            .map_err(|e| GCError::Internal(e.to_string()))?;

        let list_response = response.into_inner();
        Ok(list_response.leases)
    }

    /// Check if the GarbageTruck service is healthy
    pub async fn health_check(&self) -> Result<bool> {
        let mut client = self.client.clone();

        let request = HealthCheckRequest {};

        let response = client
            .health_check(Request::new(request))
            .await
            .map_err(|e| GCError::Internal(e.to_string()))?;

        let health_response = response.into_inner();
        Ok(health_response.healthy)
    }

    // Convenience methods updated to not require mut
    pub async fn create_temp_file_lease(
        &mut self,
        file_path: String,
        duration_seconds: u64,
        cleanup_endpoint: Option<String>,
    ) -> Result<String> {
        let cleanup_config = cleanup_endpoint.map(|endpoint| CleanupConfig {
            cleanup_endpoint: String::new(),
            cleanup_http_endpoint: endpoint,
            // FIXED: Put file_path directly in the payload AND as a top-level field
            cleanup_payload: format!(
                r#"{{"action": "delete_file", "file_path": "{}"}}"#,
                file_path
            ),
            max_retries: 3,
            retry_delay_seconds: 2,
        });

        let metadata = [
            ("file_path".to_string(), file_path.clone()),
            ("type".to_string(), "temp_file".to_string()),
        ]
        .into();

        // CRITICAL: Use the file_path as object_id so cleanup server can find it
        self.create_lease(
            file_path, // This becomes the object_id that gets sent to cleanup
            ObjectType::TemporaryFile,
            duration_seconds,
            metadata,
            cleanup_config,
        )
        .await
    }

    pub async fn create_db_row_lease(
        &self,
        table: String,
        row_id: String,
        duration_seconds: u64,
        cleanup_endpoint: Option<String>,
    ) -> Result<String> {
        let object_id = format!("{}:{}", table, row_id);
        let cleanup_config = cleanup_endpoint.map(|endpoint| CleanupConfig {
            cleanup_endpoint: String::new(),
            cleanup_http_endpoint: endpoint,
            cleanup_payload: format!(
                r#"{{"action": "delete_row", "table": "{}", "row_id": "{}"}}"#,
                table, row_id
            ),
            max_retries: 3,
            retry_delay_seconds: 2,
        });

        let metadata = [
            ("table".to_string(), table),
            ("row_id".to_string(), row_id),
            ("type".to_string(), "db_row".to_string()),
        ]
        .into();

        self.create_lease(
            object_id,
            ObjectType::DatabaseRow,
            duration_seconds,
            metadata,
            cleanup_config,
        )
        .await
    }

    pub async fn create_blob_lease(
        &self,
        bucket: String,
        key: String,
        duration_seconds: u64,
        cleanup_endpoint: Option<String>,
    ) -> Result<String> {
        let object_id = format!("{}:{}", bucket, key);
        let cleanup_config = cleanup_endpoint.map(|endpoint| CleanupConfig {
            cleanup_endpoint: String::new(),
            cleanup_http_endpoint: endpoint,
            cleanup_payload: format!(
                r#"{{"action": "delete_blob", "bucket": "{}", "key": "{}"}}"#,
                bucket, key
            ),
            max_retries: 3,
            retry_delay_seconds: 2,
        });

        let metadata = [
            ("bucket".to_string(), bucket),
            ("key".to_string(), key),
            ("type".to_string(), "blob".to_string()),
        ]
        .into();

        self.create_lease(
            object_id,
            ObjectType::BlobStorage,
            duration_seconds,
            metadata,
            cleanup_config,
        )
        .await
    }

    pub async fn create_session_lease(
        &self,
        session_id: String,
        user_id: String,
        duration_seconds: u64,
        cleanup_endpoint: Option<String>,
    ) -> Result<String> {
        let cleanup_config = cleanup_endpoint.map(|endpoint| CleanupConfig {
            cleanup_endpoint: String::new(),
            cleanup_http_endpoint: endpoint,
            cleanup_payload: format!(
                r#"{{"action": "close_session", "session_id": "{}", "user_id": "{}"}}"#,
                session_id, user_id
            ),
            max_retries: 3,
            retry_delay_seconds: 2,
        });

        let metadata = [
            ("session_id".to_string(), session_id.clone()),
            ("user_id".to_string(), user_id),
            ("type".to_string(), "websocket_session".to_string()),
        ]
        .into();

        self.create_lease(
            session_id,
            ObjectType::WebsocketSession,
            duration_seconds,
            metadata,
            cleanup_config,
        )
        .await
    }

    pub async fn create_cache_lease(
        &self,
        cache_key: String,
        duration_seconds: u64,
        cleanup_endpoint: Option<String>,
    ) -> Result<String> {
        let cleanup_config = cleanup_endpoint.map(|endpoint| CleanupConfig {
            cleanup_endpoint: String::new(),
            cleanup_http_endpoint: endpoint,
            cleanup_payload: format!(
                r#"{{"action": "invalidate_cache", "cache_key": "{}"}}"#,
                cache_key
            ),
            max_retries: 3,
            retry_delay_seconds: 2,
        });

        let metadata = [
            ("cache_key".to_string(), cache_key.clone()),
            ("type".to_string(), "cache_entry".to_string()),
        ]
        .into();

        self.create_lease(
            cache_key,
            ObjectType::CacheEntry,
            duration_seconds,
            metadata,
            cleanup_config,
        )
        .await
    }
}
