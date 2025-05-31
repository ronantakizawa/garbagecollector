pub mod config;
pub mod error;
pub mod lease;
pub mod service;
pub mod storage;
pub mod cleanup;
pub mod metrics;
pub mod shutdown;

// NEW: Startup and monitoring modules
pub mod startup;
pub mod dependencies;
pub mod monitoring;

// Client module (optional)
#[cfg(feature = "client")]
pub mod client;

// Include the generated protobuf code at crate level
pub mod proto {
    tonic::include_proto!("distributed_gc");
}

// Re-export commonly used items for convenience
pub use config::Config;
pub use error::{GCError, Result};
pub use lease::{Lease, ObjectType, LeaseState, CleanupConfig};
pub use service::GCService;
pub use shutdown::{ShutdownCoordinator, ShutdownConfig, TaskHandle, TaskType, TaskPriority, ShutdownReason};

// Re-export new modules
pub use startup::ApplicationStartup;
pub use dependencies::DependencyChecker;
pub use monitoring::SystemMonitor;

// Re-export client when feature is enabled
#[cfg(feature = "client")]
pub use client::GCClient;