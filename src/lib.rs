// src/lib.rs - Updated with correct exports

pub mod cleanup;
pub mod config;
pub mod error;
pub mod lease;
pub mod metrics;
pub mod service;
pub mod shutdown;
pub mod storage;

// Startup and dependencies modules
pub mod dependencies;
pub mod startup;

// Simulation module for experiments
pub mod simulation;

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
pub use lease::{CleanupConfig, Lease, LeaseState, ObjectType};
pub use service::GCService; // Removed GCServiceHandlers as it's private
pub use shutdown::{
    ShutdownConfig, ShutdownCoordinator, ShutdownReason, TaskHandle, TaskPriority, TaskType,
};
pub use storage::{create_storage, Storage};

// Re-export startup
pub use startup::ApplicationStartup;

// Re-export metrics
pub use metrics::Metrics;

// Re-export client when feature is enabled
#[cfg(feature = "client")]
pub use client::GCClient;
