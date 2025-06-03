// src/storage/memory.rs - Fixed implementation matching the Storage trait

use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseFilter, LeaseStats, LeaseState, ObjectType};

use super::Storage;

/// In-memory storage implementation using DashMap for thread-safe access
pub struct MemoryStorage {
    leases: DashMap<String, Lease>,
    stats: Arc<MemoryStorageStats>,
}

#[derive(Debug, Default)]
struct MemoryStorageStats {
    operations_total: AtomicU64,
    get_operations: AtomicU64,
    list_operations: AtomicU64,
    create_operations: AtomicU64,
    update_operations: AtomicU64,
    delete_operations: AtomicU64,
}

impl MemoryStorageStats {
    fn increment(&self, operation: &str) {
        self.operations_total.fetch_add(1, Ordering::Relaxed);
        match operation {
            "get" => self.get_operations.fetch_add(1, Ordering::Relaxed),
            "list" => self.list_operations.fetch_add(1, Ordering::Relaxed),
            "create" => self.create_operations.fetch_add(1, Ordering::Relaxed),
            "update" => self.update_operations.fetch_add(1, Ordering::Relaxed),
            "delete" => self.delete_operations.fetch_add(1, Ordering::Relaxed),
            _ => 0,
        };
    }
}

impl MemoryStorage {
    /// Create a new in-memory storage instance
    pub fn new() -> Self {
        Self {
            leases: DashMap::new(),
            stats: Arc::new(MemoryStorageStats::default()),
        }
    }

    /// Get internal statistics for monitoring
    pub fn get_internal_stats(&self) -> HashMap<String, u64> {
        let mut stats = HashMap::new();
        stats.insert("total_leases".to_string(), self.leases.len() as u64);
        stats.insert("operations_total".to_string(), self.stats.operations_total.load(Ordering::Relaxed));
        stats.insert("get_operations".to_string(), self.stats.get_operations.load(Ordering::Relaxed));
        stats.insert("list_operations".to_string(), self.stats.list_operations.load(Ordering::Relaxed));
        stats.insert("create_operations".to_string(), self.stats.create_operations.load(Ordering::Relaxed));
        stats.insert("update_operations".to_string(), self.stats.update_operations.load(Ordering::Relaxed));
        stats.insert("delete_operations".to_string(), self.stats.delete_operations.load(Ordering::Relaxed));
        stats
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
        self.stats.increment("create");
        
        if self.leases.contains_key(&lease.lease_id) {
            return Err(GCError::Internal(format!(
                "Lease with ID {} already exists",
                lease.lease_id
            )));
        }

        self.leases.insert(lease.lease_id.clone(), lease);
        Ok(())
    }

    async fn get_lease(&self, lease_id: &str) -> Result<Option<Lease>> {
        self.stats.increment("get");
        Ok(self.leases.get(lease_id).map(|entry| entry.clone()))
    }

    async fn update_lease(&self, lease: Lease) -> Result<()> {
        self.stats.increment("update");
        
        if !self.leases.contains_key(&lease.lease_id) {
            return Err(GCError::LeaseNotFound {
                lease_id: lease.lease_id,
            });
        }

        self.leases.insert(lease.lease_id.clone(), lease);
        Ok(())
    }

    async fn delete_lease(&self, lease_id: &str) -> Result<()> {
        self.stats.increment("delete");
        
        match self.leases.remove(lease_id) {
            Some(_) => Ok(()),
            None => Err(GCError::LeaseNotFound {
                lease_id: lease_id.to_string(),
            }),
        }
    }

    async fn list_leases(&self, filter: Option<LeaseFilter>, limit: Option<usize>) -> Result<Vec<Lease>> {
        self.stats.increment("list");
        
        let mut leases: Vec<Lease> = self.leases
            .iter()
            .map(|entry| entry.value().clone())
            .collect();

        // Apply filter if provided
        if let Some(filter) = filter {
            leases.retain(|lease| filter.matches(lease));
        }

        // Sort by creation time (newest first)
        leases.sort_by(|a, b| b.created_at.cmp(&a.created_at));

        // Apply limit if provided
        if let Some(limit) = limit {
            leases.truncate(limit);
        }

        Ok(leases)
    }

    async fn count_leases(&self, filter: LeaseFilter) -> Result<usize> {
        self.stats.increment("list");
        
        let count = self.leases
            .iter()
            .filter(|entry| filter.matches(entry.value()))
            .count();
        
        Ok(count)
    }

    async fn get_expired_leases(&self, grace_period: std::time::Duration) -> Result<Vec<Lease>> {
        self.stats.increment("list");
        
        let leases: Vec<Lease> = self.leases
            .iter()
            .filter_map(|entry| {
                let lease = entry.value();
                if lease.state == LeaseState::Active && lease.is_expired() {
                    // Check if grace period has also passed
                    let grace_expires_at = lease.expires_at + chrono::Duration::from_std(grace_period).unwrap();
                    if Utc::now() > grace_expires_at {
                        Some(lease.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect();

        Ok(leases)
    }

    async fn get_leases_by_service(&self, service_id: &str) -> Result<Vec<Lease>> {
        self.stats.increment("list");
        
        let leases: Vec<Lease> = self.leases
            .iter()
            .filter_map(|entry| {
                let lease = entry.value();
                if lease.service_id == service_id {
                    Some(lease.clone())
                } else {
                    None
                }
            })
            .collect();

        Ok(leases)
    }

    async fn get_leases_by_type(&self, object_type: ObjectType) -> Result<Vec<Lease>> {
        self.stats.increment("list");
        
        let leases: Vec<Lease> = self.leases
            .iter()
            .filter_map(|entry| {
                let lease = entry.value();
                if lease.object_type == object_type {
                    Some(lease.clone())
                } else {
                    None
                }
            })
            .collect();

        Ok(leases)
    }

    async fn get_stats(&self) -> Result<LeaseStats> {
        self.stats.increment("list");
        
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
            *stats.leases_by_service.entry(lease.service_id.clone()).or_insert(0) += 1;

            // Count by type
            let type_name = format!("{:?}", lease.object_type);
            *stats.leases_by_type.entry(type_name).or_insert(0) += 1;

            // Calculate average duration
            let lease_duration = lease.expires_at - lease.created_at;
            total_duration = total_duration + lease_duration;

            // Calculate average renewals
            total_renewals += lease.renewal_count as u64;
        }

        if stats.total_leases > 0 {
            stats.average_lease_duration = Some(total_duration / stats.total_leases as i32);
            stats.average_renewal_count = total_renewals as f64 / stats.total_leases as f64;
        }

        Ok(stats)
    }

    async fn health_check(&self) -> Result<bool> {
        // For memory storage, we're healthy if we can access the data structure
        Ok(true)
    }

    async fn cleanup(&self) -> Result<usize> {
        self.stats.increment("delete");
        
        let now = Utc::now();
        let mut cleaned_count = 0;
        
        // Find and remove expired leases that are in Released state
        let keys_to_remove: Vec<String> = self.leases
            .iter()
            .filter_map(|entry| {
                let lease = entry.value();
                if lease.state == LeaseState::Released || 
                   (lease.state == LeaseState::Expired && lease.expires_at < now) {
                    Some(lease.lease_id.clone())
                } else {
                    None
                }
            })
            .collect();

        for key in keys_to_remove {
            if self.leases.remove(&key).is_some() {
                cleaned_count += 1;
            }
        }

        Ok(cleaned_count)
    }

    async fn count_active_leases_for_service(&self, service_id: &str) -> Result<usize> {
        self.stats.increment("list");
        
        let count = self.leases
            .iter()
            .filter(|entry| {
                let lease = entry.value();
                lease.service_id == service_id && 
                lease.state == LeaseState::Active && 
                !lease.is_expired()
            })
            .count();

        Ok(count)
    }

    async fn mark_lease_expired(&self, lease_id: &str) -> Result<()> {
        self.stats.increment("update");
        
        let mut lease = self.leases.get_mut(lease_id)
            .ok_or_else(|| GCError::LeaseNotFound {
                lease_id: lease_id.to_string(),
            })?;

        lease.expire();
        Ok(())
    }

    async fn mark_lease_released(&self, lease_id: &str) -> Result<()> {
        self.stats.increment("update");
        
        let mut lease = self.leases.get_mut(lease_id)
            .ok_or_else(|| GCError::LeaseNotFound {
                lease_id: lease_id.to_string(),
            })?;

        lease.release();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lease::{CleanupConfig, ObjectType};
    use std::collections::HashMap;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_basic_crud_operations() {
        let storage = MemoryStorage::new();

        // Create a test lease
        let lease = Lease::new(
            "test-object".to_string(),
            ObjectType::TemporaryFile,
            "test-service".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        let lease_id = lease.lease_id.clone();

        // Test create
        storage.create_lease(lease.clone()).await.unwrap();

        // Test get
        let retrieved = storage.get_lease(&lease_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().lease_id, lease_id);

        // Test update
        let mut updated_lease = lease.clone();
        updated_lease.renewal_count = 1;
        storage.update_lease(updated_lease).await.unwrap();

        let retrieved = storage.get_lease(&lease_id).await.unwrap().unwrap();
        assert_eq!(retrieved.renewal_count, 1);

        // Test delete
        storage.delete_lease(&lease_id).await.unwrap();

        // Verify deletion
        let deleted = storage.get_lease(&lease_id).await.unwrap();
        assert!(deleted.is_none());
    }

    #[tokio::test]
    async fn test_list_leases_with_filter() {
        let storage = MemoryStorage::new();

        // Create test leases
        let lease1 = Lease::new(
            "object1".to_string(),
            ObjectType::TemporaryFile,
            "service1".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        let lease2 = Lease::new(
            "object2".to_string(),
            ObjectType::DatabaseRow,
            "service2".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        storage.create_lease(lease1).await.unwrap();
        storage.create_lease(lease2).await.unwrap();

        // Test list all
        let all_leases = storage.list_leases(None, None).await.unwrap();
        assert_eq!(all_leases.len(), 2);

        // Test filter by service
        let filter = LeaseFilter {
            service_id: Some("service1".to_string()),
            ..Default::default()
        };
        let filtered_leases = storage.list_leases(Some(filter), None).await.unwrap();
        assert_eq!(filtered_leases.len(), 1);
        assert_eq!(filtered_leases[0].service_id, "service1");

        // Test limit
        let limited_leases = storage.list_leases(None, Some(1)).await.unwrap();
        assert_eq!(limited_leases.len(), 1);
    }

    #[tokio::test]
    async fn test_expired_leases() {
        let storage = MemoryStorage::new();

        // Create a lease that expires immediately
        let lease = Lease::new(
            "expiring-object".to_string(),
            ObjectType::TemporaryFile,
            "test-service".to_string(),
            std::time::Duration::from_millis(1), // Very short duration
            HashMap::new(),
            None,
        );

        storage.create_lease(lease).await.unwrap();

        // Wait for expiration
        sleep(std::time::Duration::from_millis(10)).await;

        // Test get expired leases with no grace period
        let expired = storage.get_expired_leases(std::time::Duration::from_secs(0)).await.unwrap();
        assert_eq!(expired.len(), 1);

        // Test with long grace period (should not return expired lease)
        let expired_with_grace = storage.get_expired_leases(std::time::Duration::from_secs(3600)).await.unwrap();
        assert_eq!(expired_with_grace.len(), 0);
    }

    #[tokio::test]
    async fn test_service_and_type_queries() {
        let storage = MemoryStorage::new();

        let lease1 = Lease::new(
            "object1".to_string(),
            ObjectType::TemporaryFile,
            "service1".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        let lease2 = Lease::new(
            "object2".to_string(),
            ObjectType::TemporaryFile,
            "service1".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        let lease3 = Lease::new(
            "object3".to_string(),
            ObjectType::DatabaseRow,
            "service2".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        storage.create_lease(lease1).await.unwrap();
        storage.create_lease(lease2).await.unwrap();
        storage.create_lease(lease3).await.unwrap();

        // Test get by service
        let service1_leases = storage.get_leases_by_service("service1").await.unwrap();
        assert_eq!(service1_leases.len(), 2);

        // Test get by type
        let file_leases = storage.get_leases_by_type(ObjectType::TemporaryFile).await.unwrap();
        assert_eq!(file_leases.len(), 2);

        let db_leases = storage.get_leases_by_type(ObjectType::DatabaseRow).await.unwrap();
        assert_eq!(db_leases.len(), 1);
    }

    #[tokio::test]
    async fn test_lease_state_management() {
        let storage = MemoryStorage::new();

        let lease = Lease::new(
            "test-object".to_string(),
            ObjectType::TemporaryFile,
            "test-service".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        let lease_id = lease.lease_id.clone();
        storage.create_lease(lease).await.unwrap();

        // Test mark expired
        storage.mark_lease_expired(&lease_id).await.unwrap();
        let expired_lease = storage.get_lease(&lease_id).await.unwrap().unwrap();
        assert_eq!(expired_lease.state, LeaseState::Expired);

        // Test mark released
        storage.mark_lease_released(&lease_id).await.unwrap();
        let released_lease = storage.get_lease(&lease_id).await.unwrap().unwrap();
        assert_eq!(released_lease.state, LeaseState::Released);
    }

    #[tokio::test]
    async fn test_statistics() {
        let storage = MemoryStorage::new();

        // Create test leases with different states
        let mut lease1 = Lease::new(
            "object1".to_string(),
            ObjectType::TemporaryFile,
            "service1".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        let mut lease2 = Lease::new(
            "object2".to_string(),
            ObjectType::DatabaseRow,
            "service2".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );
        lease2.release();

        storage.create_lease(lease1).await.unwrap();
        storage.create_lease(lease2).await.unwrap();

        let stats = storage.get_stats().await.unwrap();
        assert_eq!(stats.total_leases, 2);
        assert_eq!(stats.active_leases, 1);
        assert_eq!(stats.released_leases, 1);
        assert_eq!(stats.leases_by_service.len(), 2);
        assert_eq!(stats.leases_by_type.len(), 2);
    }

    #[tokio::test]
    async fn test_cleanup() {
        let storage = MemoryStorage::new();

        // Create a released lease
        let mut lease = Lease::new(
            "test-object".to_string(),
            ObjectType::TemporaryFile,
            "test-service".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );
        lease.release();

        storage.create_lease(lease).await.unwrap();

        // Verify lease exists
        let all_leases = storage.list_leases(None, None).await.unwrap();
        assert_eq!(all_leases.len(), 1);

        // Run cleanup
        let cleaned = storage.cleanup().await.unwrap();
        assert_eq!(cleaned, 1);

        // Verify lease was removed
        let remaining_leases = storage.list_leases(None, None).await.unwrap();
        assert_eq!(remaining_leases.len(), 0);
    }

    #[tokio::test]
    async fn test_count_active_leases_for_service() {
        let storage = MemoryStorage::new();

        // Create active lease
        let lease1 = Lease::new(
            "object1".to_string(),
            ObjectType::TemporaryFile,
            "service1".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        // Create released lease for same service
        let mut lease2 = Lease::new(
            "object2".to_string(),
            ObjectType::TemporaryFile,
            "service1".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );
        lease2.release();

        // Create active lease for different service
        let lease3 = Lease::new(
            "object3".to_string(),
            ObjectType::TemporaryFile,
            "service2".to_string(),
            std::time::Duration::from_secs(300),
            HashMap::new(),
            None,
        );

        storage.create_lease(lease1).await.unwrap();
        storage.create_lease(lease2).await.unwrap();
        storage.create_lease(lease3).await.unwrap();

        let count = storage.count_active_leases_for_service("service1").await.unwrap();
        assert_eq!(count, 1); // Only one active lease for service1
    }

    #[tokio::test]
    async fn test_health_check() {
        let storage = MemoryStorage::new();
        let healthy = storage.health_check().await.unwrap();
        assert!(healthy);
    }
}