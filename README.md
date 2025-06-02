# GarbageTruck: A Lease-based Garbage Collection Sidecar for Distributed Systems

A high-performance, lease-based distributed garbage collection system for microservices, built with Rust and gRPC. This sidecar service automatically manages cross-service object references and reclaims orphaned resources when leases expire.

<img width="856" alt="Screenshot 2025-05-30 at 12 44 26 PM" src="https://github.com/user-attachments/assets/3b50b11b-5040-43d9-92f8-588c87f3f08c" />

## The Problem it Solves

In modern apps with multiple services, temporary files, cache entries, and database records get "orphaned" where nobody remembers to clean them up, so they pile up forever. Most databases don't auto-cleanup, and even if they did 
GarbageTruck acts like a smart janitor for your system. It hands out time-limited "leases" to services for the resources they create. If a service crashes or fails to renew the lease, the associated resources are automatically reclaimed.

**Without GarbageTruck:**  
User uploads a file → Processing service crashes → File remains forever  
**Result:** Disk fills up with abandoned files

**With GarbageTruck:**  
User uploads a file → Service receives a 1-hour lease → Service crashes → File is auto-deleted after 1 hour  
**Result:** Clean system, no orphaned resources

## 🎯 Features
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

<img width="982" alt="Screenshot 2025-05-31 at 10 50 31 PM" src="https://github.com/user-attachments/assets/e1bd40ec-904c-4afe-9e16-1cd2a84e544a" />
<img width="871" alt="Screenshot 2025-05-31 at 10 35 23 PM" src="https://github.com/user-attachments/assets/f2909eea-6660-4b4f-8c24-4224c1c0c8a6" />



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
cd garbagetruck
cargo build --release
```

2. **Start the service locally**
```bash
cargo run --bin garbagetruck-server
```

3. **Start the service via Docker-Compose (Includes PostgreSQL, Prometheus, and Grafana Support)**
```bash
docker-compose up --build
```

### Basic Testing

### 1. Install CLI (Mac / Linux)
```bash
cargo build --release --bin garbagetruck --features client
mkdir -p ~/.local/bin
cp target/release/garbagetruck ~/.local/bin/
chmod +x ~/.local/bin/garbagetruck
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.bashrc && source ~/.bashrc
garbagetruck --help
```
### 2. Run gRPC Health Check
```bash
garbagetruck health 
```

### 3. Enter basic lease
```bash
garbagetruck lease create --object-id "user-session" --object-type websocket-session --duration 3600
```
### 4. List leases
```bash
garbagetruck lease list --service my-service --limit 20
```

## gRPC Functionality Testing 


### 1. Start server
```bash
cargo build
cargo run --bin garbagetruck-server
```

### 2. Run gRPC Tests
```bash
cargo test --features grpc
```

## PostgreSQL Functionality Testing 

### 1. Start server
```bash
cargo build
RUST_LOG=debug ./target/release/garbagetruck
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

## 💸 Storage Cost Savings Experiment

The experiment simulates a realistic distributed system where:

- Jobs create temporary files (logs, uploads, processing artifacts)  
- System failures occur at configurable rates (crashes, network issues)  
- Files become orphaned without proper cleanup mechanisms  
- **GarbageTruck** automatically recovers orphaned resources via lease expiration  

---

### 📊 Results  
**Catastrophic Failure Scenario (100% job crash rate)**

#### 🚫 WITHOUT GARBAGETRUCK:
- Files created: **200**  
- Files cleaned: **0**  
- Files orphaned: **200 (100.0%)**  
- Storage used: **20000.00 MB (19.53 GB)**  
- Monthly cost: **$0.4492**  
- Cleanup efficiency: **0.0%**

#### ✅ WITH GARBAGETRUCK:
- Files created: **200**  
- Files cleaned: **200**  
- Files orphaned: **0 (0.0%)**  
- Storage used: **0.00 MB (0.00 GB)**  
- Monthly cost: **$0.0000**  
- Cleanup efficiency: **100.0%**

**💰 Result:**  
**Assuming it costs $0.02 per GB /month:**
- **Monthly cost savings:** `$0.4492`  
- **Annual cost savings:** `$5.39`

---

### 📈 Scale Projections

| Scale        | Jobs   | Storage | Monthly Cost Without GT | Monthly Cost With GT | Annual Savings |
|--------------|--------|---------|--------------------------|------------------------|----------------|
| Development  | 200    | 20GB    | $0.45                   | $0.045                | $4.86          |
| Production   | 2,000  | 200GB   | $4.50                   | $0.45                 | $48.60         |
| Enterprise   | 20,000 | 2TB     | $45.00                  | $4.50                 | $486.00        |
| Hyperscale   | 200,000| 20TB    | $450.00                 | $45.00                | $4,860.00      |

---


## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
