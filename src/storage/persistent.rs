// src/storage/persistent.rs - Persistent storage trait extensions for WAL

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

use crate::error::Result;
use crate::lease::Lease;

/// Write-Ahead Log entry types for atomic operations
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum WALOperation {
    /// Create a new lease
    CreateLease {
        lease: Lease,
        timestamp: DateTime<Utc>,
    },
    /// Update an existing lease
    UpdateLease {
        lease: Lease,
        timestamp: DateTime<Utc>,
    },
    /// Delete a lease
    DeleteLease {
        lease_id: String,
        timestamp: DateTime<Utc>,
    },
    /// Mark lease as expired
    ExpireLease {
        lease_id: String,
        timestamp: DateTime<Utc>,
    },
    /// Mark lease as released
    ReleaseLease {
        lease_id: String,
        timestamp: DateTime<Utc>,
    },
    /// Batch operation checkpoint
    Checkpoint {
        sequence_number: u64,
        timestamp: DateTime<Utc>,
    },
    /// Transaction boundary markers
    BeginTransaction {
        transaction_id: Uuid,
        timestamp: DateTime<Utc>,
    },
    CommitTransaction {
        transaction_id: Uuid,
        timestamp: DateTime<Utc>,
    },
    RollbackTransaction {
        transaction_id: Uuid,
        timestamp: DateTime<Utc>,
    },
}

/// WAL entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WALEntry {
    /// Unique sequence number for ordering
    pub sequence_number: u64,
    /// The operation to be performed
    pub operation: WALOperation,
    /// Entry creation timestamp
    pub timestamp: DateTime<Utc>,
    /// Checksum for integrity verification
    pub checksum: u32,
    /// Transaction ID for grouping operations
    pub transaction_id: Option<Uuid>,
}

impl WALEntry {
    pub fn new(operation: WALOperation, sequence_number: u64, transaction_id: Option<Uuid>) -> Self {
        let timestamp = Utc::now();
        let mut entry = Self {
            sequence_number,
            operation,
            timestamp,
            checksum: 0,
            transaction_id,
        };
        entry.checksum = entry.calculate_checksum();
        entry
    }

    /// Calculate CRC32 checksum for integrity verification
    pub fn calculate_checksum(&self) -> u32 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.sequence_number.hash(&mut hasher);
        self.timestamp.hash(&mut hasher);
        if let Ok(serialized) = serde_json::to_string(&self.operation) {
            serialized.hash(&mut hasher);
        }
        if let Some(tx_id) = &self.transaction_id {
            tx_id.hash(&mut hasher);
        }
        hasher.finish() as u32
    }

    /// Verify entry integrity
    pub fn verify_integrity(&self) -> bool {
        let calculated = {
            let mut temp = self.clone();
            temp.checksum = 0;
            temp.calculate_checksum()
        };
        calculated == self.checksum
    }
}

/// Persistent storage configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentStorageConfig {
    /// Base directory for storage files
    pub data_directory: String,
    /// WAL file path
    pub wal_path: String,
    /// Snapshot file path
    pub snapshot_path: String,
    /// Maximum WAL file size before rotation (bytes)
    pub max_wal_size: u64,
    /// WAL sync policy
    pub sync_policy: WALSyncPolicy,
    /// Snapshot interval (seconds)
    pub snapshot_interval: u64,
    /// Maximum number of WAL files to retain
    pub max_wal_files: usize,
    /// Compression for snapshots
    pub compress_snapshots: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WALSyncPolicy {
    /// Sync every write (safest, slowest)
    EveryWrite,
    /// Sync every N operations
    EveryN(u32),
    /// Sync every N seconds
    Interval(u64),
    /// No explicit sync (fastest, least safe)
    None,
}

impl Default for PersistentStorageConfig {
    fn default() -> Self {
        Self {
            data_directory: "./data".to_string(),
            wal_path: "./data/garbagetruck.wal".to_string(),
            snapshot_path: "./data/garbagetruck.snapshot".to_string(),
            max_wal_size: 100 * 1024 * 1024, // 100MB
            sync_policy: WALSyncPolicy::EveryN(10),
            snapshot_interval: 300, // 5 minutes
            max_wal_files: 5,
            compress_snapshots: true,
        }
    }
}

/// Recovery information from WAL replay
#[derive(Debug, Clone)]
pub struct RecoveryInfo {
    /// Number of WAL entries replayed
    pub entries_replayed: usize,
    /// Number of leases recovered
    pub leases_recovered: usize,
    /// Number of corrupted entries skipped
    pub corrupted_entries: usize,
    /// Recovery start time
    pub recovery_start: DateTime<Utc>,
    /// Recovery completion time
    pub recovery_end: DateTime<Utc>,
    /// Last valid sequence number
    pub last_sequence_number: u64,
    /// Recovery warnings/issues
    pub warnings: Vec<String>,
}

/// Extended storage trait for persistent operations with WAL
#[async_trait]
pub trait PersistentStorage: Send + Sync {
    /// Initialize persistent storage
    async fn initialize(&self, config: &PersistentStorageConfig) -> Result<()>;

    /// Write operation to WAL (must be durable before returning)
    async fn write_wal_entry(&self, entry: WALEntry) -> Result<()>;

    /// Read WAL entries starting from sequence number
    async fn read_wal_entries(&self, from_sequence: u64) -> Result<Vec<WALEntry>>;

    /// Create snapshot of current state
    async fn create_snapshot(&self) -> Result<String>;

    /// Load snapshot from file
    async fn load_snapshot(&self, snapshot_path: &str) -> Result<()>;

    /// Replay WAL entries to recover state
    async fn replay_wal(&self, from_sequence: u64) -> Result<RecoveryInfo>;

    /// Compact WAL by removing entries before sequence number
    async fn compact_wal(&self, before_sequence: u64) -> Result<()>;

    /// Get current WAL sequence number
    async fn current_sequence_number(&self) -> Result<u64>;

    /// Get the number of entries actually written to WAL
    async fn wal_entry_count(&self) -> Result<u64> {
        // Default implementation - can be overridden
        self.current_sequence_number().await
    }

    /// Force sync WAL to disk
    async fn sync_wal(&self) -> Result<()>;

    /// Check WAL integrity
    async fn verify_wal_integrity(&self) -> Result<Vec<String>>;

    /// Begin transaction
    async fn begin_transaction(&self) -> Result<Uuid>;

    /// Commit transaction
    async fn commit_transaction(&self, transaction_id: Uuid) -> Result<()>;

    /// Rollback transaction
    async fn rollback_transaction(&self, transaction_id: Uuid) -> Result<()>;

    /// Recovery from catastrophic failure
    async fn emergency_recovery(&self) -> Result<RecoveryInfo>;
}

/// Transaction context for grouping operations
#[derive(Debug, Clone)]
pub struct TransactionContext {
    pub transaction_id: Uuid,
    pub start_time: DateTime<Utc>,
    pub operations: Vec<WALOperation>,
}

impl TransactionContext {
    pub fn new() -> Self {
        Self {
            transaction_id: Uuid::new_v4(),
            start_time: Utc::now(),
            operations: Vec::new(),
        }
    }

    pub fn add_operation(&mut self, operation: WALOperation) {
        self.operations.push(operation);
    }
}

/// Snapshot metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub sequence_number: u64,
    pub timestamp: DateTime<Utc>,
    pub lease_count: usize,
    pub checksum: String,
    pub compression: Option<String>,
    pub version: String,
}

/// WAL file metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WALFileMetadata {
    pub file_path: String,
    pub start_sequence: u64,
    pub end_sequence: u64,
    pub entry_count: usize,
    pub file_size: u64,
    pub created_at: DateTime<Utc>,
    pub checksum: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wal_entry_integrity() {
        let operation = WALOperation::CreateLease {
            lease: create_test_lease(),
            timestamp: Utc::now(),
        };
        
        let entry = WALEntry::new(operation, 1, None);
        assert!(entry.verify_integrity());

        // Test corrupted entry
        let mut corrupted = entry.clone();
        corrupted.sequence_number = 999;
        assert!(!corrupted.verify_integrity());
    }

    #[test]
    fn test_transaction_context() {
        let mut tx = TransactionContext::new();
        assert!(tx.operations.is_empty());

        let operation = WALOperation::CreateLease {
            lease: create_test_lease(),
            timestamp: Utc::now(),
        };
        
        tx.add_operation(operation);
        assert_eq!(tx.operations.len(), 1);
    }

    fn create_test_lease() -> Lease {
        use crate::lease::{ObjectType, Lease};
        use std::collections::HashMap;
        use std::time::Duration;

        Lease::new(
            "test-object".to_string(),
            ObjectType::TemporaryFile,
            "test-service".to_string(),
            Duration::from_secs(300),
            HashMap::new(),
            None,
        )
    }
}