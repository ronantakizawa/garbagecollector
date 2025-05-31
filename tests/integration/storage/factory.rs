// tests/integration/storage/factory.rs - Storage factory tests

use anyhow::Result;
use garbagetruck::config::Config;
use garbagetruck::storage::create_storage;

use crate::helpers::test_data::create_test_lease_data; // Remove skip_if_no_env for now
use crate::integration::print_test_header;

#[tokio::test]
async fn test_storage_factory() -> Result<()> {
    print_test_header("storage factory", "🏭");

    // Test memory storage creation
    let mut config = Config::default();
    config.storage.backend = "memory".to_string();

    let memory_storage = create_storage(&config).await?;

    // Test basic operation
    let test_lease = create_test_lease_data("factory-test", "factory-service", 300);
    memory_storage.create_lease(test_lease.clone()).await?;

    let retrieved = memory_storage.get_lease(&test_lease.lease_id).await?;
    assert!(
        retrieved.is_some(),
        "Should create and retrieve lease via factory"
    );

    println!("✅ Memory storage factory works");

    // Test PostgreSQL storage creation (if available)
    #[cfg(feature = "postgres")]
    {
        if std::env::var("DATABASE_URL").is_ok() {
            let database_url = std::env::var("DATABASE_URL").unwrap();
            config.storage.backend = "postgres".to_string();
            config.storage.database_url = Some(database_url);
            config.storage.max_connections = Some(5);

            let postgres_storage = create_storage(&config).await?;

            let pg_test_lease =
                create_test_lease_data("pg-factory-test", "pg-factory-service", 300);
            postgres_storage.create_lease(pg_test_lease.clone()).await?;

            let pg_retrieved = postgres_storage.get_lease(&pg_test_lease.lease_id).await?;
            assert!(
                pg_retrieved.is_some(),
                "Should create and retrieve lease via PostgreSQL factory"
            );

            // Clean up
            let _ = postgres_storage.delete_lease(&pg_test_lease.lease_id).await;

            println!("✅ PostgreSQL storage factory works");
        } else {
            println!("⚠️  Skipping PostgreSQL factory test - DATABASE_URL not set");
        }
    }

    #[cfg(not(feature = "postgres"))]
    {
        println!("⚠️  Skipping PostgreSQL factory test - postgres feature not enabled");
    }

    Ok(())
}

#[tokio::test]
async fn test_unknown_storage_backend() -> Result<()> {
    print_test_header("unknown storage backend handling", "❌");

    let mut config = Config::default();
    config.storage.backend = "unknown-backend".to_string();

    let result = create_storage(&config).await;
    assert!(result.is_err(), "Should fail for unknown backend");

    // Check the error message instead of unwrapping
    match result {
        Err(e) => {
            assert!(
                e.to_string().contains("Unknown storage backend"),
                "Error should mention unknown backend: {}",
                e
            );
        }
        Ok(_) => panic!("Expected error for unknown backend"),
    }

    println!("✅ Unknown storage backend properly rejected");
    Ok(())
}

#[tokio::test]
async fn test_postgres_without_url() -> Result<()> {
    print_test_header("postgres backend without URL", "🔧");

    let mut config = Config::default();
    config.storage.backend = "postgres".to_string();
    config.storage.database_url = None; // Missing URL

    let result = create_storage(&config).await;
    assert!(
        result.is_err(),
        "Should fail when postgres backend selected but no URL provided"
    );

    // Check the error message
    match result {
        Err(e) => {
            let error_msg = e.to_string();
            // Accept either error message depending on whether postgres feature is enabled
            let valid_error = error_msg.contains("Database URL required")
                || error_msg.contains("required for postgres")
                || error_msg.contains("Postgres support not compiled in");
            assert!(
                valid_error,
                "Error should mention missing URL or feature not compiled: {}",
                error_msg
            );
        }
        Ok(_) => panic!("Expected error for missing postgres URL"),
    }

    println!("✅ PostgreSQL backend properly requires URL (or feature)");
    Ok(())
}
