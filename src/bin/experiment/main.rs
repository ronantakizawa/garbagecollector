// src/bin/experiment/main.rs - Storage cost savings experiment runner

use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, Level};
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
    s3_cost_per_gb_month: f64, // AWS S3 Standard pricing
}

impl Default for ExperimentConfig {
    fn default() -> Self {
        Self {
            garbagetruck_endpoint: "http://localhost:50051".to_string(),
            cleanup_server_port: 8080,
            temp_directory: PathBuf::from("./temp_experiment"),
            file_size_mb: 10.0, // 10MB files
            job_count: 50,
            crash_probability: 0.3,      // 30% crash rate
            s3_cost_per_gb_month: 0.023, // $0.023 per GB/month for S3 Standard
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize logging
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let config = ExperimentConfig::default();

    info!("🧪 Starting Storage Cost Savings Experiment");
    info!("📊 Configuration:");
    info!("  - File size: {:.1} MB", config.file_size_mb);
    info!("  - Job count: {}", config.job_count);
    info!(
        "  - Crash probability: {:.0}%",
        config.crash_probability * 100.0
    );
    info!("  - S3 cost: ${:.3}/GB/month", config.s3_cost_per_gb_month);
    info!("  - Temp directory: {:?}", config.temp_directory);

    // Run both experiments
    let without_gc_stats = run_experiment_without_garbagetruck(&config).await?;
    let with_gc_stats = run_experiment_with_garbagetruck(&config).await?;

    // Compare results
    print_experiment_results(&without_gc_stats, &with_gc_stats);

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
        job_duration_seconds: 5,
        crash_probability: config.crash_probability, // Now both are f32
        cleanup_endpoint: None,
    };

    let cost_tracker = StorageCostTracker::new(config.s3_cost_per_gb_month);
    let mut simulator = JobSimulator::new(file_config, cost_tracker, None);

    let stats = simulator.run_without_garbagetruck().await?;

    // Count actual remaining files
    let (remaining_files, remaining_bytes) = simulator.count_remaining_files()?;
    info!(
        "📁 Actual files remaining on disk: {} files, {:.2} MB",
        remaining_files,
        remaining_bytes as f64 / (1024.0 * 1024.0)
    );

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

    // Give server time to start
    sleep(Duration::from_secs(2)).await;

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
        job_duration_seconds: 5,
        crash_probability: config.crash_probability, // Now both are f32
        cleanup_endpoint: Some(cleanup_endpoint),
    };

    let cost_tracker = StorageCostTracker::new(config.s3_cost_per_gb_month);
    let mut simulator = JobSimulator::new(file_config, cost_tracker, Some(gc_client));

    let stats = simulator.run_with_garbagetruck().await?;

    // Count actual remaining files
    let (remaining_files, remaining_bytes) = simulator.count_remaining_files()?;
    info!(
        "📁 Actual files remaining on disk: {} files, {:.2} MB",
        remaining_files,
        remaining_bytes as f64 / (1024.0 * 1024.0)
    );

    Ok(stats)
}

fn print_experiment_results(without_gc: &StorageStats, with_gc: &StorageStats) {
    println!("\n📊 === EXPERIMENT RESULTS ===");

    println!("\n🚫 WITHOUT GARBAGETRUCK:");
    println!("  Files created: {}", without_gc.total_files_created);
    println!("  Files cleaned: {}", without_gc.total_files_cleaned);
    println!("  Files orphaned: {}", without_gc.orphaned_files);
    println!(
        "  Storage used: {:.2} MB",
        without_gc.current_storage_bytes() as f64 / (1024.0 * 1024.0)
    );
    println!("  Monthly cost: ${:.2}", without_gc.current_monthly_cost());
    println!(
        "  Cleanup efficiency: {:.1}%",
        without_gc.cleanup_efficiency()
    );

    println!("\n✅ WITH GARBAGETRUCK:");
    println!("  Files created: {}", with_gc.total_files_created);
    println!("  Files cleaned: {}", with_gc.total_files_cleaned);
    println!("  Files orphaned: {}", with_gc.orphaned_files);
    println!(
        "  Storage used: {:.2} MB",
        with_gc.current_storage_bytes() as f64 / (1024.0 * 1024.0)
    );
    println!("  Monthly cost: ${:.2}", with_gc.current_monthly_cost());
    println!("  Cleanup efficiency: {:.1}%", with_gc.cleanup_efficiency());

    // Calculate savings
    let storage_savings_mb = (without_gc.current_storage_bytes() as i64
        - with_gc.current_storage_bytes() as i64) as f64
        / (1024.0 * 1024.0);
    let cost_savings = without_gc.current_monthly_cost() - with_gc.current_monthly_cost();
    let efficiency_improvement = with_gc.cleanup_efficiency() - without_gc.cleanup_efficiency();

    println!("\n💰 === COST SAVINGS ANALYSIS ===");
    println!("  Storage saved: {:.2} MB", storage_savings_mb);
    println!("  Monthly cost savings: ${:.2}", cost_savings);
    println!("  Annual cost savings: ${:.2}", cost_savings * 12.0);
    println!(
        "  Cleanup efficiency improvement: {:.1} percentage points",
        efficiency_improvement
    );

    if cost_savings > 0.0 {
        let savings_percentage = (cost_savings / without_gc.current_monthly_cost()) * 100.0;
        println!("  Cost reduction: {:.1}%", savings_percentage);
        println!("\n🎉 GarbageTruck provides significant cost savings!");

        // Extrapolate to enterprise scale
        println!("\n📈 === ENTERPRISE SCALE PROJECTION ===");
        let enterprise_multiplier = 1000.0; // 1000x more files
        let enterprise_monthly_savings = cost_savings * enterprise_multiplier;
        let enterprise_annual_savings = enterprise_monthly_savings * 12.0;

        println!(
            "  At 1000x scale ({} jobs):",
            without_gc.total_files_created * 1000
        );
        println!("  Monthly savings: ${:.2}", enterprise_monthly_savings);
        println!("  Annual savings: ${:.2}", enterprise_annual_savings);

        if enterprise_annual_savings > 100000.0 {
            println!(
                "  💸 Potential for ${}K+ annual savings at enterprise scale!",
                (enterprise_annual_savings / 1000.0) as u32
            );
        }
    } else {
        println!("  ⚠️  No significant cost savings detected - may need to adjust parameters");
    }

    println!("\n✨ Experiment completed successfully!");
}
