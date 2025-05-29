use async_trait::async_trait;
use dashmap::DashMap;
use std::sync::Arc;
use std::str::FromStr;

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseFilter, LeaseStats, LeaseState, ObjectType};

#[cfg(feature = "postgres")]
use sqlx::{PgPool, Row, postgres::PgConnectOptions, query, query_as};

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

// PostgreSQL storage implementation
#[cfg(feature = "postgres")]
#[derive(Clone)]
pub struct PostgresStorage {
    pool: PgPool,
}

#[cfg(feature = "postgres")]
impl PostgresStorage {
    pub async fn new(database_url: &str, max_connections: u32) -> Result<Self> {
        let options = PgConnectOptions::from_str(database_url)
            .map_err(|e| GCError::Database(e.to_string()))?
            .application_name("distributed-gc-sidecar");
            
        let pool = sqlx::pool::PoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await
            .map_err(|e| GCError::Database(e.to_string()))?;

        // Run migrations
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| GCError::Database(format!("Migration failed: {}", e)))?;

        Ok(Self { pool })
    }

    fn object_type_to_int(obj_type: &ObjectType) -> i32 {
        match obj_type {
            ObjectType::Unknown => 0,
            ObjectType::DatabaseRow => 1,
            ObjectType::BlobStorage => 2,
            ObjectType::TemporaryFile => 3,
            ObjectType::WebsocketSession => 4,
            ObjectType::CacheEntry => 5,
            ObjectType::Custom => 6,
        }
    }

    fn int_to_object_type(val: i32) -> ObjectType {
        match val {
            1 => ObjectType::DatabaseRow,
            2 => ObjectType::BlobStorage,
            3 => ObjectType::TemporaryFile,
            4 => ObjectType::WebsocketSession,
            5 => ObjectType::CacheEntry,
            6 => ObjectType::Custom,
            _ => ObjectType::Unknown,
        }
    }

    fn lease_state_to_int(state: &LeaseState) -> i32 {
        match state {
            LeaseState::Active => 0,
            LeaseState::Expired => 1,
            LeaseState::Released => 2,
        }
    }

    fn int_to_lease_state(val: i32) -> LeaseState {
        match val {
            1 => LeaseState::Expired,
            2 => LeaseState::Released,
            _ => LeaseState::Active,
        }
    }

    fn lease_from_row(row: &sqlx::postgres::PgRow) -> Result<Lease> {
        use sqlx::Row;
        
        let object_type = Self::int_to_object_type(
            row.try_get("object_type")
                .map_err(|e| GCError::Database(e.to_string()))?
        );

        let state = Self::int_to_lease_state(
            row.try_get("state")
                .map_err(|e| GCError::Database(e.to_string()))?
        );

        let metadata_json: serde_json::Value = row.try_get("metadata")
            .map_err(|e| GCError::Database(e.to_string()))?;
        let metadata: std::collections::HashMap<String, String> = 
            serde_json::from_value(metadata_json)
                .unwrap_or_default();

        let cleanup_config_json: Option<serde_json::Value> = row.try_get("cleanup_config")
            .map_err(|e| GCError::Database(e.to_string()))?;
        let cleanup_config = cleanup_config_json
            .and_then(|v| serde_json::from_value(v).ok());

        Ok(Lease {
            lease_id: row.try_get("lease_id")
                .map_err(|e| GCError::Database(e.to_string()))?,
            object_id: row.try_get("object_id")
                .map_err(|e| GCError::Database(e.to_string()))?,
            object_type,
            service_id: row.try_get("service_id")
                .map_err(|e| GCError::Database(e.to_string()))?,
            state,
            created_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                .map_err(|e| GCError::Database(e.to_string()))?,
            expires_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("expires_at")
                .map_err(|e| GCError::Database(e.to_string()))?,
            last_renewed_at: row.try_get::<chrono::DateTime<chrono::Utc>, _>("last_renewed_at")
                .map_err(|e| GCError::Database(e.to_string()))?,
            metadata,
            cleanup_config,
            renewal_count: row.try_get::<i32, _>("renewal_count")
                .map_err(|e| GCError::Database(e.to_string()))? as u32,
        })
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl Storage for PostgresStorage {
    async fn create_lease(&self, lease: Lease) -> Result<()> {
        let metadata_json = serde_json::to_value(&lease.metadata)
            .map_err(|e| GCError::Serialization(e))?;
        let cleanup_config_json = lease.cleanup_config
            .as_ref()
            .map(|c| serde_json::to_value(c))
            .transpose()
            .map_err(|e| GCError::Serialization(e))?;

        // Use regular query instead of query! macro to avoid compile-time validation
        sqlx::query(
            r#"
            INSERT INTO leases (
                lease_id, object_id, object_type, service_id, state,
                created_at, expires_at, last_renewed_at, metadata,
                cleanup_config, renewal_count
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "#
        )
        .bind(&lease.lease_id)
        .bind(&lease.object_id)
        .bind(Self::object_type_to_int(&lease.object_type))
        .bind(&lease.service_id)
        .bind(Self::lease_state_to_int(&lease.state))
        .bind(&lease.created_at)
        .bind(&lease.expires_at)
        .bind(&lease.last_renewed_at)
        .bind(&metadata_json)
        .bind(&cleanup_config_json)
        .bind(lease.renewal_count as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| GCError::Database(e.to_string()))?;

        Ok(())
    }

    async fn get_lease(&self, lease_id: &str) -> Result<Option<Lease>> {
        let row = sqlx::query("SELECT * FROM leases WHERE lease_id = $1")
            .bind(lease_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| GCError::Database(e.to_string()))?;

        match row {
            Some(row) => Ok(Some(Self::lease_from_row(&row)?)),
            None => Ok(None),
        }
    }

    async fn update_lease(&self, lease: Lease) -> Result<()> {
        let metadata_json = serde_json::to_value(&lease.metadata)
            .map_err(|e| GCError::Serialization(e))?;
        let cleanup_config_json = lease.cleanup_config
            .as_ref()
            .map(|c| serde_json::to_value(c))
            .transpose()
            .map_err(|e| GCError::Serialization(e))?;

        let result = sqlx::query(
            r#"
            UPDATE leases 
            SET object_id = $2, object_type = $3, service_id = $4, state = $5,
                expires_at = $6, last_renewed_at = $7, metadata = $8,
                cleanup_config = $9, renewal_count = $10
            WHERE lease_id = $1
            "#
        )
        .bind(&lease.lease_id)
        .bind(&lease.object_id)
        .bind(Self::object_type_to_int(&lease.object_type))
        .bind(&lease.service_id)
        .bind(Self::lease_state_to_int(&lease.state))
        .bind(&lease.expires_at)
        .bind(&lease.last_renewed_at)
        .bind(&metadata_json)
        .bind(&cleanup_config_json)
        .bind(lease.renewal_count as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| GCError::Database(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(GCError::LeaseNotFound { lease_id: lease.lease_id });
        }

        Ok(())
    }

    async fn delete_lease(&self, lease_id: &str) -> Result<()> {
        let result = sqlx::query("DELETE FROM leases WHERE lease_id = $1")
            .bind(lease_id)
            .execute(&self.pool)
            .await
            .map_err(|e| GCError::Database(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(GCError::LeaseNotFound { lease_id: lease_id.to_string() });
        }

        Ok(())
    }

    async fn list_leases(&self, filter: LeaseFilter, limit: Option<usize>, offset: Option<usize>) -> Result<Vec<Lease>> {
        // Build query parts
        let mut where_conditions = Vec::new();
        let mut params: Vec<Box<dyn sqlx::Encode<'_, sqlx::Postgres> + Send + Sync>> = Vec::new();
        let mut param_count = 0;

        if let Some(ref service_id) = filter.service_id {
            param_count += 1;
            where_conditions.push(format!("service_id = ${}", param_count));
            params.push(Box::new(service_id.clone()));
        }

        if let Some(ref object_type) = filter.object_type {
            param_count += 1;
            where_conditions.push(format!("object_type = ${}", param_count));
            params.push(Box::new(Self::object_type_to_int(object_type)));
        }

        if let Some(ref state) = filter.state {
            param_count += 1;
            where_conditions.push(format!("state = ${}", param_count));
            params.push(Box::new(Self::lease_state_to_int(state)));
        }

        // Build complete query
        let mut query_str = "SELECT * FROM leases".to_string();
        
        if !where_conditions.is_empty() {
            query_str.push_str(" WHERE ");
            query_str.push_str(&where_conditions.join(" AND "));
        }
        
        query_str.push_str(" ORDER BY created_at DESC");

        if let Some(limit) = limit {
            param_count += 1;
            query_str.push_str(&format!(" LIMIT ${}", param_count));
        }

        if let Some(offset) = offset {
            param_count += 1;
            query_str.push_str(&format!(" OFFSET ${}", param_count));
        }

        // Execute query with manual parameter binding
        let mut query = sqlx::query(&query_str);
        
        if let Some(ref service_id) = filter.service_id {
            query = query.bind(service_id);
        }
        if let Some(ref object_type) = filter.object_type {
            query = query.bind(Self::object_type_to_int(object_type));
        }
        if let Some(ref state) = filter.state {
            query = query.bind(Self::lease_state_to_int(state));
        }
        if let Some(limit) = limit {
            query = query.bind(limit as i64);
        }
        if let Some(offset) = offset {
            query = query.bind(offset as i64);
        }

        let rows = query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| GCError::Database(e.to_string()))?;

        let mut leases = Vec::new();
        for row in rows {
            leases.push(Self::lease_from_row(&row)?);
        }

        Ok(leases)
    }

    async fn count_leases(&self, filter: LeaseFilter) -> Result<usize> {
        // Build query parts
        let mut where_conditions = Vec::new();

        if let Some(ref service_id) = filter.service_id {
            where_conditions.push("service_id = $1".to_string());
        }

        if let Some(ref object_type) = filter.object_type {
            let param_num = if filter.service_id.is_some() { 2 } else { 1 };
            where_conditions.push(format!("object_type = ${}", param_num));
        }

        if let Some(ref state) = filter.state {
            let param_num = match (filter.service_id.is_some(), filter.object_type.is_some()) {
                (true, true) => 3,
                (true, false) | (false, true) => 2,
                (false, false) => 1,
            };
            where_conditions.push(format!("state = ${}", param_num));
        }

        // Build complete query
        let mut query_str = "SELECT COUNT(*) as count FROM leases".to_string();
        
        if !where_conditions.is_empty() {
            query_str.push_str(" WHERE ");
            query_str.push_str(&where_conditions.join(" AND "));
        }

        // Execute query with manual parameter binding
        let mut query = sqlx::query(&query_str);
        
        if let Some(ref service_id) = filter.service_id {
            query = query.bind(service_id);
        }
        if let Some(ref object_type) = filter.object_type {
            query = query.bind(Self::object_type_to_int(object_type));
        }
        if let Some(ref state) = filter.state {
            query = query.bind(Self::lease_state_to_int(state));
        }

        let row = query
            .fetch_one(&self.pool)
            .await
            .map_err(|e| GCError::Database(e.to_string()))?;

        let count: i64 = row.try_get("count")
            .map_err(|e| GCError::Database(e.to_string()))?;

        Ok(count as usize)
    }

    async fn get_expired_leases(&self, grace_period: std::time::Duration) -> Result<Vec<Lease>> {
        let grace_interval = format!("{} seconds", grace_period.as_secs());
        
        let rows = sqlx::query(&format!(
            "SELECT * FROM leases WHERE state = 0 AND expires_at < NOW() - INTERVAL '{}' ORDER BY expires_at ASC",
            grace_interval
        ))
        .fetch_all(&self.pool)
        .await
        .map_err(|e| GCError::Database(e.to_string()))?;

        let mut leases = Vec::new();
        for row in rows {
            leases.push(Self::lease_from_row(&row)?);
        }

        Ok(leases)
    }

    async fn get_stats(&self) -> Result<LeaseStats> {
        // Get basic stats
        let stats_row = sqlx::query("SELECT * FROM get_lease_statistics()")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| GCError::Database(e.to_string()))?;

        // Get service stats
        let service_rows = sqlx::query("SELECT service_id, current_active_leases FROM service_stats")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| GCError::Database(e.to_string()))?;

        let mut leases_by_service = std::collections::HashMap::new();
        for row in service_rows {
            let service_id: String = row.try_get("service_id")
                .map_err(|e| GCError::Database(e.to_string()))?;
            let count: i32 = row.try_get("current_active_leases")
                .map_err(|e| GCError::Database(e.to_string()))?;
            leases_by_service.insert(service_id, count as usize);
        }

        // Get type stats
        let type_rows = sqlx::query(
            "SELECT object_type, COUNT(*) as count FROM leases WHERE state = 0 GROUP BY object_type"
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| GCError::Database(e.to_string()))?;

        let mut leases_by_type = std::collections::HashMap::new();
        for row in type_rows {
            let object_type: i32 = row.try_get("object_type")
                .map_err(|e| GCError::Database(e.to_string()))?;
            let count: i64 = row.try_get("count")
                .map_err(|e| GCError::Database(e.to_string()))?;
            let type_name = format!("{:?}", Self::int_to_object_type(object_type));
            leases_by_type.insert(type_name, count as usize);
        }

        let total_leases: i64 = stats_row.try_get("total_leases")
            .map_err(|e| GCError::Database(e.to_string()))?;
        let active_leases: i64 = stats_row.try_get("active_leases")
            .map_err(|e| GCError::Database(e.to_string()))?;
        let expired_leases: i64 = stats_row.try_get("expired_leases")
            .map_err(|e| GCError::Database(e.to_string()))?;
        let released_leases: i64 = stats_row.try_get("released_leases")
            .map_err(|e| GCError::Database(e.to_string()))?;

        let avg_duration_hours: Option<f64> = stats_row.try_get("avg_lease_duration_hours").ok();
        let avg_renewal_count: Option<f64> = stats_row.try_get("avg_renewal_count").ok();

        Ok(LeaseStats {
            total_leases: total_leases as usize,
            active_leases: active_leases as usize,
            expired_leases: expired_leases as usize,
            released_leases: released_leases as usize,
            leases_by_service,
            leases_by_type,
            average_lease_duration: avg_duration_hours
                .map(|h| chrono::Duration::hours(h as i64)),
            average_renewal_count: avg_renewal_count.unwrap_or(0.0),
        })
    }

    async fn cleanup(&self) -> Result<()> {
        // Delete expired leases that have been in expired state for more than grace period
        let result = sqlx::query(
            "DELETE FROM leases WHERE state = 1 AND expires_at < NOW() - INTERVAL '5 minutes'"
        )
        .execute(&self.pool)
        .await
        .map_err(|e| GCError::Database(e.to_string()))?;

        // Also clean up old cleanup history
        sqlx::query("SELECT cleanup_old_history()")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| GCError::Database(e.to_string()))?;

        tracing::debug!("Cleaned up {} expired lease records", result.rows_affected());
        Ok(())
    }
}

pub async fn create_storage(config: &Config) -> Result<Arc<dyn Storage + Send + Sync>> {
    match config.storage.backend.as_str() {
        "memory" => Ok(Arc::new(MemoryStorage::new())),
        #[cfg(feature = "postgres")]
        "postgres" => {
            let database_url = config.storage.database_url
                .as_ref()
                .ok_or_else(|| GCError::Configuration("Database URL required for postgres backend".to_string()))?;
            let max_connections = config.storage.max_connections.unwrap_or(10);
            
            let storage = PostgresStorage::new(database_url, max_connections).await?;
            Ok(Arc::new(storage))
        }
        #[cfg(not(feature = "postgres"))]
        "postgres" => Err(GCError::Configuration("Postgres support not compiled in. Enable 'postgres' feature.".to_string())),
        _ => Err(GCError::Configuration(format!("Unknown storage backend: {}", config.storage.backend))),
    }
}