use crate::error::{GCError, Result};
use crate::lease::{ObjectType, LeaseState, CleanupConfig};
use crate::proto::{
    distributed_gc_service_client::DistributedGcServiceClient,
    CreateLeaseRequest, RenewLeaseRequest, ReleaseLeaseRequest,
    GetLeaseRequest, ListLeasesRequest, HealthCheckRequest,
};
use std::collections::HashMap;
use std::time::Duration;
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
    /// 
    /// # Arguments
    /// * `endpoint` - The GarbageTruck service endpoint (e.g., "http://localhost:50051")
    /// * `service_id` - Your service identifier for lease ownership
    /// 
    /// # Example
    /// ```rust
    /// use garbagetruck::GCClient;
    /// 
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let mut client = GCClient::new(
    ///         "http://localhost:50051",
    ///         "my-service".to_string()
    ///     ).await?;
    ///     Ok(())
    /// }
    /// ```
    pub async fn new(endpoint: &str, service_id: String) -> Result<Self> {
        let channel = Endpoint::from_shared(endpoint.to_string())
            .map_err(|e| GCError::Internal(e.to_string()))?
            .connect()
            .await
            .map_err(|e| GCError::Internal(e.to_string()))?;
        
        let client = DistributedGcServiceClient::new(channel);
        
        Ok(Self {
            client,
            service_id,
        })
    }
    
    /// Create a lease for an object
    /// 
    /// # Arguments
    /// * `object_id` - Unique identifier for the object
    /// * `object_type` - Type of object (DatabaseRow, BlobStorage, etc.)
    /// * `lease_duration_seconds` - How long the lease should last
    /// * `metadata` - Additional metadata about the object
    /// * `cleanup_config` - Optional cleanup configuration
    /// 
    /// # Returns
    /// The lease ID that can be used to renew or release the lease
    /// 
    /// # Example
    /// ```rust
    /// use garbagetruck::{GCClient, ObjectType};
    /// use std::collections::HashMap;
    /// 
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut client = GCClient::new("http://localhost:50051", "my-service".to_string()).await?;
    /// 
    /// let lease_id = client.create_lease(
    ///     "user-123".to_string(),
    ///     ObjectType::DatabaseRow,
    ///     3600, // 1 hour
    ///     [("table".to_string(), "users".to_string())].into(),
    ///     None
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_lease(
        &mut self,
        object_id: String,
        object_type: ObjectType,
        lease_duration_seconds: u64,
        metadata: HashMap<String, String>,
        cleanup_config: Option<CleanupConfig>,
    ) -> Result<String> {
        let request = CreateLeaseRequest {
            object_id,
            object_type: crate::proto::ObjectType::from(object_type) as i32,
            service_id: self.service_id.clone(),
            lease_duration_seconds,
            metadata,
            cleanup_config: cleanup_config.map(|c| c.into()),
        };
        
        let response = self
            .client
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
    /// 
    /// # Arguments
    /// * `lease_id` - The lease ID to renew
    /// * `extend_duration_seconds` - How much longer the lease should last
    /// 
    /// # Example
    /// ```rust
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # use garbagetruck::GCClient;
    /// # let mut client = GCClient::new("http://localhost:50051", "my-service".to_string()).await?;
    /// # let lease_id = "some-lease-id".to_string();
    /// 
    /// // Extend lease by 30 minutes
    /// client.renew_lease(lease_id, 1800).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn renew_lease(
        &mut self,
        lease_id: String,
        extend_duration_seconds: u64,
    ) -> Result<()> {
        let request = RenewLeaseRequest {
            lease_id,
            service_id: self.service_id.clone(),
            extend_duration_seconds,
        };
        
        let response = self
            .client
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
    /// 
    /// # Arguments
    /// * `lease_id` - The lease ID to release
    /// 
    /// # Example
    /// ```rust
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # use garbagetruck::GCClient;
    /// # let mut client = GCClient::new("http://localhost:50051", "my-service".to_string()).await?;
    /// # let lease_id = "some-lease-id".to_string();
    /// 
    /// // Release the lease when done
    /// client.release_lease(lease_id).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn release_lease(&mut self, lease_id: String) -> Result<()> {
        let request = ReleaseLeaseRequest {
            lease_id,
            service_id: self.service_id.clone(),
        };
        
        let response = self
            .client
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
    /// 
    /// # Arguments
    /// * `lease_id` - The lease ID to look up
    /// 
    /// # Returns
    /// Lease information if found, None if not found
    pub async fn get_lease(&mut self, lease_id: String) -> Result<Option<crate::proto::LeaseInfo>> {
        let request = GetLeaseRequest { lease_id };
        
        let response = self
            .client
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
    /// 
    /// # Arguments
    /// * `service_id` - Optional service ID filter (None for all services)
    /// * `limit` - Maximum number of leases to return
    /// 
    /// # Returns
    /// Vector of lease information
    pub async fn list_leases(
        &mut self,
        service_id: Option<String>,
        limit: u32,
    ) -> Result<Vec<crate::proto::LeaseInfo>> {
        let request = ListLeasesRequest {
            service_id: service_id.unwrap_or_default(),
            object_type: 0, // All types
            state: 0,       // All states
            limit,
            page_token: String::new(),
        };
        
        let response = self
            .client
            .list_leases(Request::new(request))
            .await
            .map_err(|e| GCError::Internal(e.to_string()))?;
        
        let list_response = response.into_inner();
        Ok(list_response.leases)
    }
    
    /// Check if the GarbageTruck service is healthy
    /// 
    /// # Returns
    /// true if healthy, false otherwise
    pub async fn health_check(&mut self) -> Result<bool> {
        let request = HealthCheckRequest {};
        
        let response = self
            .client
            .health_check(Request::new(request))
            .await
            .map_err(|e| GCError::Internal(e.to_string()))?;
        
        let health_response = response.into_inner();
        Ok(health_response.healthy)
    }
}

/// Convenience methods for common garbage collection patterns
impl GCClient {
    /// Create a temporary file lease with automatic cleanup
    /// 
    /// # Arguments
    /// * `file_path` - Path to the temporary file
    /// * `duration_seconds` - How long to keep the file
    /// * `cleanup_endpoint` - Optional HTTP endpoint to call for cleanup
    /// 
    /// # Example
    /// ```rust
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # use garbagetruck::GCClient;
    /// # let mut client = GCClient::new("http://localhost:50051", "my-service".to_string()).await?;
    /// 
    /// let lease_id = client.create_temp_file_lease(
    ///     "/tmp/processing-file.txt".to_string(),
    ///     3600, // Delete after 1 hour
    ///     Some("http://my-service/cleanup-file".to_string())
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_temp_file_lease(
        &mut self,
        file_path: String,
        duration_seconds: u64,
        cleanup_endpoint: Option<String>,
    ) -> Result<String> {
        let cleanup_config = cleanup_endpoint.map(|endpoint| CleanupConfig {
            cleanup_endpoint: String::new(),
            cleanup_http_endpoint: endpoint,
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
        ].into();
        
        self.create_lease(
            file_path,
            ObjectType::TemporaryFile,
            duration_seconds,
            metadata,
            cleanup_config,
        ).await
    }
    
    /// Create a database row lease with automatic cleanup
    /// 
    /// # Arguments
    /// * `table` - Database table name
    /// * `row_id` - Row identifier
    /// * `duration_seconds` - How long to keep the row
    /// * `cleanup_endpoint` - Optional HTTP endpoint to call for cleanup
    /// 
    /// # Example
    /// ```rust
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # use garbagetruck::GCClient;
    /// # let mut client = GCClient::new("http://localhost:50051", "my-service".to_string()).await?;
    /// 
    /// let lease_id = client.create_db_row_lease(
    ///     "temporary_uploads".to_string(),
    ///     "upload-123".to_string(),
    ///     1800, // Delete after 30 minutes
    ///     Some("http://my-service/cleanup-upload".to_string())
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_db_row_lease(
        &mut self,
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
        ].into();
        
        self.create_lease(
            object_id,
            ObjectType::DatabaseRow,
            duration_seconds,
            metadata,
            cleanup_config,
        ).await
    }
    
    /// Create a blob storage lease with automatic cleanup
    /// 
    /// # Arguments
    /// * `bucket` - Storage bucket name
    /// * `key` - Object key/path
    /// * `duration_seconds` - How long to keep the blob
    /// * `cleanup_endpoint` - Optional HTTP endpoint to call for cleanup
    /// 
    /// # Example
    /// ```rust
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # use garbagetruck::GCClient;
    /// # let mut client = GCClient::new("http://localhost:50051", "my-service".to_string()).await?;
    /// 
    /// let lease_id = client.create_blob_lease(
    ///     "uploads".to_string(),
    ///     "temp/file-123.jpg".to_string(),
    ///     7200, // Delete after 2 hours
    ///     Some("http://my-service/cleanup-blob".to_string())
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_blob_lease(
        &mut self,
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
        ].into();
        
        self.create_lease(
            object_id,
            ObjectType::BlobStorage,
            duration_seconds,
            metadata,
            cleanup_config,
        ).await
    }
    
    /// Create a WebSocket session lease with automatic cleanup
    /// 
    /// # Arguments
    /// * `session_id` - Session identifier
    /// * `user_id` - User identifier
    /// * `duration_seconds` - Session timeout
    /// * `cleanup_endpoint` - Optional HTTP endpoint to call for cleanup
    /// 
    /// # Example
    /// ```rust
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # use garbagetruck::GCClient;
    /// # let mut client = GCClient::new("http://localhost:50051", "my-service".to_string()).await?;
    /// 
    /// let lease_id = client.create_session_lease(
    ///     "session-abc123".to_string(),
    ///     "user-456".to_string(),
    ///     1800, // 30 minute timeout
    ///     Some("http://my-service/close-session".to_string())
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_session_lease(
        &mut self,
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
        ].into();
        
        self.create_lease(
            session_id,
            ObjectType::WebsocketSession,
            duration_seconds,
            metadata,
            cleanup_config,
        ).await
    }
    
    /// Create a cache entry lease with automatic cleanup
    /// 
    /// # Arguments
    /// * `cache_key` - Cache key
    /// * `duration_seconds` - TTL for the cache entry
    /// * `cleanup_endpoint` - Optional HTTP endpoint to call for cleanup
    /// 
    /// # Example
    /// ```rust
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # use garbagetruck::GCClient;
    /// # let mut client = GCClient::new("http://localhost:50051", "my-service".to_string()).await?;
    /// 
    /// let lease_id = client.create_cache_lease(
    ///     "expensive_computation_result_user123".to_string(),
    ///     3600, // Cache for 1 hour
    ///     Some("http://my-service/invalidate-cache".to_string())
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn create_cache_lease(
        &mut self,
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
        ].into();
        
        self.create_lease(
            cache_key,
            ObjectType::CacheEntry,
            duration_seconds,
            metadata,
            cleanup_config,
        ).await
    }
}