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

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
