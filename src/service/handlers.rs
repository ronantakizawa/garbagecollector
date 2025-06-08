// src/service/handlers.rs - Fixed version (showing the key parts that need fixing)

// This is the specific fix for the list_leases method around line 328

use std::sync::Arc;
use tonic::{Request, Response, Status};
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{CleanupConfig, Lease, LeaseFilter, LeaseState, ObjectType};
use crate::metrics::Metrics;
use crate::proto::distributed_gc_service_server::DistributedGcService;
use crate::proto::*;
use crate::storage::Storage;

/// gRPC service handlers implementation
pub struct GCServiceHandlers {
    storage: Arc<dyn Storage>,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
}

impl GCServiceHandlers {
    pub fn new(storage: Arc<dyn Storage>, config: Arc<Config>, metrics: Arc<Metrics>) -> Self {
        Self {
            storage,
            config,
            metrics,
        }
    }
}

#[tonic::async_trait]
impl DistributedGcService for GCServiceHandlers {
    async fn create_lease(
        &self,
        request: Request<CreateLeaseRequest>,
    ) -> std::result::Result<Response<CreateLeaseResponse>, Status> {
        let req = request.into_inner();
        let start_time = std::time::Instant::now();

        debug!("Creating lease for object: {}", req.object_id);

        // Validate request
        if req.object_id.is_empty() {
            return Err(Status::invalid_argument("object_id cannot be empty"));
        }

        if req.service_id.is_empty() {
            return Err(Status::invalid_argument("service_id cannot be empty"));
        }

        // Check lease duration limits
        if req.lease_duration_seconds < self.config.gc.min_lease_duration_seconds {
            return Err(Status::invalid_argument(format!(
                "Lease duration {} is below minimum {}",
                req.lease_duration_seconds, self.config.gc.min_lease_duration_seconds
            )));
        }

        if req.lease_duration_seconds > self.config.gc.max_lease_duration_seconds {
            return Err(Status::invalid_argument(format!(
                "Lease duration {} exceeds maximum {}",
                req.lease_duration_seconds, self.config.gc.max_lease_duration_seconds
            )));
        }

        // Check service lease limits
        match self
            .storage
            .count_active_leases_for_service(&req.service_id)
            .await
        {
            Ok(count) => {
                if count >= self.config.gc.max_leases_per_service {
                    return Err(Status::resource_exhausted(format!(
                        "Service {} has reached maximum lease limit: {}",
                        req.service_id, self.config.gc.max_leases_per_service
                    )));
                }
            }
            Err(e) => {
                error!(
                    "Failed to count leases for service {}: {}",
                    req.service_id, e
                );
                return Err(Status::internal("Failed to check service lease limits"));
            }
        }

        // Convert object type
        let object_type = match ObjectType::try_from(req.object_type) {
            Ok(ot) => ot,
            Err(_) => {
                return Err(Status::invalid_argument("Invalid object type"));
            }
        };

        // Convert cleanup config if provided
        let cleanup_config = req.cleanup_config.map(|config| CleanupConfig {
            cleanup_endpoint: config.cleanup_endpoint,
            cleanup_http_endpoint: config.cleanup_http_endpoint,
            cleanup_payload: config.cleanup_payload,
            max_retries: config.max_retries,
            retry_delay_seconds: config.retry_delay_seconds,
        });

        // Create lease
        let lease = Lease::new(
            req.object_id.clone(), // Clone to avoid moving
            object_type,
            req.service_id.clone(), // Clone to avoid moving
            std::time::Duration::from_secs(req.lease_duration_seconds),
            req.metadata,
            cleanup_config,
        );

        let lease_id = lease.lease_id.clone();
        let expires_at = lease.expires_at;

        // Store lease
        match self.storage.create_lease(lease).await {
            Ok(_) => {
                self.metrics.increment_leases_created();
                self.metrics.increment_active_leases();

                let duration = start_time.elapsed();
                self.metrics
                    .record_grpc_request_duration("create_lease", duration);
                self.metrics
                    .increment_grpc_request("create_lease", "success");

                info!(
                    "✅ Created lease {} for service {}",
                    lease_id, req.service_id
                );

                Ok(Response::new(CreateLeaseResponse {
                    lease_id,
                    expires_at: Some(prost_types::Timestamp {
                        seconds: expires_at.timestamp(),
                        nanos: expires_at.timestamp_subsec_nanos() as i32,
                    }),
                    success: true,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                self.metrics.increment_grpc_request("create_lease", "error");
                error!("Failed to create lease: {}", e);
                Err(Status::from(e))
            }
        }
    }

    async fn renew_lease(
        &self,
        request: Request<RenewLeaseRequest>,
    ) -> std::result::Result<Response<RenewLeaseResponse>, Status> {
        let req = request.into_inner();
        let start_time = std::time::Instant::now();

        debug!("Renewing lease: {}", req.lease_id);

        // Get existing lease
        let mut lease = match self.storage.get_lease(&req.lease_id).await {
            Ok(Some(lease)) => lease,
            Ok(None) => {
                self.metrics
                    .increment_grpc_request("renew_lease", "not_found");
                return Err(Status::not_found("Lease not found"));
            }
            Err(e) => {
                self.metrics.increment_grpc_request("renew_lease", "error");
                return Err(Status::from(e));
            }
        };

        // Verify service ownership
        if lease.service_id != req.service_id {
            self.metrics
                .increment_grpc_request("renew_lease", "unauthorized");
            return Err(Status::permission_denied("Unauthorized access to lease"));
        }

        // Check if lease is still active
        if lease.state != LeaseState::Active {
            self.metrics
                .increment_grpc_request("renew_lease", "invalid_state");
            return Err(Status::failed_precondition("Lease is not active"));
        }

        // Renew the lease
        match lease.renew(std::time::Duration::from_secs(req.extend_duration_seconds)) {
            Ok(_) => {
                let new_expires_at = lease.expires_at;

                // Update in storage
                match self.storage.update_lease(lease).await {
                    Ok(_) => {
                        self.metrics.increment_leases_renewed();

                        let duration = start_time.elapsed();
                        self.metrics
                            .record_grpc_request_duration("renew_lease", duration);
                        self.metrics
                            .increment_grpc_request("renew_lease", "success");

                        info!(
                            "✅ Renewed lease {} for service {}",
                            req.lease_id, req.service_id
                        );

                        Ok(Response::new(RenewLeaseResponse {
                            new_expires_at: Some(prost_types::Timestamp {
                                seconds: new_expires_at.timestamp(),
                                nanos: new_expires_at.timestamp_subsec_nanos() as i32,
                            }),
                            success: true,
                            error_message: String::new(),
                        }))
                    }
                    Err(e) => {
                        self.metrics.increment_grpc_request("renew_lease", "error");
                        error!("Failed to update renewed lease: {}", e);
                        Err(Status::from(e))
                    }
                }
            }
            Err(e) => {
                self.metrics.increment_grpc_request("renew_lease", "error");
                error!("Failed to renew lease: {}", e);
                Err(Status::from(e))
            }
        }
    }

    async fn release_lease(
        &self,
        request: Request<ReleaseLeaseRequest>,
    ) -> std::result::Result<Response<ReleaseLeaseResponse>, Status> {
        let req = request.into_inner();
        let start_time = std::time::Instant::now();

        debug!("Releasing lease: {}", req.lease_id);

        // Get existing lease
        let lease = match self.storage.get_lease(&req.lease_id).await {
            Ok(Some(lease)) => lease,
            Ok(None) => {
                self.metrics
                    .increment_grpc_request("release_lease", "not_found");
                return Err(Status::not_found("Lease not found"));
            }
            Err(e) => {
                self.metrics
                    .increment_grpc_request("release_lease", "error");
                return Err(Status::from(e));
            }
        };

        // Verify service ownership
        if lease.service_id != req.service_id {
            self.metrics
                .increment_grpc_request("release_lease", "unauthorized");
            return Err(Status::permission_denied("Unauthorized access to lease"));
        }

        // Mark lease as released
        match self.storage.mark_lease_released(&req.lease_id).await {
            Ok(_) => {
                self.metrics.increment_leases_released();
                self.metrics.decrement_active_leases();

                let duration = start_time.elapsed();
                self.metrics
                    .record_grpc_request_duration("release_lease", duration);
                self.metrics
                    .increment_grpc_request("release_lease", "success");

                info!(
                    "✅ Released lease {} for service {}",
                    req.lease_id, req.service_id
                );

                Ok(Response::new(ReleaseLeaseResponse {
                    success: true,
                    error_message: String::new(),
                }))
            }
            Err(e) => {
                self.metrics
                    .increment_grpc_request("release_lease", "error");
                error!("Failed to release lease: {}", e);
                Err(Status::from(e))
            }
        }
    }

    async fn get_lease(
        &self,
        request: Request<GetLeaseRequest>,
    ) -> std::result::Result<Response<GetLeaseResponse>, Status> {
        let req = request.into_inner();
        let start_time = std::time::Instant::now();

        debug!("Getting lease: {}", req.lease_id);

        match self.storage.get_lease(&req.lease_id).await {
            Ok(Some(lease)) => {
                let duration = start_time.elapsed();
                self.metrics
                    .record_grpc_request_duration("get_lease", duration);
                self.metrics.increment_grpc_request("get_lease", "success");

                Ok(Response::new(GetLeaseResponse {
                    lease: Some((&lease).into()),
                    found: true,
                }))
            }
            Ok(None) => {
                let duration = start_time.elapsed();
                self.metrics
                    .record_grpc_request_duration("get_lease", duration);
                self.metrics
                    .increment_grpc_request("get_lease", "not_found");

                Ok(Response::new(GetLeaseResponse {
                    lease: None,
                    found: false,
                }))
            }
            Err(e) => {
                self.metrics.increment_grpc_request("get_lease", "error");
                error!("Failed to get lease: {}", e);
                Err(Status::from(e))
            }
        }
    }

    async fn list_leases(
        &self,
        request: Request<ListLeasesRequest>,
    ) -> std::result::Result<Response<ListLeasesResponse>, Status> {
        let req = request.into_inner();
        let start_time = std::time::Instant::now();

        debug!("Listing leases with filter");

        // Build filter
        let mut filter = LeaseFilter::default();

        if !req.service_id.is_empty() {
            filter.service_id = Some(req.service_id);
        }

        if req.object_type != 0 {
            if let Ok(object_type) = ObjectType::try_from(req.object_type) {
                filter.object_type = Some(object_type);
            }
        }

        if req.state != 0 {
            if let Ok(state) = LeaseState::try_from(req.state) {
                filter.state = Some(state);
            }
        }

        let limit = if req.limit > 0 {
            Some(req.limit as usize)
        } else {
            None
        };

        // FIXED: Use Some(filter) instead of filter, and remove offset parameter
        match self.storage.list_leases(Some(filter), limit).await {
            Ok(leases) => {
                let lease_infos: Vec<LeaseInfo> = leases.iter().map(|lease| lease.into()).collect();

                let duration = start_time.elapsed();
                self.metrics
                    .record_grpc_request_duration("list_leases", duration);
                self.metrics
                    .increment_grpc_request("list_leases", "success");

                Ok(Response::new(ListLeasesResponse {
                    leases: lease_infos,
                    next_page_token: String::new(), // No pagination support in simplified version
                }))
            }
            Err(e) => {
                self.metrics.increment_grpc_request("list_leases", "error");
                error!("Failed to list leases: {}", e);
                Err(Status::from(e))
            }
        }
    }

    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> std::result::Result<Response<HealthCheckResponse>, Status> {
        let start_time = std::time::Instant::now();

        debug!("Health check requested");

        match self.storage.health_check().await {
            Ok(healthy) => {
                let stats = self.storage.get_stats().await.unwrap_or_default();

                let duration = start_time.elapsed();
                self.metrics
                    .record_grpc_request_duration("health_check", duration);
                self.metrics
                    .increment_grpc_request("health_check", "success");

                Ok(Response::new(HealthCheckResponse {
                    healthy,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    active_leases: stats.active_leases as u64,
                    expired_leases: stats.expired_leases as u64,
                    uptime: Some(prost_types::Timestamp::from(std::time::SystemTime::now())),
                }))
            }
            Err(e) => {
                self.metrics.increment_grpc_request("health_check", "error");
                error!("Health check failed: {}", e);
                Err(Status::from(e))
            }
        }
    }

    async fn get_metrics(
        &self,
        _request: Request<MetricsRequest>,
    ) -> std::result::Result<Response<MetricsResponse>, Status> {
        let start_time = std::time::Instant::now();

        debug!("Metrics requested");

        match self.storage.get_stats().await {
            Ok(stats) => {
                let duration = start_time.elapsed();
                self.metrics
                    .record_grpc_request_duration("get_metrics", duration);
                self.metrics
                    .increment_grpc_request("get_metrics", "success");

                Ok(Response::new(MetricsResponse {
                    total_leases_created: self.metrics.leases_created_total.get() as u64,
                    total_leases_renewed: self.metrics.leases_renewed_total.get() as u64,
                    total_leases_expired: self.metrics.leases_expired_total.get() as u64,
                    total_leases_released: self.metrics.leases_released_total.get() as u64,
                    total_cleanup_operations: self.metrics.cleanup_operations_total.get() as u64,
                    failed_cleanup_operations: self.metrics.cleanup_failures_total.get() as u64,
                    active_leases: stats.active_leases as u64,
                    leases_by_service: stats
                        .leases_by_service
                        .into_iter()
                        .map(|(k, v)| (k, v as u64))
                        .collect(),
                    leases_by_type: stats
                        .leases_by_type
                        .into_iter()
                        .map(|(k, v)| (k, v as u64))
                        .collect(),
                }))
            }
            Err(e) => {
                self.metrics.increment_grpc_request("get_metrics", "error");
                error!("Failed to get metrics: {}", e);
                Err(Status::from(e))
            }
        }
    }
}

// Helper implementations for type conversions
impl TryFrom<i32> for ObjectType {
    type Error = ();

    fn try_from(value: i32) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(ObjectType::Unknown),
            1 => Ok(ObjectType::DatabaseRow),
            2 => Ok(ObjectType::BlobStorage),
            3 => Ok(ObjectType::TemporaryFile),
            4 => Ok(ObjectType::WebsocketSession),
            5 => Ok(ObjectType::CacheEntry),
            6 => Ok(ObjectType::Custom),
            _ => Err(()),
        }
    }
}

impl TryFrom<i32> for LeaseState {
    type Error = ();

    fn try_from(value: i32) -> std::result::Result<Self, Self::Error> {
        match value {
            0 => Ok(LeaseState::Active),
            1 => Ok(LeaseState::Expired),
            2 => Ok(LeaseState::Released),
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;

    #[tokio::test]
    async fn test_handlers_creation() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();

        let _handlers = GCServiceHandlers::new(storage, config, metrics);
        // Test passes if we can create handlers without panicking
    }

    #[tokio::test]
    async fn test_health_check() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();

        let handlers = GCServiceHandlers::new(storage, config, metrics);

        let request = Request::new(HealthCheckRequest {});
        let response = handlers.health_check(request).await.unwrap();

        assert!(response.into_inner().healthy);
    }
}
