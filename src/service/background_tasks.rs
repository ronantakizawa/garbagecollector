// src/service/background_tasks.rs - Background task management

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::metrics::Metrics;
use crate::storage::{PersistentStorage, Storage};

/// Manages all background tasks for the service
pub struct BackgroundTaskManager {
    storage: Arc<dyn Storage>,
    persistent_storage: Option<Arc<dyn PersistentStorage>>,
    config: Arc<Config>,
    metrics: Arc<Metrics>,
    task_handles: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl BackgroundTaskManager {
    pub fn new(
        storage: Arc<dyn Storage>,
        persistent_storage: Option<Arc<dyn PersistentStorage>>,
        config: Arc<Config>,
        metrics: Arc<Metrics>,
    ) -> Self {
        Self {
            storage,
            persistent_storage,
            config,
            metrics,
            task_handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Start all background tasks
    pub async fn start_all(&self, shutdown_tx: Option<Arc<tokio::sync::broadcast::Sender<()>>>) {
        info!("🚀 Starting background tasks");

        // Start WAL compaction task if using persistent storage
        if let Some(ref persistent_storage) = self.persistent_storage {
            self.start_wal_compaction_task(persistent_storage.clone(), shutdown_tx.clone()).await;
        }
        
        // Start snapshot task if configured
        if let Some(ref persistent_storage) = self.persistent_storage {
            if self.config.storage.snapshot_interval_seconds > 0 {
                self.start_snapshot_task(persistent_storage.clone(), shutdown_tx.clone()).await;
            }
        }
        
        // Start health check task
        self.start_health_check_task(shutdown_tx.clone()).await;

        // Start metrics collection task
        self.start_metrics_collection_task(shutdown_tx).await;

        info!("✅ All background tasks started");
    }

    /// Stop all background tasks
    pub async fn stop_all(&self) {
        info!("🛑 Stopping background tasks");
        
        let mut handles = self.task_handles.lock().await;
        for handle in handles.drain(..) {
            handle.abort();
        }
        
        info!("✅ All background tasks stopped");
    }

    /// Start WAL compaction background task
    async fn start_wal_compaction_task(
        &self,
        persistent_storage: Arc<dyn PersistentStorage>,
        shutdown_tx: Option<Arc<tokio::sync::broadcast::Sender<()>>>,
    ) {
        let config = self.config.clone();
        let metrics = self.metrics.clone();
        let mut shutdown_rx = shutdown_tx.as_ref().map(|tx| tx.subscribe());

        let task_handle = tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(3600)); // Check every hour
            
            info!("📦 Started WAL compaction task");
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        if let Ok(current_seq) = persistent_storage.current_sequence_number().await {
                            let threshold = config.storage.wal_compaction_threshold;
                            
                            if current_seq > threshold {
                                let compact_before = current_seq - (threshold / 2);
                                
                                info!("🗜️ Starting WAL compaction before sequence {}", compact_before);
                                
                                match persistent_storage.compact_wal(compact_before).await {
                                    Ok(_) => {
                                        info!("✅ WAL compaction completed");
                                        metrics.record_wal_compaction_success();
                                    }
                                    Err(e) => {
                                        error!("❌ WAL compaction failed: {}", e);
                                        metrics.record_wal_compaction_failure();
                                    }
                                }
                            }
                        }
                    }
                    _ = async {
                        if let Some(ref mut rx) = shutdown_rx {
                            let _ = rx.recv().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {
                        info!("🛑 WAL compaction task received shutdown signal");
                        break;
                    }
                }
            }
        });

        self.task_handles.lock().await.push(task_handle);
    }

    /// Start snapshot creation background task
    async fn start_snapshot_task(
        &self,
        persistent_storage: Arc<dyn PersistentStorage>,
        shutdown_tx: Option<Arc<tokio::sync::broadcast::Sender<()>>>,
    ) {
        let interval_seconds = self.config.storage.snapshot_interval_seconds;
        let metrics = self.metrics.clone();
        let mut shutdown_rx = shutdown_tx.as_ref().map(|tx| tx.subscribe());

        let task_handle = tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(interval_seconds));
            
            info!("📸 Started snapshot task (interval: {}s)", interval_seconds);
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        info!("📸 Creating periodic snapshot");
                        
                        match persistent_storage.create_snapshot().await {
                            Ok(snapshot_path) => {
                                info!("✅ Snapshot created: {}", snapshot_path);
                                metrics.record_snapshot_success();
                            }
                            Err(e) => {
                                error!("❌ Snapshot creation failed: {}", e);
                                metrics.record_snapshot_failure();
                            }
                        }
                    }
                    _ = async {
                        if let Some(ref mut rx) = shutdown_rx {
                            let _ = rx.recv().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {
                        info!("🛑 Snapshot task received shutdown signal");
                        break;
                    }
                }
            }
        });

        self.task_handles.lock().await.push(task_handle);
    }

    /// Start health check background task
    async fn start_health_check_task(
        &self,
        shutdown_tx: Option<Arc<tokio::sync::broadcast::Sender<()>>>,
    ) {
        let storage = self.storage.clone();
        let persistent_storage = self.persistent_storage.clone();
        let metrics = self.metrics.clone();
        let mut shutdown_rx = shutdown_tx.as_ref().map(|tx| tx.subscribe());

        let task_handle = tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(60)); // Check every minute
            
            info!("🏥 Started health check task");
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Check storage health
                        match storage.health_check().await {
                            Ok(true) => {
                                metrics.record_health_check_success("storage");
                            }
                            Ok(false) => {
                                warn!("⚠️ Storage health check failed");
                                metrics.record_health_check_failure("storage");
                            }
                            Err(e) => {
                                error!("❌ Storage health check error: {}", e);
                                metrics.record_health_check_failure("storage");
                            }
                        }

                        // Check persistent storage health if available
                        if let Some(ref persistent) = persistent_storage {
                            match persistent.verify_wal_integrity().await {
                                Ok(issues) if issues.is_empty() => {
                                    metrics.record_health_check_success("wal");
                                }
                                Ok(issues) => {
                                    warn!("⚠️ WAL integrity issues: {}", issues.len());
                                    metrics.record_health_check_failure("wal");
                                }
                                Err(e) => {
                                    error!("❌ WAL integrity check error: {}", e);
                                    metrics.record_health_check_failure("wal");
                                }
                            }
                        }
                    }
                    _ = async {
                        if let Some(ref mut rx) = shutdown_rx {
                            let _ = rx.recv().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {
                        info!("🛑 Health check task received shutdown signal");
                        break;
                    }
                }
            }
        });

        self.task_handles.lock().await.push(task_handle);
    }

    /// Start metrics collection background task
    async fn start_metrics_collection_task(
        &self,
        shutdown_tx: Option<Arc<tokio::sync::broadcast::Sender<()>>>,
    ) {
        let storage = self.storage.clone();
        let metrics = self.metrics.clone();
        let mut shutdown_rx = shutdown_tx.as_ref().map(|tx| tx.subscribe());

        let task_handle = tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(30)); // Collect every 30 seconds
            
            info!("📊 Started metrics collection task");
            
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        // Collect storage statistics
                        match storage.get_stats().await {
                            Ok(stats) => {
                                metrics.update_storage_stats(&stats);
                            }
                            Err(e) => {
                                error!("❌ Failed to collect storage stats: {}", e);
                            }
                        }
                    }
                    _ = async {
                        if let Some(ref mut rx) = shutdown_rx {
                            let _ = rx.recv().await;
                        } else {
                            std::future::pending::<()>().await;
                        }
                    } => {
                        info!("🛑 Metrics collection task received shutdown signal");
                        break;
                    }
                }
            }
        });

        self.task_handles.lock().await.push(task_handle);
    }

    /// Get background task statistics
    pub async fn get_task_stats(&self) -> BackgroundTaskStats {
        let handles = self.task_handles.lock().await;
        let total_tasks = handles.len();
        let running_tasks = handles.iter().filter(|h| !h.is_finished()).count();

        BackgroundTaskStats {
            total_tasks,
            running_tasks,
            failed_tasks: total_tasks - running_tasks,
        }
    }
}

/// Background task statistics
#[derive(Debug, Clone)]
pub struct BackgroundTaskStats {
    pub total_tasks: usize,
    pub running_tasks: usize,
    pub failed_tasks: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemoryStorage;

    #[tokio::test]
    async fn test_background_task_manager_creation() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();

        let manager = BackgroundTaskManager::new(storage, None, config, metrics);
        let stats = manager.get_task_stats().await;
        assert_eq!(stats.total_tasks, 0);
        assert_eq!(stats.running_tasks, 0);
    }

    #[tokio::test]
    async fn test_start_and_stop_tasks() {
        let storage: Arc<dyn Storage> = Arc::new(MemoryStorage::new());
        let config = Arc::new(Config::default());
        let metrics = Metrics::new();

        let manager = BackgroundTaskManager::new(storage, None, config, metrics);
        
        // Start tasks
        manager.start_all(None).await;
        
        let stats = manager.get_task_stats().await;
        assert!(stats.total_tasks > 0);
        
        // Stop tasks
        manager.stop_all().await;
        
        // Give tasks time to shutdown
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}