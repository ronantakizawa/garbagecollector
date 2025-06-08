// examples/client_usage.rs - Fixed version

use garbagetruck::GCClient;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to GarbageTruck server
    let mut client = GCClient::new("http://localhost:50051", "example-service".to_string()).await?;

    println!("✅ Connected to GarbageTruck server");

    // Create a lease for a temporary file
    let lease_id = client
        .create_temp_file_lease(
            "/tmp/example-file.txt".to_string(),
            3600, // 1 hour
            Some("http://localhost:8080/cleanup".to_string()),
        )
        .await?;

    println!("📋 Created lease: {}", lease_id);

    // Create a lease for a database row
    let db_lease_id = client
        .create_db_row_lease(
            "temp_uploads".to_string(),
            "upload-123".to_string(),
            1800, // 30 minutes
            Some("http://localhost:8080/cleanup".to_string()),
        )
        .await?;

    println!("📋 Created database lease: {}", db_lease_id);

    // List all leases
    let leases = client.list_leases(None, 10).await?;
    println!("📊 Current leases: {}", leases.len());

    // Renew the first lease
    client.renew_lease(lease_id.clone(), 1800).await?;
    println!("🔄 Renewed lease: {}", lease_id);

    // Release the leases when done
    client.release_lease(lease_id).await?;
    client.release_lease(db_lease_id).await?;
    println!("✅ Released leases");

    Ok(())
}
