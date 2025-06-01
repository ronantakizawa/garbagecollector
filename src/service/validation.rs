// src/service/validation.rs - Simplified validation

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::proto::{CreateLeaseRequest, ReleaseLeaseRequest, RenewLeaseRequest};

/// Simple request validation utilities
pub struct RequestValidator<'a> {
    config: &'a Config,
}

impl<'a> RequestValidator<'a> {
    /// Create a new request validator - no metrics needed
    pub fn new(config: &'a Config, _metrics: &'a std::sync::Arc<crate::metrics::Metrics>) -> Self {
        Self { config }
    }

    /// Validate lease creation requests - simplified
    pub fn validate_create_lease_request(&self, request: &CreateLeaseRequest) -> Result<()> {
        if request.object_id.is_empty() {
            return Err(GCError::Configuration(
                "Object ID cannot be empty".to_string(),
            ));
        }

        if request.service_id.is_empty() {
            return Err(GCError::Configuration(
                "Service ID cannot be empty".to_string(),
            ));
        }

        let duration = request.lease_duration_seconds;
        if duration < self.config.gc.min_lease_duration_seconds
            || duration > self.config.gc.max_lease_duration_seconds
        {
            return Err(GCError::InvalidLeaseDuration {
                duration,
                min: self.config.gc.min_lease_duration_seconds,
                max: self.config.gc.max_lease_duration_seconds,
            });
        }

        Ok(())
    }

    /// Validate lease renewal requests
    pub fn validate_renew_lease_request(&self, request: &RenewLeaseRequest) -> Result<()> {
        if request.lease_id.is_empty() {
            return Err(GCError::Configuration(
                "Lease ID cannot be empty".to_string(),
            ));
        }

        if request.service_id.is_empty() {
            return Err(GCError::Configuration(
                "Service ID cannot be empty".to_string(),
            ));
        }

        // Validate extension duration if provided
        if request.extend_duration_seconds > 0 {
            let duration = request.extend_duration_seconds;
            if duration > self.config.gc.max_lease_duration_seconds {
                return Err(GCError::InvalidLeaseDuration {
                    duration,
                    min: self.config.gc.min_lease_duration_seconds,
                    max: self.config.gc.max_lease_duration_seconds,
                });
            }
        }

        Ok(())
    }

    /// Validate lease release requests
    pub fn validate_release_lease_request(&self, request: &ReleaseLeaseRequest) -> Result<()> {
        if request.lease_id.is_empty() {
            return Err(GCError::Configuration(
                "Lease ID cannot be empty".to_string(),
            ));
        }

        if request.service_id.is_empty() {
            return Err(GCError::Configuration(
                "Service ID cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate lease ID format (UUID check)
    pub fn validate_lease_id(&self, lease_id: &str) -> Result<()> {
        if lease_id.is_empty() {
            return Err(GCError::Configuration(
                "Lease ID cannot be empty".to_string(),
            ));
        }

        // Try to parse as UUID
        if uuid::Uuid::parse_str(lease_id).is_err() {
            return Err(GCError::Configuration(
                "Invalid lease ID format (must be UUID)".to_string(),
            ));
        }

        Ok(())
    }
}