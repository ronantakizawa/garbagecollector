// src/metrics/interceptors.rs - Fixed gRPC interceptors for metrics collection

use std::sync::Arc;
use std::time::Instant;
use tonic::{Request, Status};

use super::Metrics;

/// Tonic interceptor for collecting gRPC metrics
#[derive(Clone)]
pub struct MetricsInterceptor {
    metrics: Arc<Metrics>,
}

impl MetricsInterceptor {
    /// Create a new metrics interceptor
    pub fn new(metrics: Arc<Metrics>) -> Self {
        Self { metrics }
    }
}

impl tonic::service::Interceptor for MetricsInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        // For now, just increment a general request counter
        // The method name will be tracked in the service methods themselves
        self.metrics
            .request_total
            .with_label_values(&["all", "received"])
            .inc();

        // Store start time in request extensions for duration tracking
        let start_time = Instant::now();
        request
            .extensions_mut()
            .insert(RequestStartTime(start_time));

        Ok(request)
    }
}

/// Helper struct to store request start time in extensions
#[derive(Clone, Copy)]
pub struct RequestStartTime(pub Instant);

/// Helper functions for metrics collection in service methods
impl MetricsInterceptor {
    /// Record a completed request with timing
    pub fn record_request_completion(
        metrics: &Arc<Metrics>,
        method: &str,
        success: bool,
        duration: std::time::Duration,
    ) {
        let status = if success { "success" } else { "error" };
        let duration_secs = duration.as_secs_f64();

        metrics.record_request(method, status, duration_secs);
    }

    /// Extract start time from request extensions
    pub fn extract_start_time<T>(request: &Request<T>) -> Option<Instant> {
        request.extensions().get::<RequestStartTime>().map(|t| t.0)
    }
}
