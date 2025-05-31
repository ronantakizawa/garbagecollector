#[cfg(feature = "client")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use garbagetruck::{GCClient, ObjectType};
    use std::collections::HashMap;
    
    tracing_subscriber::fmt::init();
    println!("🚛 GarbageTruck Client Example");
    
    // Create client
    let mut client = GCClient::new(
        "http://localhost:50051",
        "example-client-service".to_string()
    ).await?;
    
    // Check health first
    match client.health_check().await {
        Ok(healthy) => {
            if healthy {
                println!("✅ GarbageTruck service is healthy");
            } else {
                println!("⚠️  GarbageTruck service reports unhealthy");
            }
        }
        Err(e) => {
            println!("❌ Failed to connect to GarbageTruck service: {}", e);
            return Ok(());
        }
    }
    
    // Create a basic lease
    println!("\n📝 Creating basic lease...");
    let lease_id = client.create_lease(
        "example-object-123".to_string(),
        ObjectType::DatabaseRow,
        300, // 5 minutes
        [("table".to_string(), "users".to_string())].into(),
        None
    ).await?;
    println!("✅ Created lease: {}", lease_id);
    
    // Create convenience leases
    println!("\n📁 Creating temp file lease...");
    let file_lease = client.create_temp_file_lease(
        "/tmp/example-file.txt".to_string(),
        1800, // 30 minutes
        Some("http://my-service/cleanup".to_string())
    ).await?;
    println!("✅ Created temp file lease: {}", file_lease);
    
    println!("\n🗄️  Creating database row lease...");
    let db_lease = client.create_db_row_lease(
        "products".to_string(),
        "prod-456".to_string(),
        600, // 10 minutes
        Some("http://db-service/cleanup".to_string())
    ).await?;
    println!("✅ Created database lease: {}", db_lease);
    
    println!("\n💾 Creating cache lease...");
    let cache_lease = client.create_cache_lease(
        "expensive_computation_user789".to_string(),
        3600, // 1 hour
        Some("http://cache-service/invalidate".to_string())
    ).await?;
    println!("✅ Created cache lease: {}", cache_lease);
    
    println!("\n🌐 Creating session lease...");
    let session_lease = client.create_session_lease(
        "session-xyz789".to_string(),
        "user-123".to_string(),
        1800, // 30 minutes
        Some("http://websocket-service/close-session".to_string())
    ).await?;
    println!("✅ Created session lease: {}", session_lease);
    
    // List leases
    println!("\n📋 Listing leases...");
    let leases = client.list_leases(Some("example-client-service".to_string()), 10).await?;
    println!("✅ Found {} leases", leases.len());
    
    // Renew a lease
    println!("\n🔄 Renewing lease...");
    client.renew_lease(lease_id.clone(), 600).await?; // Extend by 10 minutes
    println!("✅ Renewed lease: {}", lease_id);
    
    // Get lease details
    println!("\n🔍 Getting lease details...");
    if let Some(lease_info) = client.get_lease(lease_id.clone()).await? {
        println!("✅ Lease details: ID={}, Object={}", lease_info.lease_id, lease_info.object_id);
    }
    
    // Release leases
    println!("\n🗑️  Releasing leases...");
    client.release_lease(lease_id).await?;
    client.release_lease(file_lease).await?;
    client.release_lease(db_lease).await?;
    client.release_lease(cache_lease).await?;
    client.release_lease(session_lease).await?;
    println!("✅ Released all leases");
    
    println!("\n🎉 GarbageTruck example completed successfully!");
    Ok(())
}

#[cfg(not(feature = "client"))]
fn main() {
    println!("❌ Client feature not enabled.");
    println!("💡 Run with: cargo run --example client_usage --features client");
}