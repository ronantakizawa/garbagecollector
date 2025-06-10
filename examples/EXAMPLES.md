# Examples of using GarbageTruck

### SDK Usage Patterns

#### 1. Temporary File Management
```rust
use garbagetruck::GCClient;

async fn process_upload() -> Result> {
    let mut client = GCClient::new("http://localhost:50051", "upload-service".to_string()).await?;
    
    // Create a temporary file lease with automatic cleanup
    let lease_id = client.create_temp_file_lease(
        "/tmp/user-upload-123.jpg".to_string(),
        3600, // Delete after 1 hour
        Some("http://my-service/cleanup-file".to_string()) // Cleanup endpoint
    ).await?;
    
    // Process the file...
    process_image("/tmp/user-upload-123.jpg").await?;
    
    // File will be automatically deleted after 1 hour, or we can release early
    client.release_lease(lease_id).await?;
    
    Ok(())
}
```

#### 2. Database Row Protection
```rust
async fn create_user_session() -> Result> {
    let mut client = GCClient::new("http://localhost:50051", "auth-service".to_string()).await?;
    
    // Protect a database row with automatic cleanup
    let lease_id = client.create_db_row_lease(
        "user_sessions".to_string(),
        session_id.to_string(),
        1800, // 30 minutes
        Some("http://auth-service/cleanup-session".to_string())
    ).await?;
    
    // Session is now protected - if service crashes, it will be cleaned up automatically
    
    // Extend session if user is active
    client.renew_lease(lease_id.clone(), 1800).await?;
    
    // Clean up when user logs out
    client.release_lease(lease_id).await?;
    
    Ok(())
}
```

#### 3. Blob Storage Management
```rust
async fn upload_temp_file() -> Result> {
    let mut client = GCClient::new("http://localhost:50051", "storage-service".to_string()).await?;
    
    // Upload file to S3
    let s3_key = upload_to_s3(file_data).await?;
    
    // Create lease for the uploaded blob
    let lease_id = client.create_blob_lease(
        "temp-uploads".to_string(),
        s3_key.clone(),
        7200, // 2 hours
        Some("http://storage-service/delete-s3-object".to_string())
    ).await?;
    
    // Process the uploaded file...
    let result = process_uploaded_file(&s3_key).await?;
    
    if result.should_keep {
        // Move to permanent storage and release lease
        move_to_permanent_storage(&s3_key).await?;
        client.release_lease(lease_id).await?;
    } else {
        // Just release - file will be cleaned up automatically
        client.release_lease(lease_id).await?;
    }
    
    Ok(())
}
```

#### 4. WebSocket Session Management
```rust
async fn handle_websocket_connection(session_id: String, user_id: String) -> Result> {
    let mut client = GCClient::new("http://localhost:50051", "websocket-service".to_string()).await?;
    
    // Create session lease with auto-cleanup on disconnect
    let lease_id = client.create_session_lease(
        session_id.clone(),
        user_id.clone(),
        300, // 5 minute timeout
        Some("http://websocket-service/force-disconnect".to_string())
    ).await?;
    
    // Keep renewing lease while connection is active
    loop {
        tokio::select! {
            // Renew lease on each message
            _ = receive_message() => {
                client.renew_lease(lease_id.clone(), 300).await?;
            }
            // Or handle disconnect
            _ = connection_closed() => {
                client.release_lease(lease_id).await?;
                break;
            }
        }
    }
    
    Ok(())
}
```

## ⚙️ Configuration

The service can be configured via environment variables:

### Server Configuration
```bash
export GC_SERVER_HOST=0.0.0.0
export GC_SERVER_PORT=50051
```

### Lease Management
```bash
export GC_DEFAULT_LEASE_DURATION=300      # 5 minutes
export GC_MAX_LEASE_DURATION=3600         # 1 hour  
export GC_MIN_LEASE_DURATION=30           # 30 seconds
export GC_CLEANUP_INTERVAL=60             # 1 minute
export GC_CLEANUP_GRACE_PERIOD=30         # 30 seconds
export GC_MAX_LEASES_PER_SERVICE=10000
```

### Storage Backend
```bash
# Use in-memory storage (default, for development)
export GC_STORAGE_BACKEND=memory

# Use PostgreSQL (for production)
export GC_STORAGE_BACKEND=postgres
export DATABASE_URL="postgresql://user:pass@host:5432/distributed_gc"
export GC_MAX_DB_CONNECTIONS=10
```

### Cleanup Configuration
```bash
export GC_CLEANUP_TIMEOUT=30
export GC_CLEANUP_MAX_RETRIES=3
export GC_CLEANUP_RETRY_DELAY=5
```

### WAL
```bash
export GC_STORAGE_BACKEND=persistent_file 
export GC_ENABLE_WAL=true 
export GC_DATA_DIRECTORY=/tmp/garbagetruck-data 
export GC_WAL_PATH=/tmp/garbagetruck-data/garbagetruck.wal 
export GC_WAL_SYNC_POLICY=every_write 
export GC_ENABLE_AUTO_RECOVERY=true
```