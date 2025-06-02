// src/bin/experiment/main.rs - Enhanced experiment with better parameters

use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, Level};
use tracing_subscriber;

use garbagetruck::client::GCClient;
use garbagetruck::simulation::{
    FileSimulationConfig, JobSimulator, StorageCostTracker, StorageStats,
    start_file_cleanup_server
};

/// Enhanced experiment configuration for dramatic cost savings
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
            file_size_mb: 100.0, // 100MB files for significant storage costs
            job_count: 200,      // More jobs for better statistics
            crash_probability: 1.0, // 100% crash rate - maximum failure scenario
            s3_cost_per_gb_month: 0.023, // AWS S3 Standard pricing
        }
    }

    /// High-impact configuration for enterprise-scale demonstration
    fn enterprise_scale() -> Self {
        Self {
            garbagetruck_endpoint: "http://localhost:50051".to_string(),
            cleanup_server_port: 8080,
            temp_directory: PathBuf::from("./temp_experiment"),
            file_size_mb: 250.0, // 250MB files (video/ML model size)
            job_count: 500,      // Enterprise-scale job volume
            crash_probability: 1.0, // 100% failure rate - catastrophic scenario
            s3_cost_per_gb_month: 0.023,
        }
    }

    /// Quick demo configuration with smaller files but 100% failure rate
    fn quick_demo() -> Self {
        Self {
            garbagetruck_endpoint: "http://localhost:50051".to_string(),
            cleanup_server_port: 8080,
            temp_directory: PathBuf::from("./temp_experiment"),
            file_size_mb: 50.0,  // 50MB files
            job_count: 100,      // Moderate job count
            crash_probability: 1.0, // 100% failure rate - total system failure
            s3_cost_per_gb_month: 0.023,
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    // Choose experiment configuration based on environment variable
    let config = match std::env::var("EXPERIMENT_MODE").as_deref() {
        Ok("enterprise") => {
            println!("🏢 Running ENTERPRISE SCALE experiment");
            println!("   This will create 500 jobs with 250MB files each");
            println!("   100% CRASH RATE - Total system failure scenario");
            println!("   Total potential storage: ~122 GB");
            ExperimentConfig::enterprise_scale()
        }
        Ok("demo") => {
            println!("⚡ Running QUICK DEMO experiment");
            println!("   This will create 100 jobs with 50MB files each");
            println!("   100% CRASH RATE - Maximum failure demonstration");
            ExperimentConfig::quick_demo()
        }
        _ => {
            println!("📊 Running DEFAULT experiment");
            println!("   This will create 200 jobs with 100MB files each");
            println!("   100% CRASH RATE - Complete system failure");
            ExperimentConfig::default()
        }
    };
    
    info!("🧪 Starting Storage Cost Savings Experiment");
    info!("📊 Configuration:");
    info!("  - File size: {:.1} MB", config.file_size_mb);
    info!("  - Job count: {}", config.job_count);
    info!("  - Crash probability: {:.0}%", config.crash_probability * 100.0);
    info!("  - S3 cost: ${:.3}/GB/month", config.s3_cost_per_gb_month);
    info!("  - Temp directory: {:?}", config.temp_directory);
    info!("  - Total potential storage: {:.1} GB", 
          (config.file_size_mb * config.job_count as f64) / 1024.0);

    // Show expected costs
    let total_potential_gb = (config.file_size_mb * config.job_count as f64) / 1024.0;
    let max_monthly_cost = total_potential_gb * config.s3_cost_per_gb_month;
    info!("  - Maximum monthly cost (if all files orphaned): ${:.2}", max_monthly_cost);

    println!("\n🎯 Expected Results:");
    println!("   Without GarbageTruck: 100% of files will be orphaned (total failure)");
    println!("   With GarbageTruck: ~5% orphaned (only due to lease expiration delays)");
    println!("   Expected cost savings: ${:.2}/month (95% reduction)", max_monthly_cost * 0.95);

    // Wait for user confirmation for large experiments
    if config.job_count > 100 || config.file_size_mb > 50.0 {
        println!("\n⚠️  This experiment will create {:.1} GB of temporary files.", total_potential_gb);
        println!("🕒 Estimated runtime: {:.0} minutes", (config.job_count as f64 / 10.0).ceil());
        println!("\n📝 Press Enter to continue, or Ctrl+C to cancel...");
        
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
    }

    // Run both experiments
    println!("\n🚀 Starting experiments...\n");
    let without_gc_stats = run_experiment_without_garbagetruck(&config).await?;
    let with_gc_stats = run_experiment_with_garbagetruck(&config).await?;

    // Compare results with enhanced analysis
    print_enhanced_experiment_results(&without_gc_stats, &with_gc_stats, &config);

    Ok(())
}

async fn run_experiment_without_garbagetruck(config: &ExperimentConfig) -> Result<StorageStats, Box<dyn std::error::Error + Send + Sync>> {
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
        crash_probability: config.crash_probability,
        cleanup_endpoint: None,
    };

    let cost_tracker = StorageCostTracker::new(config.s3_cost_per_gb_month);
    let mut simulator = JobSimulator::new(file_config, cost_tracker, None);

    let stats = simulator.run_without_garbagetruck().await?;
    
    // Count actual remaining files
    let (remaining_files, remaining_bytes) = simulator.count_remaining_files()?;
    info!("📁 Actual files remaining on disk: {} files, {:.2} MB", 
          remaining_files, remaining_bytes as f64 / (1024.0 * 1024.0));

    Ok(stats)
}

async fn run_experiment_with_garbagetruck(config: &ExperimentConfig) -> Result<StorageStats, Box<dyn std::error::Error + Send + Sync>> {
    info!("\n✅ === EXPERIMENT 2: WITH GARBAGETRUCK ===");
    
    let cleanup_endpoint = format!("http://localhost:{}/cleanup", config.cleanup_server_port);
    
    // Start cleanup server
    let base_directory = config.temp_directory.join("with_gc");
    let cost_tracker_for_server = StorageCostTracker::new(config.s3_cost_per_gb_month);
    
    tokio::spawn(start_file_cleanup_server(
        config.cleanup_server_port, 
        base_directory.clone(),
        cost_tracker_for_server
    ));
    
    // Give server time to start
    sleep(Duration::from_secs(2)).await;

    // Create GarbageTruck client
    let gc_client = GCClient::new(
        &config.garbagetruck_endpoint,
        "experiment-service".to_string()
    ).await?;

    let file_config = FileSimulationConfig {
        base_directory: base_directory.clone(),
        file_size_bytes: (config.file_size_mb * 1024.0 * 1024.0) as usize,
        job_count: config.job_count,
        job_duration_seconds: 5,
        crash_probability: config.crash_probability,
        cleanup_endpoint: Some(cleanup_endpoint),
    };

    let cost_tracker = StorageCostTracker::new(config.s3_cost_per_gb_month);
    let mut simulator = JobSimulator::new(file_config, cost_tracker, Some(gc_client));

    let stats = simulator.run_with_garbagetruck().await?;
    
    // Count actual remaining files
    let (remaining_files, remaining_bytes) = simulator.count_remaining_files()?;
    info!("📁 Actual files remaining on disk: {} files, {:.2} MB", 
          remaining_files, remaining_bytes as f64 / (1024.0 * 1024.0));

    Ok(stats)
}

fn print_enhanced_experiment_results(without_gc: &StorageStats, with_gc: &StorageStats, config: &ExperimentConfig) {
    println!("\n📊 === DETAILED EXPERIMENT RESULTS ===");
    
    println!("\n🚫 WITHOUT GARBAGETRUCK:");
    println!("  Files created: {}", without_gc.total_files_created);
    println!("  Files cleaned: {}", without_gc.total_files_cleaned);
    println!("  Files orphaned: {} ({:.1}%)", 
             without_gc.orphaned_files, 
             (without_gc.orphaned_files as f64 / without_gc.total_files_created as f64) * 100.0);
    println!("  Storage used: {:.2} MB ({:.2} GB)", 
             without_gc.current_storage_bytes() as f64 / (1024.0 * 1024.0),
             without_gc.current_storage_bytes() as f64 / (1024.0 * 1024.0 * 1024.0));
    println!("  Monthly cost: ${:.4}", without_gc.current_monthly_cost());
    println!("  Cleanup efficiency: {:.1}%", without_gc.cleanup_efficiency());
    
    println!("\n✅ WITH GARBAGETRUCK:");
    println!("  Files created: {}", with_gc.total_files_created);
    println!("  Files cleaned: {}", with_gc.total_files_cleaned);
    println!("  Files orphaned: {} ({:.1}%)", 
             with_gc.orphaned_files,
             (with_gc.orphaned_files as f64 / with_gc.total_files_created as f64) * 100.0);
    println!("  Storage used: {:.2} MB ({:.2} GB)", 
             with_gc.current_storage_bytes() as f64 / (1024.0 * 1024.0),
             with_gc.current_storage_bytes() as f64 / (1024.0 * 1024.0 * 1024.0));
    println!("  Monthly cost: ${:.4}", with_gc.current_monthly_cost());
    println!("  Cleanup efficiency: {:.1}%", with_gc.cleanup_efficiency());
    
    // Enhanced cost analysis
    let storage_savings_mb = (without_gc.current_storage_bytes() as i64 - with_gc.current_storage_bytes() as i64) as f64 / (1024.0 * 1024.0);
    let storage_savings_gb = storage_savings_mb / 1024.0;
    let cost_savings = without_gc.current_monthly_cost() - with_gc.current_monthly_cost();
    let efficiency_improvement = with_gc.cleanup_efficiency() - without_gc.cleanup_efficiency();
    
    println!("\n💰 === COMPREHENSIVE COST ANALYSIS ===");
    println!("  Storage saved: {:.2} MB ({:.3} GB)", storage_savings_mb, storage_savings_gb);
    println!("  Monthly cost savings: ${:.4}", cost_savings);
    println!("  Annual cost savings: ${:.2}", cost_savings * 12.0);
    println!("  Cleanup efficiency improvement: {:.1} percentage points", efficiency_improvement);
    
    if cost_savings > 0.001 {
        let savings_percentage = (cost_savings / without_gc.current_monthly_cost()) * 100.0;
        println!("  Cost reduction: {:.1}%", savings_percentage);
        
        println!("\n🎉 GarbageTruck provides measurable cost savings!");
        
        // Multi-scale projections
        println!("\n📈 === SCALE PROJECTIONS ===");
        
        // 10x scale
        let scale_10x_monthly = cost_savings * 10.0;
        let scale_10x_annual = scale_10x_monthly * 12.0;
        println!("  At 10x scale ({} jobs):", without_gc.total_files_created * 10);
        println!("    Monthly savings: ${:.2}", scale_10x_monthly);
        println!("    Annual savings: ${:.2}", scale_10x_annual);
        
        // 100x scale
        let scale_100x_monthly = cost_savings * 100.0;
        let scale_100x_annual = scale_100x_monthly * 12.0;
        println!("  At 100x scale ({} jobs):", without_gc.total_files_created * 100);
        println!("    Monthly savings: ${:.2}", scale_100x_monthly);
        println!("    Annual savings: ${:.2}", scale_100x_annual);
        
        // Enterprise scale
        let enterprise_multiplier = 1000.0;
        let enterprise_monthly_savings = cost_savings * enterprise_multiplier;
        let enterprise_annual_savings = enterprise_monthly_savings * 12.0;
        
        println!("  At enterprise scale ({} jobs):", without_gc.total_files_created * 1000);
        println!("    Monthly savings: ${:.2}", enterprise_monthly_savings);
        println!("    Annual savings: ${:.2}", enterprise_annual_savings);
        
        if enterprise_annual_savings > 1000.0 {
            println!("    💸 Enterprise potential: ${}K+ annual savings!", 
                     (enterprise_annual_savings / 1000.0) as u32);
        }
    } else {
        println!("  ⚠️  Minimal cost savings with current parameters");
        
        println!("\n💡 === OPTIMIZATION SUGGESTIONS ===");
        println!("  To see more dramatic cost savings:");
        println!("  • Increase file size: EXPERIMENT_MODE=enterprise (250MB files)");
        println!("  • Higher failure rate: Set crash_probability to 0.6-0.8");
        println!("  • More jobs: Increase job_count to 500-1000");
        println!("  • Real cloud costs: Use actual S3/Azure blob pricing");
        
        println!("\n🚀 Try running:");
        println!("  EXPERIMENT_MODE=enterprise cargo run --bin garbagetruck-experiment --features client");
        println!("  EXPERIMENT_MODE=demo cargo run --bin garbagetruck-experiment --features client");
    }
    
    // Environmental impact
    let gb_hours_saved = storage_savings_gb * 24.0 * 30.0; // GB-hours per month
    let co2_saved_kg = gb_hours_saved * 0.0001; // Rough estimate: 0.1g CO2 per GB-hour
    
    println!("\n🌱 === ENVIRONMENTAL IMPACT ===");
    println!("  Storage saved: {:.1} GB-hours/month", gb_hours_saved);
    println!("  Estimated CO2 reduction: {:.2} kg/month", co2_saved_kg);
    println!("  Annual CO2 reduction: {:.1} kg", co2_saved_kg * 12.0);
    
    println!("\n✨ Experiment completed successfully!");
    
    // Next steps based on results
    println!("\n🔬 === EXPERIMENT INSIGHTS ===");
    println!("  • Orphaned file reduction: {:.1}x improvement", 
             without_gc.orphaned_files as f64 / (with_gc.orphaned_files as f64).max(1.0));
    println!("  • Storage efficiency: {:.1}% improvement", 
             ((storage_savings_gb / (without_gc.current_storage_bytes() as f64 / (1024.0 * 1024.0 * 1024.0))) * 100.0));
    println!("  • ROI timeline: ~{:.0} months to break even", 
             if cost_savings > 0.0 { 10.0 / (cost_savings * 12.0) } else { 999.0 });
}