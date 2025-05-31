pub mod config;
pub mod error;
pub mod lease;
pub mod service;
pub mod storage;
pub mod cleanup;
pub mod metrics;

// Client module (optional)
#[cfg(feature = "client")]
pub mod client;

// Include the generated protobuf code
pub mod proto {
    tonic::include_proto!("distributed_gc");
}

// Re-export commonly used items for convenience
pub use config::Config;
pub use error::{GCError, Result};
pub use lease::{Lease, ObjectType, LeaseState, CleanupConfig};
pub use service::GCService;

// Re-export client when feature is enabled
#[cfg(feature = "client")]
pub use client::GCClient;