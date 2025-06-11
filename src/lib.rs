// src/lib.rs - Updated with persistent storage and recovery modules

pub mod cleanup;
pub mod config;
pub mod error;
pub mod lease;
pub mod metrics;
pub mod service;
pub mod shutdown;
pub mod storage;
pub mod security;

// Startup and dependencies modules
pub mod dependencies;
pub mod startup;

// Simulation module for experiments
pub mod simulation;

// Recovery module for persistent storage and failure recovery
#[cfg(feature = "persistent")]
pub mod recovery;

// Client module (optional)
#[cfg(feature = "client")]
pub mod client;

// Include the generated protobuf code at crate level
pub mod proto {
    tonic::include_proto!("distributed_gc");
}

// Re-export commonly used items for convenience
pub use config::{Config, StorageConfig};
pub use error::{GCError, Result};
pub use lease::{CleanupConfig, Lease, LeaseState, ObjectType};
pub use service::GCService;
pub use shutdown::{
    ShutdownConfig, ShutdownCoordinator, ShutdownReason, TaskHandle, TaskPriority, TaskType,
};
pub use storage::{create_storage, Storage};

// Re-export persistent storage features when available
#[cfg(feature = "persistent")]
pub use storage::{
    create_persistent_storage, 
    PersistentStorage, PersistentStorageConfig, RecoveryInfo, WALEntry, WALOperation,
    FilePersistentStorage,
};

// Re-export recovery features when available
#[cfg(feature = "persistent")]
pub use recovery::{RecoveryConfig, RecoveryManager, RecoveryStrategy, RecoveryTrigger};

// Re-export startup
pub use startup::ApplicationStartup;

// Re-export metrics
pub use metrics::Metrics;

// Re-export client when feature is enabled
#[cfg(feature = "client")]
pub use client::GCClient;

/// GarbageTruck version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Feature flags available in this build
#[derive(Debug, Clone)]
pub struct Features {
    pub client: bool,
    pub server: bool,
    pub persistent: bool,
}

impl Features {
    pub fn new() -> Self {
        Self {
            client: cfg!(feature = "client"),
            server: cfg!(feature = "server"),
            persistent: cfg!(feature = "persistent"),
        }
    }

    pub fn has_persistent_storage(&self) -> bool {
        self.persistent
    }

    pub fn has_recovery(&self) -> bool {
        self.persistent
    }

    pub fn list_enabled(&self) -> Vec<&'static str> {
        let mut features = Vec::new();
        if self.client { features.push("client"); }
        if self.server { features.push("server"); }
        if self.persistent { features.push("persistent"); }
        features
    }
}

impl Default for Features {
    fn default() -> Self {
        Self::new()
    }
}

/// Get build information
pub fn build_info() -> BuildInfo {
    BuildInfo {
        version: VERSION,
        features: Features::new(),
        build_timestamp: option_env!("VERGEN_BUILD_TIMESTAMP").unwrap_or("unknown"),
        git_sha: option_env!("VERGEN_GIT_SHA").unwrap_or("unknown"),
        rust_version: option_env!("VERGEN_RUSTC_SEMVER").unwrap_or("unknown"),
    }
}

/// Build information structure
#[derive(Debug, Clone)]
pub struct BuildInfo {
    pub version: &'static str,
    pub features: Features,
    pub build_timestamp: &'static str,
    pub git_sha: &'static str,
    pub rust_version: &'static str,
}

impl std::fmt::Display for BuildInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "GarbageTruck v{} ({})\nFeatures: {:?}\nBuilt: {} with Rust {}",
            self.version,
            self.git_sha,
            self.features.list_enabled(),
            self.build_timestamp,
            self.rust_version
        )
    }
}

/// Prelude module for common imports
pub mod prelude {
    pub use crate::{
        Config, GCError, GCService, Lease, LeaseState, ObjectType, Result, Storage,
    };

    #[cfg(feature = "client")]
    pub use crate::GCClient;

    #[cfg(feature = "persistent")]
    pub use crate::{
        PersistentStorage, PersistentStorageConfig, RecoveryConfig, RecoveryManager,
        RecoveryStrategy, WALEntry, WALOperation,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_features() {
        let features = Features::new();
        let enabled = features.list_enabled();
        
        // At least one feature should be enabled
        assert!(!enabled.is_empty());
        
        // Test feature detection
        #[cfg(feature = "persistent")]
        assert!(features.has_persistent_storage());
        
        #[cfg(not(feature = "persistent"))]
        assert!(!features.has_persistent_storage());
    }

    #[test]
    fn test_build_info() {
        let info = build_info();
        assert!(!info.version.is_empty());
        assert!(!info.rust_version.is_empty());
        
        // Test display format
        let display = format!("{}", info);
        assert!(display.contains("GarbageTruck"));
        assert!(display.contains(info.version));
    }

    #[test]
    fn test_version_constant() {
        assert!(!VERSION.is_empty());
        // Version should follow semver pattern
        assert!(VERSION.contains('.'));
    }
}