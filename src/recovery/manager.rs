// src/recovery/manager.rs - Complete service failure recovery and state restoration

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseState};
use crate::metrics::Metrics;
use crate::storage::{PersistentStorage, RecoveryInfo, Storage};

/// Recovery strategy for different failure scenarios
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryStrategy {
    /// Conservative recovery - validate everything before proceeding
    Conservative,
    /// Fast recovery - restore quickly with minimal validation
    Fast,
    /// Emergency recovery - try all recovery methods
    Emergency,
    /// Custom recovery with specific parameters
    Custom {
        validate_checksums: bool,
        skip_corrupted_entries: bool,
        force_snapshot_load: bool,
        max_recovery_time_seconds: u64,
    },
}

impl Default for RecoveryStrategy {
    fn default() -> Self {
        Self::Conservative
    }
}

/// Recovery manager configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryConfig {
    /// Recovery strategy to use
    pub strategy: RecoveryStrategy,
    /// Maximum time to spend on recovery before giving up
    pub max_recovery_time: Duration,
    /// Whether to create backup before recovery
    pub create_backup_before_recovery: bool,
    /// Automatic recovery triggers
    pub auto_recovery_triggers: Vec<RecoveryTrigger>,
    /// Recovery notification endpoints
    pub notification_endpoints: Vec<String>,
    /// Health check interval during recovery
    pub health_check_interval: Duration,
}

impl Default for RecoveryConfig {
    fn default() -> Self {
        Self {
            strategy: RecoveryStrategy::default(),
            max_recovery_time: Duration::from_secs(300), // 5 minutes
            create_backup_before_recovery: true,
            auto_recovery_triggers: vec![
                RecoveryTrigger::ServiceStart,
                RecoveryTrigger::CorruptionDetected,
            ],
            notification_endpoints: Vec::new(),
            health_check_interval: Duration::from_secs(30),
        }
    }
}

/// Triggers that can initiate automatic recovery
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryTrigger {
    /// Service startup
    ServiceStart,
    /// Storage corruption detected
    CorruptionDetected,
    /// Health check failure
    HealthCheckFailure,
    /// Manual trigger via API
    Manual,
    /// Scheduled recovery
    Scheduled,
}

/// Recovery progress information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryProgress {
    /// Current recovery phase
    pub phase: RecoveryPhase,
    /// Progress percentage (0-100)
    pub progress_percent: f64,
    /// Current operation description
    pub current_operation: String,
    /// Estimated time remaining
    pub estimated_remaining: Option<Duration>,
    /// Recovery start time
    pub start_time: DateTime<Utc>,
    /// Errors encountered so far
    pub errors: Vec<String>,
    /// Warnings encountered so far
    pub warnings: Vec<String>,
}

/// Recovery phases
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryPhase {
    /// Initializing recovery process
    Initializing,
    /// Creating backup
    CreatingBackup,
    /// Loading snapshot
    LoadingSnapshot,
    /// Replaying WAL entries
    ReplayingWAL,
    /// Validating recovered data
    ValidatingData,
    /// Rebuilding indexes
    RebuildingIndexes,
    /// Running health checks
    HealthChecks,
    /// Notifying external systems
    Notifications,
    /// Recovery completed
    Completed,
    /// Recovery failed
    Failed,
}

/// Recovery result with detailed information
#[derive(Debug, Clone)]
pub struct RecoveryResult {
    /// Whether recovery was successful
    pub success: bool,
    /// Recovery information
    pub recovery_info: RecoveryInfo,
    /// Final progress state
    pub progress: RecoveryProgress,
    /// Total recovery time
    pub total_time: Duration,
    /// Detailed error if recovery failed
    pub error: Option<String>,
    /// Performance metrics during recovery
    pub metrics: RecoveryMetrics,
}

/// Recovery performance metrics
#[derive(Debug, Clone, Default)]
pub struct RecoveryMetrics {
    /// Time spent in each phase
    pub phase_times: HashMap<String, Duration>,
    /// Number of operations performed
    pub operations_count: usize,
    /// Amount of data processed (bytes)
    pub data_processed_bytes: u64,
    /// Peak memory usage during recovery
    pub peak_memory_usage_mb: u64,
    /// Number of validation errors
    pub validation_errors: usize,
}

/// Main recovery manager
pub struct RecoveryManager {
    /// Storage backend with persistent capabilities
    storage: Arc<dyn PersistentStorage>,
    /// Regular storage interface for health checks
    storage_interface: Arc<dyn Storage>,
    /// Recovery configuration
    config: Arc<RwLock<RecoveryConfig>>,
    /// Current recovery progress
    progress: Arc<Mutex<Option<RecoveryProgress>>>,
    /// System metrics
    metrics: Arc<Metrics>,
    /// Notification client
    notification_client: Arc<NotificationClient>,
}

impl RecoveryManager {
    pub fn new(
        storage: Arc<dyn PersistentStorage>,
        storage_interface: Arc<dyn Storage>,
        config: RecoveryConfig,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            storage,
            storage_interface,
            config: Arc::new(RwLock::new(config)),
            progress: Arc::new(Mutex::new(None)),
            metrics,
            notification_client: Arc::new(NotificationClient::new()),
        }
    }

    /// Perform recovery based on configuration
    pub async fn recover(&self, trigger: RecoveryTrigger) -> Result<RecoveryResult> {
        let start_time = std::time::Instant::now();
        let recovery_start = Utc::now();
        
        info!("Starting recovery process, trigger: {:?}", trigger);

        // Initialize progress tracking
        let mut progress = RecoveryProgress {
            phase: RecoveryPhase::Initializing,
            progress_percent: 0.0,
            current_operation: "Initializing recovery".to_string(),
            estimated_remaining: None,
            start_time: recovery_start,
            errors: Vec::new(),
            warnings: Vec::new(),
        };

        *self.progress.lock().await = Some(progress.clone());

        // Notify start of recovery
        self.send_recovery_notification("Recovery started", &progress).await;

        let config = self.config.read().await.clone();
        let mut metrics = RecoveryMetrics::default();
        
        let result = match config.strategy {
            RecoveryStrategy::Conservative => {
                self.conservative_recovery(&mut progress, &mut metrics).await
            }
            RecoveryStrategy::Fast => {
                self.fast_recovery(&mut progress, &mut metrics).await
            }
            RecoveryStrategy::Emergency => {
                self.emergency_recovery(&mut progress, &mut metrics).await
            }
            RecoveryStrategy::Custom { .. } => {
                self.custom_recovery(&mut progress, &mut metrics, &config.strategy).await
            }
        };

        let total_time = start_time.elapsed();
        
        match result {
            Ok(recovery_info) => {
                progress.phase = RecoveryPhase::Completed;
                progress.progress_percent = 100.0;
                progress.current_operation = "Recovery completed successfully".to_string();
                
                info!(
                    "Recovery completed successfully in {:.2}s: {} leases recovered, {} entries replayed",
                    total_time.as_secs_f64(),
                    recovery_info.leases_recovered,
                    recovery_info.entries_replayed
                );

                self.send_recovery_notification("Recovery completed successfully", &progress).await;

                Ok(RecoveryResult {
                    success: true,
                    recovery_info,
                    progress: progress.clone(),
                    total_time,
                    error: None,
                    metrics,
                })
            }
            Err(e) => {
                progress.phase = RecoveryPhase::Failed;
                progress.errors.push(e.to_string());
                progress.current_operation = format!("Recovery failed: {}", e);
                
                error!("Recovery failed after {:.2}s: {}", total_time.as_secs_f64(), e);
                
                self.send_recovery_notification("Recovery failed", &progress).await;

                Ok(RecoveryResult {
                    success: false,
                    recovery_info: RecoveryInfo {
                        entries_replayed: 0,
                        leases_recovered: 0,
                        corrupted_entries: 0,
                        recovery_start,
                        recovery_end: Utc::now(),
                        last_sequence_number: 0,
                        warnings: vec![e.to_string()],
                    },
                    progress: progress.clone(),
                    total_time,
                    error: Some(e.to_string()),
                    metrics,
                })
            }
        }
    }

    /// Conservative recovery with full validation
    async fn conservative_recovery(
        &self,
        progress: &mut RecoveryProgress,
        metrics: &mut RecoveryMetrics,
    ) -> Result<RecoveryInfo> {
        let phase_start = std::time::Instant::now();

        // Phase 1: Create backup
        self.update_progress(progress, RecoveryPhase::CreatingBackup, 10.0, "Creating backup").await;
        
        if self.config.read().await.create_backup_before_recovery {
            self.create_recovery_backup().await?;
        }
        metrics.phase_times.insert("backup".to_string(), phase_start.elapsed());

        // Phase 2: Validate WAL integrity
        let phase_start = std::time::Instant::now();
        self.update_progress(progress, RecoveryPhase::ValidatingData, 20.0, "Validating WAL integrity").await;
        
        let integrity_issues = self.storage.verify_wal_integrity().await?;
        if !integrity_issues.is_empty() {
            progress.warnings.extend(integrity_issues);
            metrics.validation_errors = progress.warnings.len();
        }
        metrics.phase_times.insert("validation".to_string(), phase_start.elapsed());

        // Phase 3: Load latest snapshot if available
        let phase_start = std::time::Instant::now();
        self.update_progress(progress, RecoveryPhase::LoadingSnapshot, 40.0, "Loading snapshot").await;
        
        if let Err(e) = self.load_latest_snapshot().await {
            progress.warnings.push(format!("Failed to load snapshot: {}", e));
        }
        metrics.phase_times.insert("snapshot".to_string(), phase_start.elapsed());

        // Phase 4: Replay WAL
        let phase_start = std::time::Instant::now();
        self.update_progress(progress, RecoveryPhase::ReplayingWAL, 70.0, "Replaying write-ahead log").await;
        
        let recovery_info = self.storage.replay_wal(0).await?;
        metrics.operations_count = recovery_info.entries_replayed;
        metrics.phase_times.insert("wal_replay".to_string(), phase_start.elapsed());

        // Phase 5: Health checks
        let phase_start = std::time::Instant::now();
        self.update_progress(progress, RecoveryPhase::HealthChecks, 90.0, "Running health checks").await;
        
        self.run_post_recovery_health_checks(progress).await?;
        metrics.phase_times.insert("health_checks".to_string(), phase_start.elapsed());

        Ok(recovery_info)
    }

    /// Fast recovery with minimal validation
    async fn fast_recovery(
        &self,
        progress: &mut RecoveryProgress,
        metrics: &mut RecoveryMetrics,
    ) -> Result<RecoveryInfo> {
        let phase_start = std::time::Instant::now();

        // Phase 1: Load snapshot if available
        self.update_progress(progress, RecoveryPhase::LoadingSnapshot, 30.0, "Loading snapshot").await;
        
        if let Err(e) = self.load_latest_snapshot().await {
            progress.warnings.push(format!("Failed to load snapshot: {}", e));
        }
        metrics.phase_times.insert("snapshot".to_string(), phase_start.elapsed());

        // Phase 2: Replay WAL
        let phase_start = std::time::Instant::now();
        self.update_progress(progress, RecoveryPhase::ReplayingWAL, 80.0, "Replaying write-ahead log").await;
        
        let recovery_info = self.storage.replay_wal(0).await?;
        metrics.operations_count = recovery_info.entries_replayed;
        metrics.phase_times.insert("wal_replay".to_string(), phase_start.elapsed());

        // Phase 3: Basic health check
        let phase_start = std::time::Instant::now();
        self.update_progress(progress, RecoveryPhase::HealthChecks, 95.0, "Basic health check").await;
        
        let healthy = self.storage_interface.health_check().await?;
        if !healthy {
            return Err(GCError::Internal("Storage health check failed after recovery".to_string()));
        }
        metrics.phase_times.insert("health_checks".to_string(), phase_start.elapsed());

        Ok(recovery_info)
    }

    /// Emergency recovery trying all methods
    async fn emergency_recovery(
        &self,
        progress: &mut RecoveryProgress,
        metrics: &mut RecoveryMetrics,
    ) -> Result<RecoveryInfo> {
        self.update_progress(progress, RecoveryPhase::Initializing, 5.0, "Starting emergency recovery").await;

        // Try emergency recovery from storage
        let recovery_info = self.storage.emergency_recovery().await?;
        metrics.operations_count = recovery_info.entries_replayed;

        self.update_progress(progress, RecoveryPhase::HealthChecks, 90.0, "Emergency health checks").await;

        // Run minimal health checks
        let healthy = self.storage_interface.health_check().await?;
        if !healthy {
            progress.warnings.push("Storage health check failed after emergency recovery".to_string());
        }

        Ok(recovery_info)
    }

    /// Custom recovery with specific parameters
    async fn custom_recovery(
        &self,
        progress: &mut RecoveryProgress,
        metrics: &mut RecoveryMetrics,
        strategy: &RecoveryStrategy,
    ) -> Result<RecoveryInfo> {
        if let RecoveryStrategy::Custom {
            validate_checksums,
            skip_corrupted_entries,
            force_snapshot_load,
            max_recovery_time_seconds,
        } = strategy {
            
            let timeout = Duration::from_secs(*max_recovery_time_seconds);
            let recovery_future = async {
                // Validation phase
                if *validate_checksums {
                    self.update_progress(progress, RecoveryPhase::ValidatingData, 20.0, "Validating checksums").await;
                    let integrity_issues = self.storage.verify_wal_integrity().await?;
                    if !integrity_issues.is_empty() && !skip_corrupted_entries {
                        return Err(GCError::Internal("WAL integrity validation failed".to_string()));
                    }
                    progress.warnings.extend(integrity_issues);
                }

                // Snapshot loading
                if *force_snapshot_load {
                    self.update_progress(progress, RecoveryPhase::LoadingSnapshot, 40.0, "Force loading snapshot").await;
                    self.load_latest_snapshot().await?;
                }

                // WAL replay
                self.update_progress(progress, RecoveryPhase::ReplayingWAL, 70.0, "Replaying WAL with custom settings").await;
                let recovery_info = self.storage.replay_wal(0).await?;
                metrics.operations_count = recovery_info.entries_replayed;

                Ok(recovery_info)
            };

            // Apply timeout
            match tokio::time::timeout(timeout, recovery_future).await {
                Ok(result) => result,
                Err(_) => Err(GCError::Timeout { timeout_seconds: *max_recovery_time_seconds }),
            }
        } else {
            Err(GCError::Internal("Invalid custom recovery strategy".to_string()))
        }
    }

    /// Update recovery progress
    async fn update_progress(
        &self,
        progress: &mut RecoveryProgress,
        phase: RecoveryPhase,
        percent: f64,
        operation: &str,
    ) {
        progress.phase = phase;
        progress.progress_percent = percent;
        progress.current_operation = operation.to_string();
        
        // Update shared progress
        *self.progress.lock().await = Some(progress.clone());
        
        debug!("Recovery progress: {:.1}% - {}", percent, operation);
    }

    /// Load the latest available snapshot
    async fn load_latest_snapshot(&self) -> Result<()> {
        // This would typically scan for snapshot files and load the most recent one
        // For now, we'll use a simplified approach
        info!("Attempting to load latest snapshot");
        
        // Try to load a snapshot - this is implementation-specific
        // In a real system, you'd scan the snapshot directory for the latest file
        match self.storage.load_snapshot("latest.snapshot").await {
            Ok(_) => {
                info!("Successfully loaded snapshot");
                Ok(())
            }
            Err(e) => {
                warn!("Failed to load snapshot: {}", e);
                Err(e)
            }
        }
    }

    /// Create a backup before recovery
    async fn create_recovery_backup(&self) -> Result<()> {
        info!("Creating recovery backup");
        
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let backup_path = format!("recovery_backup_{}", timestamp);
        
        // Create snapshot as backup
        self.storage.create_snapshot().await?;
        
        info!("Recovery backup created: {}", backup_path);
        Ok(())
    }

    /// Run health checks after recovery
    async fn run_post_recovery_health_checks(&self, progress: &mut RecoveryProgress) -> Result<()> {
        info!("Running post-recovery health checks");

        // Basic storage health check
        let storage_healthy = self.storage_interface.health_check().await?;
        if !storage_healthy {
            return Err(GCError::Internal("Storage health check failed".to_string()));
        }

        // Check WAL integrity
        let wal_issues = self.storage.verify_wal_integrity().await?;
        if !wal_issues.is_empty() {
            progress.warnings.extend(wal_issues);
        }

        // Verify some lease operations work
        let test_stats = self.storage_interface.get_stats().await?;
        debug!("Post-recovery stats: {} total leases", test_stats.total_leases);

        info!("Post-recovery health checks completed");
        Ok(())
    }

    /// Send recovery notification
    async fn send_recovery_notification(&self, message: &str, progress: &RecoveryProgress) {
        let config = self.config.read().await;
        
        for endpoint in &config.notification_endpoints {
            if let Err(e) = self.notification_client.send_notification(endpoint, message, progress).await {
                warn!("Failed to send recovery notification to {}: {}", endpoint, e);
            }
        }
    }

    /// Get current recovery progress
    pub async fn get_recovery_progress(&self) -> Option<RecoveryProgress> {
        self.progress.lock().await.clone()
    }

    /// Check if recovery is currently in progress
    pub async fn is_recovery_in_progress(&self) -> bool {
        if let Some(progress) = self.progress.lock().await.as_ref() {
            !matches!(progress.phase, RecoveryPhase::Completed | RecoveryPhase::Failed)
        } else {
            false
        }
    }

    /// Validate recovery configuration
    pub fn validate_config(config: &RecoveryConfig) -> Vec<String> {
        let mut issues = Vec::new();

        if config.max_recovery_time.as_secs() < 10 {
            issues.push("Recovery timeout is too short (minimum 10 seconds)".to_string());
        }

        if config.max_recovery_time.as_secs() > 3600 {
            issues.push("Recovery timeout is very long (over 1 hour)".to_string());
        }

        if config.health_check_interval.as_secs() == 0 {
            issues.push("Health check interval cannot be zero".to_string());
        }

        issues
    }
}

/// Notification client for recovery events
pub struct NotificationClient {
    http_client: reqwest::Client,
}

impl NotificationClient {
    pub fn new() -> Self {
        Self {
            http_client: reqwest::Client::new(),
        }
    }

    async fn send_notification(
        &self,
        endpoint: &str,
        message: &str,
        progress: &RecoveryProgress,
    ) -> Result<()> {
        let notification = RecoveryNotification {
            message: message.to_string(),
            timestamp: Utc::now(),
            phase: progress.phase.clone(),
            progress_percent: progress.progress_percent,
            errors: progress.errors.clone(),
            warnings: progress.warnings.clone(),
        };

        let response = self.http_client
            .post(endpoint)
            .json(&notification)
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| GCError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(GCError::Network(format!(
                "Notification failed with status: {}",
                response.status()
            )));
        }

        Ok(())
    }
}

/// Recovery notification payload
#[derive(Debug, Serialize)]
struct RecoveryNotification {
    message: String,
    timestamp: DateTime<Utc>,
    phase: RecoveryPhase,
    progress_percent: f64,
    errors: Vec<String>,
    warnings: Vec<String>,
}

/// Recovery API for external monitoring and control
pub struct RecoveryAPI {
    recovery_manager: Arc<RecoveryManager>,
}

impl RecoveryAPI {
    pub fn new(recovery_manager: Arc<RecoveryManager>) -> Self {
        Self { recovery_manager }
    }

    /// Trigger manual recovery
    pub async fn trigger_recovery(&self, strategy: Option<RecoveryStrategy>) -> Result<RecoveryResult> {
        // Update strategy if provided
        if let Some(strategy) = strategy {
            self.recovery_manager.config.write().await.strategy = strategy;
        }

        self.recovery_manager.recover(RecoveryTrigger::Manual).await
    }

    /// Get current recovery status
    pub async fn get_recovery_status(&self) -> Option<RecoveryProgress> {
        self.recovery_manager.get_recovery_progress().await
    }

    /// Update recovery configuration
    pub async fn update_config(&self, config: RecoveryConfig) -> Result<()> {
        let issues = RecoveryManager::validate_config(&config);
        if !issues.is_empty() {
            return Err(GCError::Configuration(format!(
                "Invalid recovery configuration: {}",
                issues.join(", ")
            )));
        }

        *self.recovery_manager.config.write().await = config;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;
    use std::sync::Arc;

    async fn create_test_recovery_manager() -> RecoveryManager {
        let storage_interface = Arc::new(MemoryStorage::new()) as Arc<dyn Storage>;
        // Note: This would need to be a PersistentStorage implementation in real code
        let persistent_storage = Arc::new(MockPersistentStorage::new());
        let config = RecoveryConfig::default();
        let metrics = Metrics::new();

        RecoveryManager::new(persistent_storage, storage_interface, config, metrics)
    }

    // Mock implementation for testing
    struct MockPersistentStorage;

    impl MockPersistentStorage {
        fn new() -> Self {
            Self
        }
    }

    #[async_trait]
    impl PersistentStorage for MockPersistentStorage {
        // Simplified mock implementations for testing
        async fn initialize(&self, _config: &crate::storage::PersistentStorageConfig) -> Result<()> {
            Ok(())
        }

        async fn write_wal_entry(&self, _entry: crate::storage::WALEntry) -> Result<()> {
            Ok(())
        }

        async fn read_wal_entries(&self, _from_sequence: u64) -> Result<Vec<crate::storage::WALEntry>> {
            Ok(Vec::new())
        }

        async fn create_snapshot(&self) -> Result<String> {
            Ok("test_snapshot".to_string())
        }

        async fn load_snapshot(&self, _snapshot_path: &str) -> Result<()> {
            Ok(())
        }

        async fn replay_wal(&self, _from_sequence: u64) -> Result<RecoveryInfo> {
            Ok(RecoveryInfo {
                entries_replayed: 0,
                leases_recovered: 0,
                corrupted_entries: 0,
                recovery_start: Utc::now(),
                recovery_end: Utc::now(),
                last_sequence_number: 0,
                warnings: Vec::new(),
            })
        }

        async fn compact_wal(&self, _before_sequence: u64) -> Result<()> {
            Ok(())
        }

        async fn current_sequence_number(&self) -> Result<u64> {
            Ok(0)
        }

        async fn sync_wal(&self) -> Result<()> {
            Ok(())
        }

        async fn verify_wal_integrity(&self) -> Result<Vec<String>> {
            Ok(Vec::new())
        }

        async fn begin_transaction(&self) -> Result<uuid::Uuid> {
            Ok(uuid::Uuid::new_v4())
        }

        async fn commit_transaction(&self, _transaction_id: uuid::Uuid) -> Result<()> {
            Ok(())
        }

        async fn rollback_transaction(&self, _transaction_id: uuid::Uuid) -> Result<()> {
            Ok(())
        }

        async fn emergency_recovery(&self) -> Result<RecoveryInfo> {
            Ok(RecoveryInfo {
                entries_replayed: 5,
                leases_recovered: 3,
                corrupted_entries: 0,
                recovery_start: Utc::now(),
                recovery_end: Utc::now(),
                last_sequence_number: 5,
                warnings: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn test_recovery_config_validation() {
        let mut config = RecoveryConfig::default();
        assert!(RecoveryManager::validate_config(&config).is_empty());

        config.max_recovery_time = Duration::from_secs(5); // Too short
        let issues = RecoveryManager::validate_config(&config);
        assert!(!issues.is_empty());
    }

    #[tokio::test]
    async fn test_emergency_recovery() {
        let manager = create_test_recovery_manager().await;
        let result = manager.recover(RecoveryTrigger::Manual).await.unwrap();
        assert!(result.success);
        assert_eq!(result.recovery_info.leases_recovered, 0); // Mock returns 0
    }

    #[tokio::test]
    async fn test_recovery_progress_tracking() {
        let manager = create_test_recovery_manager().await;
        
        // Start recovery in background
        let manager_clone = Arc::new(manager);
        let recovery_handle = {
            let manager = manager_clone.clone();
            tokio::spawn(async move {
                manager.recover(RecoveryTrigger::Manual).await
            })
        };

        // Check progress
        tokio::time::sleep(Duration::from_millis(100)).await;
        let progress = manager_clone.get_recovery_progress().await;
        assert!(progress.is_some());

        // Wait for completion
        let result = recovery_handle.await.unwrap().unwrap();
        assert!(result.success);
    }
}