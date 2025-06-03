// src/simulation/mod.rs - Fixed simulation with working cleanup

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, error, debug};
use uuid::Uuid;

use crate::client::GCClient;

/// File simulation configuration
#[derive(Debug, Clone)]
pub struct FileSimulationConfig {
    pub base_directory: PathBuf,
    pub file_size_bytes: usize,
    pub job_count: u32,
    pub job_duration_seconds: u64,
    pub crash_probability: f32, // 0.0 to 1.0
    pub cleanup_endpoint: Option<String>,
}

/// Tracks storage costs and statistics
#[derive(Debug, Clone)]
pub struct StorageCostTracker {
    total_files_created: Arc<AtomicU64>,
    total_files_cleaned: Arc<AtomicU64>,
    total_bytes_created: Arc<AtomicU64>,
    total_bytes_cleaned: Arc<AtomicU64>,
    orphaned_files: Arc<AtomicU64>,
    orphaned_bytes: Arc<AtomicU64>,
    cost_per_gb_per_month: f64, // e.g., $0.023 for S3 Standard
}

impl StorageCostTracker {
    pub fn new(cost_per_gb_per_month: f64) -> Self {
        Self {
            total_files_created: Arc::new(AtomicU64::new(0)),
            total_files_cleaned: Arc::new(AtomicU64::new(0)),
            total_bytes_created: Arc::new(AtomicU64::new(0)),
            total_bytes_cleaned: Arc::new(AtomicU64::new(0)),
            orphaned_files: Arc::new(AtomicU64::new(0)),
            orphaned_bytes: Arc::new(AtomicU64::new(0)),
            cost_per_gb_per_month,
        }
    }

    pub fn file_created(&self, size_bytes: u64) {
        self.total_files_created.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_created.fetch_add(size_bytes, Ordering::Relaxed);
    }

    pub fn file_cleaned(&self, size_bytes: u64) {
        self.total_files_cleaned.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_cleaned.fetch_add(size_bytes, Ordering::Relaxed);
    }

    pub fn file_orphaned(&self, size_bytes: u64) {
        self.orphaned_files.fetch_add(1, Ordering::Relaxed);
        self.orphaned_bytes.fetch_add(size_bytes, Ordering::Relaxed);
    }

    pub fn get_stats(&self) -> StorageStats {
        StorageStats {
            total_files_created: self.total_files_created.load(Ordering::Relaxed),
            total_files_cleaned: self.total_files_cleaned.load(Ordering::Relaxed),
            total_bytes_created: self.total_bytes_created.load(Ordering::Relaxed),
            total_bytes_cleaned: self.total_bytes_cleaned.load(Ordering::Relaxed),
            orphaned_files: self.orphaned_files.load(Ordering::Relaxed),
            orphaned_bytes: self.orphaned_bytes.load(Ordering::Relaxed),
            cost_per_gb_per_month: self.cost_per_gb_per_month,
        }
    }
}

/// Storage statistics snapshot
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
        self.total_bytes_created.saturating_sub(self.total_bytes_cleaned)
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
            return 0.0;
        }
        (self.total_files_cleaned as f64 / self.total_files_created as f64) * 100.0
    }
}

/// Simulates a job that creates temporary files
pub struct JobSimulator {
    config: FileSimulationConfig,
    cost_tracker: StorageCostTracker,
    gc_client: Option<GCClient>,
    active_files: HashMap<String, PathBuf>,
}

impl JobSimulator {
    pub fn new(
        config: FileSimulationConfig, 
        cost_tracker: StorageCostTracker,
        gc_client: Option<GCClient>
    ) -> Self {
        Self {
            config,
            cost_tracker,
            gc_client,
            active_files: HashMap::new(),
        }
    }

    /// Run the experiment without GarbageTruck (files get orphaned)
    pub async fn run_without_garbagetruck(&mut self) -> Result<StorageStats, Box<dyn std::error::Error + Send + Sync>> {
        info!("🚫 Running experiment WITHOUT GarbageTruck");
        info!("Files will be orphaned when jobs crash");

        // Ensure base directory exists
        fs::create_dir_all(&self.config.base_directory)?;

        for job_id in 0..self.config.job_count {
            self.simulate_job_without_gc(job_id).await?;
            
            // Small delay between jobs
            sleep(Duration::from_millis(100)).await;
        }

        // Wait a bit for any pending operations
        sleep(Duration::from_secs(2)).await;

        Ok(self.cost_tracker.get_stats())
    }

    /// Run the experiment with GarbageTruck (files get cleaned up)
    pub async fn run_with_garbagetruck(&mut self) -> Result<StorageStats, Box<dyn std::error::Error + Send + Sync>> {
        info!("✅ Running experiment WITH GarbageTruck");
        info!("Files will be automatically cleaned up when leases expire");

        if self.gc_client.is_none() {
            return Err("GarbageTruck client not configured".into());
        }

        // Ensure base directory exists
        fs::create_dir_all(&self.config.base_directory)?;

        for job_id in 0..self.config.job_count {
            self.simulate_job_with_gc(job_id).await?;
            
            // Small delay between jobs
            sleep(Duration::from_millis(50)).await;
        }

        // CRITICAL: Wait much longer for lease expirations and cleanup
        info!("⏳ Waiting for lease expirations and cleanup to complete...");
        info!("   This may take 30-60 seconds for all cleanups to process");
        
        // Wait in chunks and report progress
        for i in 1..=6 {
            sleep(Duration::from_secs(10)).await;
            let (remaining_files, remaining_bytes) = self.count_remaining_files()?;
            info!("   Progress check {}/6: {} files remaining ({:.1} MB)", 
                  i, remaining_files, remaining_bytes as f64 / (1024.0 * 1024.0));
        }

        Ok(self.cost_tracker.get_stats())
    }

    async fn simulate_job_without_gc(&mut self, job_id: u32) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let file_path = self.create_temporary_file(job_id)?;
        
        // Simulate job work
        let should_crash = fastrand::f32() < self.config.crash_probability;
        
        if should_crash {
            warn!("💥 Job {} crashed! File will be orphaned: {:?}", job_id, file_path);
            // File is orphaned - track it
            self.cost_tracker.file_orphaned(self.config.file_size_bytes as u64);
        } else {
            // Job completed successfully - clean up file
            debug!("✅ Job {} completed successfully", job_id);
            self.cleanup_file(&file_path)?;
            self.cost_tracker.file_cleaned(self.config.file_size_bytes as u64);
        }

        Ok(())
    }

    async fn simulate_job_with_gc(&mut self, job_id: u32) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let file_path = self.create_temporary_file(job_id)?;
        let gc_client = self.gc_client.as_mut().unwrap();
        
        // FIXED: Create lease for the file with very short duration for fast cleanup
        let lease_id = gc_client.create_temp_file_lease(
            file_path.to_string_lossy().to_string(),
            10, // 10 seconds - short enough to see results quickly
            self.config.cleanup_endpoint.clone()
        ).await?;

        debug!("📋 Created lease {} for file: {:?}", lease_id, file_path);

        // Simulate job work
        let should_crash = fastrand::f32() < self.config.crash_probability;
        
        if should_crash {
            warn!("💥 Job {} crashed! But lease will handle cleanup: {:?}", job_id, file_path);
            // Don't clean up manually - let the lease expire and handle it
            // File should be cleaned up automatically by GarbageTruck
        } else {
            // Job completed successfully - release lease early and clean up manually
            debug!("✅ Job {} completed successfully, releasing lease", job_id);
            if let Err(e) = gc_client.release_lease(lease_id).await {
                warn!("Failed to release lease: {}", e);
            }
            // Clean up file manually since job succeeded
            self.cleanup_file(&file_path)?;
            self.cost_tracker.file_cleaned(self.config.file_size_bytes as u64);
        }

        Ok(())
    }

    fn create_temporary_file(&self, job_id: u32) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
        let filename = format!("temp_job_{}_{}.dat", job_id, Uuid::new_v4());
        let file_path = self.config.base_directory.join(filename);

        // Create file with specified size
        let data = vec![0u8; self.config.file_size_bytes];
        fs::write(&file_path, data)?;

        self.cost_tracker.file_created(self.config.file_size_bytes as u64);
        debug!("📁 Created temporary file: {:?} ({} bytes)", file_path, self.config.file_size_bytes);

        Ok(file_path)
    }

    fn cleanup_file(&self, file_path: &Path) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if file_path.exists() {
            fs::remove_file(file_path)?;
            debug!("🗑️  Cleaned up file: {:?}", file_path);
        }
        Ok(())
    }

    /// Count actual files remaining in the directory and update cost tracker
    pub fn count_remaining_files(&self) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
        let mut file_count = 0;
        let mut total_size = 0;

        if self.config.base_directory.exists() {
            for entry in fs::read_dir(&self.config.base_directory)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    file_count += 1;
                    total_size += entry.metadata()?.len();
                }
            }
        }

        // CRITICAL: Update the cost tracker with actual remaining files
        let stats = self.cost_tracker.get_stats();
        let expected_cleaned = stats.total_files_created - file_count;
        let expected_cleaned_bytes = expected_cleaned * self.config.file_size_bytes as u64;
        
        // Reset and update cleaned counts based on actual file system state
        self.cost_tracker.total_files_cleaned.store(expected_cleaned, Ordering::Relaxed);
        self.cost_tracker.total_bytes_cleaned.store(expected_cleaned_bytes, Ordering::Relaxed);

        Ok((file_count, total_size))
    }
}

/// Enhanced HTTP cleanup handler for file deletion with working integration
pub async fn start_file_cleanup_server(port: u16, base_directory: PathBuf, cost_tracker: StorageCostTracker) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use warp::Filter;
    
    let cost_tracker = Arc::new(cost_tracker);
    let base_dir = Arc::new(base_directory);
    
    let cleanup = warp::path("cleanup")
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::any().map(move || cost_tracker.clone()))
        .and(warp::any().map(move || base_dir.clone()))
        .and_then(handle_cleanup_request);

    let health = warp::path("health")
        .and(warp::get())
        .map(|| warp::reply::json(&serde_json::json!({
            "status": "running",
            "service": "file-cleanup-handler"
        })));

    let routes = cleanup.or(health);

    info!("🧹 Starting file cleanup server on port {}", port);
    warp::serve(routes)
        .run(([127, 0, 0, 1], port))
        .await;

    Ok(())
}

async fn handle_cleanup_request(
    request: serde_json::Value,
    cost_tracker: Arc<StorageCostTracker>,
    base_dir: Arc<PathBuf>,
) -> Result<impl warp::Reply, warp::Rejection> {
    info!("🧹 Received cleanup request: {}", request);
    
    // FIXED: Extract file path from the request payload
    let file_path_str = if let Some(file_path) = request.get("file_path").and_then(|v| v.as_str()) {
        file_path.to_string()
    } else if let Some(payload) = request.get("payload").and_then(|v| v.as_str()) {
        // Try to parse the payload JSON for file_path
        if let Ok(payload_json) = serde_json::from_str::<serde_json::Value>(payload) {
            if let Some(file_path) = payload_json.get("file_path").and_then(|v| v.as_str()) {
                file_path.to_string()
            } else {
                // Fall back to object_id if no file_path
                request.get("object_id").and_then(|v| v.as_str()).unwrap_or("").to_string()
            }
        } else {
            request.get("object_id").and_then(|v| v.as_str()).unwrap_or("").to_string()
        }
    } else {
        // Use object_id as file path
        request.get("object_id").and_then(|v| v.as_str()).unwrap_or("").to_string()
    };

    if file_path_str.is_empty() {
        warn!("🚫 No file path provided in cleanup request");
        return Ok(warp::reply::json(&serde_json::json!({
            "success": false,
            "error": "No file_path or object_id provided"
        })));
    }

    let file_path = Path::new(&file_path_str);
    
    // Security check - ensure file is within our base directory OR use the filename from object_id
    let target_file = if file_path.starts_with(&**base_dir) {
        file_path.to_path_buf()
    } else {
        // If object_id doesn't start with base_dir, treat it as a filename within base_dir
        let filename = file_path.file_name().unwrap_or_else(|| {
            std::ffi::OsStr::new(&file_path_str)
        });
        base_dir.join(filename)
    };

    if target_file.exists() {
        match fs::remove_file(&target_file) {
            Ok(_) => {
                // FIXED: Get actual file size before deletion, or use estimated size
                let file_size = 100 * 1024 * 1024; // 100MB estimated
                cost_tracker.file_cleaned(file_size);
                info!("✅ Successfully cleaned up file: {:?}", target_file);
                
                Ok(warp::reply::json(&serde_json::json!({
                    "success": true,
                    "message": "File cleaned up successfully",
                    "file_path": target_file.to_string_lossy()
                })))
            }
            Err(e) => {
                error!("❌ Failed to clean up file {:?}: {}", target_file, e);
                Ok(warp::reply::json(&serde_json::json!({
                    "success": false,
                    "error": format!("Failed to delete file: {}", e)
                })))
            }
        }
    } else {
        info!("ℹ️  File not found (already deleted?): {:?}", target_file);
        // Don't treat this as an error - file might already be cleaned up
        Ok(warp::reply::json(&serde_json::json!({
            "success": true,
            "message": "File not found (already cleaned up)",
            "file_path": target_file.to_string_lossy()
        })))
    }
}