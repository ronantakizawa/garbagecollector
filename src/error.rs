use thiserror::Error;

#[derive(Error, Debug)]
pub enum GCError {
    #[error("Lease not found: {lease_id}")]
    LeaseNotFound { lease_id: String },

    #[error("Lease already expired: {lease_id}")]
    LeaseExpired { lease_id: String },

    #[error("Invalid lease duration: {duration} seconds (min: {min}, max: {max})")]
    InvalidLeaseDuration { duration: u64, min: u64, max: u64 },

    #[error("Service has too many leases: {service_id} (current: {current}, max: {max})")]
    ServiceLeaseLimit {
        service_id: String,
        current: usize,
        max: usize,
    },

    #[error("Unauthorized access to lease: {lease_id} by service: {service_id}")]
    UnauthorizedAccess {
        lease_id: String,
        service_id: String,
    },

    #[error("Storage error: {0}")]
    Storage(#[from] anyhow::Error),

    #[error("Cleanup error: {0}")]
    Cleanup(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Database error: {0}")]
    Database(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Timeout error: operation timed out after {timeout_seconds} seconds")]
    Timeout { timeout_seconds: u64 },

    #[error("Rate limit exceeded for service: {service_id}")]
    RateLimit { service_id: String },

    #[error("Internal error: {0}")]
    Internal(String),
}

impl From<GCError> for tonic::Status {
    fn from(err: GCError) -> Self {
        match err {
            GCError::LeaseNotFound { .. } => tonic::Status::not_found(err.to_string()),
            GCError::LeaseExpired { .. } => tonic::Status::failed_precondition(err.to_string()),
            GCError::InvalidLeaseDuration { .. } => {
                tonic::Status::invalid_argument(err.to_string())
            }
            GCError::ServiceLeaseLimit { .. } => tonic::Status::resource_exhausted(err.to_string()),
            GCError::UnauthorizedAccess { .. } => tonic::Status::permission_denied(err.to_string()),
            GCError::Configuration(_) => tonic::Status::failed_precondition(err.to_string()),
            GCError::RateLimit { .. } => tonic::Status::resource_exhausted(err.to_string()),
            GCError::Timeout { .. } => tonic::Status::deadline_exceeded(err.to_string()),
            _ => tonic::Status::internal(err.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, GCError>;
