use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// Create our own serializable enums instead of using protobuf enums
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ObjectType {
    Unknown = 0,
    DatabaseRow = 1,
    BlobStorage = 2,
    TemporaryFile = 3,
    WebsocketSession = 4,
    CacheEntry = 5,
    Custom = 6,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum LeaseState {
    Active = 0,
    Expired = 1,
    Released = 2,
}

// Use our own CleanupConfig struct instead of protobuf
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CleanupConfig {
    pub cleanup_endpoint: String,
    pub cleanup_http_endpoint: String,
    pub cleanup_payload: String,
    pub max_retries: u32,
    pub retry_delay_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Lease {
    pub lease_id: String,
    pub object_id: String,
    pub object_type: ObjectType,
    pub service_id: String,
    pub state: LeaseState,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_renewed_at: DateTime<Utc>,
    pub metadata: HashMap<String, String>,
    pub cleanup_config: Option<CleanupConfig>,
    pub renewal_count: u32,
}

impl Lease {
    pub fn new(
        object_id: String,
        object_type: ObjectType,
        service_id: String,
        lease_duration: std::time::Duration,
        metadata: HashMap<String, String>,
        cleanup_config: Option<CleanupConfig>,
    ) -> Self {
        let now = Utc::now();
        let expires_at = now + chrono::Duration::from_std(lease_duration).unwrap();

        Self {
            lease_id: Uuid::new_v4().to_string(),
            object_id,
            object_type,
            service_id,
            state: LeaseState::Active,
            created_at: now,
            expires_at,
            last_renewed_at: now,
            metadata,
            cleanup_config,
            renewal_count: 0,
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    pub fn is_active(&self) -> bool {
        self.state == LeaseState::Active && !self.is_expired()
    }

    pub fn time_until_expiry(&self) -> Option<chrono::Duration> {
        if self.is_expired() {
            None
        } else {
            Some(self.expires_at - Utc::now())
        }
    }

    pub fn renew(&mut self, extend_duration: std::time::Duration) -> crate::error::Result<()> {
        if !self.is_active() {
            return Err(crate::error::GCError::LeaseExpired {
                lease_id: self.lease_id.clone(),
            });
        }

        let now = Utc::now();
        self.expires_at = now + chrono::Duration::from_std(extend_duration).unwrap();
        self.last_renewed_at = now;
        self.renewal_count += 1;

        Ok(())
    }

    pub fn release(&mut self) {
        self.state = LeaseState::Released;
    }

    pub fn expire(&mut self) {
        self.state = LeaseState::Expired;
    }

    pub fn should_cleanup(&self, grace_period: std::time::Duration) -> bool {
        if self.state != LeaseState::Expired {
            return false;
        }

        let grace_expires_at = self.expires_at + chrono::Duration::from_std(grace_period).unwrap();
        Utc::now() > grace_expires_at
    }

    pub fn age(&self) -> chrono::Duration {
        Utc::now() - self.created_at
    }

    pub fn time_since_last_renewal(&self) -> chrono::Duration {
        Utc::now() - self.last_renewed_at
    }
}

// Conversion functions between our types and protobuf types
impl From<crate::proto::ObjectType> for ObjectType {
    fn from(proto_type: crate::proto::ObjectType) -> Self {
        match proto_type {
            crate::proto::ObjectType::Unknown => ObjectType::Unknown,
            crate::proto::ObjectType::DatabaseRow => ObjectType::DatabaseRow,
            crate::proto::ObjectType::BlobStorage => ObjectType::BlobStorage,
            crate::proto::ObjectType::TemporaryFile => ObjectType::TemporaryFile,
            crate::proto::ObjectType::WebsocketSession => ObjectType::WebsocketSession,
            crate::proto::ObjectType::CacheEntry => ObjectType::CacheEntry,
            crate::proto::ObjectType::Custom => ObjectType::Custom,
        }
    }
}

impl From<ObjectType> for crate::proto::ObjectType {
    fn from(obj_type: ObjectType) -> Self {
        match obj_type {
            ObjectType::Unknown => crate::proto::ObjectType::Unknown,
            ObjectType::DatabaseRow => crate::proto::ObjectType::DatabaseRow,
            ObjectType::BlobStorage => crate::proto::ObjectType::BlobStorage,
            ObjectType::TemporaryFile => crate::proto::ObjectType::TemporaryFile,
            ObjectType::WebsocketSession => crate::proto::ObjectType::WebsocketSession,
            ObjectType::CacheEntry => crate::proto::ObjectType::CacheEntry,
            ObjectType::Custom => crate::proto::ObjectType::Custom,
        }
    }
}

impl From<crate::proto::LeaseState> for LeaseState {
    fn from(proto_state: crate::proto::LeaseState) -> Self {
        match proto_state {
            crate::proto::LeaseState::Active => LeaseState::Active,
            crate::proto::LeaseState::Expired => LeaseState::Expired,
            crate::proto::LeaseState::Released => LeaseState::Released,
        }
    }
}

impl From<LeaseState> for crate::proto::LeaseState {
    fn from(state: LeaseState) -> Self {
        match state {
            LeaseState::Active => crate::proto::LeaseState::Active,
            LeaseState::Expired => crate::proto::LeaseState::Expired,
            LeaseState::Released => crate::proto::LeaseState::Released,
        }
    }
}

impl From<crate::proto::CleanupConfig> for CleanupConfig {
    fn from(proto_config: crate::proto::CleanupConfig) -> Self {
        Self {
            cleanup_endpoint: proto_config.cleanup_endpoint,
            cleanup_http_endpoint: proto_config.cleanup_http_endpoint,
            cleanup_payload: proto_config.cleanup_payload,
            max_retries: proto_config.max_retries,
            retry_delay_seconds: proto_config.retry_delay_seconds,
        }
    }
}

impl From<CleanupConfig> for crate::proto::CleanupConfig {
    fn from(config: CleanupConfig) -> Self {
        Self {
            cleanup_endpoint: config.cleanup_endpoint,
            cleanup_http_endpoint: config.cleanup_http_endpoint,
            cleanup_payload: config.cleanup_payload,
            max_retries: config.max_retries,
            retry_delay_seconds: config.retry_delay_seconds,
        }
    }
}

impl From<&Lease> for crate::proto::LeaseInfo {
    fn from(lease: &Lease) -> Self {
        use prost_types::Timestamp;

        fn datetime_to_timestamp(dt: DateTime<Utc>) -> Option<Timestamp> {
            Some(Timestamp {
                seconds: dt.timestamp(),
                nanos: dt.timestamp_subsec_nanos() as i32,
            })
        }

        Self {
            lease_id: lease.lease_id.clone(),
            object_id: lease.object_id.clone(),
            object_type: crate::proto::ObjectType::from(lease.object_type.clone()) as i32,
            service_id: lease.service_id.clone(),
            state: crate::proto::LeaseState::from(lease.state.clone()) as i32,
            created_at: datetime_to_timestamp(lease.created_at),
            expires_at: datetime_to_timestamp(lease.expires_at),
            last_renewed_at: datetime_to_timestamp(lease.last_renewed_at),
            metadata: lease.metadata.clone(),
            cleanup_config: lease.cleanup_config.as_ref().map(|c| c.clone().into()),
            renewal_count: lease.renewal_count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseStats {
    pub total_leases: usize,
    pub active_leases: usize,
    pub expired_leases: usize,
    pub released_leases: usize,
    pub leases_by_service: HashMap<String, usize>,
    pub leases_by_type: HashMap<String, usize>,
    pub average_lease_duration: Option<chrono::Duration>,
    pub average_renewal_count: f64,
}

impl Default for LeaseStats {
    fn default() -> Self {
        Self {
            total_leases: 0,
            active_leases: 0,
            expired_leases: 0,
            released_leases: 0,
            leases_by_service: HashMap::new(),
            leases_by_type: HashMap::new(),
            average_lease_duration: None,
            average_renewal_count: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LeaseFilter {
    pub service_id: Option<String>,
    pub object_type: Option<ObjectType>,
    pub state: Option<LeaseState>,
    pub created_after: Option<DateTime<Utc>>,
    pub created_before: Option<DateTime<Utc>>,
    pub expires_after: Option<DateTime<Utc>>,
    pub expires_before: Option<DateTime<Utc>>,
}

impl LeaseFilter {
    pub fn matches(&self, lease: &Lease) -> bool {
        if let Some(ref service_id) = self.service_id {
            if lease.service_id != *service_id {
                return false;
            }
        }

        if let Some(ref object_type) = self.object_type {
            if lease.object_type != *object_type {
                return false;
            }
        }

        if let Some(ref state) = self.state {
            if lease.state != *state {
                return false;
            }
        }

        if let Some(created_after) = self.created_after {
            if lease.created_at <= created_after {
                return false;
            }
        }

        if let Some(created_before) = self.created_before {
            if lease.created_at >= created_before {
                return false;
            }
        }

        if let Some(expires_after) = self.expires_after {
            if lease.expires_at <= expires_after {
                return false;
            }
        }

        if let Some(expires_before) = self.expires_before {
            if lease.expires_at >= expires_before {
                return false;
            }
        }

        true
    }
}
