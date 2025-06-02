// src/bin/cleanup-server/main.rs - Standalone cleanup server

use std::path::PathBuf;
use std::sync::Arc;
use clap::Parser;
use tracing::{info, Level};
use tracing_subscriber;
use warp::Filter;

use garbagetruck::simulation::StorageCostTracker;

#[derive(Parser)]
#[command(name = "garbagetruck-cleanup-server")]
#[command(about = "Standalone HTTP cleanup server for GarbageTruck experiments")]
#[command(version)]
struct Args {
    /// Port to listen on
    #[arg(short, long, default_value = "8080")]
    port: u16,

    /// Base directory for file cleanup (security restriction)
    #[arg(short, long, default_value = "./temp_experiment")]
    base_directory: PathBuf,

    /// S3 cost per GB per month for tracking
    #[arg(long, default_value = "0.023")]
    cost_per_gb_month: f64,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = Args::parse();

    // Initialize logging
    let log_level = if args.verbose { Level::DEBUG } else { Level::INFO };
    tracing_subscriber::fmt()
        .with_max_level(log_level)
        .with_target(false)
        .init();

    info!("🧹 Starting GarbageTruck Cleanup Server");
    info!("📋 Configuration:");
    info!("  Port: {}", args.port);
    info!("  Base directory: {:?}", args.base_directory);
    info!("  Cost tracking: ${:.3}/GB/month", args.cost_per_gb_month);

    // Create base directory if it doesn't exist
    if !args.base_directory.exists() {
        std::fs::create_dir_all(&args.base_directory)?;
        info!("📁 Created base directory: {:?}", args.base_directory);
    }

    // Initialize cost tracker
    let cost_tracker = Arc::new(StorageCostTracker::new(args.cost_per_gb_month));
    let base_dir = Arc::new(args.base_directory);

    // Clone Arcs for each filter to avoid move conflicts
    let cost_tracker_cleanup = cost_tracker.clone();
    let base_dir_cleanup = base_dir.clone();
    let cost_tracker_stats = cost_tracker.clone();

    // Create warp filters
    let cleanup = warp::path("cleanup")
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::any().map(move || cost_tracker_cleanup.clone()))
        .and(warp::any().map(move || base_dir_cleanup.clone()))
        .and_then(handle_cleanup_request);

    let health = warp::path("health")
        .and(warp::get())
        .map(|| {
            warp::reply::json(&serde_json::json!({
                "status": "running",
                "service": "garbagetruck-cleanup-server",
                "version": env!("CARGO_PKG_VERSION")
            }))
        });

    let stats = warp::path("stats")
        .and(warp::get())
        .and(warp::any().map(move || cost_tracker_stats.clone()))
        .and_then(handle_stats_request);

    let routes = cleanup.or(health).or(stats);

    info!("🚀 Cleanup server starting on http://localhost:{}", args.port);
    info!("📊 Endpoints:");
    info!("  POST /cleanup - Delete files");
    info!("  GET  /health  - Health check");
    info!("  GET  /stats   - Cleanup statistics");
    info!("");
    info!("💡 Test with:");
    info!("  curl http://localhost:{}/health", args.port);
    info!("  curl -X POST http://localhost:{}/cleanup -H 'Content-Type: application/json' -d '{{\"file_path\": \"./temp_experiment/test.txt\"}}'", args.port);
    info!("");
    info!("🛑 Press Ctrl+C to stop");

    warp::serve(routes)
        .run(([0, 0, 0, 0], args.port))
        .await;

    Ok(())
}

async fn handle_cleanup_request(
    request: serde_json::Value,
    cost_tracker: Arc<StorageCostTracker>,
    base_dir: Arc<PathBuf>,
) -> Result<impl warp::Reply, warp::Rejection> {
    use std::fs;
    use std::path::Path;
    use tracing::{debug, error, warn};

    debug!("🧹 Received cleanup request: {}", request);
    
    // Extract file path from the request
    if let Some(file_path_str) = request.get("file_path").and_then(|v| v.as_str()) {
        let file_path = Path::new(file_path_str);
        
        // Security check - ensure file is within our base directory
        let canonical_base = base_dir.canonicalize().unwrap_or_else(|_| base_dir.as_ref().clone());
        let canonical_file = file_path.canonicalize().unwrap_or_else(|_| file_path.to_path_buf());
        
        if canonical_file.starts_with(&canonical_base) && file_path.exists() {
            match fs::metadata(file_path) {
                Ok(metadata) => {
                    let file_size = metadata.len();
                    match fs::remove_file(file_path) {
                        Ok(_) => {
                            cost_tracker.file_cleaned(file_size);
                            info!("✅ Successfully cleaned up file: {:?} ({} bytes)", file_path, file_size);
                            
                            Ok(warp::reply::json(&serde_json::json!({
                                "success": true,
                                "message": "File cleaned up successfully",
                                "file_path": file_path_str,
                                "file_size_bytes": file_size,
                                "timestamp": chrono::Utc::now().to_rfc3339()
                            })))
                        }
                        Err(e) => {
                            error!("❌ Failed to clean up file {:?}: {}", file_path, e);
                            Ok(warp::reply::json(&serde_json::json!({
                                "success": false,
                                "error": format!("Failed to delete file: {}", e),
                                "file_path": file_path_str
                            })))
                        }
                    }
                }
                Err(e) => {
                    warn!("⚠️  Could not get metadata for file {:?}: {}", file_path, e);
                    Ok(warp::reply::json(&serde_json::json!({
                        "success": false,
                        "error": format!("Could not access file: {}", e),
                        "file_path": file_path_str
                    })))
                }
            }
        } else {
            warn!("🚫 Security violation or file not found: {:?}", file_path);
            warn!("   Base directory: {:?}", canonical_base);
            warn!("   Requested file: {:?}", canonical_file);
            
            Ok(warp::reply::json(&serde_json::json!({
                "success": false,
                "error": "Invalid file path or file not found (security check failed)",
                "file_path": file_path_str
            })))
        }
    } else {
        warn!("🚫 No file_path provided in cleanup request");
        Ok(warp::reply::json(&serde_json::json!({
            "success": false,
            "error": "No file_path provided in request body"
        })))
    }
}

async fn handle_stats_request(
    cost_tracker: Arc<StorageCostTracker>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let stats = cost_tracker.get_stats();
    
    Ok(warp::reply::json(&serde_json::json!({
        "total_files_created": stats.total_files_created,
        "total_files_cleaned": stats.total_files_cleaned,
        "total_bytes_created": stats.total_bytes_created,
        "total_bytes_cleaned": stats.total_bytes_cleaned,
        "orphaned_files": stats.orphaned_files,
        "orphaned_bytes": stats.orphaned_bytes,
        "current_storage_bytes": stats.current_storage_bytes(),
        "current_monthly_cost": stats.current_monthly_cost(),
        "orphaned_monthly_cost": stats.orphaned_monthly_cost(),
        "cleanup_efficiency_percent": stats.cleanup_efficiency(),
        "cost_per_gb_per_month": stats.cost_per_gb_per_month,
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}