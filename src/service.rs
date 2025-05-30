use std::sync::Arc;
use std::time::{Duration, Instant};
use tonic::{Request, Response, Status};
use tracing::{debug, error, info, warn};

use crate::cleanup::CleanupExecutor;
use crate::config::Config;
use crate::error::{GCError, Result};
use crate::lease::{Lease, LeaseFilter, ObjectType, LeaseState, CleanupConfig};
use crate::metrics::{Metrics, AlertThresholds};
use crate::proto::{
    distributed_gc_service_server::DistributedGcService,
    CreateLeaseRequest, CreateLeaseResponse, RenewLeaseRequest, RenewLeaseResponse,
    ReleaseLeaseRequest, ReleaseLeaseResponse, GetLeaseRequest, GetLeaseResponse,
    ListLeasesRequest, ListLeasesResponse, HealthCheckRequest, HealthCheckResponse, 
    MetricsRequest, MetricsResponse, LeaseInfo,
};
use crate::storage::{create_storage, Storage};

#[derive(Clone)]
pub struct GCService {
    config: Config,
    storage: Arc<dyn Storage + Send + Sync>,
    cleanup_executor: CleanupExecutor,
    metrics: Arc<Metrics>,
    start_time: std::time::Instant,
}

impl GCService {
    pub async fn new(config: Config) -> Result<Self> {
        config.validate().map_err(|e| GCError::Configuration(e.to_string()))?;
        
        let storage = create_storage(&config).await?;
        let cleanup_executor = CleanupExecutor::new(
            Duration::from_secs(config.cleanup.default_timeout_seconds),
            config.cleanup.default_max_retries,
            Duration::from_secs(config.cleanup.default_retry_delay_seconds),
        );
        
        // Create metrics with custom alerting thresholds
        let alerting_thresholds = AlertThresholds {
            cleanup_failure_rate_threshold: 0.5,
            cleanup_failure_window_minutes: 5,
            consecutive_cleanup_failures_threshold: 5,
            consecutive_lease_failures_threshold: 10,
            storage_unavailable_threshold_seconds: 30,
            alert_cooldown_minutes: 15,
        };
        
        let metrics = Arc::new(
            Metrics::with_alerting_thresholds(alerting_thresholds)
                .map_err(|e| GCError::Internal(e.to_string()))?
        );
        
        // Start the alerting monitor
        let _alerting_handle = metrics.start_alerting_monitor();
        
        Ok(Self {
            config,
            storage,
            cleanup_executor,
            metrics,
            start_time: Instant::now(),
        })
    }
    
    /// Get a reference to the metrics for use in middleware
    pub fn get_metrics(&self) -> Arc<Metrics> {
        self.metrics.clone()
    }
    
    pub async fn start_cleanup_loop(&self) {
        let mut interval = tokio::time::interval(self.config.cleanup_interval());
        let grace_period = self.config.cleanup_grace_period();
        
        info!(
            interval_seconds = self.config.gc.cleanup_interval_seconds,
            grace_period_seconds = self.config.gc.cleanup_grace_period_seconds,
            "Starting cleanup loop"
        );
        
        loop {
            interval.tick().await;
            
            let start = Instant::now();
            match self.run_cleanup_cycle(grace_period).await {
                Ok(cleaned_count) => {
                    let duration = start.elapsed();
                    debug!(
                        cleaned_count = cleaned_count,
                        duration_ms = duration.as_millis(),
                        "Cleanup cycle completed"
                    );
                }
                Err(e) => {
                    error!(error = %e, "Cleanup cycle failed");
                    self.metrics.record_storage_error(
                        "cleanup_cycle",
                        &self.config.storage.backend,
                        "cleanup_cycle_failure"
                    );
                }
            }
        }
    }
    
    async fn run_cleanup_cycle(&self, grace_period: Duration) -> Result<usize> {
        // Get expired leases that need cleanup
        let expired_leases = match self.storage.get_expired_leases(grace_period).await {
            Ok(leases) => {
                self.metrics.record_storage_operation("get_expired_leases", &self.config.storage.backend);
                leases
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "get_expired_leases",
                    &self.config.storage.backend,
                    "storage_error"
                );
                return Err(e);
            }
        };
        
        if expired_leases.is_empty() {
            return Ok(0);
        }
        
        info!(count = expired_leases.len(), "Found expired leases to clean up");
        
        // Execute cleanup operations
        let cleanup_results = self.cleanup_executor.cleanup_batch(expired_leases.clone()).await;
        
        let mut successful_cleanups = 0;
        let mut failed_cleanups = 0;
        
        // Update lease states based on cleanup results
        for (lease, result) in expired_leases.iter().zip(cleanup_results.iter()) {
            if result.success {
                // Mark lease as expired in storage
                let mut updated_lease = lease.clone();
                updated_lease.expire();
                
                match self.storage.update_lease(updated_lease).await {
                    Ok(_) => {
                        successful_cleanups += 1;
                        self.metrics.lease_expired(
                            &lease.service_id,
                            &format!("{:?}", lease.object_type)
                        );
                        self.metrics.cleanup_succeeded();
                        self.metrics.record_storage_operation("update_lease", &self.config.storage.backend);
                    }
                    Err(e) => {
                        warn!(
                            lease_id = %lease.lease_id,
                            error = %e,
                            "Failed to update lease state after successful cleanup"
                        );
                        self.metrics.record_storage_error(
                            "update_lease",
                            &self.config.storage.backend,
                            "update_after_cleanup_failed"
                        );
                    }
                }
            } else {
                failed_cleanups += 1;
                self.metrics.cleanup_failed("cleanup_operation_failed");
                
                if let Some(ref error) = result.error {
                    warn!(
                        lease_id = %lease.lease_id,
                        object_id = %lease.object_id,
                        error = %error,
                        "Cleanup operation failed"
                    );
                }
            }
        }
        
        // Clean up storage (remove expired leases that have been processed)
        match self.storage.cleanup().await {
            Ok(_) => {
                self.metrics.record_storage_operation("cleanup", &self.config.storage.backend);
            }
            Err(e) => {
                warn!(error = %e, "Storage cleanup failed");
                self.metrics.record_storage_error(
                    "cleanup",
                    &self.config.storage.backend,
                    "storage_cleanup_failed"
                );
            }
        }
        
        info!(
            successful = successful_cleanups,
            failed = failed_cleanups,
            "Cleanup cycle completed"
        );
        
        Ok(successful_cleanups)
    }
    
    fn validate_lease_request(&self, request: &CreateLeaseRequest) -> Result<()> {
        if request.object_id.is_empty() {
            return Err(GCError::Configuration("Object ID cannot be empty".to_string()));
        }
        
        if request.service_id.is_empty() {
            return Err(GCError::Configuration("Service ID cannot be empty".to_string()));
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
    
    async fn check_service_lease_limit(&self, service_id: &str) -> Result<()> {
        let filter = LeaseFilter {
            service_id: Some(service_id.to_string()),
            state: Some(LeaseState::Active),
            ..Default::default()
        };
        
        let active_count = match self.storage.count_leases(filter).await {
            Ok(count) => {
                self.metrics.record_storage_operation("count_leases", &self.config.storage.backend);
                count
            }
            Err(e) => {
                self.metrics.record_storage_error(
                    "count_leases",
                    &self.config.storage.backend,
                    "count_operation_failed"
                );
                return Err(e);
            }
        };
        
        if active_count >= self.config.gc.max_leases_per_service {
            return Err(GCError::ServiceLeaseLimit {
                service_id: service_id.to_string(),
                current: active_count,
                max: self.config.gc.max_leases_per_service,
            });
        }
        
        Ok(())
    }
}

#[tonic::async_trait]
impl DistributedGcService for GCService {
    async fn create_lease(
        &self,
        request: Request<CreateLeaseRequest>,
    ) -> std::result::Result<Response<CreateLeaseResponse>, Status> {
        let start = Instant::now();
        let req = request.into_inner();
        
        let result: Result<CreateLeaseResponse> = async {
            self.validate_lease_request(&req)?;
            self.check_service_lease_limit(&req.service_id).await?;
            
            let lease_duration = if req.lease_duration_seconds > 0 {
                Duration::from_secs(req.lease_duration_seconds)
            } else {
                self.config.default_lease_duration()
            };
            
            // Convert protobuf types to our internal types
            let object_type = match crate::proto::ObjectType::try_from(req.object_type) {
                Ok(proto_type) => ObjectType::from(proto_type),
                Err(_) => ObjectType::Unknown,
            };
            
            let cleanup_config = req.cleanup_config.map(|config| CleanupConfig::from(config));
            
            let lease = Lease::new(
                req.object_id,
                object_type,
                req.service_id,
                lease_duration,
                req.metadata,
                cleanup_config,
            );
            
            match self.storage.create_lease(lease.clone()).await {
                Ok(_) => {
                    // Update metrics for successful lease creation
                    self.metrics.lease_created(&lease.service_id, &format!("{:?}", lease.object_type));
                    self.metrics.record_storage_operation("create_lease", &self.config.storage.backend);
                    
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
                    // Update metrics for failed lease creation
                    self.metrics.lease_creation_failed("storage_error");
                    self.metrics.record_storage_error(
                        "create_lease",
                        &self.config.storage.backend,
                        "create_operation_failed"
                    );
                    Err(e)
                }
            }
        }.await;
        
        let duration = start.elapsed().as_secs_f64();
        match result {
            Ok(response) => {
                self.metrics.record_request("create_lease", "success", duration);
                self.metrics.grpc_requests_total.with_label_values(&["create_lease"]).inc();
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics.record_request("create_lease", "error", duration);
                self.metrics.grpc_requests_total.with_label_values(&["create_lease"]).inc();
                error!(error = %e, "Failed to create lease");
                
                let response = CreateLeaseResponse {
                    lease_id: String::new(),
                    expires_at: None,
                    success: false,
                    error_message: e.to_string(),
                };
                Ok(Response::new(response))
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
        
        let result: Result<RenewLeaseResponse> = async {
            let mut lease = match self.storage.get_lease(&req.lease_id).await {
                Ok(Some(lease)) => {
                    self.metrics.record_storage_operation("get_lease", &self.config.storage.backend);
                    lease
                }
                Ok(None) => {
                    return Err(GCError::LeaseNotFound { lease_id: req.lease_id.clone() });
                }
                Err(e) => {
                    self.metrics.record_storage_error(
                        "get_lease",
                        &self.config.storage.backend,
                        "get_operation_failed"
                    );
                    return Err(e);
                }
            };
            
            // Verify ownership
            if lease.service_id != req.service_id {
                return Err(GCError::UnauthorizedAccess {
                    lease_id: req.lease_id,
                    service_id: req.service_id,
                });
            }
            
            let extend_duration = if req.extend_duration_seconds > 0 {
                Duration::from_secs(req.extend_duration_seconds)
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
                    // Update metrics
                    self.metrics.lease_renewed();
                    self.metrics.record_storage_operation("update_lease", &self.config.storage.backend);
                    
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
                        "update_operation_failed"
                    );
                    Err(e)
                }
            }
        }.await;
        
        let duration = start.elapsed().as_secs_f64();
        match result {
            Ok(response) => {
                self.metrics.record_request("renew_lease", "success", duration);
                self.metrics.grpc_requests_total.with_label_values(&["renew_lease"]).inc();
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics.record_request("renew_lease", "error", duration);
                self.metrics.grpc_requests_total.with_label_values(&["renew_lease"]).inc();
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
        
        let result: Result<ReleaseLeaseResponse> = async {
            let mut lease = match self.storage.get_lease(&req.lease_id).await {
                Ok(Some(lease)) => {
                    self.metrics.record_storage_operation("get_lease", &self.config.storage.backend);
                    lease
                }
                Ok(None) => {
                    return Err(GCError::LeaseNotFound { lease_id: req.lease_id.clone() });
                }
                Err(e) => {
                    self.metrics.record_storage_error(
                        "get_lease",
                        &self.config.storage.backend,
                        "get_operation_failed"
                    );
                    return Err(e);
                }
            };
            
            // Verify ownership
            if lease.service_id != req.service_id {
                return Err(GCError::UnauthorizedAccess {
                    lease_id: req.lease_id,
                    service_id: req.service_id,
                });
            }
            
            lease.release();
            
            match self.storage.update_lease(lease.clone()).await {
                Ok(_) => {
                    // Update metrics
                    self.metrics.lease_released(&lease.service_id, &format!("{:?}", lease.object_type));
                    self.metrics.record_storage_operation("update_lease", &self.config.storage.backend);
                    
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
                        "update_operation_failed"
                    );
                    Err(e)
                }
            }
        }.await;
        
        let duration = start.elapsed().as_secs_f64();
        match result {
            Ok(response) => {
                self.metrics.record_request("release_lease", "success", duration);
                self.metrics.grpc_requests_total.with_label_values(&["release_lease"]).inc();
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics.record_request("release_lease", "error", duration);
                self.metrics.grpc_requests_total.with_label_values(&["release_lease"]).inc();
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
        
        let result: Result<GetLeaseResponse> = async {
            match self.storage.get_lease(&req.lease_id).await {
                Ok(lease) => {
                    self.metrics.record_storage_operation("get_lease", &self.config.storage.backend);
                    Ok(GetLeaseResponse {
                        lease: lease.as_ref().map(|l| l.into()),
                        found: lease.is_some(),
                    })
                }
                Err(e) => {
                    self.metrics.record_storage_error(
                        "get_lease",
                        &self.config.storage.backend,
                        "get_operation_failed"
                    );
                    Err(e)
                }
            }
        }.await;
        
        let duration = start.elapsed().as_secs_f64();
        match result {
            Ok(response) => {
                self.metrics.record_request("get_lease", "success", duration);
                self.metrics.grpc_requests_total.with_label_values(&["get_lease"]).inc();
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics.record_request("get_lease", "error", duration);
                self.metrics.grpc_requests_total.with_label_values(&["get_lease"]).inc();
                error!(lease_id = %req.lease_id, error = %e, "Failed to get lease");
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
        
        let result: Result<ListLeasesResponse> = async {
            let filter = LeaseFilter {
                service_id: if req.service_id.is_empty() { None } else { Some(req.service_id) },
                object_type: if req.object_type == 0 { 
                    None 
                } else { 
                    crate::proto::ObjectType::try_from(req.object_type)
                        .ok()
                        .map(|pt| ObjectType::from(pt))
                },
                state: if req.state == 0 { 
                    None 
                } else { 
                    crate::proto::LeaseState::try_from(req.state)
                        .ok()
                        .map(|ps| LeaseState::from(ps))
                },
                ..Default::default()
            };
            
            let limit = if req.limit > 0 { Some(req.limit as usize) } else { Some(100) };
            let offset = if req.page_token.is_empty() {
                None
            } else {
                req.page_token.parse::<usize>().ok()
            };
            
            match self.storage.list_leases(filter, limit, offset).await {
                Ok(leases) => {
                    self.metrics.record_storage_operation("list_leases", &self.config.storage.backend);
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
                        "list_operation_failed"
                    );
                    Err(e)
                }
            }
        }.await;
        
        let duration = start.elapsed().as_secs_f64();
        match result {
            Ok(response) => {
                self.metrics.record_request("list_leases", "success", duration);
                self.metrics.grpc_requests_total.with_label_values(&["list_leases"]).inc();
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics.record_request("list_leases", "error", duration);
                self.metrics.grpc_requests_total.with_label_values(&["list_leases"]).inc();
                error!(error = %e, "Failed to list leases");
                Err(Status::from(e))
            }
        }
    }
    
    async fn health_check(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> std::result::Result<Response<HealthCheckResponse>, Status> {
        let start = Instant::now();
        
        let result: Result<HealthCheckResponse> = async {
            match self.storage.get_stats().await {
                Ok(stats) => {
                    self.metrics.record_storage_operation("get_stats", &self.config.storage.backend);
                    
                    // Get alert summary for health status
                    let alert_summary = self.metrics.get_alert_summary().await;
                    let is_healthy = alert_summary.fatal_alerts == 0;
                    
                    Ok(HealthCheckResponse {
                        healthy: is_healthy,
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
                        "stats_operation_failed"
                    );
                    Err(e)
                }
            }
        }.await;
        
        let duration = start.elapsed().as_secs_f64();
        match result {
            Ok(response) => {
                self.metrics.record_request("health_check", "success", duration);
                self.metrics.grpc_requests_total.with_label_values(&["health_check"]).inc();
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics.record_request("health_check", "error", duration);
                self.metrics.grpc_requests_total.with_label_values(&["health_check"]).inc();
                error!(error = %e, "Health check failed");
                Err(Status::from(e))
            }
        }
    }
    
    async fn get_metrics(
        &self,
        _request: Request<MetricsRequest>,
    ) -> std::result::Result<Response<MetricsResponse>, Status> {
        let start = Instant::now();
        
        let result: Result<MetricsResponse> = async {
            match self.storage.get_stats().await {
                Ok(stats) => {
                    self.metrics.record_storage_operation("get_stats", &self.config.storage.backend);
                    
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
                        leases_by_service: stats.leases_by_service.iter()
                            .map(|(k, v)| (k.clone(), *v as u64))
                            .collect(),
                        leases_by_type: stats.leases_by_type.iter()
                            .map(|(k, v)| (k.clone(), *v as u64))
                            .collect(),
                    })
                }
                Err(e) => {
                    self.metrics.record_storage_error(
                        "get_stats",
                        &self.config.storage.backend,
                        "stats_operation_failed"
                    );
                    Err(e)
                }
            }
        }.await;
        
        let duration = start.elapsed().as_secs_f64();
        match result {
            Ok(response) => {
                self.metrics.record_request("get_metrics", "success", duration);
                self.metrics.grpc_requests_total.with_label_values(&["get_metrics"]).inc();
                Ok(Response::new(response))
            }
            Err(e) => {
                self.metrics.record_request("get_metrics", "error", duration);
                self.metrics.grpc_requests_total.with_label_values(&["get_metrics"]).inc();
                error!(error = %e, "Failed to get metrics");
                Err(Status::from(e))
            }
        }
    }
}