// src/storage/file_persistent.rs - File-based persistent storage with WAL

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseFilter, LeaseStats, ObjectType};
use crate::storage::{Storage, memory::MemoryStorage};
use super::persistent::{
    PersistentStorage, PersistentStorageConfig, RecoveryInfo, SnapshotMetadata,
    TransactionContext, WALEntry, WALFileMetadata, WALOperation, WALSyncPolicy,
};

/// File-based persistent storage with write-ahead logging
pub struct FilePersistentStorage {
    /// In-memory storage for fast access
    memory_storage: Arc<MemoryStorage>,
    /// Current WAL file writer
    wal_writer: Arc<Mutex<Option<BufWriter<File>>>>,
    /// Storage configuration
    config: Arc<RwLock<PersistentStorageConfig>>,
    /// Current sequence number
    sequence_counter: AtomicU64,
    /// Active transactions
    transactions: Arc<DashMap<Uuid, TransactionContext>>,
    /// WAL sync counter
    operations_since_sync: AtomicU64,
    /// Background task handles
    background_tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    /// Recovery state
    recovery_info: Arc<RwLock<Option<RecoveryInfo>>>,
}

impl FilePersistentStorage {
    pub fn new() -> Self {
        Self {
            memory_storage: Arc::new(MemoryStorage::new()),
            wal_writer: Arc::new(Mutex::new(None)),
            config: Arc::new(RwLock::new(PersistentStorageConfig::default())),
            sequence_counter: AtomicU64::new(0),
            transactions: Arc::new(DashMap::new()),
            operations_since_sync: AtomicU64::new(0),
            background_tasks: Arc::new(Mutex::new(Vec::new())),
            recovery_info: Arc::new(RwLock::new(None)),
        }
    }

    /// Ensure data directory exists
    async fn ensure_data_directory(&self, config: &PersistentStorageConfig) -> Result<()> {
        let data_dir = Path::new(&config.data_directory);
        if !data_dir.exists() {
            tokio::fs::create_dir_all(data_dir).await.map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Failed to create data directory: {}", e))
            })?;
            info!("Created data directory: {}", config.data_directory);
        }
        Ok(())
    }

    /// Initialize WAL writer
    async fn initialize_wal_writer(&self, config: &PersistentStorageConfig) -> Result<()> {
        let wal_path = Path::new(&config.wal_path);
        
        // Ensure parent directory exists
        if let Some(parent) = wal_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Failed to create WAL directory: {}", e))
            })?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&config.wal_path)
            .await
            .map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Failed to open WAL file: {}", e))
            })?;

        let writer = BufWriter::new(file);
        *self.wal_writer.lock().await = Some(writer);

        info!("Initialized WAL writer: {}", config.wal_path);
        Ok(())
    }

    /// Start background tasks
    async fn start_background_tasks(&self) {
        let config = self.config.read().await.clone();
        
        // Snapshot task
        if config.snapshot_interval > 0 {
            let storage_clone = self.clone_for_task();
            let snapshot_task = tokio::spawn(async move {
                let mut interval = tokio::time::interval(
                    std::time::Duration::from_secs(config.snapshot_interval)
                );
                
                loop {
                    interval.tick().await;
                    if let Err(e) = storage_clone.create_snapshot().await {
                        error!("Background snapshot failed: {}", e);
                    }
                }
            });
            
            self.background_tasks.lock().await.push(snapshot_task);
        }

        // WAL sync task
        let storage_clone = self.clone_for_task();
        let sync_config = config.sync_policy.clone();
        let sync_task = tokio::spawn(async move {
            match sync_config {
                WALSyncPolicy::Interval(seconds) => {
                    let mut interval = tokio::time::interval(
                        std::time::Duration::from_secs(seconds)
                    );
                    
                    loop {
                        interval.tick().await;
                        if let Err(e) = storage_clone.sync_wal().await {
                            error!("Background WAL sync failed: {}", e);
                        }
                    }
                }
                _ => {
                    // For other policies, no background sync needed
                    std::future::pending::<()>().await;
                }
            }
        });
        
        self.background_tasks.lock().await.push(sync_task);
        
        info!("Started background tasks");
    }

    /// Clone for background tasks (simplified interface)
    fn clone_for_task(&self) -> Self {
        Self {
            memory_storage: self.memory_storage.clone(),
            wal_writer: self.wal_writer.clone(),
            config: self.config.clone(),
            sequence_counter: AtomicU64::new(self.sequence_counter.load(Ordering::Relaxed)),
            transactions: self.transactions.clone(),
            operations_since_sync: AtomicU64::new(0),
            background_tasks: Arc::new(Mutex::new(Vec::new())),
            recovery_info: self.recovery_info.clone(),
        }
    }

    /// Write operation and update memory
    async fn execute_operation(&self, operation: WALOperation) -> Result<()> {
        // First write to WAL
        let sequence = self.sequence_counter.fetch_add(1, Ordering::Relaxed);
        let entry = WALEntry::new(operation.clone(), sequence, None);
        self.write_wal_entry(entry).await?;

        // Then apply to memory storage
        self.apply_operation_to_memory(&operation).await?;

        Ok(())
    }

    /// Apply operation to in-memory storage
    async fn apply_operation_to_memory(&self, operation: &WALOperation) -> Result<()> {
        match operation {
            WALOperation::CreateLease { lease, .. } => {
                self.memory_storage.create_lease(lease.clone()).await
            }
            WALOperation::UpdateLease { lease, .. } => {
                self.memory_storage.update_lease(lease.clone()).await
            }
            WALOperation::DeleteLease { lease_id, .. } => {
                self.memory_storage.delete_lease(lease_id).await
            }
            WALOperation::ExpireLease { lease_id, .. } => {
                self.memory_storage.mark_lease_expired(lease_id).await
            }
            WALOperation::ReleaseLease { lease_id, .. } => {
                self.memory_storage.mark_lease_released(lease_id).await
            }
            WALOperation::Checkpoint { .. } => {
                // Checkpoints don't modify state
                Ok(())
            }
            WALOperation::BeginTransaction { .. } |
            WALOperation::CommitTransaction { .. } |
            WALOperation::RollbackTransaction { .. } => {
                // Transaction markers don't modify lease state
                Ok(())
            }
        }
    }

    /// Check if WAL sync is needed
    async fn check_sync_needed(&self) -> bool {
        let config = self.config.read().await;
        let ops_since_sync = self.operations_since_sync.load(Ordering::Relaxed);

        match config.sync_policy {
            WALSyncPolicy::EveryWrite => true,
            WALSyncPolicy::EveryN(n) => ops_since_sync >= n as u64,
            _ => false,
        }
    }

    /// Load existing WAL files and determine next sequence number
    async fn load_wal_sequence_number(&self, config: &PersistentStorageConfig) -> Result<u64> {
        let wal_path = Path::new(&config.wal_path);
        
        if !wal_path.exists() {
            return Ok(0); // Start from 0 if no WAL exists
        }

        let file = File::open(wal_path).await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to open existing WAL: {}", e))
        })?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut max_sequence = 0u64;
        let mut has_entries = false;

        while let Some(line) = lines.next_line().await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to read WAL line: {}", e))
        })? {
            if let Ok(entry) = serde_json::from_str::<WALEntry>(&line) {
                max_sequence = max_sequence.max(entry.sequence_number);
                has_entries = true;
            }
        }

        // If we have entries, next sequence is max + 1, otherwise start at 0
        if has_entries {
            Ok(max_sequence + 1)
        } else {
            Ok(0)
        }
    }

    /// Create compressed snapshot
    async fn create_compressed_snapshot(&self, snapshot_path: &str) -> Result<SnapshotMetadata> {
        use flate2::{write::GzEncoder, Compression};
        use std::io::Write;

        // Get all leases from memory
        let leases = self.memory_storage.list_leases(None, None).await?;
        let sequence_number = self.sequence_counter.load(Ordering::Relaxed);

        // Serialize leases
        let serialized = serde_json::to_string(&leases).map_err(|e| {
            GCError::Serialization(e)
        })?;

        // Compress and write
        let compressed = {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(serialized.as_bytes()).map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Compression failed: {}", e))
            })?;
            encoder.finish().map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Compression finish failed: {}", e))
            })?
        };

        tokio::fs::write(snapshot_path, &compressed).await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to write snapshot: {}", e))
        })?;

        // Calculate checksum
        let checksum = format!("{:x}", md5::compute(&compressed));

        let metadata = SnapshotMetadata {
            sequence_number,
            timestamp: Utc::now(),
            lease_count: leases.len(),
            checksum,
            compression: Some("gzip".to_string()),
            version: env!("CARGO_PKG_VERSION").to_string(),
        };

        // Write metadata
        let metadata_path = format!("{}.meta", snapshot_path);
        let metadata_json = serde_json::to_string_pretty(&metadata).map_err(|e| {
            GCError::Serialization(e)
        })?;
        
        tokio::fs::write(metadata_path, metadata_json).await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to write snapshot metadata: {}", e))
        })?;

        info!(
            "Created compressed snapshot: {} leases, {} bytes, sequence {}",
            leases.len(),
            compressed.len(),
            sequence_number
        );

        Ok(metadata)
    }
}

#[async_trait]
impl PersistentStorage for FilePersistentStorage {
    async fn initialize(&self, config: &PersistentStorageConfig) -> Result<()> {
        info!("Initializing file-based persistent storage");

        // Store config
        *self.config.write().await = config.clone();

        // Ensure directories exist
        self.ensure_data_directory(config).await?;

        // Load sequence number from existing WAL
        let next_sequence = self.load_wal_sequence_number(config).await?;
        self.sequence_counter.store(next_sequence, Ordering::Relaxed);

        // Initialize WAL writer
        self.initialize_wal_writer(config).await?;

        // Perform recovery if needed (but only if WAL exists)
        let recovery_info = if next_sequence > 0 {
            self.replay_wal(0).await?
        } else {
            // No WAL to replay, create empty recovery info
            RecoveryInfo {
                entries_replayed: 0,
                leases_recovered: 0,
                corrupted_entries: 0,
                recovery_start: Utc::now(),
                recovery_end: Utc::now(),
                last_sequence_number: 0,
                warnings: Vec::new(),
            }
        };
        
        *self.recovery_info.write().await = Some(recovery_info.clone());

        // Start background tasks
        self.start_background_tasks().await;

        info!(
            "Persistent storage initialized: {} leases recovered, current sequence: {}",
            recovery_info.leases_recovered,
            self.sequence_counter.load(Ordering::Relaxed)
        );

        Ok(())
    }

    async fn write_wal_entry(&self, entry: WALEntry) -> Result<()> {
        let mut writer_guard = self.wal_writer.lock().await;
        let writer = writer_guard.as_mut().ok_or_else(|| {
            GCError::Storage(anyhow::anyhow!("WAL writer not initialized"))
        })?;

        // Serialize and write entry
        let serialized = serde_json::to_string(&entry).map_err(|e| {
            GCError::Serialization(e)
        })?;

        writer.write_all(serialized.as_bytes()).await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to write WAL entry: {}", e))
        })?;

        writer.write_all(b"\n").await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to write WAL newline: {}", e))
        })?;

        // Check if sync is needed
        if self.check_sync_needed().await {
            writer.flush().await.map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Failed to flush WAL: {}", e))
            })?;
            self.operations_since_sync.store(0, Ordering::Relaxed);
        } else {
            self.operations_since_sync.fetch_add(1, Ordering::Relaxed);
        }

        debug!("Wrote WAL entry: sequence {}", entry.sequence_number);
        Ok(())
    }

    async fn read_wal_entries(&self, from_sequence: u64) -> Result<Vec<WALEntry>> {
        let config = self.config.read().await;
        let wal_path = Path::new(&config.wal_path);

        if !wal_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(wal_path).await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to open WAL for reading: {}", e))
        })?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut entries = Vec::new();

        while let Some(line) = lines.next_line().await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to read WAL line: {}", e))
        })? {
            match serde_json::from_str::<WALEntry>(&line) {
                Ok(entry) => {
                    if entry.sequence_number >= from_sequence {
                        if entry.verify_integrity() {
                            entries.push(entry);
                        } else {
                            warn!("Corrupted WAL entry at sequence {}", entry.sequence_number);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to parse WAL entry: {}", e);
                }
            }
        }

        debug!("Read {} WAL entries from sequence {}", entries.len(), from_sequence);
        Ok(entries)
    }

    async fn create_snapshot(&self) -> Result<String> {
        let config = self.config.read().await;
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S").to_string();
        let snapshot_path = format!("{}.{}", config.snapshot_path, timestamp);

        if config.compress_snapshots {
            self.create_compressed_snapshot(&snapshot_path).await?;
        } else {
            // Uncompressed snapshot
            let leases = self.memory_storage.list_leases(None, None).await?;
            let serialized = serde_json::to_string_pretty(&leases).map_err(|e| {
                GCError::Serialization(e)
            })?;

            tokio::fs::write(&snapshot_path, serialized).await.map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Failed to write snapshot: {}", e))
            })?;
        }

        info!("Created snapshot: {}", snapshot_path);
        Ok(snapshot_path)
    }

    async fn load_snapshot(&self, snapshot_path: &str) -> Result<()> {
        info!("Loading snapshot from: {}", snapshot_path);

        let data = tokio::fs::read(snapshot_path).await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to read snapshot: {}", e))
        })?;

        // Try to load metadata to determine if compressed
        let metadata_path = format!("{}.meta", snapshot_path);
        let is_compressed = if let Ok(metadata_data) = tokio::fs::read_to_string(&metadata_path).await {
            if let Ok(metadata) = serde_json::from_str::<SnapshotMetadata>(&metadata_data) {
                metadata.compression.is_some()
            } else {
                false
            }
        } else {
            false
        };

        let content = if is_compressed {
            use flate2::read::GzDecoder;
            use std::io::Read;

            let mut decoder = GzDecoder::new(&data[..]);
            let mut decompressed = String::new();
            decoder.read_to_string(&mut decompressed).map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Failed to decompress snapshot: {}", e))
            })?;
            decompressed
        } else {
            String::from_utf8(data).map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Invalid UTF-8 in snapshot: {}", e))
            })?
        };

        let leases: Vec<Lease> = serde_json::from_str(&content).map_err(|e| {
            GCError::Serialization(e)
        })?;

        let lease_count = leases.len();

        // Clear existing data and load leases
        // Note: In a real implementation, you'd want to be more careful about this
        for lease in leases {
            self.memory_storage.create_lease(lease).await?;
        }

        info!("Loaded snapshot with {} leases", lease_count);
        Ok(())
    }

    async fn replay_wal(&self, from_sequence: u64) -> Result<RecoveryInfo> {
        let recovery_start = Utc::now();
        info!("Starting WAL replay from sequence {}", from_sequence);

        let entries = self.read_wal_entries(from_sequence).await?;
        let mut recovery_info = RecoveryInfo {
            entries_replayed: 0,
            leases_recovered: 0,
            corrupted_entries: 0,
            recovery_start,
            recovery_end: recovery_start,
            last_sequence_number: from_sequence,
            warnings: Vec::new(),
        };

        let mut active_transactions: HashMap<Uuid, Vec<WALOperation>> = HashMap::new();
        let mut lease_count = 0;

        for entry in entries {
            if !entry.verify_integrity() {
                recovery_info.corrupted_entries += 1;
                recovery_info.warnings.push(
                    format!("Corrupted entry at sequence {}", entry.sequence_number)
                );
                continue;
            }

            match &entry.operation {
                WALOperation::BeginTransaction { transaction_id, .. } => {
                    active_transactions.insert(*transaction_id, Vec::new());
                }
                WALOperation::CommitTransaction { transaction_id, .. } => {
                    if let Some(operations) = active_transactions.remove(transaction_id) {
                        // Apply all operations in the transaction
                        for op in operations {
                            if let Err(e) = self.apply_operation_to_memory(&op).await {
                                recovery_info.warnings.push(
                                    format!("Failed to apply transaction operation: {}", e)
                                );
                            } else {
                                if matches!(op, WALOperation::CreateLease { .. }) {
                                    lease_count += 1;
                                }
                            }
                        }
                    }
                }
                WALOperation::RollbackTransaction { transaction_id, .. } => {
                    active_transactions.remove(transaction_id);
                }
                operation => {
                    if let Some(tx_id) = entry.transaction_id {
                        // Add to transaction
                        if let Some(tx_ops) = active_transactions.get_mut(&tx_id) {
                            tx_ops.push(operation.clone());
                        }
                    } else {
                        // Apply immediately (non-transactional)
                        if let Err(e) = self.apply_operation_to_memory(operation).await {
                            recovery_info.warnings.push(
                                format!("Failed to apply operation at sequence {}: {}", 
                                        entry.sequence_number, e)
                            );
                        } else {
                            if matches!(operation, WALOperation::CreateLease { .. }) {
                                lease_count += 1;
                            }
                        }
                    }
                }
            }

            recovery_info.entries_replayed += 1;
            recovery_info.last_sequence_number = entry.sequence_number;
        }

        // Handle incomplete transactions
        if !active_transactions.is_empty() {
            recovery_info.warnings.push(
                format!("Found {} incomplete transactions during recovery", 
                        active_transactions.len())
            );
        }

        recovery_info.leases_recovered = lease_count;
        recovery_info.recovery_end = Utc::now();

        // Update sequence counter
        if recovery_info.last_sequence_number > 0 {
            self.sequence_counter.store(
                recovery_info.last_sequence_number + 1, 
                Ordering::Relaxed
            );
        }

        info!(
            "WAL replay completed: {} entries replayed, {} leases recovered, {} corrupted entries",
            recovery_info.entries_replayed,
            recovery_info.leases_recovered,
            recovery_info.corrupted_entries
        );

        Ok(recovery_info)
    }

    async fn compact_wal(&self, before_sequence: u64) -> Result<()> {
        let config = self.config.read().await;
        let wal_path = Path::new(&config.wal_path);
        
        if !wal_path.exists() {
            return Ok(());
        }

        info!("Compacting WAL before sequence {}", before_sequence);

        // Read all entries after the compaction point
        let entries_to_keep = self.read_wal_entries(before_sequence).await?;

        // Create backup of current WAL
        let backup_path = format!("{}.backup", config.wal_path);
        tokio::fs::copy(&config.wal_path, &backup_path).await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to backup WAL: {}", e))
        })?;

        // Write compacted WAL
        let temp_path = format!("{}.compact", config.wal_path);
        {
            let temp_file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&temp_path)
                .await
                .map_err(|e| {
                    GCError::Storage(anyhow::anyhow!("Failed to create temp WAL: {}", e))
                })?;

            let mut writer = BufWriter::new(temp_file);
            
            for entry in entries_to_keep {
                let serialized = serde_json::to_string(&entry).map_err(|e| {
                    GCError::Serialization(e)
                })?;
                
                writer.write_all(serialized.as_bytes()).await.map_err(|e| {
                    GCError::Storage(anyhow::anyhow!("Failed to write compacted entry: {}", e))
                })?;
                writer.write_all(b"\n").await.map_err(|e| {
                    GCError::Storage(anyhow::anyhow!("Failed to write newline: {}", e))
                })?;
            }
            
            writer.flush().await.map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Failed to flush compacted WAL: {}", e))
            })?;
        }

        // Replace original WAL with compacted version
        tokio::fs::rename(&temp_path, &config.wal_path).await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to replace WAL with compacted version: {}", e))
        })?;

        // Reinitialize WAL writer
        self.initialize_wal_writer(&config).await?;

        info!("WAL compaction completed");
        Ok(())
    }

    async fn current_sequence_number(&self) -> Result<u64> {
        // Return the current sequence number (the next one to be used)
        // This represents how many entries have been written
        Ok(self.sequence_counter.load(Ordering::Relaxed))
    }

    async fn wal_entry_count(&self) -> Result<u64> {
        // Count actual entries in the WAL file
        let config = self.config.read().await;
        let wal_path = Path::new(&config.wal_path);
        
        if !wal_path.exists() {
            return Ok(0);
        }

        let file = File::open(wal_path).await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to open WAL for counting: {}", e))
        })?;

        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let mut count = 0u64;

        while let Some(line) = lines.next_line().await.map_err(|e| {
            GCError::Storage(anyhow::anyhow!("Failed to read WAL line: {}", e))
        })? {
            if !line.trim().is_empty() && serde_json::from_str::<WALEntry>(&line).is_ok() {
                count += 1;
            }
        }

        Ok(count)
    }

    async fn sync_wal(&self) -> Result<()> {
        let mut writer_guard = self.wal_writer.lock().await;
        if let Some(writer) = writer_guard.as_mut() {
            writer.flush().await.map_err(|e| {
                GCError::Storage(anyhow::anyhow!("Failed to sync WAL: {}", e))
            })?;
            self.operations_since_sync.store(0, Ordering::Relaxed);
        }
        Ok(())
    }

    async fn verify_wal_integrity(&self) -> Result<Vec<String>> {
        let entries = self.read_wal_entries(0).await?;
        let mut issues = Vec::new();
        let mut last_sequence = None;

        for entry in entries {
            // Check sequence ordering
            if let Some(last) = last_sequence {
                if entry.sequence_number <= last {
                    issues.push(format!(
                        "Sequence number {} is not greater than previous {}",
                        entry.sequence_number, last
                    ));
                }
            }

            // Check integrity
            if !entry.verify_integrity() {
                issues.push(format!(
                    "Integrity check failed for sequence {}",
                    entry.sequence_number
                ));
            }

            last_sequence = Some(entry.sequence_number);
        }

        if issues.is_empty() {
            info!("WAL integrity verification passed");
        } else {
            warn!("WAL integrity issues found: {}", issues.len());
        }

        Ok(issues)
    }

    async fn begin_transaction(&self) -> Result<Uuid> {
        let transaction_id = Uuid::new_v4();
        let tx_context = TransactionContext::new();
        
        // Write begin transaction to WAL
        let operation = WALOperation::BeginTransaction {
            transaction_id,
            timestamp: Utc::now(),
        };
        
        let sequence = self.sequence_counter.fetch_add(1, Ordering::Relaxed);
        let entry = WALEntry::new(operation, sequence, Some(transaction_id));
        self.write_wal_entry(entry).await?;

        // Store transaction context
        self.transactions.insert(transaction_id, tx_context);

        debug!("Started transaction: {}", transaction_id);
        Ok(transaction_id)
    }

    async fn commit_transaction(&self, transaction_id: Uuid) -> Result<()> {
        // Remove transaction context
        let tx_context = self.transactions.remove(&transaction_id)
            .ok_or_else(|| GCError::Internal(format!("Transaction {} not found", transaction_id)))?
            .1;

        // Write commit to WAL
        let operation = WALOperation::CommitTransaction {
            transaction_id,
            timestamp: Utc::now(),
        };
        
        let sequence = self.sequence_counter.fetch_add(1, Ordering::Relaxed);
        let entry = WALEntry::new(operation, sequence, Some(transaction_id));
        self.write_wal_entry(entry).await?;

        // Apply all operations in the transaction to memory
        for op in tx_context.operations {
            self.apply_operation_to_memory(&op).await?;
        }

        debug!("Committed transaction: {}", transaction_id);
        Ok(())
    }

    async fn rollback_transaction(&self, transaction_id: Uuid) -> Result<()> {
        // Remove transaction context
        self.transactions.remove(&transaction_id)
            .ok_or_else(|| GCError::Internal(format!("Transaction {} not found", transaction_id)))?;

        // Write rollback to WAL
        let operation = WALOperation::RollbackTransaction {
            transaction_id,
            timestamp: Utc::now(),
        };
        
        let sequence = self.sequence_counter.fetch_add(1, Ordering::Relaxed);
        let entry = WALEntry::new(operation, sequence, Some(transaction_id));
        self.write_wal_entry(entry).await?;

        debug!("Rolled back transaction: {}", transaction_id);
        Ok(())
    }

    async fn emergency_recovery(&self) -> Result<RecoveryInfo> {
        warn!("Starting emergency recovery");
        
        // Try to load from latest snapshot first
        let config = self.config.read().await;
        let snapshot_dir = Path::new(&config.data_directory);
        
        let mut latest_snapshot = None;
        let mut latest_time = None;

        if let Ok(mut entries) = tokio::fs::read_dir(snapshot_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Some(name) = entry.file_name().to_str() {
                    if name.starts_with("garbagetruck.snapshot") && !name.ends_with(".meta") {
                        if let Ok(metadata) = entry.metadata().await {
                            if let Ok(modified) = metadata.modified() {
                                if latest_time.map_or(true, |t| modified > t) {
                                    latest_time = Some(modified);
                                    latest_snapshot = Some(entry.path());
                                }
                            }
                        }
                    }
                }
            }
        }

        let recovery_start = Utc::now();
        let mut recovery_info = RecoveryInfo {
            entries_replayed: 0,
            leases_recovered: 0,
            corrupted_entries: 0,
            recovery_start,
            recovery_end: recovery_start,
            last_sequence_number: 0,
            warnings: Vec::new(),
        };

        // Load snapshot if available
        if let Some(snapshot_path) = latest_snapshot {
            info!("Loading latest snapshot: {:?}", snapshot_path);
            if let Err(e) = self.load_snapshot(snapshot_path.to_str().unwrap()).await {
                recovery_info.warnings.push(format!("Failed to load snapshot: {}", e));
            } else {
                let leases = self.memory_storage.list_leases(None, None).await?;
                recovery_info.leases_recovered = leases.len();
            }
        }

        // Replay WAL from beginning
        let wal_recovery = self.replay_wal(0).await?;
        recovery_info.entries_replayed = wal_recovery.entries_replayed;
        recovery_info.corrupted_entries = wal_recovery.corrupted_entries;
        recovery_info.last_sequence_number = wal_recovery.last_sequence_number;
        recovery_info.warnings.extend(wal_recovery.warnings);
        recovery_info.recovery_end = Utc::now();

        warn!(
            "Emergency recovery completed: {} leases recovered, {} WAL entries replayed",
            recovery_info.leases_recovered,
            recovery_info.entries_replayed
        );

        Ok(recovery_info)
    }
}

// Implement the regular Storage trait by delegating to memory storage
#[async_trait]
impl Storage for FilePersistentStorage {
    async fn create_lease(&self, lease: Lease) -> Result<()> {
        let operation = WALOperation::CreateLease {
            lease: lease.clone(),
            timestamp: Utc::now(),
        };
        self.execute_operation(operation).await
    }

    async fn get_lease(&self, lease_id: &str) -> Result<Option<Lease>> {
        self.memory_storage.get_lease(lease_id).await
    }

    async fn update_lease(&self, lease: Lease) -> Result<()> {
        let operation = WALOperation::UpdateLease {
            lease: lease.clone(),
            timestamp: Utc::now(),
        };
        self.execute_operation(operation).await
    }

    async fn delete_lease(&self, lease_id: &str) -> Result<()> {
        let operation = WALOperation::DeleteLease {
            lease_id: lease_id.to_string(),
            timestamp: Utc::now(),
        };
        self.execute_operation(operation).await
    }

    async fn list_leases(
        &self,
        filter: Option<LeaseFilter>,
        limit: Option<usize>,
    ) -> Result<Vec<Lease>> {
        self.memory_storage.list_leases(filter, limit).await
    }

    async fn count_leases(&self, filter: LeaseFilter) -> Result<usize> {
        self.memory_storage.count_leases(filter).await
    }

    async fn get_expired_leases(&self, grace_period: std::time::Duration) -> Result<Vec<Lease>> {
        self.memory_storage.get_expired_leases(grace_period).await
    }

    async fn get_leases_by_service(&self, service_id: &str) -> Result<Vec<Lease>> {
        self.memory_storage.get_leases_by_service(service_id).await
    }

    async fn get_leases_by_type(&self, object_type: ObjectType) -> Result<Vec<Lease>> {
        self.memory_storage.get_leases_by_type(object_type).await
    }

    async fn get_stats(&self) -> Result<LeaseStats> {
        self.memory_storage.get_stats().await
    }

    async fn health_check(&self) -> Result<bool> {
        // Check both memory storage and persistent layer
        let memory_ok = self.memory_storage.health_check().await?;
        
        // Check if we can write to WAL
        let wal_ok = self.wal_writer.lock().await.is_some();
        
        Ok(memory_ok && wal_ok)
    }

    async fn cleanup(&self) -> Result<usize> {
        self.memory_storage.cleanup().await
    }

    async fn count_active_leases_for_service(&self, service_id: &str) -> Result<usize> {
        self.memory_storage.count_active_leases_for_service(service_id).await
    }

    async fn mark_lease_expired(&self, lease_id: &str) -> Result<()> {
        let operation = WALOperation::ExpireLease {
            lease_id: lease_id.to_string(),
            timestamp: Utc::now(),
        };
        self.execute_operation(operation).await
    }

    async fn mark_lease_released(&self, lease_id: &str) -> Result<()> {
        let operation = WALOperation::ReleaseLease {
            lease_id: lease_id.to_string(),
            timestamp: Utc::now(),
        };
        self.execute_operation(operation).await
    }
}

impl Drop for FilePersistentStorage {
    fn drop(&mut self) {
        // Note: In a real implementation, you'd want to handle graceful shutdown
        // of background tasks here, but that requires async Drop which Rust doesn't have
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use crate::lease::ObjectType;
    use std::collections::HashMap;

    async fn create_test_storage() -> (FilePersistentStorage, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = FilePersistentStorage::new();
        
        let config = PersistentStorageConfig {
            data_directory: temp_dir.path().to_string_lossy().to_string(),
            wal_path: temp_dir.path().join("test.wal").to_string_lossy().to_string(),
            snapshot_path: temp_dir.path().join("test.snapshot").to_string_lossy().to_string(),
            max_wal_size: 1024 * 1024,
            sync_policy: WALSyncPolicy::EveryWrite,
            snapshot_interval: 0, // Disable background snapshots in tests
            max_wal_files: 3,
            compress_snapshots: false,
        };
        
        storage.initialize(&config).await.unwrap();
        (storage, temp_dir)
    }

    fn create_test_lease() -> Lease {
        Lease::new(
            "test-object".to_string(),
            ObjectType::TemporaryFile,
            "test-service".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        )
    }

    #[tokio::test]
    async fn test_basic_operations() {
        let (storage, _temp_dir) = create_test_storage().await;
        
        let lease = create_test_lease();
        let lease_id = lease.lease_id.clone();
        
        // Create lease
        storage.create_lease(lease.clone()).await.unwrap();
        
        // Get lease
        let retrieved = storage.get_lease(&lease_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().lease_id, lease_id);
        
        // Delete lease
        storage.delete_lease(&lease_id).await.unwrap();
        
        // Verify deletion
        let deleted = storage.get_lease(&lease_id).await.unwrap();
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_wal_recovery() {
        let temp_dir = TempDir::new().unwrap();
        let config = PersistentStorageConfig {
            data_directory: temp_dir.path().to_string_lossy().to_string(),
            wal_path: temp_dir.path().join("test.wal").to_string_lossy().to_string(),
            snapshot_path: temp_dir.path().join("test.snapshot").to_string_lossy().to_string(),
            max_wal_size: 1024 * 1024,
            sync_policy: WALSyncPolicy::EveryWrite,
            snapshot_interval: 0,
            max_wal_files: 3,
            compress_snapshots: false,
        };

        // Create first storage instance and add data
        {
            let storage = FilePersistentStorage::new();
            storage.initialize(&config).await.unwrap();
            
            let lease = create_test_lease();
            storage.create_lease(lease).await.unwrap();
        }
        
        // Create second storage instance to test recovery
        let storage2 = FilePersistentStorage::new();
        storage2.initialize(&config).await.unwrap();
        
        // Verify data was recovered
        let leases = storage2.list_leases(None, None).await.unwrap();
        assert_eq!(leases.len(), 1);
    }

    #[tokio::test]
    async fn test_transactions() {
        let (storage, _temp_dir) = create_test_storage().await;
        
        // Begin transaction
        let tx_id = storage.begin_transaction().await.unwrap();
        
        // Create lease in transaction
        let lease = create_test_lease();
        let lease_id = lease.lease_id.clone();
        
        // For testing transactions, we'd need to modify the storage to support
        // transactional operations. This is a simplified test.
        storage.create_lease(lease).await.unwrap();
        
        // Commit transaction
        storage.commit_transaction(tx_id).await.unwrap();
        
        // Verify lease exists
        let retrieved = storage.get_lease(&lease_id).await.unwrap();
        assert!(retrieved.is_some());
    }

    #[tokio::test]
    async fn test_snapshot_creation() {
        let (storage, _temp_dir) = create_test_storage().await;
        
        // Add some data
        let lease = create_test_lease();
        storage.create_lease(lease).await.unwrap();
        
        // Create snapshot
        let snapshot_path = storage.create_snapshot().await.unwrap();
        assert!(tokio::fs::metadata(&snapshot_path).await.is_ok());
    }

    #[tokio::test]
    async fn test_wal_integrity() {
        let (storage, _temp_dir) = create_test_storage().await;
        
        // Add some operations
        let lease = create_test_lease();
        storage.create_lease(lease).await.unwrap();
        
        // Verify integrity
        let issues = storage.verify_wal_integrity().await.unwrap();
        assert!(issues.is_empty());
    }
}