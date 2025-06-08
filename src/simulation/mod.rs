// src/simulation/mod.rs - Fixed simulation with proper cleanup integration

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::fs;
use tokio::time::{interval, sleep};
use tracing::{debug, error, info, warn};
use warp::Filter;

use crate::client::GCClient;
use crate::error::{GCError, Result};

/// Configuration for file simulation experiments
#[derive(Debug, Clone)]
pub struct FileSimulationConfig {
    pub base_directory: PathBuf,
    pub file_size_bytes: usize,
    pub job_count: u32,
    pub job_duration_seconds: u64,
    pub crash_probability: f32,
    pub cleanup_endpoint: Option<String>,
}

/// Tracks storage costs and cleanup efficiency
#[derive(Debug, Clone)]
pub struct StorageCostTracker {
    inner: Arc<Mutex<StorageStatsInner>>,
    pub cost_per_gb_per_month: f64,
}

#[derive(Debug, Clone)]
struct StorageStatsInner {
    pub total_files_created: u64,
    pub total_files_cleaned: u64,
    pub total_bytes_created: u64,
    pub total_bytes_cleaned: u64,
    pub orphaned_files: u64,
    pub orphaned_bytes: u64,
}

/// Public storage statistics
#[derive(Debug, Clone)]
pub struct StorageStats {
    pub total_files_created: u64,
    pub total_files_cleaned: u64,
    pub total_bytes_created: u64,
    pub total_bytes_cleaned: u64,
    pub orphaned_files: u64,
    pub orphaned_bytes: u64,
    pub cost_per_gb_per_month: f64,
}

impl StorageStats {
    pub fn current_storage_bytes(&self) -> u64 {
        self.total_bytes_created
            .saturating_sub(self.total_bytes_cleaned)
    }

    pub fn current_monthly_cost(&self) -> f64 {
        let gb = self.current_storage_bytes() as f64 / (1024.0 * 1024.0 * 1024.0);
        gb * self.cost_per_gb_per_month
    }

    pub fn orphaned_monthly_cost(&self) -> f64 {
        let gb = self.orphaned_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        gb * self.cost_per_gb_per_month
    }

    pub fn cleanup_efficiency(&self) -> f64 {
        if self.total_files_created == 0 {
            return 100.0;
        }
        (self.total_files_cleaned as f64 / self.total_files_created as f64) * 100.0
    }
}

impl StorageCostTracker {
    pub fn new(cost_per_gb_per_month: f64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(StorageStatsInner {
                total_files_created: 0,
                total_files_cleaned: 0,
                total_bytes_created: 0,
                total_bytes_cleaned: 0,
                orphaned_files: 0,
                orphaned_bytes: 0,
            })),
            cost_per_gb_per_month,
        }
    }

    pub fn file_created(&self, size_bytes: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.total_files_created += 1;
        inner.total_bytes_created += size_bytes;
    }

    pub fn file_cleaned(&self, size_bytes: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.total_files_cleaned += 1;
        inner.total_bytes_cleaned += size_bytes;
    }

    pub fn file_orphaned(&self, size_bytes: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.orphaned_files += 1;
        inner.orphaned_bytes += size_bytes;
    }

    pub fn get_stats(&self) -> StorageStats {
        let inner = self.inner.lock().unwrap();
        StorageStats {
            total_files_created: inner.total_files_created,
            total_files_cleaned: inner.total_files_cleaned,
            total_bytes_created: inner.total_bytes_created,
            total_bytes_cleaned: inner.total_bytes_cleaned,
            orphaned_files: inner.orphaned_files,
            orphaned_bytes: inner.orphaned_bytes,
            cost_per_gb_per_month: self.cost_per_gb_per_month,
        }
    }
}

/// Simulates jobs that create temporary files
pub struct JobSimulator {
    config: FileSimulationConfig,
    cost_tracker: StorageCostTracker,
    gc_client: Option<GCClient>,
    active_leases: Arc<Mutex<HashMap<String, String>>>, // file_path -> lease_id
}

impl JobSimulator {
    pub fn new(
        config: FileSimulationConfig,
        cost_tracker: StorageCostTracker,
        gc_client: Option<GCClient>,
    ) -> Self {
        Self {
            config,
            cost_tracker,
            gc_client,
            active_leases: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Run experiment without GarbageTruck (files will be orphaned when jobs crash)
    pub async fn run_without_garbagetruck(&mut self) -> Result<StorageStats> {
        info!("🚫 Running simulation WITHOUT GarbageTruck");

        // Create base directory
        if !self.config.base_directory.exists() {
            fs::create_dir_all(&self.config.base_directory)
                .await
                .map_err(|e| GCError::Internal(format!("Failed to create directory: {}", e)))?;
        }

        let mut handles = Vec::new();

        for job_id in 0..self.config.job_count {
            let config = self.config.clone();
            let cost_tracker = self.cost_tracker.clone();

            let handle = tokio::spawn(async move {
                let file_path = config.base_directory.join(format!("job_{}.dat", job_id));

                // Create file
                let file_data = vec![0u8; config.file_size_bytes];
                if let Err(e) = fs::write(&file_path, file_data).await {
                    error!("Failed to create file {:?}: {}", file_path, e);
                    return;
                }

                cost_tracker.file_created(config.file_size_bytes as u64);
                debug!("Created file: {:?}", file_path);

                // Simulate job work
                sleep(Duration::from_secs(config.job_duration_seconds)).await;

                // Simulate crash - don't clean up file
                if fastrand::f32() < config.crash_probability {
                    info!("💥 Job {} crashed - file will be orphaned", job_id);
                    cost_tracker.file_orphaned(config.file_size_bytes as u64);
                } else {
                    // Job completed successfully - clean up file
                    if let Err(e) = fs::remove_file(&file_path).await {
                        warn!("Failed to clean up file {:?}: {}", file_path, e);
                        cost_tracker.file_orphaned(config.file_size_bytes as u64);
                    } else {
                        cost_tracker.file_cleaned(config.file_size_bytes as u64);
                        debug!("✅ Job {} completed - file cleaned up", job_id);
                    }
                }
            });

            handles.push(handle);

            // Small delay between job starts
            sleep(Duration::from_millis(50)).await;
        }

        // Wait for all jobs to complete
        for handle in handles {
            if let Err(e) = handle.await {
                error!("Job task failed: {}", e);
            }
        }

        info!("🚫 Simulation without GarbageTruck completed");
        Ok(self.cost_tracker.get_stats())
    }

    /// Run experiment with GarbageTruck (files will be cleaned up when leases expire)
    pub async fn run_with_garbagetruck(&mut self) -> Result<StorageStats> {
        info!("✅ Running simulation WITH GarbageTruck");

        let gc_client = self
            .gc_client
            .as_ref()
            .ok_or_else(|| GCError::Internal("No GarbageTruck client provided".to_string()))?;

        // Create base directory
        if !self.config.base_directory.exists() {
            fs::create_dir_all(&self.config.base_directory)
                .await
                .map_err(|e| GCError::Internal(format!("Failed to create directory: {}", e)))?;
        }

        let mut handles = Vec::new();

        for job_id in 0..self.config.job_count {
            let config = self.config.clone();
            let cost_tracker = self.cost_tracker.clone();
            let gc_client = gc_client.clone();
            let active_leases = self.active_leases.clone();

            let handle = tokio::spawn(async move {
                let file_path = config.base_directory.join(format!("job_{}.dat", job_id));
                let file_path_str = file_path.to_string_lossy().to_string();

                // Create file
                let file_data = vec![0u8; config.file_size_bytes];
                if let Err(e) = fs::write(&file_path, file_data).await {
                    error!("Failed to create file {:?}: {}", file_path, e);
                    return;
                }

                cost_tracker.file_created(config.file_size_bytes as u64);
                debug!("Created file: {:?}", file_path);

                // FIXED: Create lease with shorter duration for faster cleanup in experiments
                let lease_duration = 20; // Use 20 seconds instead of longer duration
                match gc_client
                    .create_temp_file_lease(
                        file_path_str.clone(),
                        lease_duration,
                        config.cleanup_endpoint.clone(),
                    )
                    .await
                {
                    Ok(lease_id) => {
                        debug!("📋 Created lease {} for file: {:?}", lease_id, file_path);

                        // Track active lease
                        {
                            let mut leases = active_leases.lock().unwrap();
                            leases.insert(file_path_str.clone(), lease_id.clone());
                        }

                        // Simulate job work
                        sleep(Duration::from_secs(config.job_duration_seconds)).await;

                        // Simulate crash or success
                        if fastrand::f32() < config.crash_probability {
                            info!("💥 Job {} crashed - GarbageTruck will clean up via lease expiration", job_id);
                            // Don't release lease - let it expire and trigger cleanup
                        } else {
                            // Job completed successfully - release lease explicitly
                            if let Err(e) = gc_client.release_lease(lease_id.clone()).await {
                                warn!("Failed to release lease {}: {}", lease_id, e);
                            } else {
                                debug!("✅ Job {} completed - lease released", job_id);
                            }

                            // Remove from active leases
                            {
                                let mut leases = active_leases.lock().unwrap();
                                leases.remove(&file_path_str);
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to create lease for file {:?}: {}", file_path, e);
                        // Fallback: track as orphaned since we can't protect it
                        cost_tracker.file_orphaned(config.file_size_bytes as u64);
                    }
                }
            });

            handles.push(handle);

            // Small delay between job starts
            sleep(Duration::from_millis(50)).await;
        }

        // Wait for all jobs to complete
        for handle in handles {
            if let Err(e) = handle.await {
                error!("Job task failed: {}", e);
            }
        }

        // Wait a bit for cleanup to happen
        info!("⏳ Waiting for GarbageTruck cleanup to process expired leases...");
        sleep(Duration::from_secs(5)).await;

        info!("✅ Simulation with GarbageTruck completed");
        Ok(self.cost_tracker.get_stats())
    }

    /// Count remaining files on disk
    pub fn count_remaining_files(&self) -> Result<(u32, u64)> {
        let mut file_count = 0;
        let mut total_bytes = 0;

        if !self.config.base_directory.exists() {
            return Ok((0, 0));
        }

        match std::fs::read_dir(&self.config.base_directory) {
            Ok(entries) => {
                for entry in entries {
                    if let Ok(entry) = entry {
                        if let Ok(metadata) = entry.metadata() {
                            if metadata.is_file() {
                                file_count += 1;
                                total_bytes += metadata.len();
                            }
                        }
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Failed to read directory {:?}: {}",
                    self.config.base_directory, e
                );
            }
        }

        Ok((file_count, total_bytes))
    }
}

/// Start a simple HTTP cleanup server for file deletion
pub async fn start_file_cleanup_server(
    port: u16,
    base_directory: PathBuf,
    cost_tracker: StorageCostTracker,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use warp::Filter;

    let base_dir = Arc::new(base_directory);
    let cost_tracker = Arc::new(cost_tracker);

    let cleanup = warp::path("cleanup")
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::any().map({
            let cost_tracker = cost_tracker.clone();
            move || cost_tracker.clone()
        }))
        .and(warp::any().map({
            let base_dir = base_dir.clone();
            move || base_dir.clone()
        }))
        .and_then(handle_cleanup_request);

    let health = warp::path("health").and(warp::get()).map(|| {
        warp::reply::json(&serde_json::json!({
            "status": "ok",
            "service": "file-cleanup-server"
        }))
    });

    let routes = cleanup.or(health);

    info!("🧹 Starting file cleanup server on port {}", port);

    warp::serve(routes).run(([127, 0, 0, 1], port)).await;

    Ok(())
}

async fn handle_cleanup_request(
    request: serde_json::Value,
    cost_tracker: Arc<StorageCostTracker>,
    base_dir: Arc<PathBuf>,
) -> std::result::Result<impl warp::Reply, warp::Rejection> {
    use std::fs;

    debug!("🧹 Cleanup request: {}", request);

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
            String::new()
        }
    } else {
        String::new()
    };

    if file_path_str.is_empty() {
        return Ok(warp::reply::json(&serde_json::json!({
            "success": false,
            "error": "No file_path provided"
        })));
    }

    let file_path = Path::new(&file_path_str);

    // FIXED: Security check that works with relative paths like "./temp_experiment/with_gc/job_1.dat"
    let is_safe = if file_path.is_absolute() {
        file_path.starts_with(base_dir.as_ref())
    } else {
        // For relative paths, check if they don't escape and if they exist
        if file_path_str.contains("..") {
            false
        } else {
            // Handle paths that start with "./"
            let clean_path = if file_path_str.starts_with("./") {
                Path::new(&file_path_str[2..])
            } else {
                file_path
            };

            // Check if file exists (either directly or relative to current dir)
            file_path.exists() || {
                let current_dir_path = std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from("."))
                    .join(clean_path);
                current_dir_path.exists()
            }
        }
    };

    if is_safe
        && (file_path.exists() || {
            // Also try the path relative to current directory for "./..." paths
            let current_dir_path = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(file_path);
            current_dir_path.exists()
        })
    {
        // Determine the actual file path to use for deletion
        let actual_file_path = if file_path.exists() {
            file_path.to_path_buf()
        } else {
            // Try relative to current directory
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(file_path)
        };

        match fs::metadata(&actual_file_path) {
            Ok(metadata) => {
                let file_size = metadata.len();
                match fs::remove_file(&actual_file_path) {
                    Ok(_) => {
                        cost_tracker.file_cleaned(file_size);
                        info!(
                            "✅ Cleaned up file: {:?} ({} bytes)",
                            actual_file_path, file_size
                        );

                        Ok(warp::reply::json(&serde_json::json!({
                            "success": true,
                            "message": "File cleaned up successfully",
                            "file_path": file_path_str,
                            "actual_path": format!("{:?}", actual_file_path),
                            "file_size_bytes": file_size
                        })))
                    }
                    Err(e) => {
                        error!("Failed to delete file {:?}: {}", actual_file_path, e);
                        Ok(warp::reply::json(&serde_json::json!({
                            "success": false,
                            "error": format!("Failed to delete file: {}", e)
                        })))
                    }
                }
            }
            Err(e) => {
                warn!("Could not get file metadata {:?}: {}", actual_file_path, e);
                Ok(warp::reply::json(&serde_json::json!({
                    "success": true,
                    "message": "File already cleaned up or not found"
                })))
            }
        }
    } else {
        warn!(
            "🚫 Invalid file path or security check failed: {:?}",
            file_path
        );
        Ok(warp::reply::json(&serde_json::json!({
            "success": false,
            "error": "Invalid file path or security check failed"
        })))
    }
}
