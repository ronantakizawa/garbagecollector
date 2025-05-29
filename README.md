# GarbageTruck: A Lease-based Garbage Collection Sidecar for Distributed Systems

A high-performance, lease-based distributed garbage collection system for microservices, built with Rust and gRPC. This sidecar service automatically manages cross-service object references and reclaims orphaned resources when leases expire.

![ChatGPT Image May 29, 2025, 03_51_31 PM](https://github.com/user-attachments/assets/f8a018ba-fc0e-41b0-b022-b704a0049dc3)


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

2. **Start the service**
```bash
# Start with debug logging
RUST_LOG=distributed_gc_sidecar=info ./target/release/gc-sidecar
```

3. **Install grpcurl for testing**
```bash
# On macOS
brew install grpcurl

# On Ubuntu/Debian  
sudo apt-get install grpcurl
```

### Basic Testing

```bash
# Health check
grpcurl -plaintext -import-path proto -proto gc_service.proto localhost:50051 distributed_gc.DistributedGCService/HealthCheck

# Create a lease
grpcurl -plaintext -import-path proto -proto gc_service.proto -d '{
  "object_id": "session-12345",
  "object_type": "WEBSOCKET_SESSION",
  "service_id": "web-service",
  "lease_duration_seconds": 60,
  "metadata": {"user_id": "user123"}
}' localhost:50051 distributed_gc.DistributedGCService/CreateLease

# List leases
grpcurl -plaintext -import-path proto -proto gc_service.proto -d '{"limit": 10}' localhost:50051 distributed_gc.DistributedGCService/ListLeases
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

## 🙏 Acknowledgments

- Built with [Tonic](https://github.com/hyperium/tonic) for gRPC support
- Uses [Tokio](https://tokio.rs/) for async runtime
- Metrics powered by [Prometheus](https://prometheus.io/)
- High-performance collections with [DashMap](https://github.com/xacrimon/dashmap)
