// src/shutdown.rs - Graceful shutdown management system

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, watch, Mutex};
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

/// Graceful shutdown coordinator that manages the lifecycle of background tasks
#[derive(Debug, Clone)]
pub struct ShutdownCoordinator {
    /// Broadcast sender for shutdown signals
    shutdown_tx: broadcast::Sender<ShutdownReason>,
    /// Tracks registered tasks and their status
    task_registry: Arc<Mutex<TaskRegistry>>,
    /// Configuration for shutdown behavior
    config: ShutdownConfig,
}

/// Reasons for shutdown initiation
#[derive(Debug, Clone)]
pub enum ShutdownReason {
    /// Graceful shutdown requested (SIGTERM, Ctrl+C)
    Graceful,
    /// Forced shutdown due to timeout
    Forced,
    /// Critical error requiring immediate shutdown
    Critical(String),
    /// Service restart requested
    Restart,
}

/// Configuration for shutdown behavior
#[derive(Debug, Clone)]
pub struct ShutdownConfig {
    /// Maximum time to wait for tasks to complete gracefully
    pub graceful_timeout: Duration,
    /// Time to wait between shutdown phases
    pub phase_delay: Duration,
    /// Whether to force kill tasks after timeout
    pub force_kill_on_timeout: bool,
    /// Maximum time to wait for metrics collection during shutdown
    pub metrics_collection_timeout: Duration,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            graceful_timeout: Duration::from_secs(30),
            phase_delay: Duration::from_millis(500),
            force_kill_on_timeout: true,
            metrics_collection_timeout: Duration::from_secs(5),
        }
    }
}

/// Registry of active background tasks
#[derive(Debug)]
struct TaskRegistry {
    tasks: std::collections::HashMap<String, TaskInfo>,
    shutdown_started: bool,
    shutdown_reason: Option<ShutdownReason>,
    shutdown_start_time: Option<Instant>,
}

/// Information about a registered background task
#[derive(Debug)]
struct TaskInfo {
    _name: String,
    _task_type: TaskType,
    handle: tokio::task::JoinHandle<()>,
    shutdown_tx: Option<watch::Sender<bool>>,
    status: TaskStatus,
    _registered_at: Instant,
    priority: TaskPriority,
}

/// Types of background tasks
#[derive(Debug, Clone, PartialEq)]
pub enum TaskType {
    /// Core cleanup loop
    CleanupLoop,
    /// Metrics collection and monitoring
    MetricsMonitor,
    /// System monitoring (memory, etc.)
    SystemMonitor,
    /// Alerting system
    AlertingMonitor,
    /// gRPC server
    GrpcServer,
    /// Custom user task
    Custom(String),
}

/// Task execution status
#[derive(Debug, Clone, PartialEq)]
enum TaskStatus {
    /// Task is running normally
    Running,
    /// Shutdown signal sent, waiting for completion
    ShuttingDown,
    /// Task completed successfully
    Completed,
    /// Task failed or was cancelled
    Failed(String),
}

/// Priority for shutdown ordering
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum TaskPriority {
    /// Shutdown last (servers, listeners)
    Low = 1,
    /// Normal shutdown order
    Normal = 2,
    /// Shutdown early (monitoring, alerting)
    High = 3,
    /// Shutdown first (critical cleanup tasks)
    Critical = 4,
}

/// Handle for managing a specific background task
#[derive(Debug, Clone)]
pub struct TaskHandle {
    name: String,
    shutdown_rx: watch::Receiver<bool>,
    coordinator: ShutdownCoordinator,
}

/// Shutdown statistics for monitoring and debugging
#[derive(Debug, Clone)]
pub struct ShutdownStats {
    pub reason: ShutdownReason,
    pub total_tasks: usize,
    pub completed_tasks: usize,
    pub failed_tasks: usize,
    pub forced_kills: usize,
    pub total_duration: Duration,
    pub phase_durations: std::collections::HashMap<String, Duration>,
}

impl ShutdownCoordinator {
    /// Create a new shutdown coordinator
    pub fn new(config: ShutdownConfig) -> Self {
        let (shutdown_tx, _) = broadcast::channel(16);

        Self {
            shutdown_tx,
            task_registry: Arc::new(Mutex::new(TaskRegistry {
                tasks: std::collections::HashMap::new(),
                shutdown_started: false,
                shutdown_reason: None,
                shutdown_start_time: None,
            })),
            config,
        }
    }

    /// Register a new background task
    pub async fn register_task(
        &self,
        name: String,
        task_type: TaskType,
        priority: TaskPriority,
    ) -> TaskHandle {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        // Create a placeholder handle for now - will be updated when task starts
        let handle = tokio::spawn(async {});

        let task_info = TaskInfo {
            _name: name.clone(),
            _task_type: task_type,
            handle,
            shutdown_tx: Some(shutdown_tx),
            status: TaskStatus::Running,
            _registered_at: Instant::now(),
            priority,
        };

        {
            let mut registry = self.task_registry.lock().await;
            registry.tasks.insert(name.clone(), task_info);
        }

        debug!("Registered background task: {}", name);

        TaskHandle {
            name,
            shutdown_rx,
            coordinator: self.clone(),
        }
    }

    /// Update task handle after spawning the actual task
    pub async fn update_task_handle(&self, name: &str, handle: tokio::task::JoinHandle<()>) {
        let mut registry = self.task_registry.lock().await;
        if let Some(task_info) = registry.tasks.get_mut(name) {
            task_info.handle = handle;
        }
    }

    /// Start listening for shutdown signals (SIGTERM, SIGINT)
    pub async fn listen_for_signals(&self) {
        let coordinator = self.clone();

        tokio::spawn(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};

                let mut sigterm =
                    signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");
                let mut sigint =
                    signal(SignalKind::interrupt()).expect("Failed to register SIGINT handler");
                let mut sigquit =
                    signal(SignalKind::quit()).expect("Failed to register SIGQUIT handler");

                tokio::select! {
                    _ = sigterm.recv() => {
                        info!("Received SIGTERM, initiating graceful shutdown");
                        coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
                    }
                    _ = sigint.recv() => {
                        info!("Received SIGINT (Ctrl+C), initiating graceful shutdown");
                        coordinator.initiate_shutdown(ShutdownReason::Graceful).await;
                    }
                    _ = sigquit.recv() => {
                        info!("Received SIGQUIT, initiating restart");
                        coordinator.initiate_shutdown(ShutdownReason::Restart).await;
                    }
                }
            }

            #[cfg(not(unix))]
            {
                // Windows support
                tokio::signal::ctrl_c()
                    .await
                    .expect("Failed to listen for Ctrl+C");
                info!("Received Ctrl+C, initiating graceful shutdown");
                coordinator
                    .initiate_shutdown(ShutdownReason::Graceful)
                    .await;
            }
        });
    }

    /// Initiate shutdown process
    pub async fn initiate_shutdown(&self, reason: ShutdownReason) {
        let shutdown_start = Instant::now();

        {
            let mut registry = self.task_registry.lock().await;
            if registry.shutdown_started {
                warn!("Shutdown already in progress, ignoring duplicate signal");
                return;
            }

            registry.shutdown_started = true;
            registry.shutdown_reason = Some(reason.clone());
            registry.shutdown_start_time = Some(shutdown_start);
        }

        info!("🛑 Initiating graceful shutdown: {:?}", reason);

        // Send shutdown signal to all subscribers
        let _ = self.shutdown_tx.send(reason.clone());

        // Execute shutdown sequence
        match self.execute_shutdown_sequence().await {
            Ok(stats) => {
                info!(
                    "✅ Graceful shutdown completed successfully in {:.2}s",
                    stats.total_duration.as_secs_f64()
                );
                info!(
                    "📊 Shutdown stats: {} tasks completed, {} failed, {} forced",
                    stats.completed_tasks, stats.failed_tasks, stats.forced_kills
                );
            }
            Err(e) => {
                error!("❌ Shutdown sequence failed: {}", e);
            }
        }
    }

    /// Execute the complete shutdown sequence
    async fn execute_shutdown_sequence(&self) -> Result<ShutdownStats, Box<dyn std::error::Error>> {
        let start_time = Instant::now();
        let mut phase_durations = std::collections::HashMap::new();

        // Phase 1: Signal all tasks to shutdown
        let phase_start = Instant::now();
        self.signal_all_tasks_shutdown().await;
        phase_durations.insert("signal_tasks".to_string(), phase_start.elapsed());

        tokio::time::sleep(self.config.phase_delay).await;

        // Phase 2: Wait for tasks to complete gracefully (ordered by priority)
        let phase_start = Instant::now();
        let graceful_stats = self.wait_for_graceful_completion().await;
        phase_durations.insert("graceful_wait".to_string(), phase_start.elapsed());

        // Phase 3: Force kill remaining tasks if enabled
        let phase_start = Instant::now();
        let force_kills = if self.config.force_kill_on_timeout {
            self.force_kill_remaining_tasks().await
        } else {
            0
        };
        phase_durations.insert("force_kill".to_string(), phase_start.elapsed());

        // Phase 4: Final cleanup and metrics collection
        let phase_start = Instant::now();
        self.final_cleanup().await;
        phase_durations.insert("final_cleanup".to_string(), phase_start.elapsed());

        let registry = self.task_registry.lock().await;
        let reason = registry
            .shutdown_reason
            .clone()
            .unwrap_or(ShutdownReason::Graceful);

        Ok(ShutdownStats {
            reason,
            total_tasks: registry.tasks.len(),
            completed_tasks: graceful_stats.0,
            failed_tasks: graceful_stats.1,
            forced_kills: force_kills,
            total_duration: start_time.elapsed(),
            phase_durations,
        })
    }

    /// Send shutdown signal to all registered tasks
    async fn signal_all_tasks_shutdown(&self) {
        info!("📢 Signaling shutdown to all background tasks");

        let mut registry = self.task_registry.lock().await;
        let mut signaled_count = 0;

        for (name, task_info) in registry.tasks.iter_mut() {
            if let Some(ref shutdown_tx) = task_info.shutdown_tx {
                match shutdown_tx.send(true) {
                    Ok(_) => {
                        task_info.status = TaskStatus::ShuttingDown;
                        signaled_count += 1;
                        debug!("Sent shutdown signal to task: {}", name);
                    }
                    Err(e) => {
                        warn!("Failed to send shutdown signal to task {}: {}", name, e);
                        task_info.status = TaskStatus::Failed(format!("Signal failed: {}", e));
                    }
                }
            }
        }

        info!("Sent shutdown signals to {} tasks", signaled_count);
    }

    /// Wait for tasks to complete gracefully, respecting priority order
    async fn wait_for_graceful_completion(&self) -> (usize, usize) {
        info!(
            "⏳ Waiting for tasks to complete gracefully (timeout: {:?})",
            self.config.graceful_timeout
        );

        let start_time = Instant::now();
        let mut completed_count = 0;
        let mut failed_count = 0;

        // Get tasks sorted by priority (highest first)
        let task_names_by_priority = {
            let registry = self.task_registry.lock().await;
            let mut tasks: Vec<_> = registry
                .tasks
                .iter()
                .map(|(name, info)| (name.clone(), info.priority.clone()))
                .collect();
            tasks.sort_by(|a, b| b.1.cmp(&a.1)); // Sort by priority descending
            tasks.into_iter().map(|(name, _)| name).collect::<Vec<_>>()
        };

        // Wait for each priority group
        for priority in [
            TaskPriority::Critical,
            TaskPriority::High,
            TaskPriority::Normal,
            TaskPriority::Low,
        ] {
            let priority_tasks: Vec<_> = task_names_by_priority
                .iter()
                .filter(|name| {
                    let registry_guard = self.task_registry.try_lock();
                    if let Ok(registry) = registry_guard {
                        registry
                            .tasks
                            .get(*name)
                            .map(|info| info.priority == priority)
                            .unwrap_or(false)
                    } else {
                        false
                    }
                })
                .cloned()
                .collect();

            if priority_tasks.is_empty() {
                continue;
            }

            info!(
                "Waiting for {:?} priority tasks: {:?}",
                priority, priority_tasks
            );

            for task_name in priority_tasks {
                let remaining_time = self
                    .config
                    .graceful_timeout
                    .saturating_sub(start_time.elapsed());

                if remaining_time.is_zero() {
                    warn!(
                        "Graceful shutdown timeout reached, remaining tasks will be force-killed"
                    );
                    break;
                }

                // Wait for this specific task
                let result = {
                    let mut registry = self.task_registry.lock().await;
                    if let Some(task_info) = registry.tasks.get_mut(&task_name) {
                        // Don't wait if task is already completed/failed
                        match &task_info.status {
                            TaskStatus::Completed | TaskStatus::Failed(_) => {
                                continue;
                            }
                            _ => {}
                        }

                        drop(registry); // Release lock before awaiting
                        timeout(remaining_time, async {
                            // Check if task is done by polling handle status
                            loop {
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                let registry = self.task_registry.lock().await;
                                if let Some(task_info) = registry.tasks.get(&task_name) {
                                    if task_info.handle.is_finished() {
                                        break;
                                    }
                                } else {
                                    break;
                                }
                            }
                        })
                        .await
                    } else {
                        continue;
                    }
                };

                // Update task status based on result
                let mut registry = self.task_registry.lock().await;
                if let Some(task_info) = registry.tasks.get_mut(&task_name) {
                    match result {
                        Ok(_) => {
                            task_info.status = TaskStatus::Completed;
                            completed_count += 1;
                            info!("✅ Task '{}' completed gracefully", task_name);
                        }
                        Err(_) => {
                            failed_count += 1;
                            warn!("⏰ Task '{}' didn't complete within timeout", task_name);
                        }
                    }
                }
            }

            // Small delay between priority groups
            tokio::time::sleep(self.config.phase_delay).await;
        }

        info!(
            "Graceful completion phase finished: {} completed, {} failed/timeout",
            completed_count, failed_count
        );

        (completed_count, failed_count)
    }

    /// Force kill remaining tasks that didn't shutdown gracefully
    async fn force_kill_remaining_tasks(&self) -> usize {
        info!("🔨 Force killing remaining tasks");

        let mut registry = self.task_registry.lock().await;
        let mut killed_count = 0;

        for (name, task_info) in registry.tasks.iter_mut() {
            match &task_info.status {
                TaskStatus::Running | TaskStatus::ShuttingDown => {
                    task_info.handle.abort();
                    task_info.status = TaskStatus::Failed("Force killed".to_string());
                    killed_count += 1;
                    warn!("🔨 Force killed task: {}", name);
                }
                _ => {
                    // Task already completed or failed
                }
            }
        }

        if killed_count > 0 {
            warn!("Force killed {} tasks", killed_count);
        }

        killed_count
    }

    /// Perform final cleanup operations
    async fn final_cleanup(&self) {
        info!("🧹 Performing final cleanup");

        // Give a moment for any final operations
        tokio::time::sleep(Duration::from_millis(100)).await;

        let registry = self.task_registry.lock().await;
        let total_tasks = registry.tasks.len();
        let completed_tasks = registry
            .tasks
            .values()
            .filter(|t| matches!(t.status, TaskStatus::Completed))
            .count();
        let failed_tasks = registry
            .tasks
            .values()
            .filter(|t| matches!(t.status, TaskStatus::Failed(_)))
            .count();

        info!(
            "📊 Final task status: {}/{} completed, {} failed",
            completed_tasks, total_tasks, failed_tasks
        );

        // Log any tasks that had issues
        for (name, task_info) in registry.tasks.iter() {
            if let TaskStatus::Failed(reason) = &task_info.status {
                warn!("Task '{}' failed: {}", name, reason);
            }
        }
    }

    /// Get current shutdown status
    pub async fn is_shutdown_initiated(&self) -> bool {
        let registry = self.task_registry.lock().await;
        registry.shutdown_started
    }

    /// Get shutdown statistics (if shutdown has been initiated)
    pub async fn get_shutdown_stats(&self) -> Option<ShutdownStats> {
        let registry = self.task_registry.lock().await;

        if let (Some(reason), Some(start_time)) =
            (&registry.shutdown_reason, registry.shutdown_start_time)
        {
            let completed_tasks = registry
                .tasks
                .values()
                .filter(|t| matches!(t.status, TaskStatus::Completed))
                .count();
            let failed_tasks = registry
                .tasks
                .values()
                .filter(|t| matches!(t.status, TaskStatus::Failed(_)))
                .count();
            let forced_kills = registry
                .tasks
                .values()
                .filter(|t| {
                    if let TaskStatus::Failed(ref reason) = t.status {
                        reason.contains("Force killed")
                    } else {
                        false
                    }
                })
                .count();

            Some(ShutdownStats {
                reason: reason.clone(),
                total_tasks: registry.tasks.len(),
                completed_tasks,
                failed_tasks,
                forced_kills,
                total_duration: start_time.elapsed(),
                phase_durations: std::collections::HashMap::new(), // Not tracked in this method
            })
        } else {
            None
        }
    }

    /// Create a shutdown receiver for external use
    pub fn subscribe_to_shutdown(&self) -> broadcast::Receiver<ShutdownReason> {
        self.shutdown_tx.subscribe()
    }
}

impl TaskHandle {
    /// Check if shutdown has been requested for this task
    pub fn is_shutdown_requested(&self) -> bool {
        *self.shutdown_rx.borrow()
    }

    /// Wait for shutdown signal asynchronously
    pub async fn wait_for_shutdown(&mut self) {
        let _ = self.shutdown_rx.changed().await;
    }

    /// Get the task name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Mark this task as completed
    pub async fn mark_completed(&self) {
        let mut registry = self.coordinator.task_registry.lock().await;
        if let Some(task_info) = registry.tasks.get_mut(&self.name) {
            task_info.status = TaskStatus::Completed;
        }
    }

    /// Mark this task as failed
    pub async fn mark_failed(&self, reason: String) {
        let mut registry = self.coordinator.task_registry.lock().await;
        if let Some(task_info) = registry.tasks.get_mut(&self.name) {
            task_info.status = TaskStatus::Failed(reason);
        }
    }
}

/// Convenience macro for shutdown-aware loops
#[macro_export]
macro_rules! shutdown_loop {
    ($handle:expr, $interval:expr, $body:block) => {
        let mut interval = tokio::time::interval($interval);
        loop {
            tokio::select! {
                _ = interval.tick() => $body,
                _ = $handle.wait_for_shutdown() => {
                    tracing::info!("Shutdown requested for task: {}", $handle.name());
                    break;
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::sleep;

    #[tokio::test]
    async fn test_shutdown_coordinator_creation() {
        let config = ShutdownConfig::default();
        let coordinator = ShutdownCoordinator::new(config);

        assert!(!coordinator.is_shutdown_initiated().await);
    }

    #[tokio::test]
    async fn test_task_registration() {
        let coordinator = ShutdownCoordinator::new(ShutdownConfig::default());

        let _handle = coordinator
            .register_task(
                "test-task".to_string(),
                TaskType::Custom("test".to_string()),
                TaskPriority::Normal,
            )
            .await;

        // Verify task was registered
        let registry = coordinator.task_registry.lock().await;
        assert!(registry.tasks.contains_key("test-task"));
    }

    #[tokio::test]
    async fn test_graceful_shutdown() {
        let config = ShutdownConfig {
            graceful_timeout: Duration::from_millis(500),
            ..Default::default()
        };
        let coordinator = ShutdownCoordinator::new(config);

        // Register a test task
        let handle = coordinator
            .register_task(
                "test-task".to_string(),
                TaskType::Custom("test".to_string()),
                TaskPriority::Normal,
            )
            .await;

        // Spawn a task that responds to shutdown
        let task_handle = tokio::spawn({
            let mut handle = handle.clone();
            async move {
                handle.wait_for_shutdown().await;
                // Simulate some cleanup work
                sleep(Duration::from_millis(100)).await;
                handle.mark_completed().await;
            }
        });

        coordinator
            .update_task_handle("test-task", task_handle)
            .await;

        // Initiate shutdown
        coordinator
            .initiate_shutdown(ShutdownReason::Graceful)
            .await;

        // Verify shutdown was initiated
        assert!(coordinator.is_shutdown_initiated().await);

        // Check final stats
        if let Some(stats) = coordinator.get_shutdown_stats().await {
            assert_eq!(stats.total_tasks, 1);
            assert_eq!(stats.completed_tasks, 1);
            assert_eq!(stats.failed_tasks, 0);
        }
    }

    #[tokio::test]
    async fn test_force_kill_timeout() {
        let config = ShutdownConfig {
            graceful_timeout: Duration::from_millis(100),
            force_kill_on_timeout: true,
            ..Default::default()
        };
        let coordinator = ShutdownCoordinator::new(config);

        // Register a task that won't respond to shutdown
        let _handle = coordinator
            .register_task(
                "stubborn-task".to_string(),
                TaskType::Custom("test".to_string()),
                TaskPriority::Normal,
            )
            .await;

        // Spawn a task that ignores shutdown signals
        let task_handle = tokio::spawn(async {
            // This task will be force-killed
            loop {
                sleep(Duration::from_millis(50)).await;
            }
        });

        coordinator
            .update_task_handle("stubborn-task", task_handle)
            .await;

        // Initiate shutdown
        coordinator
            .initiate_shutdown(ShutdownReason::Graceful)
            .await;

        // Check that task was force-killed
        if let Some(stats) = coordinator.get_shutdown_stats().await {
            assert_eq!(stats.total_tasks, 1);
            assert_eq!(stats.forced_kills, 1);
        }
    }

    #[tokio::test]
    async fn test_priority_shutdown_order() {
        let coordinator = ShutdownCoordinator::new(ShutdownConfig::default());

        // Register tasks with different priorities
        let _critical = coordinator
            .register_task(
                "critical-task".to_string(),
                TaskType::Custom("critical".to_string()),
                TaskPriority::Critical,
            )
            .await;

        let _normal = coordinator
            .register_task(
                "normal-task".to_string(),
                TaskType::Custom("normal".to_string()),
                TaskPriority::Normal,
            )
            .await;

        let _low = coordinator
            .register_task(
                "low-task".to_string(),
                TaskType::Custom("low".to_string()),
                TaskPriority::Low,
            )
            .await;

        // Verify tasks were registered with correct priorities
        let registry = coordinator.task_registry.lock().await;
        assert_eq!(registry.tasks.len(), 3);

        assert_eq!(
            registry.tasks.get("critical-task").unwrap().priority,
            TaskPriority::Critical
        );
        assert_eq!(
            registry.tasks.get("normal-task").unwrap().priority,
            TaskPriority::Normal
        );
        assert_eq!(
            registry.tasks.get("low-task").unwrap().priority,
            TaskPriority::Low
        );
    }
}
