// src/lib.rs - Library root to expose modules for testing

pub mod config;
pub mod error;
pub mod lease;
pub mod service;
pub mod storage;
pub mod cleanup;
pub mod metrics;

// Include the generated protobuf code
pub mod proto {
    tonic::include_proto!("distributed_gc");
}

// Re-export commonly used items for convenience
pub use config::Config;
pub use error::{GCError, Result};
pub use lease::{Lease, ObjectType, LeaseState, CleanupConfig};
pub use service::GCService;