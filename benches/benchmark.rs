// benches/benchmark.rs - Comprehensive GarbageTruck Benchmark Suite
use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::Semaphore;
use uuid::Uuid;

// Import from the main crate
use garbagetruck::proto::{
    distributed_gc_service_client::DistributedGcServiceClient, CleanupConfig, CreateLeaseRequest,
    GetLeaseRequest, ListLeasesRequest, ObjectType, ReleaseLeaseRequest, RenewLeaseRequest,
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
    service_suffix: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let request = CreateLeaseRequest {
        object_id: format!("bench-object-{}", Uuid::new_v4()),
        object_type: ObjectType::CacheEntry as i32,
        service_id: format!("benchmark-service-{}", service_suffix),
        lease_duration_seconds: 300,
        metadata: HashMap::new(),
        cleanup_config: None,
    };

    let response = client.create_lease(request).await?;
    Ok(response.into_inner().lease_id)
}

async fn create_lease_with_cleanup_benchmark(
    client: &mut DistributedGcServiceClient<tonic::transport::Channel>,
    service_suffix: &str,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let request = CreateLeaseRequest {
        object_id: format!("cleanup-bench-{}", Uuid::new_v4()),
        object_type: ObjectType::TemporaryFile as i32,
        service_id: format!("benchmark-service-{}", service_suffix),
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

    let response = client.create_lease(request).await?;
    Ok(response.into_inner().lease_id)
}

async fn renew_lease_benchmark(
    client: &mut DistributedGcServiceClient<tonic::transport::Channel>,
    lease_id: &str,
    service_suffix: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request = RenewLeaseRequest {
        lease_id: lease_id.to_string(),
        service_id: format!("benchmark-service-{}", service_suffix),
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
    service_suffix: &str,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let request = ListLeasesRequest {
        service_id: format!("benchmark-service-{}", service_suffix),
        object_type: 0,
        state: 0,
        limit: 100,
        page_token: String::new(),
    };

    let response = client.list_leases(request).await?;
    Ok(response.into_inner().leases.len())
}

async fn release_lease_benchmark(
    client: &mut DistributedGcServiceClient<tonic::transport::Channel>,
    lease_id: &str,
    service_suffix: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let request = ReleaseLeaseRequest {
        lease_id: lease_id.to_string(),
        service_id: format!("benchmark-service-{}", service_suffix),
    };

    let _response = client.release_lease(request).await?;
    Ok(())
}

// Basic lease operations benchmark
fn bench_lease_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    // Setup: Create some leases for renewal/get benchmarks
    let lease_ids: Vec<String> = rt.block_on(async {
        let mut ids = Vec::new();

        if let Ok(mut client) = setup_client().await {
            for i in 0..10 {
                if let Ok(lease_id) = create_lease_benchmark(&mut client, &format!("setup-{}", i)).await {
                    ids.push(lease_id);
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
    group.throughput(Throughput::Elements(1));

    // Benchmark lease creation
    group.bench_function("create_lease", |b| {
        b.iter(|| {
            rt.block_on(async {
                if let Ok(mut client) = setup_client().await {
                    let _ = create_lease_benchmark(&mut client, &Uuid::new_v4().to_string()).await;
                }
            })
        })
    });

    // Benchmark lease renewal
    if !lease_ids.is_empty() {
        group.bench_function("renew_lease", |b| {
            b.iter(|| {
                rt.block_on(async {
                    if let Ok(mut client) = setup_client().await {
                        let lease_id = &lease_ids[0];
                        let _ = renew_lease_benchmark(&mut client, lease_id, "setup-0").await;
                    }
                })
            })
        });

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

    group.bench_function("list_leases", |b| {
        b.iter(|| {
            rt.block_on(async {
                if let Ok(mut client) = setup_client().await {
                    let _ = list_leases_benchmark(&mut client, "setup-0").await;
                }
            })
        })
    });

    group.finish();
}

// Concurrent operations benchmark
fn bench_concurrent_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let service_available = rt.block_on(async { setup_client().await.is_ok() });

    if !service_available {
        println!("Warning: GC service not available for concurrent benchmarks");
        return;
    }

    let mut group = c.benchmark_group("concurrent_operations");

    for concurrency in [1, 5, 10, 25, 50].iter() {
        group.throughput(Throughput::Elements(*concurrency as u64));
        
        group.bench_with_input(
            BenchmarkId::new("concurrent_create", concurrency),
            concurrency,
            |b, &concurrency| {
                b.iter(|| {
                    rt.block_on(async move {
                        let semaphore = Arc::new(Semaphore::new(concurrency));
                        let mut handles = Vec::new();

                        for i in 0..concurrency {
                            let permit = semaphore.clone().acquire_owned().await.unwrap();
                            let handle = tokio::spawn(async move {
                                let _permit = permit;
                                if let Ok(mut client) = setup_client().await {
                                    let service_suffix = format!("concurrent-{}-{}", concurrency, i);
                                    let _ = create_lease_benchmark(&mut client, &service_suffix).await;
                                }
                            });
                            handles.push(handle);
                        }

                        for handle in handles {
                            let _ = handle.await;
                        }
                    })
                })
            },
        );

        // Test concurrent renewals (simplified to avoid setup issues)
        if *concurrency <= 10 {  // Only test smaller concurrency levels for renewals
            group.bench_with_input(
                BenchmarkId::new("concurrent_renew", concurrency),
                concurrency,
                |b, &concurrency| {
                    b.iter(|| {
                        rt.block_on(async move {
                            // Create leases on-demand for renewal test
                            let mut lease_data = Vec::new();
                            
                            // First create the leases we'll renew
                            if let Ok(mut client) = setup_client().await {
                                for i in 0..concurrency {
                                    let service_suffix = format!("renew-{}-{}", concurrency, i);
                                    if let Ok(lease_id) = create_lease_benchmark(&mut client, &service_suffix).await {
                                        lease_data.push((lease_id, service_suffix));
                                    }
                                }
                            }
                            
                            // Now renew them concurrently
                            if lease_data.len() == concurrency {
                                let semaphore = Arc::new(Semaphore::new(concurrency));
                                let mut handles = Vec::new();

                                for (lease_id, service_suffix) in lease_data {
                                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                                    let handle = tokio::spawn(async move {
                                        let _permit = permit;
                                        if let Ok(mut client) = setup_client().await {
                                            let _ = renew_lease_benchmark(&mut client, &lease_id, &service_suffix).await;
                                        }
                                    });
                                    handles.push(handle);
                                }

                                for handle in handles {
                                    let _ = handle.await;
                                }
                            }
                        })
                    })
                },
            );
        }
    }

    group.finish();
}

// Cleanup operations benchmark
fn bench_cleanup_operations(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let service_available = rt.block_on(async { setup_client().await.is_ok() });

    if !service_available {
        println!("Warning: GC service not available for cleanup benchmarks");
        return;
    }

    let mut group = c.benchmark_group("cleanup_operations");
    group.throughput(Throughput::Elements(1));

    group.bench_function("create_lease_with_cleanup", |b| {
        b.iter(|| {
            rt.block_on(async {
                if let Ok(mut client) = setup_client().await {
                    let service_suffix = Uuid::new_v4().to_string();
                    let _ = create_lease_with_cleanup_benchmark(&mut client, &service_suffix).await;
                }
            })
        })
    });

    // Benchmark full lifecycle with cleanup config
    group.bench_function("full_lifecycle_with_cleanup", |b| {
        b.iter(|| {
            rt.block_on(async {
                if let Ok(mut client) = setup_client().await {
                    let service_suffix = Uuid::new_v4().to_string();
                    
                    // Create lease with cleanup
                    if let Ok(lease_id) = create_lease_with_cleanup_benchmark(&mut client, &service_suffix).await {
                        // Renew it once
                        let _ = renew_lease_benchmark(&mut client, &lease_id, &service_suffix).await;
                        
                        // Get lease info
                        let _ = get_lease_benchmark(&mut client, &lease_id).await;
                        
                        // Release it
                        let _ = release_lease_benchmark(&mut client, &lease_id, &service_suffix).await;
                    }
                }
            })
        })
    });

    group.finish();
}

// Memory usage and stress testing
fn bench_memory_usage(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let service_available = rt.block_on(async { setup_client().await.is_ok() });

    if !service_available {
        println!("Warning: GC service not available for memory usage benchmarks");
        return;
    }

    let mut group = c.benchmark_group("memory_usage");
    group.sample_size(10);

    for lease_count in [100, 500, 1000, 2500].iter() {
        group.throughput(Throughput::Elements(*lease_count as u64));
        
        group.bench_with_input(
            BenchmarkId::new("create_many_leases", lease_count),
            lease_count,
            |b, &lease_count| {
                b.iter(|| {
                    rt.block_on(async move {
                        if let Ok(mut client) = setup_client().await {
                            let service_suffix = Uuid::new_v4().to_string();
                            
                            for i in 0..lease_count {
                                let request = CreateLeaseRequest {
                                    object_id: format!("memory-test-{}-{}", service_suffix, i),
                                    object_type: ObjectType::CacheEntry as i32,
                                    service_id: format!("memory-test-service-{}", service_suffix),
                                    lease_duration_seconds: 3600,
                                    metadata: [
                                        ("iteration".to_string(), i.to_string()),
                                        ("batch".to_string(), "memory_test".to_string()),
                                        ("service".to_string(), service_suffix.clone()),
                                    ].into(),
                                    cleanup_config: None,
                                };

                                let _ = client.create_lease(request).await;

                                // Yield occasionally to prevent timeouts
                                if i % 100 == 0 {
                                    tokio::task::yield_now().await;
                                }
                            }
                        }
                    })
                })
            },
        );
    }

    // Test memory efficiency by creating and releasing leases
    group.bench_function("create_and_release_cycle", |b| {
        b.iter(|| {
            rt.block_on(async {
                if let Ok(mut client) = setup_client().await {
                    let service_suffix = Uuid::new_v4().to_string();
                    let mut lease_ids = Vec::new();
                    
                    // Create 50 leases
                    for i in 0..50 {
                        let request = CreateLeaseRequest {
                            object_id: format!("cycle-test-{}-{}", service_suffix, i),
                            object_type: ObjectType::TemporaryFile as i32,
                            service_id: format!("cycle-test-service-{}", service_suffix),
                            lease_duration_seconds: 300,
                            metadata: HashMap::new(),
                            cleanup_config: None,
                        };

                        if let Ok(response) = client.create_lease(request).await {
                            lease_ids.push(response.into_inner().lease_id);
                        }
                    }
                    
                    // Release all leases
                    for lease_id in lease_ids {
                        let _ = release_lease_benchmark(&mut client, &lease_id, &service_suffix).await;
                    }
                }
            })
        })
    });

    group.finish();
}

// Network latency impact benchmark
fn bench_network_latency(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let service_available = rt.block_on(async { setup_client().await.is_ok() });

    if !service_available {
        println!("Warning: GC service not available for network latency benchmarks");
        return;
    }

    let mut group = c.benchmark_group("network_latency");

    // Test payload size impact
    for metadata_size in [0, 10, 100, 500].iter() {
        group.bench_with_input(
            BenchmarkId::new("create_with_metadata_size", metadata_size),
            metadata_size,
            |b, &metadata_size| {
                b.iter(|| {
                    rt.block_on(async {
                        if let Ok(mut client) = setup_client().await {
                            let service_suffix = Uuid::new_v4().to_string();
                            
                            // Create metadata of specified size
                            let mut metadata = HashMap::new();
                            for i in 0..metadata_size {
                                metadata.insert(
                                    format!("key_{}", i),
                                    format!("value_with_some_data_{}", i),
                                );
                            }
                            
                            let request = CreateLeaseRequest {
                                object_id: format!("latency-test-{}", service_suffix),
                                object_type: ObjectType::CacheEntry as i32,
                                service_id: format!("latency-test-service-{}", service_suffix),
                                lease_duration_seconds: 300,
                                metadata,
                                cleanup_config: None,
                            };

                            let _ = client.create_lease(request).await;
                        }
                    })
                })
            },
        );
    }

    // Test connection reuse vs new connections
    group.bench_function("reused_connection", |b| {
        let client = rt.block_on(async { setup_client().await.ok() });
        
        if let Some(mut client) = client {
            b.iter(|| {
                rt.block_on(async {
                    let service_suffix = Uuid::new_v4().to_string();
                    let _ = create_lease_benchmark(&mut client, &service_suffix).await;
                })
            })
        }
    });

    group.bench_function("new_connection_each_time", |b| {
        b.iter(|| {
            rt.block_on(async {
                if let Ok(mut client) = setup_client().await {
                    let service_suffix = Uuid::new_v4().to_string();
                    let _ = create_lease_benchmark(&mut client, &service_suffix).await;
                }
            })
        })
    });

    // Test rapid sequential operations (burst testing)
    group.bench_function("burst_operations", |b| {
        b.iter(|| {
            rt.block_on(async {
                if let Ok(mut client) = setup_client().await {
                    let service_suffix = Uuid::new_v4().to_string();
                    
                    // Rapid-fire 10 operations
                    for i in 0..10 {
                        let request = CreateLeaseRequest {
                            object_id: format!("burst-{}-{}", service_suffix, i),
                            object_type: ObjectType::CacheEntry as i32,
                            service_id: format!("burst-service-{}", service_suffix),
                            lease_duration_seconds: 300,
                            metadata: HashMap::new(),
                            cleanup_config: None,
                        };
                        
                        let _ = client.create_lease(request).await;
                    }
                }
            })
        })
    });

    group.finish();
}

// Define the benchmark groups
criterion_group!(
    gc_benches,
    bench_lease_operations,
    bench_concurrent_operations,
    bench_cleanup_operations,
    bench_memory_usage,
    bench_network_latency
);

criterion_main!(gc_benches);

#[cfg(test)]
mod benchmark_tests {
    use super::*;

    #[test]
    fn test_benchmark_setup() {
        let rt = Runtime::new().unwrap();

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
                for i in 0..10 {
                    let service_suffix = format!("baseline-{}", i);
                    if create_lease_benchmark(&mut client, &service_suffix).await.is_err() {
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

    #[test]
    fn test_concurrent_performance() {
        let rt = Runtime::new().unwrap();

        let result = rt.block_on(async {
            if let Ok(_client) = setup_client().await {
                let start = Instant::now();
                let concurrency = 5;
                
                let mut handles = Vec::new();
                for i in 0..concurrency {
                    let handle = tokio::spawn(async move {
                        if let Ok(mut client) = setup_client().await {
                            let service_suffix = format!("concurrent-test-{}", i);
                            create_lease_benchmark(&mut client, &service_suffix).await.ok()
                        } else {
                            None
                        }
                    });
                    handles.push(handle);
                }

                let results: Vec<_> = futures::future::join_all(handles).await;
                let successful = results.into_iter().filter_map(|r| r.ok()).count();
                
                let duration = start.elapsed();
                let ops_per_sec = successful as f64 / duration.as_secs_f64();

                println!("Concurrent performance ({} threads): {:.2} ops/sec", concurrency, ops_per_sec);
                
                // Should handle at least 5 ops/sec with concurrency
                assert!(
                    ops_per_sec > 5.0,
                    "Concurrent performance below threshold: {:.2} ops/sec",
                    ops_per_sec
                );

                Ok(())
            } else {
                println!("Skipping concurrent test - GC service not available");
                Ok(())
            }
        });

        result.unwrap();
    }
}