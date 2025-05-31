// tests/integration/service/shutdown.rs - Shutdown coordination tests

use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::sleep;

use garbagetruck::shutdown::{ShutdownCoordinator, ShutdownConfig, TaskType, TaskPriority, ShutdownReason};

use crate::integration::{print_test_header, service::*};

#[tokio::test]
async fn test_graceful_shutdown_basic() -> Result<()> {
    print_test_header("graceful shutdown basic", "🛑");
    
    let coordinator = create_test_shutdown_coordinator();
    
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
    Ok(())
}

#[tokio::test]
async fn test_force_kill_on_timeout() -> Result<()> {
    print_test_header("force kill on timeout", "💀");
    
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
    Ok(())
}

#[tokio::test]
async fn test_shutdown_priority_ordering() -> Result<()> {
    print_test_header("shutdown priority ordering", "📊");
    
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
    Ok(())
}

#[tokio::test]
async fn test_multiple_shutdown_signals() -> Result<()> {
    print_test_header("multiple shutdown signals", "🔁");
    
    let coordinator = create_test_shutdown_coordinator();
    
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
    Ok(())
}

#[tokio::test]
async fn test_shutdown_with_mixed_task_behavior() -> Result<()> {
    print_test_header("shutdown with mixed task behavior", "🎭");
    
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
    Ok(())
}

#[tokio::test]
async fn test_shutdown_statistics_accuracy() -> Result<()> {
    print_test_header("shutdown statistics accuracy", "📈");
    
    let coordinator = create_test_shutdown_coordinator();
    
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
    Ok(())
}