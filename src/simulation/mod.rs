// src/simulation/mod.rs - Fixed with proper cleanup integration

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::client::GCClient;
use crate::lease::ObjectType;

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

/// Tracks storage costs and statistics with real-time verification
#[derive(Debug, Clone)]
pub struct StorageCostTracker {
    total_files_created: Arc<AtomicU64>,
    total_files_cleaned: Arc<AtomicU64>,
    total_bytes_created: Arc<AtomicU64>,
    total_bytes_cleaned: Arc<AtomicU64>,
    orphaned_files: Arc<AtomicU64>,
    orphaned_bytes: Arc<AtomicU64>,
    cleanup_requests_sent: Arc<AtomicU64>,
    cleanup_requests_successful: Arc<AtomicU64>,
    cost_per_gb_per_month: f64,
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
            cleanup_requests_sent: Arc::new(AtomicU64::new(0)),
            cleanup_requests_successful: Arc::new(AtomicU64::new(0)),
            cost_per_gb_per_month,
        }
    }

    pub fn file_created(&self, size_bytes: u64) {
        self.total_files_created.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_created
            .fetch_add(size_bytes, Ordering::Relaxed);
    }

    pub fn file_cleaned(&self, size_bytes: u64) {
        self.total_files_cleaned.fetch_add(1, Ordering::Relaxed);
        self.total_bytes_cleaned
            .fetch_add(size_bytes, Ordering::Relaxed);
    }

    pub fn file_orphaned(&self, size_bytes: u64) {
        self.orphaned_files.fetch_add(1, Ordering::Relaxed);
        self.orphaned_bytes.fetch_add(size_bytes, Ordering::Relaxed);
    }

    pub fn cleanup_request_sent(&self) {
        self.cleanup_requests_sent.fetch_add(1, Ordering::Relaxed);
    }

    pub fn cleanup_request_successful(&self, size_bytes: u64) {
        self.cleanup_requests_successful
            .fetch_add(1, Ordering::Relaxed);
        self.file_cleaned(size_bytes);
    }

    pub fn get_stats(&self) -> StorageStats {
        StorageStats {
            total_files_created: self.total_files_created.load(Ordering::Relaxed),
            total_files_cleaned: self.total_files_cleaned.load(Ordering::Relaxed),
            total_bytes_created: self.total_bytes_created.load(Ordering::Relaxed),
            total_bytes_cleaned: self.total_bytes_cleaned.load(Ordering::Relaxed),
            orphaned_files: self.orphaned_files.load(Ordering::Relaxed),
            orphaned_bytes: self.orphaned_bytes.load(Ordering::Relaxed),
            cleanup_requests_sent: self.cleanup_requests_sent.load(Ordering::Relaxed),
            cleanup_requests_successful: self.cleanup_requests_successful.load(Ordering::Relaxed),
            cost_per_gb_per_month: self.cost_per_gb_per_month,
        }
    }
}

/// Enhanced storage statistics with cleanup tracking
#[derive(Debug, Clone)]
pub struct StorageStats {
    pub total_files_created: u64,
    pub total_files_cleaned: u64,
    pub total_bytes_created: u64,
    pub total_bytes_cleaned: u64,
    pub orphaned_files: u64,
    pub orphaned_bytes: u64,
    pub cleanup_requests_sent: u64,
    pub cleanup_requests_successful: u64,
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
            return 0.0;
        }
        (self.total_files_cleaned as f64 / self.total_files_created as f64) * 100.0
    }

    pub fn cleanup_success_rate(&self) -> f64 {
        if self.cleanup_requests_sent == 0 {
            return 0.0;
        }
        (self.cleanup_requests_successful as f64 / self.cleanup_requests_sent as f64) * 100.0
    }
}

/// Enhanced job simulator with real cleanup verification
pub struct JobSimulator {
    config: FileSimulationConfig,
    cost_tracker: StorageCostTracker,
    gc_client: Option<GCClient>,
    cleanup_client: Option<reqwest::Client>,
    active_files: HashMap<String, PathBuf>,
}

impl JobSimulator {
    pub fn new(
        config: FileSimulationConfig,
        cost_tracker: StorageCostTracker,
        gc_client: Option<GCClient>,
    ) -> Self {
        let cleanup_client = if gc_client.is_some() {
            Some(
                reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
                    .expect("Failed to create cleanup client"),
            )
        } else {
            None
        };

        Self {
            config,
            cost_tracker,
            gc_client,
            cleanup_client,
            active_files: HashMap::new(),
        }
    }

    /// Run the experiment without GarbageTruck (files get orphaned)
    pub async fn run_without_garbagetruck(
        &mut self,
    ) -> Result<StorageStats, Box<dyn std::error::Error + Send + Sync>> {
        info!("🚫 Running experiment WITHOUT GarbageTruck");
        info!("Files will be orphaned when jobs crash");

        // Ensure base directory exists
        fs::create_dir_all(&self.config.base_directory)?;

        for job_id in 0..self.config.job_count {
            self.simulate_job_without_gc(job_id).await?;

            // Small delay between jobs
            sleep(Duration::from_millis(50)).await;
        }

        // Wait a bit for any pending operations
        sleep(Duration::from_secs(2)).await;

        Ok(self.cost_tracker.get_stats())
    }

    /// Run the experiment with GarbageTruck (files get cleaned up)
    pub async fn run_with_garbagetruck(
        &mut self,
    ) -> Result<StorageStats, Box<dyn std::error::Error + Send + Sync>> {
        info!("✅ Running experiment WITH GarbageTruck");
        info!("Files will be automatically cleaned up when leases expire");

        if self.gc_client.is_none() {
            return Err("GarbageTruck client not configured".into());
        }

        // Verify cleanup server is accessible
        if let Some(ref endpoint) = self.config.cleanup_endpoint {
            self.verify_cleanup_server(endpoint).await?;
        }

        // Ensure base directory exists
        fs::create_dir_all(&self.config.base_directory)?;

        for job_id in 0..self.config.job_count {
            self.simulate_job_with_gc(job_id).await?;

            // Small delay between jobs
            sleep(Duration::from_millis(50)).await;
        }

        // Wait longer for lease expirations and cleanup
        info!("Waiting for lease expirations and cleanup...");
        sleep(Duration::from_secs(15)).await; // Increased wait time

        // After waiting, check what files remain and count them as orphaned if they weren't cleaned up
        self.reconcile_remaining_files().await?;

        // Verify cleanup happened
        self.verify_cleanup_effectiveness().await?;

        Ok(self.cost_tracker.get_stats())
    }

    async fn verify_cleanup_server(
        &self,
        endpoint: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let Some(ref client) = self.cleanup_client {
            let health_url = endpoint.replace("/cleanup", "/health");
            match client.get(&health_url).send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        info!("✅ Cleanup server is accessible at {}", health_url);
                        Ok(())
                    } else {
                        Err(format!("Cleanup server returned status: {}", response.status()).into())
                    }
                }
                Err(e) => {
                    Err(format!("Cannot reach cleanup server at {}: {}", health_url, e).into())
                }
            }
        } else {
            Err("No cleanup client configured".into())
        }
    }

    async fn reconcile_remaining_files(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        info!("🔍 Reconciling remaining files after cleanup period...");

        let (remaining_files, remaining_bytes) = self.count_remaining_files()?;

        if remaining_files > 0 {
            warn!(
                "Found {} orphaned files ({} bytes) that weren't cleaned up",
                remaining_files, remaining_bytes
            );

            // Count these as orphaned since GarbageTruck didn't clean them up
            self.cost_tracker.file_orphaned(remaining_bytes);

            // For more detailed analysis, count individual files
            if remaining_files <= 50 {
                // Only for reasonable numbers
                if let Ok(entries) = fs::read_dir(&self.config.base_directory) {
                    for entry in entries.flatten() {
                        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                            debug!("Orphaned file: {:?} ({} bytes)", entry.path(), size);
                        }
                    }
                }
            }
        } else {
            info!("✅ All files were successfully cleaned up!");
        }

        Ok(())
    }

    async fn verify_cleanup_effectiveness(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let stats = self.cost_tracker.get_stats();

        info!("🔍 Cleanup Verification:");
        info!("  Cleanup requests sent: {}", stats.cleanup_requests_sent);
        info!(
            "  Cleanup requests successful: {}",
            stats.cleanup_requests_successful
        );
        info!(
            "  Cleanup success rate: {:.1}%",
            stats.cleanup_success_rate()
        );

        if stats.cleanup_requests_sent > 0 && stats.cleanup_success_rate() < 50.0 {
            warn!(
                "⚠️  Low cleanup success rate: {:.1}%",
                stats.cleanup_success_rate()
            );
        }

        Ok(())
    }

    async fn simulate_job_without_gc(
        &mut self,
        job_id: u32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let file_path = self.create_temporary_file(job_id)?;

        // Simulate job work
        let should_crash = fastrand::f32() < self.config.crash_probability;

        if should_crash {
            warn!(
                "💥 Job {} crashed! File will be orphaned: {:?}",
                job_id, file_path
            );
            // File is orphaned - track it
            self.cost_tracker
                .file_orphaned(self.config.file_size_bytes as u64);
        } else {
            // Job completed successfully - clean up file
            debug!("✅ Job {} completed successfully", job_id);
            self.cleanup_file(&file_path)?;
            self.cost_tracker
                .file_cleaned(self.config.file_size_bytes as u64);
        }

        Ok(())
    }

    async fn simulate_job_with_gc(
        &mut self,
        job_id: u32,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let file_path = self.create_temporary_file(job_id)?;
        let gc_client = self.gc_client.as_mut().unwrap();

        // Create lease with longer duration (30 seconds for proper testing)
        let lease_id = gc_client
            .create_temp_file_lease(
                file_path.to_string_lossy().to_string(),
                30, // 30 seconds instead of 5
                self.config.cleanup_endpoint.clone(),
            )
            .await?;

        debug!("📋 Created lease {} for file: {:?}", lease_id, file_path);

        // Simulate job work
        let should_crash = fastrand::f32() < self.config.crash_probability;

        if should_crash {
            warn!(
                "💥 Job {} crashed! Lease will handle cleanup: {:?}",
                job_id, file_path
            );
            // Don't clean up manually - let the lease expire and handle it
            // Mark this file as potentially cleaned by GarbageTruck (will be verified later)
            // Don't increment orphaned count yet - let the cleanup system handle it
        } else {
            // Job completed successfully - release lease early and clean up manually
            debug!("✅ Job {} completed successfully, releasing lease", job_id);
            if let Err(e) = gc_client.release_lease(lease_id).await {
                warn!("Failed to release lease: {}", e);
            }
            // Also clean up file manually since job succeeded
            self.cleanup_file(&file_path)?;
            self.cost_tracker
                .file_cleaned(self.config.file_size_bytes as u64);
        }

        Ok(())
    }

    fn create_temporary_file(
        &self,
        job_id: u32,
    ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
        let filename = format!("temp_job_{}_{}.dat", job_id, Uuid::new_v4());
        let file_path = self.config.base_directory.join(filename);

        // Create file with specified size
        let data = vec![0u8; self.config.file_size_bytes];
        fs::write(&file_path, data)?;

        self.cost_tracker
            .file_created(self.config.file_size_bytes as u64);
        debug!(
            "📁 Created temporary file: {:?} ({} bytes)",
            file_path, self.config.file_size_bytes
        );

        Ok(file_path)
    }

    fn cleanup_file(
        &self,
        file_path: &Path,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if file_path.exists() {
            fs::remove_file(file_path)?;
            debug!("🗑️  Cleaned up file: {:?}", file_path);
        }
        Ok(())
    }

    /// Count actual files remaining in the directory with detailed breakdown
    pub fn count_remaining_files(
        &self,
    ) -> Result<(u64, u64), Box<dyn std::error::Error + Send + Sync>> {
        let mut file_count = 0;
        let mut total_size = 0;

        if self.config.base_directory.exists() {
            for entry in fs::read_dir(&self.config.base_directory)? {
                let entry = entry?;
                if entry.file_type()?.is_file() {
                    file_count += 1;
                    total_size += entry.metadata()?.len();

                    // Log some sample files for debugging
                    if file_count <= 5 {
                        debug!(
                            "Remaining file: {:?} ({} bytes)",
                            entry.path(),
                            entry.metadata()?.len()
                        );
                    }
                }
            }
        }

        info!(
            "📊 Directory scan: {} files, {} bytes total",
            file_count, total_size
        );
        Ok((file_count, total_size))
    }
}

/// Enhanced HTTP cleanup handler with proper file deletion
pub async fn start_file_cleanup_server(
    port: u16,
    base_directory: PathBuf,
    cost_tracker: StorageCostTracker,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use warp::Filter;

    let cost_tracker = Arc::new(cost_tracker);
    let base_dir = Arc::new(base_directory);

    // Create base directory if it doesn't exist
    if !base_dir.exists() {
        fs::create_dir_all(&**base_dir)?;
    }

    info!("🧹 Starting enhanced file cleanup server on port {}", port);
    info!("   Base directory: {:?}", base_dir);

    // Clone the Arc references for each route
    let cost_tracker_cleanup = cost_tracker.clone();
    let base_dir_cleanup = base_dir.clone();
    let cost_tracker_health = cost_tracker.clone();

    let cleanup = warp::path("cleanup")
        .and(warp::post())
        .and(warp::body::json())
        .and(warp::any().map(move || cost_tracker_cleanup.clone()))
        .and(warp::any().map(move || base_dir_cleanup.clone()))
        .and_then(handle_cleanup_request);

    let health = warp::path("health")
        .and(warp::get())
        .and(warp::any().map(move || cost_tracker_health.clone()))
        .and_then(handle_health_request);

    let routes = cleanup.or(health);

    warp::serve(routes).run(([127, 0, 0, 1], port)).await;

    Ok(())
}

async fn handle_cleanup_request(
    request: serde_json::Value,
    cost_tracker: Arc<StorageCostTracker>,
    base_dir: Arc<PathBuf>,
) -> Result<impl warp::Reply, warp::Rejection> {
    cost_tracker.cleanup_request_sent();

    info!("🧹 Received cleanup request: {}", request);

    // Extract file path from the request (handle both formats)
    let file_path_str = request
        .get("file_path")
        .and_then(|v| v.as_str())
        .or_else(|| request.get("object_id").and_then(|v| v.as_str()));

    if let Some(file_path_str) = file_path_str {
        let file_path = Path::new(file_path_str);

        // Try absolute path first, then relative to base directory
        let target_path = if file_path.is_absolute() {
            file_path.to_path_buf()
        } else {
            base_dir.join(file_path)
        };

        debug!("🎯 Attempting to delete file: {:?}", target_path);

        if target_path.exists() {
            match fs::metadata(&target_path) {
                Ok(metadata) => {
                    let file_size = metadata.len();
                    match fs::remove_file(&target_path) {
                        Ok(_) => {
                            cost_tracker.cleanup_request_successful(file_size);
                            info!(
                                "✅ Successfully cleaned up file: {:?} ({} bytes)",
                                target_path, file_size
                            );

                            Ok(warp::reply::json(&serde_json::json!({
                                "success": true,
                                "message": "File cleaned up successfully",
                                "file_path": file_path_str,
                                "file_size": file_size,
                                "timestamp": chrono::Utc::now().to_rfc3339()
                            })))
                        }
                        Err(e) => {
                            error!("❌ Failed to clean up file {:?}: {}", target_path, e);
                            Ok(warp::reply::json(&serde_json::json!({
                                "success": false,
                                "error": format!("Failed to delete file: {}", e),
                                "file_path": file_path_str
                            })))
                        }
                    }
                }
                Err(e) => {
                    error!("❌ Failed to get metadata for {:?}: {}", target_path, e);
                    Ok(warp::reply::json(&serde_json::json!({
                        "success": false,
                        "error": format!("Failed to access file: {}", e),
                        "file_path": file_path_str
                    })))
                }
            }
        } else {
            warn!("🚫 File not found: {:?}", target_path);
            // Consider this a success since the file is already gone
            Ok(warp::reply::json(&serde_json::json!({
                "success": true,
                "message": "File already deleted or not found",
                "file_path": file_path_str
            })))
        }
    } else {
        warn!("🚫 No file_path provided in cleanup request");
        Ok(warp::reply::json(&serde_json::json!({
            "success": false,
            "error": "No file_path provided"
        })))
    }
}

async fn handle_health_request(
    cost_tracker: Arc<StorageCostTracker>,
) -> Result<impl warp::Reply, warp::Rejection> {
    let stats = cost_tracker.get_stats();

    Ok(warp::reply::json(&serde_json::json!({
        "status": "running",
        "service": "enhanced-file-cleanup-handler",
        "cleanup_requests_received": stats.cleanup_requests_sent,
        "cleanup_requests_successful": stats.cleanup_requests_successful,
        "cleanup_success_rate": stats.cleanup_success_rate(),
        "timestamp": chrono::Utc::now().to_rfc3339()
    })))
}
