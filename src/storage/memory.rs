// src/storage/memory.rs - In-memory storage implementation

use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use tracing::debug;

use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseFilter, LeaseState, LeaseStats};

use super::{DetailedStorageStats, ExtendedStorage, Storage, StorageInfo};

/// In-memory storage implementation using DashMap for thread-safe operations
#[derive(Clone)]
pub struct MemoryStorage {
    leases: Arc<DashMap<String, Lease>>,
    stats: Arc<DashMap<String, u64>>, // For tracking operation counts
}

impl MemoryStorage {
    /// Create a new in-memory storage instance
    pub fn new() -> Self {
        Self {
            leases: Arc::new(DashMap::new()),
            stats: Arc::new(DashMap::new()),
        }
    }

    /// Get the current number of stored leases
    pub fn len(&self) -> usize {
        self.leases.len()
    }

    /// Check if storage is empty
    pub fn is_empty(&self) -> bool {
        self.leases.is_empty()
    }

    /// Clear all leases (useful for testing)
    #[cfg(test)]
    pub fn clear(&self) {
        self.leases.clear();
        self.stats.clear();
    }

    /// Increment operation counter
    fn increment_stat(&self, operation: &str) {
        let mut counter = self.stats.entry(operation.to_string()).or_insert(0);
        *counter += 1;
    }

    /// Get operation count
    fn get_stat(&self, operation: &str) -> u64 {
        self.stats.get(operation).map(|v| *v).unwrap_or(0)
    }
}

impl Default for MemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn create_lease(&self, lease: Lease) -> Result<()> {
        let lease_id = lease.lease_id.clone();

        // Check if lease already exists
        if self.leases.contains_key(&lease_id) {
            return Err(GCError::Internal(format!(
                "Lease {} already exists",
                lease_id
            )));
        }

        self.leases.insert(lease_id.clone(), lease);
        self.increment_stat("create_lease");

        debug!("Created lease {} in memory storage", lease_id);
        Ok(())
    }

    async fn get_lease(&self, lease_id: &str) -> Result<Option<Lease>> {
        self.increment_stat("get_lease");
        Ok(self.leases.get(lease_id).map(|entry| entry.clone()))
    }

    async fn update_lease(&self, lease: Lease) -> Result<()> {
        let lease_id = lease.lease_id.clone();

        if self.leases.contains_key(&lease_id) {
            self.leases.insert(lease_id.clone(), lease);
            self.increment_stat("update_lease");
            debug!("Updated lease {} in memory storage", lease_id);
            Ok(())
        } else {
            Err(GCError::LeaseNotFound { lease_id })
        }
    }

    async fn delete_lease(&self, lease_id: &str) -> Result<()> {
        if self.leases.remove(lease_id).is_some() {
            self.increment_stat("delete_lease");
            debug!("Deleted lease {} from memory storage", lease_id);
            Ok(())
        } else {
            Err(GCError::LeaseNotFound {
                lease_id: lease_id.to_string(),
            })
        }
    }

    async fn list_leases(
        &self,
        filter: LeaseFilter,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> Result<Vec<Lease>> {
        self.increment_stat("list_leases");

        let mut leases: Vec<Lease> = self
            .leases
            .iter()
            .map(|entry| entry.value().clone())
            .filter(|lease| filter.matches(lease))
            .collect();

        // Sort by creation time (newest first)
        leases.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        // Apply offset and limit
        let start = offset.unwrap_or(0);
        let end = if let Some(limit) = limit {
            std::cmp::min(start + limit, leases.len())
        } else {
            leases.len()
        };

        if start >= leases.len() {
            Ok(vec![])
        } else {
            Ok(leases[start..end].to_vec())
        }
    }

    async fn count_leases(&self, filter: LeaseFilter) -> Result<usize> {
        self.increment_stat("count_leases");

        let count = self
            .leases
            .iter()
            .filter(|entry| filter.matches(entry.value()))
            .count();
        Ok(count)
    }

    async fn get_expired_leases(&self, grace_period: std::time::Duration) -> Result<Vec<Lease>> {
        self.increment_stat("get_expired_leases");

        let expired_leases: Vec<Lease> = self
            .leases
            .iter()
            .map(|entry| entry.value().clone())
            .filter(|lease| {
                // A lease is considered expired if:
                // 1. It's past its expiration time (is_expired()), OR
                // 2. It's explicitly marked as expired state
                // AND it should be cleaned up (past grace period)
                let is_expired = lease.is_expired() || lease.state == LeaseState::Expired;
                is_expired && lease.should_cleanup(grace_period)
            })
            .collect();

        debug!(
            "Found {} expired leases ready for cleanup",
            expired_leases.len()
        );
        Ok(expired_leases)
    }

    async fn get_stats(&self) -> Result<LeaseStats> {
        self.increment_stat("get_stats");

        let mut stats = LeaseStats::default();
        let mut total_duration = chrono::Duration::zero();
        let mut total_renewals = 0u64;

        for entry in self.leases.iter() {
            let lease = entry.value();
            stats.total_leases += 1;

            match lease.state {
                LeaseState::Active => {
                    if lease.is_expired() {
                        stats.expired_leases += 1;
                    } else {
                        stats.active_leases += 1;
                    }
                }
                LeaseState::Expired => stats.expired_leases += 1,
                LeaseState::Released => stats.released_leases += 1,
            }

            // Count by service
            *stats
                .leases_by_service
                .entry(lease.service_id.clone())
                .or_insert(0) += 1;

            // Count by type
            let type_name = format!("{:?}", lease.object_type);
            *stats.leases_by_type.entry(type_name).or_insert(0) += 1;

            // Calculate averages
            total_duration += lease.expires_at - lease.created_at;
            total_renewals += lease.renewal_count as u64;
        }

        if stats.total_leases > 0 {
            stats.average_lease_duration = Some(total_duration / stats.total_leases as i32);
            stats.average_renewal_count = total_renewals as f64 / stats.total_leases as f64;
        }

        debug!(
            "Memory storage stats: {} total, {} active, {} expired",
            stats.total_leases, stats.active_leases, stats.expired_leases
        );

        Ok(stats)
    }

    async fn cleanup(&self) -> Result<()> {
        self.increment_stat("cleanup");

        // Remove expired leases that have passed their grace period
        let grace_period = std::time::Duration::from_secs(300); // 5 minutes default
        let to_remove: Vec<String> = self
            .leases
            .iter()
            .filter_map(|entry| {
                let lease = entry.value();
                if lease.state == LeaseState::Expired {
                    // Check if grace period has passed
                    if lease.should_cleanup(grace_period) {
                        Some(lease.lease_id.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        let removed_count = to_remove.len();
        for lease_id in to_remove {
            self.leases.remove(&lease_id);
        }

        if removed_count > 0 {
            debug!(
                "Cleaned up {} expired leases from memory storage",
                removed_count
            );
        }

        Ok(())
    }
}

#[async_trait]
impl ExtendedStorage for MemoryStorage {
    async fn get_info(&self) -> Result<StorageInfo> {
        Ok(StorageInfo::memory())
    }

    async fn migrate(&self) -> Result<()> {
        // Memory storage doesn't need migrations
        Ok(())
    }

    async fn create_indexes(&self) -> Result<()> {
        // Memory storage doesn't use indexes
        Ok(())
    }

    async fn get_detailed_stats(&self) -> Result<DetailedStorageStats> {
        // Estimate memory usage (very rough)
        let lease_count = self.leases.len();
        let estimated_size_per_lease = 512; // Rough estimate in bytes
        let estimated_total_size = (lease_count * estimated_size_per_lease) as u64;

        Ok(DetailedStorageStats {
            total_storage_size_bytes: Some(estimated_total_size),
            index_size_bytes: Some(0),   // No indexes in memory
            connection_pool_stats: None, // No connection pool
            query_performance: Some(super::QueryPerformanceStats {
                average_query_time_ms: 0.1, // Very fast for memory operations
                slowest_query_time_ms: 1,
                total_queries: self.get_stat("get_lease") + self.get_stat("list_leases"),
                failed_queries: 0, // Memory operations rarely fail
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lease::{Lease, LeaseFilter, ObjectType};
    use std::collections::HashMap;
    use std::time::Duration;

    fn create_test_lease(id: &str, service_id: &str) -> Lease {
        Lease::new(
            format!("object-{}", id),
            ObjectType::DatabaseRow,
            service_id.to_string(),
            Duration::from_secs(300),
            HashMap::new(),
            None,
        )
    }

    #[tokio::test]
    async fn test_create_and_get_lease() {
        let storage = MemoryStorage::new();
        let lease = create_test_lease("1", "service-1");
        let lease_id = lease.lease_id.clone();

        // Create lease
        storage.create_lease(lease.clone()).await.unwrap();

        // Get lease
        let retrieved = storage.get_lease(&lease_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().lease_id, lease_id);
    }

    #[tokio::test]
    async fn test_update_lease() {
        let storage = MemoryStorage::new();
        let mut lease = create_test_lease("1", "service-1");
        let lease_id = lease.lease_id.clone();

        // Create lease
        storage.create_lease(lease.clone()).await.unwrap();

        // Update lease
        lease.renewal_count = 5;
        storage.update_lease(lease.clone()).await.unwrap();

        // Verify update
        let updated = storage.get_lease(&lease_id).await.unwrap().unwrap();
        assert_eq!(updated.renewal_count, 5);
    }

    #[tokio::test]
    async fn test_delete_lease() {
        let storage = MemoryStorage::new();
        let lease = create_test_lease("1", "service-1");
        let lease_id = lease.lease_id.clone();

        // Create lease
        storage.create_lease(lease).await.unwrap();

        // Delete lease
        storage.delete_lease(&lease_id).await.unwrap();

        // Verify deletion
        let retrieved = storage.get_lease(&lease_id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_list_leases_with_filter() {
        let storage = MemoryStorage::new();

        // Create leases for different services
        let lease1 = create_test_lease("1", "service-1");
        let lease2 = create_test_lease("2", "service-1");
        let lease3 = create_test_lease("3", "service-2");

        storage.create_lease(lease1).await.unwrap();
        storage.create_lease(lease2).await.unwrap();
        storage.create_lease(lease3).await.unwrap();

        // Filter by service
        let filter = LeaseFilter {
            service_id: Some("service-1".to_string()),
            ..Default::default()
        };

        let leases = storage.list_leases(filter, None, None).await.unwrap();
        assert_eq!(leases.len(), 2);
        assert!(leases.iter().all(|l| l.service_id == "service-1"));
    }

    #[tokio::test]
    async fn test_count_leases() {
        let storage = MemoryStorage::new();

        // Create leases
        storage
            .create_lease(create_test_lease("1", "service-1"))
            .await
            .unwrap();
        storage
            .create_lease(create_test_lease("2", "service-1"))
            .await
            .unwrap();

        let filter = LeaseFilter {
            service_id: Some("service-1".to_string()),
            ..Default::default()
        };

        let count = storage.count_leases(filter).await.unwrap();
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn test_get_stats() {
        let storage = MemoryStorage::new();

        // Create leases
        storage
            .create_lease(create_test_lease("1", "service-1"))
            .await
            .unwrap();
        storage
            .create_lease(create_test_lease("2", "service-2"))
            .await
            .unwrap();

        let stats = storage.get_stats().await.unwrap();
        assert_eq!(stats.total_leases, 2);
        assert_eq!(stats.active_leases, 2);
        assert_eq!(stats.leases_by_service.len(), 2);
    }

    #[tokio::test]
    async fn test_cleanup() {
        let storage = MemoryStorage::new();

        // Create an expired lease
        let mut lease = create_test_lease("1", "service-1");

        // CRITICAL FIX: Set expiration far enough in the past
        // The cleanup() method uses a 5-minute (300 second) grace period internally
        // So we need to expire the lease more than 5 minutes ago
        lease.expires_at = chrono::Utc::now() - chrono::Duration::minutes(10); // 10 minutes ago
        lease.expire(); // Set state to Expired

        storage.create_lease(lease).await.unwrap();

        // Run cleanup - this uses the internal grace period of 300 seconds
        storage.cleanup().await.unwrap();

        // After cleanup, the lease should be removed since it expired > 5 minutes ago
        let stats = storage.get_stats().await.unwrap();
        assert_eq!(
            stats.total_leases, 0,
            "Cleanup should have removed the expired lease"
        );
    }

    #[tokio::test]
    async fn test_extended_storage_features() {
        let storage = MemoryStorage::new();

        // Test storage info
        let info = storage.get_info().await.unwrap();
        assert_eq!(info.backend_type, "memory");
        assert!(!info.supports_transactions);

        // Test detailed stats
        let detailed_stats = storage.get_detailed_stats().await.unwrap();
        assert!(detailed_stats.total_storage_size_bytes.is_some());
        assert!(detailed_stats.query_performance.is_some());
    }
}
