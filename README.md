# GarbageTruck: A Lease-based Garbage Collection Sidecar for Distributed Systems

A high-performance, lease-based distributed garbage collection system for microservices, built with Rust and gRPC. This sidecar service automatically manages cross-service object references and reclaims orphaned resources when leases expire.

<img width="592" alt="Screenshot 2025-05-30 at 12 43 20 PM" src="https://github.com/user-attachments/assets/f745ec72-c6cf-473a-af05-713c2fc1cac2" />

## 🎯 Features

### Core Functionality
- **Lease-Based GC**: Time-boxed leases for object references with automatic expiration
- **Cross-Service Management**: Handle references between microservices safely
- **Automatic Cleanup**: Configurable cleanup operations for different resource types
- **High Performance**: Built with Rust for maximum performance and safety

### Object Types Supported
- Database rows and records
- Blob storage objects (S3, Azure Blob, etc.)
- Temporary files and cached data
- WebSocket sessions and connections
- Cache entries and in-memory objects
- Custom resource types

### Advanced Features
- **Multi-backend Storage**: In-memory (development) and PostgreSQL
- **Retry Logic**: Configurable retry policies for cleanup operations
- **Metrics & Monitoring**: Prometheus metrics with comprehensive tracking
- **Health Checks**: Built-in health monitoring and status reporting
- **Graceful Degradation**: Continues operating even when some cleanups fail

## 🏗️ Architecture

```
┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
│   Service A     │    │   Service B     │    │   Service C     │
│                 │    │                 │    │                 │
│ ┌─────────────┐ │    │ ┌─────────────┐ │    │ ┌─────────────┐ │
│ │ GC Client   │◄┼────┼─│ GC Client   │◄┼────┼─│ GC Client   │ │
│ └─────────────┘ │    │ └─────────────┘ │    │ └─────────────┘ │
└─────────────────┘    └─────────────────┘    └─────────────────┘
         │                       │                       │
         └───────────────────────┼───────────────────────┘
                                 │
                ┌─────────────────▼─────────────────┐
                │     GarbageTruck Sidecar Service  │
                │   (Port 50051 - gRPC)             │
                │                                   │
                │ ┌─────────────┐ ┌─────────────┐   │
                │ │ Lease Mgr   │ │ Cleanup Exec│   │
                │ └─────────────┘ └─────────────┘   │
                │                                   │
                │ ┌─────────────┐ ┌─────────────┐   │
                │ │ Storage     │ │ Metrics     │   │
                │ └─────────────┘ └─────────────┘   │
                └───────────────────────────────────┘
                                 │
                    ┌─────────────▼─────────────┐
                    │     Cleanup Targets       │
                    │                           │
                    │ • HTTP/gRPC Endpoints     │
                    │ • Database Operations     │
                    │ • File System Cleanup    │
                    │ • Custom Handlers         │
                    └───────────────────────────┘
```

## 📦 Use Cases 

### 1. Cross-Service Object Lifecycle Management

**Scenario**: Microservices (e.g., A, B, C) share object references (e.g., DB rows, blobs).  
**Problem**: No centralized way to detect when an object is no longer needed.  
**Solution**: Garbage Truck issues time-boxed leases. When all expire, the object is reclaimed automatically.

### 2. Temporary Resource Cleanup

**Scenario**: APIs or batch jobs generate temporary files or cache entries.  
**Problem**: Orphaned resources accumulate and consume disk/memory.  
**Solution**: Register objects with leases; Garbage Truck deletes them after expiration.

### 3. Session and Connection Expiry

**Scenario**: Applications maintain WebSocket or user sessions.  
**Problem**: Sessions remain active after user disconnects or crashes.  
**Solution**: Lease expires on inactivity, and the GC closes connections or cleans up session state.

### 4. Database Row TTL Enforcement

**Scenario**: Temporary data (e.g., carts, ephemeral records) stored in a database.  
**Problem**: Old data persists without cleanup, bloating the DB.  
**Solution**: Associate each row with a lease; expired leases trigger deletion.

### 5. Blob/Object Storage Cleanup

**Scenario**: Services upload files to S3, Azure Blob, etc.  
**Problem**: No tracking of file usage lifecycle across services.  
**Solution**: Attach leases to object IDs and clean up expired blobs automatically.

## 🚀 Quick Start

### Prerequisites
- Rust 1.75+
- Protocol Buffers compiler (`protoc`)
- grpcurl (for testing)

### Installation

1. **Clone and build the project**
```bash
git clone <repository-url>
cd distributed-gc-sidecar
cargo build --release
```

2. **Start the service locally**
```bash
RUST_LOG=distributed_gc_sidecar=info ./target/release/gc-sidecar
```

3. **Start the service via Docker-Compose (Includes PostgreSQL, Prometheus, and Grafana Support)**
```bash
docker-compose up --build
```

4. **Install grpcurl for testing**
```bash
# On macOS
brew install grpcurl

# On Ubuntu/Debian  
sudo apt-get install grpcurl
```

### Basic Testing

### 1. Start server
```bash
cargo build
RUST_LOG=debug ./target/release/gc-sidecar
```
### 2. Run gRPC Health Check
```bash
grpcurl -plaintext -import-path proto -proto gc_service.proto localhost:50051 distributed_gc.DistributedGCService/HealthCheck
```

### 3. Enter basic lease
```bash
grpcurl -plaintext -import-path proto -proto gc_service.proto -d '{
  "object_id": "session-12345",
  "object_type": "WEBSOCKET_SESSION",
  "service_id": "web-service",
  "lease_duration_seconds": 60,
  "metadata": {"user_id": "user123"}
}' localhost:50051 distributed_gc.DistributedGCService/CreateLease
```
### 4. List leases
```bash
grpcurl -plaintext -import-path proto -proto gc_service.proto -d '{"limit": 10}' localhost:50051 distributed_gc.DistributedGCService/ListLeases
```

## gRPC Functionality Testing 


### 1. Start server
```bash
cargo build
RUST_LOG=debug ./target/release/gc-sidecar
```

### 2. Run gRPC Tests
```bash
cargo test --features grpc
```

## PostgreSQL Functionality Testing 

### 1. Start server
```bash
cargo build
RUST_LOG=debug ./target/release/gc-sidecar
```

### 2. Start a Test Postgres Database (via Docker)

Start a local Postgres instance (if you don’t have one running already):

```bash
docker run --name sqlx-test-db \
  -e POSTGRES_USER=testuser \
  -e POSTGRES_PASSWORD=testpass \
  -e POSTGRES_DB=testdb \
  -p 5432:5432 \
  -d postgres:16
```

### 3. Set the Database URL

Export the database URL for use by tests:

```bash
export DATABASE_URL=postgres://testuser:testpass@localhost:5432/testdb
```

### 4. Apply the Schema Using Docker

Run migrations directly inside your running container:

```bash
docker exec -i sqlx-test-db psql -U testuser -d testdb < migrations/001_initial.sql
```

### 5. Run the Test 

```bash
cargo test
```

### 6. Reset the database
```bash
docker exec -it sqlx-test-db psql -U testuser -d postgres -c 'DROP DATABASE IF EXISTS testdb;'
docker exec -it sqlx-test-db psql -U testuser -d postgres -c 'CREATE DATABASE testdb;'
psql $DATABASE_URL -f migrations/001_initial.sql
```

## ⚙️ Configuration

The service can be configured via environment variables:

### Server Configuration
```bash
export GC_SERVER_HOST=0.0.0.0
export GC_SERVER_PORT=50051
```

### Lease Management
```bash
export GC_DEFAULT_LEASE_DURATION=300      # 5 minutes
export GC_MAX_LEASE_DURATION=3600         # 1 hour  
export GC_MIN_LEASE_DURATION=30           # 30 seconds
export GC_CLEANUP_INTERVAL=60             # 1 minute
export GC_CLEANUP_GRACE_PERIOD=30         # 30 seconds
export GC_MAX_LEASES_PER_SERVICE=10000
```

### Storage Backend
```bash
# Use in-memory storage (default, for development)
export GC_STORAGE_BACKEND=memory

# Use PostgreSQL (for production)
export GC_STORAGE_BACKEND=postgres
export DATABASE_URL="postgresql://user:pass@host:5432/distributed_gc"
export GC_MAX_DB_CONNECTIONS=10
```

### Cleanup Configuration
```bash
export GC_CLEANUP_TIMEOUT=30
export GC_CLEANUP_MAX_RETRIES=3
export GC_CLEANUP_RETRY_DELAY=5
```

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
