// src/bin/experiment/main.rs - Default-only experiment with extended waits

use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, Level};
use tracing_subscriber;

use garbagetruck::client::GCClient;
use garbagetruck::simulation::{
    start_file_cleanup_server, FileSimulationConfig, JobSimulator, StorageCostTracker, StorageStats,
};

/// Experiment configuration
#[derive(Debug)]
struct ExperimentConfig {
    garbagetruck_endpoint: String,
    cleanup_server_port: u16,
    temp_directory: PathBuf,
    file_size_mb: f64,
    job_count: u32,
    crash_probability: f32,
    s3_cost_per_gb_month: f64,
}

impl ExperimentConfig {
    /// Default configuration optimized for demonstrating cost savings
    fn default() -> Self {
        Self {
            garbagetruck_endpoint: "http://localhost:50051".to_string(),
            cleanup_server_port: 8080,
            temp_directory: PathBuf::from("./temp_experiment"),
            file_size_mb: 100.0,     // 100MB files for significant storage costs
            job_count: 200,          // More jobs for better statistics
            crash_probability: 0.05, // 5% crash rate for realistic scenario
            s3_cost_per_gb_month: 0.023, // AWS S3 Standard pricing
        }
    }

    /// (Unused) High-impact configuration for enterprise-scale demonstration
    #[allow(dead_code)]
    fn enterprise_scale() -> Self {
        Self {
            garbagetruck_endpoint: "http://localhost:50051".to_string(),
            cleanup_server_port: 8080,
            temp_directory: PathBuf::from("./temp_experiment"),
            file_size_mb: 250.0,
            job_count: 500,
            crash_probability: 0.3,
            s3_cost_per_gb_month: 0.023,
        }
    }

    /// (Unused) Quick demo configuration with smaller files but high failure rate
    #[allow(dead_code)]
    fn quick_demo() -> Self {
        Self {
            garbagetruck_endpoint: "http://localhost:50051".to_string(),
            cleanup_server_port: 8080,
            temp_directory: PathBuf::from("./temp_experiment"),
            file_size_mb: 50.0,
            job_count: 100,
            crash_probability: 0.6,
            s3_cost_per_gb_month: 0.023,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize logging
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    // Always run the default experiment
    let config = ExperimentConfig::default();

    info!("🧪 Starting Storage Cost Savings Experiment (DEFAULT)");
    info!("📊 Configuration:");
    info!("  - File size: {:.1} MB", config.file_size_mb);
    info!("  - Job count: {}", config.job_count);
    info!(
        "  - Crash probability: {:.0}%",
        config.crash_probability * 100.0
    );
    info!("  - S3 cost: ${:.3}/GB/month", config.s3_cost_per_gb_month);
    info!("  - Temp directory: {:?}", config.temp_directory);
    info!(
        "  - Total potential storage: {:.1} GB",
        (config.file_size_mb * config.job_count as f64) / 1024.0
    );

    // Show expected costs
    let total_potential_gb = (config.file_size_mb * config.job_count as f64) / 1024.0;
    let max_monthly_cost = total_potential_gb * config.s3_cost_per_gb_month;
    info!(
        "  - Maximum monthly cost (if all files orphaned): ${:.2}",
        max_monthly_cost
    );

    println!("\n🎯 Expected Results (DEFAULT):");
    println!(
        "   Without GarbageTruck: {:.0}% of files will be orphaned",
        config.crash_probability * 100.0
    );
    println!("   With GarbageTruck: ~5% orphaned (only due to lease expiration delays)");
    println!(
        "   Expected cost savings: ${:.2}/month",
        max_monthly_cost * (config.crash_probability as f64 - 0.05)
    );

    println!("\n🚀 Starting experiment...\n");

    // Run the two phases of the experiment
    let without_gc_stats = run_experiment_without_garbagetruck(&config).await?;
    // Wait longer before starting the second phase
    sleep(Duration::from_secs(10)).await;
    let with_gc_stats = run_experiment_with_garbagetruck(&config).await?;

    // Allow additional time for any asynchronous cleanup telemetry to flush
    sleep(Duration::from_secs(5)).await;

    Ok(())
}

async fn run_experiment_without_garbagetruck(
    config: &ExperimentConfig,
) -> Result<StorageStats, Box<dyn std::error::Error + Send + Sync>> {
    info!("\n🚫 === EXPERIMENT 1: WITHOUT GARBAGETRUCK ===");

    // Clean up any existing files
    if config.temp_directory.exists() {
        std::fs::remove_dir_all(&config.temp_directory)?;
    }

    let file_config = FileSimulationConfig {
        base_directory: config.temp_directory.join("without_gc"),
        file_size_bytes: (config.file_size_mb * 1024.0 * 1024.0) as usize,
        job_count: config.job_count,
        job_duration_seconds: 10, // increased duration
        crash_probability: config.crash_probability,
        cleanup_endpoint: None,
    };

    let cost_tracker = StorageCostTracker::new(config.s3_cost_per_gb_month);
    let mut simulator = JobSimulator::new(file_config, cost_tracker, None);

    let stats = simulator.run_without_garbagetruck().await?;

    // Count actual remaining files
    let (remaining_files, remaining_bytes) = simulator.count_remaining_files()?;

    Ok(stats)
}

async fn run_experiment_with_garbagetruck(
    config: &ExperimentConfig,
) -> Result<StorageStats, Box<dyn std::error::Error + Send + Sync>> {
    info!("\n✅ === EXPERIMENT 2: WITH GARBAGETRUCK ===");

    let cleanup_endpoint = format!("http://localhost:{}/cleanup", config.cleanup_server_port);

    // Start cleanup server
    let base_directory = config.temp_directory.join("with_gc");
    let cost_tracker_for_server = StorageCostTracker::new(config.s3_cost_per_gb_month);

    tokio::spawn(start_file_cleanup_server(
        config.cleanup_server_port,
        base_directory.clone(),
        cost_tracker_for_server,
    ));

    // Give server more time to start
    sleep(Duration::from_secs(5)).await;

    // Create GarbageTruck client
    let gc_client = GCClient::new(
        &config.garbagetruck_endpoint,
        "experiment-service".to_string(),
    )
    .await?;

    let file_config = FileSimulationConfig {
        base_directory: base_directory.clone(),
        file_size_bytes: (config.file_size_mb * 1024.0 * 1024.0) as usize,
        job_count: config.job_count,
        job_duration_seconds: 10, // increased duration
        crash_probability: config.crash_probability,
        cleanup_endpoint: Some(cleanup_endpoint),
    };

    let cost_tracker = StorageCostTracker::new(config.s3_cost_per_gb_month);
    let mut simulator = JobSimulator::new(file_config, cost_tracker, Some(gc_client));

    let stats = simulator.run_with_garbagetruck().await?;

    // Count actual remaining files
    let (remaining_files, remaining_bytes) = simulator.count_remaining_files()?;

    Ok(stats)
}
