# GarbageTruck: Garbage Collection for Microservice Architectures

A high-performance, lease-based distributed garbage collection system for microservices, built with Rust and gRPC. This sidecar service automatically manages the lifecycle of temporary files, preventing the generation of orphaned data and reducing infrastructure costs. 

<img width="856" alt="Screenshot 2025-05-30 at 12 44 26 PM" src="https://github.com/user-attachments/assets/3b50b11b-5040-43d9-92f8-588c87f3f08c" />

## The Problem it Solves

In modern apps with multiple services, temporary files, cache entries, and database records get "orphaned" where nobody remembers to clean them up, so they pile up forever. 
GarbageTruck acts like a smart janitor for your system. It hands out time-limited "leases" to services for the resources they create. If a service crashes or fails to renew the lease, the associated resources are automatically reclaimed.

**Without GarbageTruck:**  
User uploads a file → Processing service crashes → File remains forever  
**Result:** Disk fills up with abandoned files

**With GarbageTruck:**  
User uploads a file → Service receives a 1-hour lease → Service crashes → File is auto-deleted after 1 hour  
**Result:** Clean system, no orphaned resources

## 🚀 Quick Start

### Prerequisites
- Rust 1.75+
- Protocol Buffers compiler (`protoc`)
- grpcurl (for testing)

### Installation

1. **Add GarbageTruck to your project**
```bash
# For using the client library in your application
cargo add garbagetruck

# For CLI tools only
cargo install garbagetruck --features client

# For server deployment  
cargo install garbagetruck --features "client,server"
```

Or add to your Cargo.toml:
```toml
[dependencies]
garbagetruck = "0.1.0"
```

2. **Start the server locally**
```bash
garbagetruck-server
```

3. **Start the service via Docker-Compose (Includes Prometheus, and Grafana Support)**
```bash
git clone https://github.com/ronantakizawa/garbagetruck.git
cd garbagetruck
cargo build --release --features "client,server"
docker-compose up --build
```

### Basic Testing

### 1. Install CLI (Mac / Linux)
```bash
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
garbagetruck-server
```

### 2. Run gRPC Tests
```bash
cargo test --features grpc
```

## 📄 License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.
