use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseFilter, LeaseStats, LeaseState};

#[async_trait]
pub trait Storage: Send + Sync {
    async fn create_lease(&self, lease: Lease) -> Result<()>;
    async fn get_lease(&self, lease_id: &str) -> Result<Option<Lease>>;
    async fn update_lease(&self, lease: Lease) -> Result<()>;
    async fn delete_lease(&self, lease_id: &str) -> Result<()>;
    async fn list_leases(&self, filter: LeaseFilter, limit: Option<usize>, offset: Option<usize>) -> Result<Vec<Lease>>;
    async fn count_leases(&self, filter: LeaseFilter) -> Result<usize>;
    async fn get_expired_leases(&self, grace_period: std::time::Duration) -> Result<Vec<Lease>>;
    async fn get_stats(&self) -> Result<LeaseStats>;
    async fn cleanup(&self) -> Result<()>;
}

// In-memory storage implementation
#[derive(Clone)]
pub struct MemoryStorage {
    leases: Arc<DashMap<String, Lease>>,
}

impl MemoryStorage {
    pub fn new() -> Self {
        Self {
            leases: Arc::new(DashMap::new()),
        }
    }
}

#[async_trait]
impl Storage for MemoryStorage {
    async fn create_lease(&self, lease: Lease) -> Result<()> {
        let lease_id = lease.lease_id.clone();
        self.leases.insert(lease_id, lease);
        Ok(())
    }
    
    async fn get_lease(&self, lease_id: &str) -> Result<Option<Lease>> {
        Ok(self.leases.get(lease_id).map(|entry| entry.clone()))
    }
    
    async fn update_lease(&self, lease: Lease) -> Result<()> {
        let lease_id = lease.lease_id.clone();
        if self.leases.contains_key(&lease_id) {
            self.leases.insert(lease_id, lease);
            Ok(())
        } else {
            Err(GCError::LeaseNotFound { lease_id })
        }
    }
    
    async fn delete_lease(&self, lease_id: &str) -> Result<()> {
        if self.leases.remove(lease_id).is_some() {
            Ok(())
        } else {
            Err(GCError::LeaseNotFound { lease_id: lease_id.to_string() })
        }
    }
    
    async fn list_leases(&self, filter: LeaseFilter, limit: Option<usize>, offset: Option<usize>) -> Result<Vec<Lease>> {
        let mut leases: Vec<Lease> = self.leases
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
        let count = self.leases
            .iter()
            .filter(|entry| filter.matches(entry.value()))
            .count();
        Ok(count)
    }
    
    async fn get_expired_leases(&self, grace_period: std::time::Duration) -> Result<Vec<Lease>> {
        let expired_leases: Vec<Lease> = self.leases
            .iter()
            .map(|entry| entry.value().clone())
            .filter(|lease| {
                lease.is_expired() && lease.should_cleanup(grace_period)
            })
            .collect();
        
        Ok(expired_leases)
    }
    
    async fn get_stats(&self) -> Result<LeaseStats> {
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
            
            // Calculate averages
            total_duration = total_duration + (lease.expires_at - lease.created_at);
            total_renewals += lease.renewal_count as u64;
        }
        
        if stats.total_leases > 0 {
            stats.average_lease_duration = Some(total_duration / stats.total_leases as i32);
            stats.average_renewal_count = total_renewals as f64 / stats.total_leases as f64;
        }
        
        Ok(stats)
    }
    
    async fn cleanup(&self) -> Result<()> {
        // Remove expired leases that have passed their grace period
        let grace_period = std::time::Duration::from_secs(300); // 5 minutes default
        let to_remove: Vec<String> = self.leases
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
        
        for lease_id in to_remove {
            self.leases.remove(&lease_id);
        }
        
        Ok(())
    }
}

pub async fn create_storage(config: &Config) -> Result<Arc<dyn Storage + Send + Sync>> {
    match config.storage.backend.as_str() {
        "memory" => Ok(Arc::new(MemoryStorage::new())),
        #[cfg(feature = "postgres")]
        "postgres" => {
            // PostgreSQL implementation would go here
            unimplemented!("PostgreSQL storage not implemented in this example");
        }
        #[cfg(not(feature = "postgres"))]
        "postgres" => Err(GCError::Configuration("Postgres support not compiled in".to_string())),
        _ => Err(GCError::Configuration(format!("Unknown storage backend: {}", config.storage.backend))),
    }
}