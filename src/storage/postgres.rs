// src/storage/postgres.rs - PostgreSQL storage implementation

#[cfg(feature = "postgres")]
use async_trait::async_trait;
#[cfg(feature = "postgres")]
use sqlx::{PgPool, Row, postgres::PgConnectOptions, query};
#[cfg(feature = "postgres")]
use std::str::FromStr;
#[cfg(feature = "postgres")]
use tracing::{debug, info, warn};

#[cfg(feature = "postgres")]
use crate::error::{GCError, Result};
#[cfg(feature = "postgres")]
use crate::lease::{Lease, LeaseFilter, LeaseStats, LeaseState, ObjectType};

#[cfg(feature = "postgres")]
use super::{Storage, ExtendedStorage, StorageInfo, DetailedStorageStats, ConnectionPoolStats, QueryPerformanceStats};

/// PostgreSQL storage implementation
#[cfg(feature = "postgres")]
#[derive(Clone)]
pub struct PostgresStorage {
    pool: PgPool,
    max_connections: u32,
}

#[cfg(feature = "postgres")]
impl PostgresStorage {
    /// Create a new PostgreSQL storage instance
    pub async fn new(database_url: &str, max_connections: u32) -> Result<Self> {
        info!("Connecting to PostgreSQL database with {} max connections", max_connections);
        
        let options = PgConnectOptions::from_str(database_url)
            .map_err(|e| GCError::Database(e.to_string()))?
            .application_name("garbage-truck");
            
        let pool = sqlx::pool::PoolOptions::new()
            .max_connections(max_connections)
            .connect_with(options)
            .await
            .map_err(|e| GCError::Database(e.to_string()))?;

        // Run migrations
        info!("Running database migrations");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .map_err(|e| GCError::Database(format!("Migration failed: {}", e)))?;

        info!("PostgreSQL storage initialized successfully");
        Ok(Self { pool, max_connections })
    }
    
    /// Get the connection pool
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
    
    /// Convert ObjectType enum to integer for database storage
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

    /// Convert integer to ObjectType enum from database
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

    /// Convert LeaseState enum to integer for database storage
    fn lease_state_to_int(state: &LeaseState) -> i32 {
        match state {
            LeaseState::Active => 0,
            LeaseState::Expired => 1,
            LeaseState::Released => 2,
        }
    }

    /// Convert integer to LeaseState enum from database
    fn int_to_lease_state(val: i32) -> LeaseState {
        match val {
            1 => LeaseState::Expired,
            2 => LeaseState::Released,
            _ => LeaseState::Active,
        }
    }

    /// Convert database row to Lease struct
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
    
    /// Build WHERE clause for filtering queries
    fn build_where_clause(filter: &LeaseFilter, param_offset: i32) -> (String, Vec<String>) {
        let mut conditions = Vec::new();
        let mut params = Vec::new();
        let mut param_count = param_offset;

        if let Some(ref service_id) = filter.service_id {
            param_count += 1;
            conditions.push(format!("service_id = ${}", param_count));
            params.push(service_id.clone());
        }

        if let Some(ref object_type) = filter.object_type {
            param_count += 1;
            conditions.push(format!("object_type = ${}", param_count));
            params.push(Self::object_type_to_int(object_type).to_string());
        }

        if let Some(ref state) = filter.state {
            param_count += 1;
            conditions.push(format!("state = ${}", param_count));
            params.push(Self::lease_state_to_int(state).to_string());
        }

        if let Some(created_after) = filter.created_after {
            param_count += 1;
            conditions.push(format!("created_at > ${}", param_count));
            params.push(created_after.to_rfc3339());
        }

        if let Some(created_before) = filter.created_before {
            param_count += 1;
            conditions.push(format!("created_at < ${}", param_count));
            params.push(created_before.to_rfc3339());
        }

        if let Some(expires_after) = filter.expires_after {
            param_count += 1;
            conditions.push(format!("expires_at > ${}", param_count));
            params.push(expires_after.to_rfc3339());
        }

        if let Some(expires_before) = filter.expires_before {
            param_count += 1;
            conditions.push(format!("expires_at < ${}", param_count));
            params.push(expires_before.to_rfc3339());
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        (where_clause, params)
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

        debug!("Created lease {} in PostgreSQL storage", lease.lease_id);
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

        debug!("Updated lease {} in PostgreSQL storage", lease.lease_id);
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

        debug!("Deleted lease {} from PostgreSQL storage", lease_id);
        Ok(())
    }

    async fn list_leases(&self, filter: LeaseFilter, limit: Option<usize>, offset: Option<usize>) -> Result<Vec<Lease>> {
        let (where_clause, params) = Self::build_where_clause(&filter, 0);
        
        let mut query_str = format!("SELECT * FROM leases{} ORDER BY created_at DESC", where_clause);
        
        let mut param_count = params.len() as i32;
        if let Some(limit) = limit {
            param_count += 1;
            query_str.push_str(&format!(" LIMIT ${}", param_count));
        }

        if let Some(offset) = offset {
            param_count += 1;
            query_str.push_str(&format!(" OFFSET ${}", param_count));
        }

        let mut query = sqlx::query(&query_str);
        
        // Bind filter parameters
        for param in params {
            query = query.bind(param);
        }
        
        // Bind limit and offset
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

        debug!("Listed {} leases from PostgreSQL storage", leases.len());
        Ok(leases)
    }

    async fn count_leases(&self, filter: LeaseFilter) -> Result<usize> {
        let (where_clause, params) = Self::build_where_clause(&filter, 0);
        let query_str = format!("SELECT COUNT(*) as count FROM leases{}", where_clause);

        let mut query = sqlx::query(&query_str);
        for param in params {
            query = query.bind(param);
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

        debug!("Found {} expired leases in PostgreSQL storage", leases.len());
        Ok(leases)
    }

    async fn get_stats(&self) -> Result<LeaseStats> {
        // Get basic stats using stored procedure if available, fallback to queries
        let stats_result = sqlx::query("SELECT * FROM get_lease_statistics()")
            .fetch_optional(&self.pool)
            .await;

        let (total_leases, active_leases, expired_leases, released_leases, avg_duration, avg_renewals) = 
            if let Ok(Some(stats_row)) = stats_result {
                (
                    stats_row.try_get::<i64, _>("total_leases").unwrap_or(0),
                    stats_row.try_get::<i64, _>("active_leases").unwrap_or(0),
                    stats_row.try_get::<i64, _>("expired_leases").unwrap_or(0),
                    stats_row.try_get::<i64, _>("released_leases").unwrap_or(0),
                    stats_row.try_get::<Option<f64>, _>("avg_lease_duration_hours").unwrap_or(None),
                    stats_row.try_get::<Option<f64>, _>("avg_renewal_count").unwrap_or(None),
                )
            } else {
                // Fallback to individual queries
                warn!("Stored procedure get_lease_statistics() not available, using fallback queries");
                let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM leases")
                    .fetch_one(&self.pool).await.unwrap_or(0);
                let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM leases WHERE state = 0 AND expires_at > NOW()")
                    .fetch_one(&self.pool).await.unwrap_or(0);
                let expired: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM leases WHERE state = 1 OR (state = 0 AND expires_at <= NOW())")
                    .fetch_one(&self.pool).await.unwrap_or(0);
                let released: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM leases WHERE state = 2")
                    .fetch_one(&self.pool).await.unwrap_or(0);
                    
                (total, active, expired, released, None, None)
            };

        // Get service stats
        let service_rows = sqlx::query("SELECT service_id, COUNT(*) as count FROM leases WHERE state = 0 GROUP BY service_id")
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();

        let mut leases_by_service = std::collections::HashMap::new();
        for row in service_rows {
            let service_id: String = row.try_get("service_id").unwrap_or_default();
            let count: i64 = row.try_get("count").unwrap_or(0);
            leases_by_service.insert(service_id, count as usize);
        }

        // Get type stats
        let type_rows = sqlx::query("SELECT object_type, COUNT(*) as count FROM leases WHERE state = 0 GROUP BY object_type")
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();

        let mut leases_by_type = std::collections::HashMap::new();
        for row in type_rows {
            let object_type: i32 = row.try_get("object_type").unwrap_or(0);
            let count: i64 = row.try_get("count").unwrap_or(0);
            let type_name = format!("{:?}", Self::int_to_object_type(object_type));
            leases_by_type.insert(type_name, count as usize);
        }

        Ok(LeaseStats {
            total_leases: total_leases as usize,
            active_leases: active_leases as usize,
            expired_leases: expired_leases as usize,
            released_leases: released_leases as usize,
            leases_by_service,
            leases_by_type,
            average_lease_duration: avg_duration
                .map(|h| chrono::Duration::hours(h as i64)),
            average_renewal_count: avg_renewals.unwrap_or(0.0),
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

        // Also clean up old cleanup history if procedure exists
        let _ = sqlx::query("SELECT cleanup_old_history()")
            .fetch_optional(&self.pool)
            .await;

        debug!("Cleaned up {} expired lease records from PostgreSQL", result.rows_affected());
        Ok(())
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl ExtendedStorage for PostgresStorage {
    async fn get_info(&self) -> Result<StorageInfo> {
        Ok(StorageInfo::postgres(self.max_connections))
    }

    async fn migrate(&self) -> Result<()> {
        info!("Running PostgreSQL migrations");
        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| GCError::Database(format!("Migration failed: {}", e)))?;
        Ok(())
    }

    async fn create_indexes(&self) -> Result<()> {
        info!("Creating PostgreSQL indexes for performance");
        
        // Create indexes if they don't exist
        let index_queries = vec![
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_leases_service_id ON leases(service_id)",
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_leases_object_type ON leases(object_type)",
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_leases_state ON leases(state)",
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_leases_expires_at ON leases(expires_at)",
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_leases_created_at ON leases(created_at)",
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_leases_service_state ON leases(service_id, state)",
        ];

        for query in index_queries {
            if let Err(e) = sqlx::query(query).execute(&self.pool).await {
                warn!("Failed to create index: {} - {}", query, e);
                // Continue with other indexes even if one fails
            }
        }

        info!("PostgreSQL indexes creation completed");
        Ok(())
    }

    async fn get_detailed_stats(&self) -> Result<DetailedStorageStats> {
        // Get database size information
        let size_query = "SELECT pg_size_pretty(pg_database_size(current_database())) as size, pg_database_size(current_database()) as size_bytes";
        let size_row = sqlx::query(size_query)
            .fetch_optional(&self.pool)
            .await
            .unwrap_or(None);

        let total_size = size_row
            .and_then(|row| row.try_get::<i64, _>("size_bytes").ok())
            .map(|size| size as u64);

        // Get index size
        let index_size_query = "SELECT SUM(pg_relation_size(indexrelid)) as index_size FROM pg_stat_user_indexes WHERE schemaname = 'public'";
        let index_size = sqlx::query_scalar::<_, Option<i64>>(index_size_query)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(None)
            .map(|size| size as u64);

        // Get connection pool stats
        let pool_stats = ConnectionPoolStats {
            active_connections: self.pool.size() as usize,
            idle_connections: self.pool.num_idle() as usize,
            max_connections: self.max_connections as usize,
            total_acquired: 0, // Not easily available from sqlx
            total_creation_time_ms: 0, // Not easily available from sqlx
        };

        // Get query performance stats (simplified)
        let query_stats = QueryPerformanceStats {
            average_query_time_ms: 5.0, // Rough estimate for PostgreSQL
            slowest_query_time_ms: 100,
            total_queries: 0, // Would need pg_stat_statements extension
            failed_queries: 0,
        };

        Ok(DetailedStorageStats {
            total_storage_size_bytes: total_size,
            index_size_bytes: index_size,
            connection_pool_stats: Some(pool_stats),
            query_performance: Some(query_stats),
        })
    }
}

// Provide empty implementations when postgres feature is not enabled
#[cfg(not(feature = "postgres"))]
pub struct PostgresStorage;

#[cfg(not(feature = "postgres"))]
impl PostgresStorage {
    pub async fn new(_database_url: &str, _max_connections: u32) -> crate::error::Result<Self> {
        Err(crate::error::GCError::Configuration("PostgreSQL support not compiled in. Enable 'postgres' feature.".to_string()))
    }
}