## 📂 Source File Overview

- **src/bin/main.rs** – Application entry point and minimal startup coordination  
- **src/lib.rs** – Crate root with module declarations and public API re-exports  
- **src/startup.rs** – Application startup orchestration and component initialization  
- **src/dependencies.rs** – External dependency checking and health validation  
- **src/monitoring.rs** – System monitoring tasks and performance tracking  
- **src/config.rs** – Configuration loading, validation, and environment variable parsing  
- **src/error.rs** – Error types and `Result` definitions for the entire crate  
- **src/lease.rs** – Core lease data structures and business logic  
- **src/cleanup.rs** – Cleanup executor for expired lease processing  
- **src/client.rs** – gRPC client SDK and convenience methods for service interaction  
- **src/shutdown.rs** – Graceful shutdown coordination and task management  

### 📦 Service Module (`src/service/`)
- **mod.rs** – Core service struct, business logic, and cleanup loop management  
- **handlers.rs** – gRPC method implementations and request/response handling  
- **validation.rs** – Request validation logic and input sanitization  

### 💾 Storage Module (`src/storage/`)
- **mod.rs** – Storage trait definitions, factory function, and shared types  
- **memory.rs** – In-memory storage implementation using DashMap for development/testing  
- **postgres.rs** – PostgreSQL storage implementation with connection pooling for production  

### 📊 Metrics Module (`src/metrics/`)
- **mod.rs** – Core Prometheus metrics definitions and main `Metrics` struct  
- **alerting.rs** – Alert management system with thresholds and notification logic  
- **interceptors.rs** – gRPC interceptors for automatic request/response metrics collection  
- **monitoring.rs** – Background monitoring tasks and extended metric implementations 

## 🧪 Test File Structure Overview

### 📂 Main Test Directory (`tests/`)

- **`lib.rs`** – Main test library entry point and module coordination  
- **`comprehensive_test.sh`** – Shell script for end-to-end testing scenarios  
- **`shutdown.rs`** – Legacy shutdown tests (consider moving to integration structure)  

---

### 🔧 Test Helpers (`tests/helpers/`)

- **`mod.rs`** – Common test utilities, port management, and service availability checks  
- **`mock_server.rs`** – `MockCleanupServer` implementation for simulating cleanup endpoints  
- **`test_data.rs`** – Test lease data generators and factory methods for consistent test objects  
- **`assertions.rs`** – Custom domain-specific assertions for lease validation and cleanup verification  

---

### 🏗️ Integration Tests (`tests/integration/`)

- **`mod.rs`** – Integration test harness, `TestHarness` struct, and common test coordination utilities  

---

### 💾 Storage Integration Tests (`tests/integration/storage/`)

- **`mod.rs`** – Storage test utilities, common interface tests, and backend factory testing  
- **`memory.rs`** – In-memory storage implementation tests (CRUD, filtering, statistics, cleanup)  
- **`postgres.rs`** – PostgreSQL storage tests (advanced queries, concurrent operations, performance)  
- **`factory.rs`** – Storage factory pattern tests and backend selection validation  

---

### 🌐 gRPC Integration Tests (`tests/integration/grpc/`)

- **`mod.rs`** – gRPC test utilities, request builders, and common gRPC test patterns  
- **`basic.rs`** – Basic gRPC operations (health check, CRUD, lease lifecycle management)  
- **`auth.rs`** – Authorization and security tests (unauthorized access, lease ownership validation)  
- **`concurrent.rs`** – Concurrency and stress tests (parallel operations, race conditions, load testing)  
- **`cleanup.rs`** – Cleanup integration tests (automatic cleanup, retry logic, cleanup server communication)  

---

### 🔄 Service Integration Tests (`tests/integration/service/`)

- **`mod.rs`** – Service-level test utilities, test service creation, and shutdown coordination helpers  
- **`shutdown.rs`** – Graceful shutdown coordination tests (priority ordering, timeout handling, signal management)  
- **`lifecycle.rs`** – Service lifecycle management tests (startup/shutdown cycles, component integration, restart scenarios)  

---

### 🔗 Cross-Backend Tests (`tests/integration/cross_backend/`)

- **`mod.rs`** – Cross-backend consistency tests and storage behavior validation across different backends  
- **`consistency.rs`** – Data consistency verification tests ensuring identical behavior between memory and PostgreSQL storage  

---

### 🗄️ Database-Specific Tests (`tests/integration/database/`)  
*Note: These tests are conditionally compiled with the `postgres` feature*

- **`mod.rs`** – Database test utilities and PostgreSQL-specific testing infrastructure  
- **`schema.rs`** – Database schema integrity tests (table structure, constraints, indexes)  
- **`triggers.rs`** – Database trigger functionality tests (automatic statistics updates, state transitions)  
- **`functions.rs`** – Database function tests (stored procedures, statistics calculations, cleanup operations)  
- **`performance.rs`** – Database performance and indexing tests (query optimization, concurrent access)  