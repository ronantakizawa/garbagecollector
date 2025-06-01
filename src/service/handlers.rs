// src/service/handlers.rs - Simplified gRPC method implementations

use std::time::{Duration, Instant};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info};

use crate::error::{GCError, Result};
use crate::lease::{CleanupConfig, Lease, LeaseFilter, LeaseState, ObjectType};
use crate::proto::{
    CreateLeaseRequest, CreateLeaseResponse, GetLeaseRequest, GetLeaseResponse, HealthCheckRequest,
    HealthCheckResponse, LeaseInfo, ListLeasesRequest, ListLeasesResponse, MetricsRequest,
    MetricsResponse, ReleaseLeaseRequest, ReleaseLeaseResponse, RenewLeaseRequest,
    RenewLeaseResponse,
};

use super::{validation::RequestValidator, GCService};

/// Trait defining gRPC service handlers
#[async_trait::async_trait]
pub trait GCServiceHandlers {
    async fn create_lease_impl(&self, request: CreateLeaseRequest) -> Result<CreateLeaseResponse>;
    async fn renew_lease_impl(&self, request: RenewLeaseRequest) -> Result<RenewLeaseResponse>;
    async fn release_lease_impl(
        &self,
        request: ReleaseLeaseRequest,
    ) -> Result<ReleaseLeaseResponse>;
    async fn get_lease_impl(&self, request: GetLeaseRequest) -> Result<GetLeaseResponse>;
    async fn list_leases_impl(&self, request: ListLeasesRequest) -> Result<ListLeasesResponse>;
    async fn health_check_impl(&self, request: HealthCheckRequest) -> Result<HealthCheckResponse>;
    async fn get_metrics_impl(&self, request: MetricsRequest) -> Result<MetricsResponse>;
}

#[async_trait::async_trait]
impl GCServiceHandlers for GCService {
    /// Create a new lease for an object
    async fn create_lease_impl(&self, request: CreateLeaseRequest) -> Result<CreateLeaseResponse> {
        // Simple validation
        let metrics = self.get_metrics();
        let validator = RequestValidator::new(self.get_config(), &metrics);
        validator.validate_create_lease_request(&request)?;

        // Check service lease limits
        self.check_service_lease_limit(&request.service_id).await?;

        let lease_duration = if request.lease_duration_seconds > 0 {
            Duration::from_secs(request.lease_duration_seconds)
        } else {
            self.config.default_lease_duration()
        };

        // Convert protobuf types to our internal types
        let object_type = match crate::proto::ObjectType::try_from(request.object_type) {
            Ok(proto_type) => ObjectType::from(proto_type),
            Err(_) => ObjectType::Unknown,
        };

        let cleanup_config = request.cleanup_config.map(CleanupConfig::from);

        let lease = Lease::new(
            request.object_id,
            object_type,
            request.service_id,
            lease_duration,
            request.metadata,
            cleanup_config,
        );

        match self.storage.create_lease(lease.clone()).await {
            Ok(_) => {
                // Record successful lease creation
                self.metrics
                    .lease_created(&lease.service_id, &format!("{:?}", lease.object_type));
                self.metrics
                    .record_storage_operation("create_lease", &self.config.storage.backend);

                info!(
                    lease_id = %lease.lease_id,
                    object_id = %lease.object_id,
                    service_id = %lease.service_id,
                    expires_at = %lease.expires_at,
                    "Created new lease"
                );

                Ok(CreateLeaseResponse {
                    lease_id: lease.lease_id,
                    expires_at: Some(prost_types::Timestamp {
                        seconds: lease.expires_at.timestamp(),
                        nanos: lease.expires_at.timestamp_subsec_nanos() as i32,
                    }),
                    success: true,
                    error_message: String::new(),
                })
            }
            Err(e) => {
                // Record failed lease creation
                self.metrics.record_storage_error(
                    "create_lease",
                    &self.config.storage.backend,
                    "create_operation_failed",
                );
                Err(e)
            }
        }
    }

    /// Renew an existing lease to extend its lifetime
    async fn renew_lease_impl(&self, request: RenewLeaseRequest) -> Result<RenewLeaseResponse> {
        // Simple validation
        let metrics = self.get_metrics();
        let validator = RequestValidator::new(self.get_config(), &metrics);
        validator.validate_renew_lease_request(&request)?;

        let mut lease = match self.storage.get_lease(&request.lease_id).await {
            Ok(Some(lease)) => {
                self.metrics
                    .record_storage_operation("get_lease", &self.config.storage.backend);
                lease
            }
            Ok(None) => {
                return Err(GCError::LeaseNotFound {
                    lease_id: request.lease_id.clone(),
                });
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "get_lease",
                    &self.config.storage.backend,
                    "get_operation_failed",
                );
                return Err(e);
            }
        };

        // Verify ownership
        if lease.service_id != request.service_id {
            return Err(GCError::UnauthorizedAccess {
                lease_id: request.lease_id,
                service_id: request.service_id,
            });
        }

        let extend_duration = if request.extend_duration_seconds > 0 {
            Duration::from_secs(request.extend_duration_seconds)
        } else {
            self.config.default_lease_duration()
        };

        // Validate extension duration
        if extend_duration.as_secs() > self.config.gc.max_lease_duration_seconds {
            return Err(GCError::InvalidLeaseDuration {
                duration: extend_duration.as_secs(),
                min: self.config.gc.min_lease_duration_seconds,
                max: self.config.gc.max_lease_duration_seconds,
            });
        }

        lease.renew(extend_duration)?;

        match self.storage.update_lease(lease.clone()).await {
            Ok(_) => {
                // Record successful renewal
                self.metrics.lease_renewed();
                self.metrics
                    .record_storage_operation("update_lease", &self.config.storage.backend);

                debug!(
                    lease_id = %lease.lease_id,
                    new_expires_at = %lease.expires_at,
                    renewal_count = lease.renewal_count,
                    "Renewed lease"
                );

                Ok(RenewLeaseResponse {
                    new_expires_at: Some(prost_types::Timestamp {
                        seconds: lease.expires_at.timestamp(),
                        nanos: lease.expires_at.timestamp_subsec_nanos() as i32,
                    }),
                    success: true,
                    error_message: String::new(),
                })
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "update_lease",
                    &self.config.storage.backend,
                    "update_operation_failed",
                );
                Err(e)
            }
        }
    }

    /// Release a lease immediately, triggering cleanup
    async fn release_lease_impl(
        &self,
        request: ReleaseLeaseRequest,
    ) -> Result<ReleaseLeaseResponse> {
        // Simple validation
        let metrics = self.get_metrics();
        let validator = RequestValidator::new(self.get_config(), &metrics);
        validator.validate_release_lease_request(&request)?;

        let mut lease = match self.storage.get_lease(&request.lease_id).await {
            Ok(Some(lease)) => {
                self.metrics
                    .record_storage_operation("get_lease", &self.config.storage.backend);
                lease
            }
            Ok(None) => {
                return Err(GCError::LeaseNotFound {
                    lease_id: request.lease_id.clone(),
                });
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "get_lease",
                    &self.config.storage.backend,
                    "get_operation_failed",
                );
                return Err(e);
            }
        };

        // Verify ownership
        if lease.service_id != request.service_id {
            return Err(GCError::UnauthorizedAccess {
                lease_id: request.lease_id,
                service_id: request.service_id,
            });
        }

        lease.release();

        match self.storage.update_lease(lease.clone()).await {
            Ok(_) => {
                // Record successful release
                self.metrics
                    .lease_released(&lease.service_id, &format!("{:?}", lease.object_type));
                self.metrics
                    .record_storage_operation("update_lease", &self.config.storage.backend);

                info!(
                    lease_id = %lease.lease_id,
                    object_id = %lease.object_id,
                    service_id = %lease.service_id,
                    "Released lease"
                );

                Ok(ReleaseLeaseResponse {
                    success: true,
                    error_message: String::new(),
                })
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "update_lease",
                    &self.config.storage.backend,
                    "update_operation_failed",
                );
                Err(e)
            }
        }
    }

    /// Get information about a specific lease
    async fn get_lease_impl(&self, request: GetLeaseRequest) -> Result<GetLeaseResponse> {
        // Simple validation
        let metrics = self.get_metrics();
        let validator = RequestValidator::new(self.get_config(), &metrics);
        validator.validate_lease_id(&request.lease_id)?;

        match self.storage.get_lease(&request.lease_id).await {
            Ok(lease) => {
                self.metrics
                    .record_storage_operation("get_lease", &self.config.storage.backend);
                Ok(GetLeaseResponse {
                    lease: lease.as_ref().map(|l| l.into()),
                    found: lease.is_some(),
                })
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "get_lease",
                    &self.config.storage.backend,
                    "get_operation_failed",
                );
                Err(e)
            }
        }
    }

    /// List leases with optional filtering
    async fn list_leases_impl(&self, request: ListLeasesRequest) -> Result<ListLeasesResponse> {
        let filter = LeaseFilter {
            service_id: if request.service_id.is_empty() {
                None
            } else {
                Some(request.service_id)
            },
            object_type: if request.object_type == 0 {
                None
            } else {
                crate::proto::ObjectType::try_from(request.object_type)
                    .ok()
                    .map(ObjectType::from)
            },
            state: if request.state == 0 {
                None
            } else {
                crate::proto::LeaseState::try_from(request.state)
                    .ok()
                    .map(LeaseState::from)
            },
            ..Default::default()
        };

        let limit = if request.limit > 0 {
            Some(request.limit as usize)
        } else {
            Some(100)
        };
        let offset = if request.page_token.is_empty() {
            None
        } else {
            request.page_token.parse::<usize>().ok()
        };

        match self.storage.list_leases(filter, limit, offset).await {
            Ok(leases) => {
                self.metrics
                    .record_storage_operation("list_leases", &self.config.storage.backend);
                let lease_infos: Vec<LeaseInfo> = leases.iter().map(|l| l.into()).collect();

                // Generate next page token
                let next_page_token = if lease_infos.len() == limit.unwrap_or(100) {
                    (offset.unwrap_or(0) + lease_infos.len()).to_string()
                } else {
                    String::new()
                };

                Ok(ListLeasesResponse {
                    leases: lease_infos,
                    next_page_token,
                })
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "list_leases",
                    &self.config.storage.backend,
                    "list_operation_failed",
                );
                Err(e)
            }
        }
    }

    /// Check the health status of the GarbageTruck service
    async fn health_check_impl(&self, _request: HealthCheckRequest) -> Result<HealthCheckResponse> {
        match self.storage.get_stats().await {
            Ok(stats) => {
                self.metrics
                    .record_storage_operation("get_stats", &self.config.storage.backend);

                // Simple health check - if we can get stats, we're healthy
                Ok(HealthCheckResponse {
                    healthy: true,
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    active_leases: stats.active_leases as u64,
                    expired_leases: stats.expired_leases as u64,
                    uptime: Some(prost_types::Timestamp {
                        seconds: self.start_time.elapsed().as_secs() as i64,
                        nanos: 0,
                    }),
                })
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "get_stats",
                    &self.config.storage.backend,
                    "stats_operation_failed",
                );
                Err(e)
            }
        }
    }

    /// Get metrics about the GarbageTruck service
    async fn get_metrics_impl(&self, _request: MetricsRequest) -> Result<MetricsResponse> {
        match self.storage.get_stats().await {
            Ok(stats) => {
                self.metrics
                    .record_storage_operation("get_stats", &self.config.storage.backend);

                // Update metrics with current stats
                self.metrics.update_lease_counts(&stats);

                Ok(MetricsResponse {
                    total_leases_created: self.metrics.leases_created_total.get(),
                    total_leases_renewed: self.metrics.leases_renewed_total.get(),
                    total_leases_expired: self.metrics.leases_expired_total.get(),
                    total_leases_released: self.metrics.leases_released_total.get(),
                    total_cleanup_operations: self.metrics.cleanup_operations_total.get(),
                    failed_cleanup_operations: self.metrics.cleanup_failures_total.get(),
                    active_leases: stats.active_leases as u64,
                    leases_by_service: stats
                        .leases_by_service
                        .iter()
                        .map(|(k, v)| (k.clone(), *v as u64))
                        .collect(),
                    leases_by_type: stats
                        .leases_by_type
                        .iter()
                        .map(|(k, v)| (k.clone(), *v as u64))
                        .collect(),
                })
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "get_stats",
                    &self.config.storage.backend,
                    "stats_operation_failed",
                );
                Err(e)
            }
        }
    }
}

/// Implementation of the actual gRPC service trait
#[tonic::async_trait]
impl crate::proto::distributed_gc_service_server::DistributedGcService for GCService {
    async fn create_lease(
        &self,
        request: Request<CreateLeaseRequest>,
    ) -> std::result::Result<Response<CreateLeaseResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();

        let result = self.create_lease_impl(req).await;
        let duration = start.elapsed().as_secs_f64();

        match result {
            Ok(response) => {
                self.metrics
                    .record_request("create_lease", "success", duration);
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics
                    .record_request("create_lease", "error", duration);
                error!(error = %e, "Failed to create lease");
                Err(Status::from(e))
            }
        }
    }

    async fn renew_lease(
        &self,
        request: Request<RenewLeaseRequest>,
    ) -> std::result::Result<Response<RenewLeaseResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();
        let lease_id = req.lease_id.clone();

        let result = self.renew_lease_impl(req).await;
        let duration = start.elapsed().as_secs_f64();

        match result {
            Ok(response) => {
                self.metrics
                    .record_request("renew_lease", "success", duration);
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics
                    .record_request("renew_lease", "error", duration);
                error!(lease_id = %lease_id, error = %e, "Failed to renew lease");

                let response = RenewLeaseResponse {
                    new_expires_at: None,
                    success: false,
                    error_message: e.to_string(),
                };
                Ok(Response::new(response))
            }
        }
    }

    async fn release_lease(
        &self,
        request: Request<ReleaseLeaseRequest>,
    ) -> std::result::Result<Response<ReleaseLeaseResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();
        let lease_id = req.lease_id.clone();

        let result = self.release_lease_impl(req).await;
        let duration = start.elapsed().as_secs_f64();

        match result {
            Ok(response) => {
                self.metrics
                    .record_request("release_lease", "success", duration);
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics
                    .record_request("release_lease", "error", duration);
                error!(lease_id = %lease_id, error = %e, "Failed to release lease");

                let response = ReleaseLeaseResponse {
                    success: false,
                    error_message: e.to_string(),
                };
                Ok(Response::new(response))
            }
        }
    }

    async fn get_lease(
        &self,
        request: Request<GetLeaseRequest>,
    ) -> std::result::Result<Response<GetLeaseResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();

        let result = self.get_lease_impl(req).await;
        let duration = start.elapsed().as_secs_f64();

        match result {
            Ok(response) => {
                self.metrics
                    .record_request("get_lease", "success", duration);
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics.record_request("get_lease", "error", duration);
                error!(error = %e, "Failed to get lease");
                Err(Status::from(e))
            }
        }
    }

    async fn list_leases(
        &self,
        request: Request<ListLeasesRequest>,
    ) -> std::result::Result<Response<ListLeasesResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();

        let result = self.list_leases_impl(req).await;
        let duration = start.elapsed().as_secs_f64();

        match result {
            Ok(response) => {
                self.metrics
                    .record_request("list_leases", "success", duration);
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics
                    .record_request("list_leases", "error", duration);
                error!(error = %e, "Failed to list leases");
                Err(Status::from(e))
            }
        }
    }

    async fn health_check(
        &self,
        request: Request<HealthCheckRequest>,
    ) -> std::result::Result<Response<HealthCheckResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();

        let result = self.health_check_impl(req).await;
        let duration = start.elapsed().as_secs_f64();

        match result {
            Ok(response) => {
                self.metrics
                    .record_request("health_check", "success", duration);
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics
                    .record_request("health_check", "error", duration);
                error!(error = %e, "Health check failed");
                Err(Status::from(e))
            }
        }
    }

    async fn get_metrics(
        &self,
        request: Request<MetricsRequest>,
    ) -> std::result::Result<Response<MetricsResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();

        let result = self.get_metrics_impl(req).await;
        let duration = start.elapsed().as_secs_f64();

        match result {
            Ok(response) => {
                self.metrics
                    .record_request("get_metrics", "success", duration);
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics
                    .record_request("get_metrics", "error", duration);
                error!(error = %e, "Failed to get metrics");
                Err(Status::from(e))
            }
        }
    }
}
