// tests/integration/service/lifecycle.rs - Service lifecycle integration tests

use anyhow::Result;
use std::time::Duration;
use tokio::time::sleep;

use garbagetruck::shutdown::{ShutdownReason, TaskPriority, TaskType};

use crate::integration::{print_test_header, service::*};

#[tokio::test]
async fn test_gc_service_integration_with_shutdown() -> Result<()> {
    print_test_header("GC service integration with shutdown", "🔄");

    // Create GC service
    let gc_service = match create_test_service().await {
        Ok(service) => service,
        Err(e) => {
            println!("Skipping GC service integration test: {}", e);
            return Ok(()); // Skip if service creation fails (e.g., missing dependencies)
        }
    };

    // Create shutdown coordinator
    let coordinator = create_test_shutdown_coordinator();

    // Register cleanup task
    let cleanup_handle = coordinator
        .register_task(
            "cleanup-task".to_string(),
            TaskType::CleanupLoop,
            TaskPriority::Critical,
        )
        .await;

    // Start cleanup loop with shutdown support
    let cleanup_task = {
        let service = gc_service.clone();
        let handle = cleanup_handle.clone();

        tokio::spawn(async move {
            service.start_cleanup_loop_with_shutdown(handle).await;
        })
    };

    coordinator
        .update_task_handle("cleanup-task", cleanup_task)
        .await;

    // Let the cleanup loop run for a bit
    sleep(Duration::from_millis(500)).await;

    // Initiate shutdown
    coordinator
        .initiate_shutdown(ShutdownReason::Graceful)
        .await;

    // Verify shutdown completed
    let stats = coordinator.get_shutdown_stats().await.unwrap();
    assert_eq!(stats.total_tasks, 1);
    assert_eq!(stats.completed_tasks, 1);

    println!("✅ GC service integration test completed");
    Ok(())
}

#[tokio::test]
async fn test_realistic_service_shutdown_scenario() -> Result<()> {
    print_test_header("realistic service shutdown scenario", "🏢");

    let coordinator = create_test_shutdown_coordinator();

    // Simulate a realistic service with various components
    let components = vec![
        (
            "database_pool",
            TaskType::Custom("database".to_string()),
            TaskPriority::Critical,
            300,
        ),
        (
            "cache_manager",
            TaskType::Custom("cache".to_string()),
            TaskPriority::High,
            150,
        ),
        (
            "metrics_collector",
            TaskType::SystemMonitor,
            TaskPriority::High,
            100,
        ),
        (
            "http_server",
            TaskType::Custom("server".to_string()),
            TaskPriority::Low,
            200,
        ),
        (
            "cleanup_worker",
            TaskType::CleanupLoop,
            TaskPriority::Critical,
            400,
        ),
    ];

    for (name, task_type, priority, cleanup_time) in components {
        let handle = coordinator
            .register_task(name.to_string(), task_type, priority)
            .await;

        let task = tokio::spawn({
            let mut handle = handle.clone();
            let task_name = name.to_string();

            async move {
                // Simulate normal operation
                println!("🚀 {} started", task_name);

                handle.wait_for_shutdown().await;
                println!("🛑 {} received shutdown signal", task_name);

                // Simulate component-specific cleanup
                simulate_resource_cleanup(&task_name, cleanup_time).await;

                handle.mark_completed().await;
                println!("✅ {} shutdown completed", task_name);
            }
        });

        coordinator.update_task_handle(name, task).await;
    }

    // Let all components start
    sleep(Duration::from_millis(100)).await;

    println!("🛑 Initiating service shutdown...");
    let shutdown_start = std::time::Instant::now();

    coordinator
        .initiate_shutdown(ShutdownReason::Graceful)
        .await;

    let shutdown_duration = shutdown_start.elapsed();
    let stats = coordinator.get_shutdown_stats().await.unwrap();

    // Validate realistic shutdown scenario
    assert_eq!(stats.total_tasks, 5);
    assert_eq!(stats.completed_tasks, 5);
    assert_eq!(stats.failed_tasks, 0);
    assert_eq!(stats.forced_kills, 0);
    assert!(shutdown_duration < Duration::from_secs(8)); // Should complete well within timeout

    println!("✅ Realistic service shutdown completed");
    println!("Shutdown took: {:.2}s", shutdown_duration.as_secs_f64());
    println!("Final statistics: {:?}", stats);
    Ok(())
}

#[tokio::test]
async fn test_service_startup_and_shutdown_cycle() -> Result<()> {
    print_test_header("service startup and shutdown cycle", "🔄");

    // Test full service lifecycle
    let _gc_service = create_test_service().await?;
    let coordinator = create_test_shutdown_coordinator();

    // Register multiple service components
    let components = [
        (
            "startup_validator",
            TaskType::Custom("startup".to_string()),
            TaskPriority::Critical,
        ),
        (
            "config_loader",
            TaskType::Custom("config".to_string()),
            TaskPriority::High,
        ),
        (
            "dependency_checker",
            TaskType::Custom("deps".to_string()),
            TaskPriority::High,
        ),
        (
            "service_monitor",
            TaskType::SystemMonitor,
            TaskPriority::Normal,
        ),
    ];

    for (name, task_type, priority) in &components {
        let handle = coordinator
            .register_task(name.to_string(), task_type.clone(), priority.clone())
            .await;

        let task = tokio::spawn({
            let mut handle = handle.clone();
            let component_name = name.to_string();

            async move {
                println!("🚀 Starting component: {}", component_name);

                // Simulate startup work
                sleep(Duration::from_millis(100)).await;
                println!("✅ Component ready: {}", component_name);

                // Wait for shutdown
                handle.wait_for_shutdown().await;
                println!("🛑 Shutting down component: {}", component_name);

                // Simulate shutdown work
                sleep(Duration::from_millis(50)).await;
                handle.mark_completed().await;
                println!("✅ Component stopped: {}", component_name);
            }
        });

        coordinator.update_task_handle(name, task).await;
    }

    // Simulate service running
    sleep(Duration::from_millis(300)).await;

    // Graceful shutdown
    coordinator
        .initiate_shutdown(ShutdownReason::Graceful)
        .await;

    let stats = coordinator.get_shutdown_stats().await.unwrap();
    assert_eq!(stats.total_tasks, components.len());
    assert_eq!(stats.completed_tasks, components.len());
    assert_eq!(stats.failed_tasks, 0);

    println!("✅ Service lifecycle test completed");
    Ok(())
}

#[tokio::test]
async fn test_service_restart_scenario() -> Result<()> {
    print_test_header("service restart scenario", "🔄");

    let coordinator = create_test_shutdown_coordinator();

    // Register a service component
    let handle = coordinator
        .register_task(
            "restartable-service".to_string(),
            TaskType::Custom("service".to_string()),
            TaskPriority::Normal,
        )
        .await;

    let task = tokio::spawn({
        let mut handle = handle.clone();
        async move {
            println!("🚀 Service started and running");

            handle.wait_for_shutdown().await;
            println!("🔄 Service restarting (graceful shutdown for restart)");

            // Simulate saving state for restart
            sleep(Duration::from_millis(100)).await;

            handle.mark_completed().await;
            println!("✅ Service ready for restart");
        }
    });

    coordinator
        .update_task_handle("restartable-service", task)
        .await;

    // Let service run
    sleep(Duration::from_millis(200)).await;

    // Initiate restart
    coordinator.initiate_shutdown(ShutdownReason::Restart).await;

    let stats = coordinator.get_shutdown_stats().await.unwrap();
    assert_eq!(stats.total_tasks, 1);
    assert_eq!(stats.completed_tasks, 1);
    assert!(matches!(stats.reason, ShutdownReason::Restart));

    println!("✅ Service restart scenario completed");
    Ok(())
}
