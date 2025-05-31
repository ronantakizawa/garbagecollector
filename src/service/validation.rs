// src/service/validation.rs - Request validation logic

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::metrics::Metrics;
use crate::proto::{CreateLeaseRequest, ReleaseLeaseRequest, RenewLeaseRequest};

/// Request validation utilities
pub struct RequestValidator<'a> {
    config: &'a Config,
    metrics: &'a std::sync::Arc<Metrics>,
}

impl<'a> RequestValidator<'a> {
    /// Create a new request validator
    pub fn new(config: &'a Config, metrics: &'a std::sync::Arc<Metrics>) -> Self {
        Self { config, metrics }
    }

    /// Validate incoming lease creation requests with enhanced error categorization
    pub fn validate_create_lease_request(&self, request: &CreateLeaseRequest) -> Result<()> {
        if request.object_id.is_empty() {
            self.metrics
                .record_startup_error("lease_validation", "empty_object_id");
            return Err(GCError::Configuration(
                "Object ID cannot be empty".to_string(),
            ));
        }

        if request.service_id.is_empty() {
            self.metrics
                .record_startup_error("lease_validation", "empty_service_id");
            return Err(GCError::Configuration(
                "Service ID cannot be empty".to_string(),
            ));
        }

        let duration = request.lease_duration_seconds;
        if duration < self.config.gc.min_lease_duration_seconds
            || duration > self.config.gc.max_lease_duration_seconds
        {
            self.metrics
                .record_startup_error("lease_validation", "invalid_duration");
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
            self.metrics
                .record_startup_error("lease_validation", "empty_lease_id");
            return Err(GCError::Configuration(
                "Lease ID cannot be empty".to_string(),
            ));
        }

        if request.service_id.is_empty() {
            self.metrics
                .record_startup_error("lease_validation", "empty_service_id");
            return Err(GCError::Configuration(
                "Service ID cannot be empty".to_string(),
            ));
        }

        // Validate extension duration if provided
        if request.extend_duration_seconds > 0 {
            let duration = request.extend_duration_seconds;
            if duration > self.config.gc.max_lease_duration_seconds {
                self.metrics
                    .record_startup_error("lease_validation", "invalid_extension_duration");
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
            self.metrics
                .record_startup_error("lease_validation", "empty_lease_id");
            return Err(GCError::Configuration(
                "Lease ID cannot be empty".to_string(),
            ));
        }

        if request.service_id.is_empty() {
            self.metrics
                .record_startup_error("lease_validation", "empty_service_id");
            return Err(GCError::Configuration(
                "Service ID cannot be empty".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate service ID format and constraints
    pub fn validate_service_id(&self, service_id: &str) -> Result<()> {
        if service_id.is_empty() {
            return Err(GCError::Configuration(
                "Service ID cannot be empty".to_string(),
            ));
        }

        if service_id.len() > 255 {
            return Err(GCError::Configuration(
                "Service ID too long (max 255 characters)".to_string(),
            ));
        }

        // Check for valid characters (alphanumeric, hyphens, underscores)
        if !service_id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            return Err(GCError::Configuration(
                "Service ID can only contain alphanumeric characters, hyphens, and underscores"
                    .to_string(),
            ));
        }

        Ok(())
    }

    /// Validate object ID format and constraints
    pub fn validate_object_id(&self, object_id: &str) -> Result<()> {
        if object_id.is_empty() {
            return Err(GCError::Configuration(
                "Object ID cannot be empty".to_string(),
            ));
        }

        if object_id.len() > 1024 {
            return Err(GCError::Configuration(
                "Object ID too long (max 1024 characters)".to_string(),
            ));
        }

        Ok(())
    }

    /// Validate lease ID format (typically UUID)
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

    /// Validate lease duration constraints
    pub fn validate_lease_duration(&self, duration_seconds: u64) -> Result<()> {
        if duration_seconds < self.config.gc.min_lease_duration_seconds {
            return Err(GCError::InvalidLeaseDuration {
                duration: duration_seconds,
                min: self.config.gc.min_lease_duration_seconds,
                max: self.config.gc.max_lease_duration_seconds,
            });
        }

        if duration_seconds > self.config.gc.max_lease_duration_seconds {
            return Err(GCError::InvalidLeaseDuration {
                duration: duration_seconds,
                min: self.config.gc.min_lease_duration_seconds,
                max: self.config.gc.max_lease_duration_seconds,
            });
        }

        Ok(())
    }

    /// Validate metadata constraints
    pub fn validate_metadata(
        &self,
        metadata: &std::collections::HashMap<String, String>,
    ) -> Result<()> {
        // Check metadata size limits
        if metadata.len() > 50 {
            return Err(GCError::Configuration(
                "Too many metadata entries (max 50)".to_string(),
            ));
        }

        for (key, value) in metadata {
            // Validate key constraints
            if key.is_empty() {
                return Err(GCError::Configuration(
                    "Metadata key cannot be empty".to_string(),
                ));
            }

            if key.len() > 255 {
                return Err(GCError::Configuration(
                    "Metadata key too long (max 255 characters)".to_string(),
                ));
            }

            // Validate value constraints
            if value.len() > 1024 {
                return Err(GCError::Configuration(
                    "Metadata value too long (max 1024 characters)".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Comprehensive validation for create lease request
    pub fn validate_create_lease_comprehensive(&self, request: &CreateLeaseRequest) -> Result<()> {
        // Basic request validation
        self.validate_create_lease_request(request)?;

        // Additional validations
        self.validate_service_id(&request.service_id)?;
        self.validate_object_id(&request.object_id)?;
        self.validate_metadata(&request.metadata)?;

        // Validate cleanup configuration if provided
        if let Some(ref cleanup_config) = request.cleanup_config {
            self.validate_cleanup_config(cleanup_config)?;
        }

        Ok(())
    }

    /// Validate cleanup configuration
    pub fn validate_cleanup_config(&self, config: &crate::proto::CleanupConfig) -> Result<()> {
        // At least one cleanup method should be specified
        if config.cleanup_endpoint.is_empty() && config.cleanup_http_endpoint.is_empty() {
            return Err(GCError::Configuration(
                "At least one cleanup endpoint must be specified".to_string(),
            ));
        }

        // Validate URL format for HTTP endpoint
        if !config.cleanup_http_endpoint.is_empty()
            && url::Url::parse(&config.cleanup_http_endpoint).is_err()
        {
            return Err(GCError::Configuration(
                "Invalid HTTP cleanup endpoint URL format".to_string(),
            ));
        }

        // Validate retry configuration
        if config.max_retries > 10 {
            return Err(GCError::Configuration(
                "Maximum retries cannot exceed 10".to_string(),
            ));
        }

        if config.retry_delay_seconds > 300 {
            return Err(GCError::Configuration(
                "Retry delay cannot exceed 300 seconds".to_string(),
            ));
        }

        Ok(())
    }
}
