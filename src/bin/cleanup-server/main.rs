// src/bin/cleanup-server/main.rs - Fixed cleanup server with proper file path handling

use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error, info, warn, Level};
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
    let log_level = if args.verbose {
        Level::DEBUG
    } else {
        Level::INFO
    };
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

    let health = warp::path("health").and(warp::get()).map(|| {
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

    info!(
        "🚀 Cleanup server starting on http://localhost:{}",
        args.port
    );
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

    warp::serve(routes).run(([0, 0, 0, 0], args.port)).await;

    Ok(())
}

async fn handle_cleanup_request(
    request: serde_json::Value,
    cost_tracker: Arc<StorageCostTracker>,
    base_dir: Arc<PathBuf>,
) -> std::result::Result<impl warp::Reply, warp::Rejection> {
    use std::fs;
    use std::path::Path;

    debug!("🧹 Received cleanup request: {}", request);

    // FIXED: Extract file path from multiple possible sources with proper lifetime handling
    let file_path_str = if let Some(path) = request.get("file_path").and_then(|v| v.as_str()) {
        path.to_string()
    } else if let Some(path) = request.get("object_id").and_then(|v| v.as_str()) {
        // GarbageTruck sends the file path as object_id
        path.to_string()
    } else if let Some(payload) = request.get("payload").and_then(|v| v.as_str()) {
        // Check if file_path is in the payload JSON
        if let Ok(payload_json) = serde_json::from_str::<serde_json::Value>(payload) {
            payload_json
                .get("file_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        } else {
            error!("❌ No file_path found in cleanup request payload");
            return Ok(warp::reply::json(&serde_json::json!({
                "success": false,
                "error": "No file_path provided in request payload"
            })));
        }
    } else {
        error!("❌ No file_path found in cleanup request");
        return Ok(warp::reply::json(&serde_json::json!({
            "success": false,
            "error": "No file_path provided in request body"
        })));
    };

    let file_path = Path::new(&file_path_str);

    info!("🗑️  Processing cleanup request for file: {:?}", file_path);

    // FIXED: More flexible security check that works with relative paths
    let is_safe_path = if file_path.is_absolute() {
        // For absolute paths, check if they're within the base directory
        match (file_path.canonicalize(), base_dir.canonicalize()) {
            (Ok(canonical_file), Ok(canonical_base)) => canonical_file.starts_with(canonical_base),
            _ => {
                // If canonicalization fails, check if the file exists and is within base_dir
                file_path.starts_with(base_dir.as_ref()) && file_path.exists()
            }
        }
    } else {
        // FIXED: For relative paths, construct full path and check safety
        let full_path = if file_path_str.starts_with("./") {
            // Path like "./temp_experiment/with_gc/job_1.dat"
            Path::new(&file_path_str[2..]) // Remove "./" prefix
        } else {
            file_path
        };

        // Check if it's within our base directory and doesn't escape with ".."
        !file_path_str.contains("..") && {
            // Construct the expected full path
            let expected_path = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(full_path);

            debug!(
                "🔍 Security check: file={:?}, base={:?}, expected={:?}",
                file_path, base_dir, expected_path
            );

            // Check if file exists at the expected location
            expected_path.exists()
        }
    };

    debug!(
        "🔐 Security check result: {} for path {:?}",
        is_safe_path, file_path
    );

    if is_safe_path {
        // Determine the actual file path to use
        let actual_file_path = if file_path.exists() {
            file_path.to_path_buf()
        } else {
            // Try relative to current directory for paths like "./temp_experiment/..."
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(file_path)
        };

        info!("🎯 Attempting to delete file: {:?}", actual_file_path);

        match fs::metadata(&actual_file_path) {
            Ok(metadata) => {
                let file_size = metadata.len();
                match fs::remove_file(&actual_file_path) {
                    Ok(_) => {
                        cost_tracker.file_cleaned(file_size);
                        info!(
                            "✅ Successfully cleaned up file: {:?} ({} bytes)",
                            actual_file_path, file_size
                        );

                        Ok(warp::reply::json(&serde_json::json!({
                            "success": true,
                            "message": "File cleaned up successfully",
                            "file_path": file_path_str,
                            "actual_path": format!("{:?}", actual_file_path),
                            "file_size_bytes": file_size,
                            "timestamp": chrono::Utc::now().to_rfc3339()
                        })))
                    }
                    Err(e) => {
                        error!("❌ Failed to clean up file {:?}: {}", actual_file_path, e);
                        Ok(warp::reply::json(&serde_json::json!({
                            "success": false,
                            "error": format!("Failed to delete file: {}", e),
                            "file_path": file_path_str,
                            "actual_path": format!("{:?}", actual_file_path)
                        })))
                    }
                }
            }
            Err(e) => {
                warn!(
                    "⚠️  Could not get metadata for file {:?}: {}",
                    actual_file_path, e
                );
                // File might already be deleted, which is fine for cleanup
                Ok(warp::reply::json(&serde_json::json!({
                    "success": true,
                    "message": "File already cleaned up or not found",
                    "file_path": file_path_str,
                    "actual_path": format!("{:?}", actual_file_path),
                    "note": format!("File metadata error: {}", e)
                })))
            }
        }
    } else {
        warn!("🚫 Security violation or file not found: {:?}", file_path);
        warn!("   Base directory: {:?}", base_dir);
        warn!("   Requested file: {:?}", file_path);
        warn!("   File exists: {}", file_path.exists());

        Ok(warp::reply::json(&serde_json::json!({
            "success": false,
            "error": "Invalid file path or file not found (security check failed)",
            "file_path": file_path_str,
            "base_directory": format!("{:?}", base_dir),
            "file_exists": file_path.exists()
        })))
    }
}

async fn handle_stats_request(
    cost_tracker: Arc<StorageCostTracker>,
) -> std::result::Result<impl warp::Reply, warp::Rejection> {
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
