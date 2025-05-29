// benches/gc_benchmarks.rs
use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use distributed_gc_sidecar::proto::{
    distributed_gc_service_client::DistributedGcServiceClient,
    CreateLeaseRequest, RenewLeaseRequest, GetLeaseRequest, ListLeasesRequest,
    ObjectType, CleanupConfig,
};
use std::collections::HashMap;
use std::time::Duration;
use tokio::runtime::Runtime;
use uuid::Uuid;

async fn setup_client() -> DistributedGcServiceClient<tonic::transport::Channel> {
    DistributedGcServiceClient::connect("http://localhost:50051")
        .await
        .expect("Failed to connect to GC service")
}

async fn create_lease_benchmark(client: &mut DistributedGcServiceClient<tonic::transport::Channel>) {
    let request = CreateLeaseRequest {
        object_id: format!("bench-object-{}", Uuid::new_v4()),
        object_type: ObjectType::CacheEntry as i32,
        service_id: "benchmark-service".to_string(),
        lease_duration_seconds: 300,
        metadata: HashMap::new(),
        cleanup_config: None,
    };
    
    let _response = client.create_lease(request).await.expect("Create lease failed");
}

async fn renew_lease_benchmark(
    client: &mut DistributedGcServiceClient<tonic::transport::Channel>,
    lease_id: &str,
) {
    let request = RenewLeaseRequest {
        lease_id: lease_id.to_string(),
        service_id: "benchmark-service".to_string(),
        extend_duration_seconds: 300,
    };
    
    let _response = client.renew_lease(request).await.expect("Renew lease failed");
}

async fn get_lease_benchmark(
    client: &mut DistributedGcServiceClient<tonic::transport::Channel>,
    lease_id: &str,
) {
    let request = GetLeaseRequest {
        lease_id: lease_id.to_string(),
    };
    
    let _response = client.get_lease(request).await.expect("Get lease failed");
}

async fn list_leases_benchmark(client: &mut DistributedGcServiceClient<tonic::transport::Channel>) {
    let request = ListLeasesRequest {
        service_id: "benchmark-service".to_string(),
        object_type: 0,
        state: 0,
        limit: 100,
        page_token: String::new(),
    };
    
    let _response = client.list_leases(request).await.expect("List leases failed");
}

fn bench_lease_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    // Setup: Create some leases for renewal/get benchmarks
    let lease_ids: Vec<String> = rt.block_on(async {
        let mut client = setup_client().await;
        let mut ids = Vec::new();
        
        for i in 0..10 {
            let request = CreateLeaseRequest {
                object_id: format!("bench-setup-{}", i),
                object_type: ObjectType::DatabaseRow as i32,
                service_id: "benchmark-service".to_string(),
                lease_duration_seconds: 3600, // Long duration for benchmarking
                metadata: HashMap::new(),
                cleanup_config: None,
            };
            
            let response = client.create_lease(request).await.unwrap();
            ids.push(response.into_inner().lease_id);
        }
        
        ids
    });
    
    let mut group = c.benchmark_group("lease_operations");
    
    // Benchmark lease creation
    group.bench_function("create_lease", |b| {
        b.to_async(&rt).iter(|| async {
            let mut client = setup_client().await;
            create_lease_benchmark(&mut client).await;
        })
    });
    
    // Benchmark lease renewal
    group.bench_function("renew_lease", |b| {
        b.to_async(&rt).iter(|| async {
            let mut client = setup_client().await;
            let lease_id = &lease_ids[0]; // Use first lease for renewal
            renew_lease_benchmark(&mut client, lease_id).await;
        })
    });
    
    // Benchmark lease retrieval
    group.bench_function("get_lease", |b| {
        b.to_async(&rt).iter(|| async {
            let mut client = setup_client().await;
            let lease_id = &lease_ids[0];
            get_lease_benchmark(&mut client, lease_id).await;
        })
    });
    
    // Benchmark lease listing
    group.bench_function("list_leases", |b| {
        b.to_async(&rt).iter(|| async {
            let mut client = setup_client().await;
            list_leases_benchmark(&mut client).await;
        })
    });
    
    group.finish();
}

fn bench_concurrent_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("concurrent_operations");
    
    for concurrency in [1, 5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("concurrent_create", concurrency),
            concurrency,
            |b, &concurrency| {
                b.to_async(&rt).iter(|| async move {
                    let tasks: Vec<_> = (0..concurrency)
                        .map(|_| {
                            tokio::spawn(async {
                                let mut client = setup_client().await;
                                create_lease_benchmark(&mut client).await;
                            })
                        })
                        .collect();
                    
                    for task in tasks {
                        task.await.unwrap();
                    }
                })
            },
        );
    }
    
    group.finish();
}

fn bench_lease_with_cleanup_config(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    c.bench_function("create_lease_with_cleanup", |b| {
        b.to_async(&rt).iter(|| async {
            let mut client = setup_client().await;
            let request = CreateLeaseRequest {
                object_id: format!("cleanup-bench-{}", Uuid::new_v4()),
                object_type: ObjectType::TemporaryFile as i32,
                service_id: "benchmark-service".to_string(),
                lease_duration_seconds: 300,
                metadata: [("bench".to_string(), "true".to_string())].into(),
                cleanup_config: Some(CleanupConfig {
                    cleanup_endpoint: String::new(),
                    cleanup_http_endpoint: "http://localhost:8080/cleanup".to_string(),
                    cleanup_payload: r#"{"action":"delete","source":"benchmark"}"#.to_string(),
                    max_retries: 3,
                    retry_delay_seconds: 1,
                }),
            };
            
            let _response = client.create_lease(request).await.expect("Create lease failed");
        })
    });
}

fn bench_memory_usage(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("memory_usage");
    
    for lease_count in [100, 1000, 5000].iter() {
        group.bench_with_input(
            BenchmarkId::new("create_many_leases", lease_count),
            lease_count,
            |b, &lease_count| {
                b.to_async(&rt).iter(|| async move {
                    let mut client = setup_client().await;
                    
                    for i in 0..lease_count {
                        let request = CreateLeaseRequest {
                            object_id: format!("memory-test-{}-{}", Uuid::new_v4(), i),
                            object_type: ObjectType::CacheEntry as i32,
                            service_id: format!("memory-test-service-{}", i % 10),
                            lease_duration_seconds: 3600,
                            metadata: [
                                ("iteration".to_string(), i.to_string()),
                                ("batch".to_string(), "memory_test".to_string()),
                            ].into(),
                            cleanup_config: None,
                        };
                        
                        let _response = client.create_lease(request).await.expect("Create lease failed");
                        
                        // Yield occasionally to prevent timeouts
                        if i % 100 == 0 {
                            tokio::task::yield_now().await;
                        }
                    }
                })
            },
        );
    }
    
    group.finish();
}

criterion_group!(
    benches,
    bench_lease_operations,
    bench_concurrent_operations,
    bench_lease_with_cleanup_config,
    bench_memory_usage
);
criterion_main!(benches);

#[cfg(test)]
mod benchmark_tests {
    use super::*;
    
    #[tokio::test]
    async fn test_benchmark_setup() {
        // Verify that the benchmark setup works
        let mut client = setup_client().await;
        create_lease_benchmark(&mut client).await;
    }
    
    #[tokio::test]
    async fn test_performance_baseline() {
        let mut client = setup_client().await;
        let start = std::time::Instant::now();
        
        // Create 100 leases and measure time
        for _i in 0..100 {
            create_lease_benchmark(&mut client).await;
        }
        
        let duration = start.elapsed();
        let ops_per_sec = 100.0 / duration.as_secs_f64();
        
        println!("Performance baseline: {:.2} ops/sec", ops_per_sec);
        
        // Assert that we can handle at least 50 ops/sec
        assert!(ops_per_sec > 50.0, "Performance below acceptable threshold");
    }
}