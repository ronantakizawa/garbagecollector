// src/recovery/mod.rs - Recovery module declaration

pub mod manager;

// Re-export commonly used items
pub use manager::{
    RecoveryConfig, RecoveryManager, RecoveryAPI, RecoveryStrategy, RecoveryTrigger,
    RecoveryProgress, RecoveryResult, RecoveryPhase, RecoveryMetrics,
};