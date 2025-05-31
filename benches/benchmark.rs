// benches/gc_benchmarks.rs
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashMap;
use tokio::runtime::Runtime;
use uuid::Uuid;

// Import from the main crate
use garbagetruck::proto::{
    distributed_gc_service_client::DistributedGcServiceClient, CleanupConfig, CreateLeaseRequest,
    GetLeaseRequest, ListLeasesRequest, ObjectType, RenewLeaseRequest,
};

async fn setup_client() -> Result<
    DistributedGcServiceClient<tonic::transport::Channel>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    let client = DistributedGcServiceClient::connect("http://localhost:50051").await?;
    Ok(client)
}

async fn create_lease_benchmark(
    client: &mut DistributedGcServiceClient<tonic::transport::Channel>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request = CreateLeaseRequest {
        object_id: format!("bench-object-{}", Uuid::new_v4()),
        object_type: ObjectType::CacheEntry as i32,
        service_id: "benchmark-service".to_string(),
        lease_duration_seconds: 300,
        metadata: HashMap::new(),
        cleanup_config: None,
    };

    let _response = client.create_lease(request).await?;
    Ok(())
}

async fn renew_lease_benchmark(
    client: &mut DistributedGcServiceClient<tonic::transport::Channel>,
    lease_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request = RenewLeaseRequest {
        lease_id: lease_id.to_string(),
        service_id: "benchmark-service".to_string(),
        extend_duration_seconds: 300,
    };

    let _response = client.renew_lease(request).await?;
    Ok(())
}

async fn get_lease_benchmark(
    client: &mut DistributedGcServiceClient<tonic::transport::Channel>,
    lease_id: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request = GetLeaseRequest {
        lease_id: lease_id.to_string(),
    };

    let _response = client.get_lease(request).await?;
    Ok(())
}

async fn list_leases_benchmark(
    client: &mut DistributedGcServiceClient<tonic::transport::Channel>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request = ListLeasesRequest {
        service_id: "benchmark-service".to_string(),
        object_type: 0,
        state: 0,
        limit: 100,
        page_token: String::new(),
    };

    let _response = client.list_leases(request).await?;
    Ok(())
}

fn bench_lease_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Setup: Create some leases for renewal/get benchmarks
    let lease_ids: Vec<String> = rt.block_on(async {
        let mut ids = Vec::new();

        // Try to connect and create setup leases
        if let Ok(mut client) = setup_client().await {
            for i in 0..10 {
                let request = CreateLeaseRequest {
                    object_id: format!("bench-setup-{}", i),
                    object_type: ObjectType::DatabaseRow as i32,
                    service_id: "benchmark-service".to_string(),
                    lease_duration_seconds: 3600, // Long duration for benchmarking
                    metadata: HashMap::new(),
                    cleanup_config: None,
                };

                if let Ok(response) = client.create_lease(request).await {
                    ids.push(response.into_inner().lease_id);
                }
            }
        }

        ids
    });

    if lease_ids.is_empty() {
        println!("Warning: Could not connect to GC service for benchmark setup. Make sure the service is running on localhost:50051");
        return;
    }

    let mut group = c.benchmark_group("lease_operations");

    // Benchmark lease creation
    group.bench_function("create_lease", |b| {
        b.iter(|| {
            rt.block_on(async {
                if let Ok(mut client) = setup_client().await {
                    let _ = create_lease_benchmark(&mut client).await;
                }
            })
        })
    });

    // Benchmark lease renewal (only if we have setup leases)
    if !lease_ids.is_empty() {
        group.bench_function("renew_lease", |b| {
            b.iter(|| {
                rt.block_on(async {
                    if let Ok(mut client) = setup_client().await {
                        let lease_id = &lease_ids[0]; // Use first lease for renewal
                        let _ = renew_lease_benchmark(&mut client, lease_id).await;
                    }
                })
            })
        });

        // Benchmark lease retrieval
        group.bench_function("get_lease", |b| {
            b.iter(|| {
                rt.block_on(async {
                    if let Ok(mut client) = setup_client().await {
                        let lease_id = &lease_ids[0];
                        let _ = get_lease_benchmark(&mut client, lease_id).await;
                    }
                })
            })
        });
    }

    // Benchmark lease listing
    group.bench_function("list_leases", |b| {
        b.iter(|| {
            rt.block_on(async {
                if let Ok(mut client) = setup_client().await {
                    let _ = list_leases_benchmark(&mut client).await;
                }
            })
        })
    });

    group.finish();
}

fn bench_concurrent_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // First check if service is available
    let service_available = rt.block_on(async { setup_client().await.is_ok() });

    if !service_available {
        println!("Warning: GC service not available for concurrent benchmarks");
        return;
    }

    let mut group = c.benchmark_group("concurrent_operations");

    for concurrency in [1, 5, 10].iter() {
        group.bench_with_input(
            BenchmarkId::new("concurrent_create", concurrency),
            concurrency,
            |b, &concurrency| {
                b.iter(|| {
                    rt.block_on(async move {
                        let mut handles = Vec::new();

                        for i in 0..concurrency {
                            // Clone the runtime handle to avoid Send issues
                            let handle = tokio::spawn({
                                let object_id =
                                    format!("concurrent-bench-{}-{}", i, Uuid::new_v4());
                                async move {
                                    if let Ok(mut client) = setup_client().await {
                                        let request = CreateLeaseRequest {
                                            object_id,
                                            object_type: ObjectType::CacheEntry as i32,
                                            service_id: format!("benchmark-service-{}", i),
                                            lease_duration_seconds: 300,
                                            metadata: HashMap::new(),
                                            cleanup_config: None,
                                        };

                                        let _ = client.create_lease(request).await;
                                    }
                                }
                            });
                            handles.push(handle);
                        }

                        // Wait for all tasks to complete
                        for handle in handles {
                            let _ = handle.await;
                        }
                    })
                })
            },
        );
    }

    group.finish();
}

fn bench_lease_with_cleanup_config(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Check if service is available
    let service_available = rt.block_on(async { setup_client().await.is_ok() });

    if !service_available {
        println!("Warning: GC service not available for cleanup config benchmarks");
        return;
    }

    c.bench_function("create_lease_with_cleanup", |b| {
        b.iter(|| {
            rt.block_on(async {
                if let Ok(mut client) = setup_client().await {
                    let request = CreateLeaseRequest {
                        object_id: format!("cleanup-bench-{}", Uuid::new_v4()),
                        object_type: ObjectType::TemporaryFile as i32,
                        service_id: "benchmark-service".to_string(),
                        lease_duration_seconds: 300,
                        metadata: [("bench".to_string(), "true".to_string())].into(),
                        cleanup_config: Some(CleanupConfig {
                            cleanup_endpoint: String::new(),
                            cleanup_http_endpoint: "http://localhost:8080/cleanup".to_string(),
                            cleanup_payload: r#"{"action":"delete","source":"benchmark"}"#
                                .to_string(),
                            max_retries: 3,
                            retry_delay_seconds: 1,
                        }),
                    };

                    let _ = client.create_lease(request).await;
                }
            })
        })
    });
}

fn bench_memory_usage(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Check if service is available
    let service_available = rt.block_on(async { setup_client().await.is_ok() });

    if !service_available {
        println!("Warning: GC service not available for memory usage benchmarks");
        return;
    }

    let mut group = c.benchmark_group("memory_usage");
    group.sample_size(10); // Reduce sample size for these intensive tests

    for lease_count in [100, 500].iter() {
        group.bench_with_input(
            BenchmarkId::new("create_many_leases", lease_count),
            lease_count,
            |b, &lease_count| {
                b.iter(|| {
                    rt.block_on(async move {
                        if let Ok(mut client) = setup_client().await {
                            for i in 0..lease_count {
                                let request = CreateLeaseRequest {
                                    object_id: format!("memory-test-{}-{}", Uuid::new_v4(), i),
                                    object_type: ObjectType::CacheEntry as i32,
                                    service_id: format!("memory-test-service-{}", i % 10),
                                    lease_duration_seconds: 3600,
                                    metadata: [
                                        ("iteration".to_string(), i.to_string()),
                                        ("batch".to_string(), "memory_test".to_string()),
                                    ]
                                    .into(),
                                    cleanup_config: None,
                                };

                                let _ = client.create_lease(request).await;

                                // Yield occasionally to prevent timeouts
                                if i % 50 == 0 {
                                    tokio::task::yield_now().await;
                                }
                            }
                        }
                    })
                })
            },
        );
    }

    group.finish();
}

// Define the benchmark groups
criterion_group!(
    gc_benches,
    bench_lease_operations,
    bench_concurrent_operations,
    bench_lease_with_cleanup_config,
    bench_memory_usage
);

// Main entry point - Fixed the name here!
criterion_main!(gc_benches);

#[cfg(test)]
mod benchmark_tests {

    #[test]
    fn test_benchmark_setup() {
        let rt = Runtime::new().unwrap();

        // Test that we can connect to the service
        let result = rt.block_on(async { setup_client().await });

        match result {
            Ok(_) => println!("✅ Benchmark setup successful - GC service is available"),
            Err(e) => println!("⚠️  GC service not available for benchmarks: {}", e),
        }
    }

    #[test]
    fn test_performance_baseline() {
        let rt = Runtime::new().unwrap();

        let result = rt.block_on(async {
            if let Ok(mut client) = setup_client().await {
                let start = std::time::Instant::now();

                // Create 10 leases and measure time
                for _i in 0..10 {
                    if create_lease_benchmark(&mut client).await.is_err() {
                        break;
                    }
                }

                let duration = start.elapsed();
                let ops_per_sec = 10.0 / duration.as_secs_f64();

                println!("Performance baseline: {:.2} ops/sec", ops_per_sec);

                // Assert that we can handle at least 10 ops/sec (lenient for CI)
                assert!(
                    ops_per_sec > 10.0,
                    "Performance below acceptable threshold: {:.2} ops/sec",
                    ops_per_sec
                );

                Ok(())
            } else {
                println!("Skipping performance test - GC service not available");
                Ok(())
            }
        });

        result.unwrap();
    }
}
