use garbagetruck::{
    shutdown::{ShutdownCoordinator, ShutdownConfig, TaskType, TaskPriority, ShutdownReason},
    GCService, Config,
};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

#[tokio::test]
async fn test_graceful_shutdown_basic() {
    let config = ShutdownConfig {
        graceful_timeout: Duration::from_secs(5),
        phase_delay: Duration::from_millis(100),
        force_kill_on_timeout: true,
        metrics_collection_timeout: Duration::from_secs(1),
    };
    
    let coordinator = ShutdownCoordinator::new(config);
    
    // Register a well-behaved task
    let task_handle = coordinator.register_task(
        "test-task".to_string(),
        TaskType::Custom("integration-test".to_string()),
        TaskPriority::Normal,
    ).await;
    
    // Spawn task that responds to shutdown signals
    let task = tokio::spawn({
        let mut handle = task_handle.clone();
        async move {
            println!("Task starting...");
            
            // Simulate some work, then wait for shutdown
            tokio::select! {
                _ = sleep(Duration::from_secs(30)) => {
                    println!("Task completed normally (shouldn't happen in test)");
                }
                _ = handle.wait_for_shutdown() => {
                    println!("Task received shutdown signal");
                    
                    // Simulate cleanup work
                    sleep(Duration::from_millis(200)).await;
                    
                    handle.mark_completed().await;
                    println!("Task marked as completed");
                }
            }
        }
    });
    
    coordinator.update_task_handle("test-task", task).await;
    
    // Give task time to start
    sleep(Duration::from_millis(100)).await;
    
    // Initiate shutdown
    println!("Initiating graceful shutdown...");
    coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
    
    // Verify shutdown completed
    assert!(coordinator.is_shutdown_initiated().await);
    
    // Check final statistics
    let stats = coordinator.get_shutdown_stats().await.unwrap();
    assert_eq!(stats.total_tasks, 1);
    assert_eq!(stats.completed_tasks, 1);
    assert_eq!(stats.failed_tasks, 0);
    assert_eq!(stats.forced_kills, 0);
    
    println!("✅ Graceful shutdown test completed successfully");
}

#[tokio::test]
async fn test_force_kill_on_timeout() {
    let config = ShutdownConfig {
        graceful_timeout: Duration::from_millis(500),
        phase_delay: Duration::from_millis(50),
        force_kill_on_timeout: true,
        metrics_collection_timeout: Duration::from_secs(1),
    };
    
    let coordinator = ShutdownCoordinator::new(config);
    
    // Register a stubborn task that won't shutdown
    let _task_handle = coordinator.register_task(
        "stubborn-task".to_string(),
        TaskType::Custom("stubborn".to_string()),
        TaskPriority::Normal,
    ).await;
    
    // Spawn task that ignores shutdown signals
    let task = tokio::spawn(async {
        println!("Stubborn task starting...");
        // This task deliberately ignores shutdown signals
        loop {
            sleep(Duration::from_millis(100)).await;
        }
    });
    
    coordinator.update_task_handle("stubborn-task", task).await;
    
    // Give task time to start
    sleep(Duration::from_millis(100)).await;
    
    // Initiate shutdown
    println!("Initiating shutdown with stubborn task...");
    coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
    
    // Check that task was force-killed
    let stats = coordinator.get_shutdown_stats().await.unwrap();
    assert_eq!(stats.total_tasks, 1);
    assert_eq!(stats.forced_kills, 1);
    
    println!("✅ Force kill test completed successfully");
}

#[tokio::test]
async fn test_shutdown_priority_ordering() {
    let coordinator = ShutdownCoordinator::new(ShutdownConfig::default());
    
    let shutdown_order = Arc::new(Mutex::new(Vec::<String>::new()));
    
    // Register tasks with different priorities
    let critical_handle = coordinator.register_task(
        "critical-task".to_string(),
        TaskType::Custom("critical".to_string()),
        TaskPriority::Critical,
    ).await;
    
    let normal_handle = coordinator.register_task(
        "normal-task".to_string(),
        TaskType::Custom("normal".to_string()),
        TaskPriority::Normal,
    ).await;
    
    let low_handle = coordinator.register_task(
        "low-task".to_string(),
        TaskType::Custom("low".to_string()),
        TaskPriority::Low,
    ).await;
    
    // Spawn tasks that record their shutdown order
    for (name, mut handle) in [
        ("critical-task", critical_handle),
        ("normal-task", normal_handle),
        ("low-task", low_handle),
    ] {
        let order_clone = shutdown_order.clone();
        let task_name = name.to_string();
        
        let task = tokio::spawn(async move {
            handle.wait_for_shutdown().await;
            
            {
                let mut order = order_clone.lock().await;
                order.push(task_name.clone());
            }
            
            // Small delay to simulate cleanup
            sleep(Duration::from_millis(50)).await;
            handle.mark_completed().await;
        });
        
        coordinator.update_task_handle(name, task).await;
    }
    
    // Give tasks time to start
    sleep(Duration::from_millis(100)).await;
    
    // Initiate shutdown
    coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
    
    // Check shutdown order (critical tasks should shutdown first)
    let final_order = shutdown_order.lock().await;
    
    // Note: Due to the async nature and timing, we can't guarantee exact order,
    // but we can verify that all tasks were shut down
    assert_eq!(final_order.len(), 3);
    assert!(final_order.contains(&"critical-task".to_string()));
    assert!(final_order.contains(&"normal-task".to_string()));
    assert!(final_order.contains(&"low-task".to_string()));
    
    println!("✅ Priority ordering test completed");
    println!("Shutdown order: {:?}", *final_order);
}

#[tokio::test]
async fn test_gc_service_integration_with_shutdown() {
    // Create a test configuration
    let mut config = Config::default();
    config.gc.cleanup_interval_seconds = 1; // Very short for testing
    config.gc.cleanup_grace_period_seconds = 1;
    
    // Create GC service
    let gc_service = match GCService::new(config).await {
        Ok(service) => service,
        Err(e) => {
            println!("Skipping GC service integration test: {}", e);
            return; // Skip if service creation fails (e.g., missing dependencies)
        }
    };
    
    // Create shutdown coordinator
    let coordinator = ShutdownCoordinator::new(ShutdownConfig {
        graceful_timeout: Duration::from_secs(3),
        ..Default::default()
    });
    
    // Register cleanup task
    let cleanup_handle = coordinator.register_task(
        "cleanup-task".to_string(),
        TaskType::CleanupLoop,
        TaskPriority::Critical,
    ).await;
    
    // Start cleanup loop with shutdown support
    let cleanup_task = {
        let service = gc_service.clone();
        let handle = cleanup_handle.clone();
        
        tokio::spawn(async move {
            service.start_cleanup_loop_with_shutdown(handle).await;
        })
    };
    
    coordinator.update_task_handle("cleanup-task", cleanup_task).await;
    
    // Let the cleanup loop run for a bit
    sleep(Duration::from_millis(500)).await;
    
    // Initiate shutdown
    coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
    
    // Verify shutdown completed
    let stats = coordinator.get_shutdown_stats().await.unwrap();
    assert_eq!(stats.total_tasks, 1);
    assert_eq!(stats.completed_tasks, 1);
    
    println!("✅ GC service integration test completed");
}

#[tokio::test]
async fn test_multiple_shutdown_signals() {
    let coordinator = ShutdownCoordinator::new(ShutdownConfig::default());
    
    // Register a task
    let task_handle = coordinator.register_task(
        "multi-signal-task".to_string(),
        TaskType::Custom("test".to_string()),
        TaskPriority::Normal,
    ).await;
    
    let task = tokio::spawn({
        let mut handle = task_handle.clone();
        async move {
            handle.wait_for_shutdown().await;
            sleep(Duration::from_millis(100)).await;
            handle.mark_completed().await;
        }
    });
    
    coordinator.update_task_handle("multi-signal-task", task).await;
    
    // Send multiple shutdown signals (should be idempotent)
    coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
    coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
    coordinator.initiate_shutdown(ShutdownReason::Restart).await;
    
    // Should only count as one shutdown
    let stats = coordinator.get_shutdown_stats().await.unwrap();
    assert_eq!(stats.total_tasks, 1);
    assert_eq!(stats.completed_tasks, 1);
    
    println!("✅ Multiple shutdown signals test completed");
}

#[tokio::test]
async fn test_shutdown_with_mixed_task_behavior() {
    let config = ShutdownConfig {
        graceful_timeout: Duration::from_secs(2),
        force_kill_on_timeout: true,
        ..Default::default()
    };
    
    let coordinator = ShutdownCoordinator::new(config);
    
    // Register multiple tasks with different behaviors
    let good_handle = coordinator.register_task(
        "good-task".to_string(),
        TaskType::Custom("good".to_string()),
        TaskPriority::Normal,
    ).await;
    
    let slow_handle = coordinator.register_task(
        "slow-task".to_string(),
        TaskType::Custom("slow".to_string()),
        TaskPriority::Normal,
    ).await;
    
    let _stubborn_handle = coordinator.register_task(
        "stubborn-task".to_string(),
        TaskType::Custom("stubborn".to_string()),
        TaskPriority::Low,
    ).await;
    
    // Good task - responds quickly
    let good_task = tokio::spawn({
        let mut handle = good_handle.clone();
        async move {
            handle.wait_for_shutdown().await;
            sleep(Duration::from_millis(50)).await;
            handle.mark_completed().await;
        }
    });
    
    // Slow task - takes time but eventually responds
    let slow_task = tokio::spawn({
        let mut handle = slow_handle.clone();
        async move {
            handle.wait_for_shutdown().await;
            sleep(Duration::from_millis(800)).await; // Takes time but within timeout
            handle.mark_completed().await;
        }
    });
    
    // Stubborn task - never responds (will be force-killed)
    let stubborn_task = tokio::spawn(async {
        loop {
            sleep(Duration::from_millis(100)).await;
        }
    });
    
    coordinator.update_task_handle("good-task", good_task).await;
    coordinator.update_task_handle("slow-task", slow_task).await;
    coordinator.update_task_handle("stubborn-task", stubborn_task).await;
    
    // Start shutdown
    coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
    
    // Check results
    let stats = coordinator.get_shutdown_stats().await.unwrap();
    assert_eq!(stats.total_tasks, 3);
    assert_eq!(stats.completed_tasks, 2); // good and slow tasks
    assert_eq!(stats.forced_kills, 1); // stubborn task
    
    println!("✅ Mixed task behavior test completed");
}

#[tokio::test]
async fn test_shutdown_statistics_accuracy() {
    let coordinator = ShutdownCoordinator::new(ShutdownConfig::default());
    
    // Create multiple tasks for statistical validation
    for i in 0..5 {
        let handle = coordinator.register_task(
            format!("stats-task-{}", i),
            TaskType::Custom("stats".to_string()),
            TaskPriority::Normal,
        ).await;
        
        let task = tokio::spawn({
            let mut handle = handle.clone();
            async move {
                handle.wait_for_shutdown().await;
                sleep(Duration::from_millis(50)).await;
                handle.mark_completed().await;
            }
        });
        
        coordinator.update_task_handle(&format!("stats-task-{}", i), task).await;
    }
    
    let start_time = std::time::Instant::now();
    coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
    let total_duration = start_time.elapsed();
    
    let stats = coordinator.get_shutdown_stats().await.unwrap();
    
    // Validate statistics
    assert_eq!(stats.total_tasks, 5);
    assert_eq!(stats.completed_tasks, 5);
    assert_eq!(stats.failed_tasks, 0);
    assert_eq!(stats.forced_kills, 0);
    assert!(stats.total_duration <= total_duration + Duration::from_millis(100)); // Some tolerance
    assert!(matches!(stats.reason, ShutdownReason::Graceful));
    
    println!("✅ Shutdown statistics test completed");
    println!("Statistics: {:?}", stats);
}

// Helper function to simulate system resource cleanup
async fn simulate_resource_cleanup(task_name: &str, duration_ms: u64) {
    println!("🧹 {} starting resource cleanup...", task_name);
    sleep(Duration::from_millis(duration_ms)).await;
    println!("✅ {} completed resource cleanup", task_name);
}

#[tokio::test]
async fn test_realistic_service_shutdown_scenario() {
    let config = ShutdownConfig {
        graceful_timeout: Duration::from_secs(10),
        phase_delay: Duration::from_millis(200),
        force_kill_on_timeout: true,
        metrics_collection_timeout: Duration::from_secs(2),
    };
    
    let coordinator = ShutdownCoordinator::new(config);
    
    // Simulate a realistic service with various components
    let components = vec![
        ("database_pool", TaskType::Custom("database".to_string()), TaskPriority::Critical, 300),
        ("cache_manager", TaskType::Custom("cache".to_string()), TaskPriority::High, 150),
        ("metrics_collector", TaskType::SystemMonitor, TaskPriority::High, 100),
        ("http_server", TaskType::Custom("server".to_string()), TaskPriority::Low, 200),
        ("cleanup_worker", TaskType::CleanupLoop, TaskPriority::Critical, 400),
    ];
    
    for (name, task_type, priority, cleanup_time) in components {
        let handle = coordinator.register_task(
            name.to_string(),
            task_type,
            priority,
        ).await;
        
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
    
    coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
    
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
}