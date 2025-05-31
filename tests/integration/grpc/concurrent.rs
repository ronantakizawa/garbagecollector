// tests/integration/grpc/concurrent.rs - Concurrency tests

use anyhow::Result;
use uuid::Uuid;

use garbagetruck::proto::ObjectType;

use crate::integration::{print_test_header, TestHarness};

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_concurrent_operations() -> Result<()> {
    print_test_header("concurrent operations", "🚀");

    let harness = TestHarness::new().await?;

    // Create multiple leases concurrently
    let mut handles = vec![];

    for i in 0..10 {
        let mut client = harness.gc_client.clone();
        let handle = tokio::spawn(async move {
            let object_id = format!("concurrent-test-{}-{}", i, Uuid::new_v4());
            let request = super::create_lease_request(
                &object_id,
                ObjectType::CacheEntry,
                &format!("test-service-{}", i),
                300,
            );

            client.create_lease(request).await
        });
        handles.push(handle);
    }

    // Wait for all operations to complete
    let mut successful_creates = 0;
    for handle in handles {
        if let Ok(response) = handle.await? {
            if response.into_inner().success {
                successful_creates += 1;
            }
        }
    }

    assert!(
        successful_creates >= 8,
        "Most concurrent operations should succeed"
    );
    println!(
        "✅ Concurrent operations passed ({}/10 successful)",
        successful_creates
    );
    Ok(())
}

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_concurrent_lease_renewals() -> Result<()> {
    print_test_header("concurrent lease renewals", "🔄");

    let mut harness = TestHarness::new().await?;

    // Create a single lease
    let object_id = format!("renewal-test-{}", Uuid::new_v4());
    let lease_id = harness.create_test_lease(&object_id, 300).await?;

    // Try to renew it concurrently from multiple tasks
    let mut handles = vec![];

    for i in 0..5 {
        let mut client = harness.gc_client.clone();
        let lease_id_clone = lease_id.clone();
        let handle = tokio::spawn(async move {
            use garbagetruck::proto::RenewLeaseRequest;

            let request = RenewLeaseRequest {
                lease_id: lease_id_clone,
                service_id: "test-service".to_string(),
                extend_duration_seconds: 300 + (i * 60), // Different extensions
            };

            client.renew_lease(request).await
        });
        handles.push(handle);
    }

    // Wait for all renewals to complete
    let mut successful_renewals = 0;
    for handle in handles {
        if let Ok(response) = handle.await? {
            if response.into_inner().success {
                successful_renewals += 1;
            }
        }
    }

    // At least some renewals should succeed (concurrent renewals are allowed)
    assert!(
        successful_renewals >= 1,
        "At least one renewal should succeed"
    );
    println!(
        "✅ Concurrent renewals passed ({}/5 successful)",
        successful_renewals
    );
    Ok(())
}

#[cfg(feature = "grpc")]
#[tokio::test]
async fn test_stress_lease_creation() -> Result<()> {
    print_test_header("stress lease creation", "💪");

    let harness = TestHarness::new().await?;
    let start_time = std::time::Instant::now();

    // Create many leases rapidly
    let mut handles = vec![];
    let num_leases = 50;

    for i in 0..num_leases {
        let mut client = harness.gc_client.clone();
        let handle = tokio::spawn(async move {
            let object_id = format!("stress-test-{}-{}", i, Uuid::new_v4());
            let request = super::create_lease_request(
                &object_id,
                ObjectType::TemporaryFile,
                "stress-test-service",
                300,
            );

            client.create_lease(request).await
        });
        handles.push(handle);
    }

    // Wait for all operations to complete
    let mut successful_creates = 0;
    for handle in handles {
        if let Ok(response) = handle.await? {
            if response.into_inner().success {
                successful_creates += 1;
            }
        }
    }

    let duration = start_time.elapsed();
    let success_rate = (successful_creates as f64 / num_leases as f64) * 100.0;

    assert!(
        success_rate >= 80.0,
        "Should have at least 80% success rate"
    );
    assert!(duration.as_secs() < 30, "Should complete within 30 seconds");

    println!(
        "✅ Stress test passed: {}/{} leases created ({:.1}%) in {:.2}s",
        successful_creates,
        num_leases,
        success_rate,
        duration.as_secs_f64()
    );
    Ok(())
}
