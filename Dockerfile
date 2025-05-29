# Build stage
FROM rust:1.82-slim as builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy manifests first for better caching
COPY Cargo.toml Cargo.lock ./
COPY build.rs ./

# Create proto directory and copy proto files
COPY proto/ ./proto/

# Create src directory with a dummy main.rs to build dependencies
RUN mkdir -p src && \
    echo "fn main() {}" > src/main.rs && \
    mkdir -p target

# Build dependencies first (this layer will be cached if dependencies don't change)
RUN cargo build --release
RUN rm -rf src target/release/deps/distributed_gc_sidecar* target/release/deps/gc_sidecar*

# Copy the actual source code
COPY src/ ./src/

# Build the application
RUN cargo build --release

# Verify binary was created
RUN ls -la target/release/

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

# Create app user for security
RUN groupadd -r gcuser && useradd -r -g gcuser gcuser

# Create app directory
WORKDIR /app

# Copy the binary from builder stage
COPY --from=builder /app/target/release/gc-sidecar ./gc-sidecar

# Copy proto files for potential reflection needs
COPY --from=builder /app/proto/ ./proto/

# Copy migrations (if using postgres)
COPY migrations/ ./migrations/

# Make binary executable and change ownership
RUN chmod +x ./gc-sidecar && \
    chown -R gcuser:gcuser /app

# Switch to app user
USER gcuser

# Expose gRPC port
EXPOSE 50051

# Expose metrics port  
EXPOSE 9090

# Health check using the binary itself
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:9090/health || exit 1

# Set environment variables
ENV RUST_LOG=distributed_gc_sidecar=info
ENV GC_SERVER_HOST=0.0.0.0
ENV GC_SERVER_PORT=50051

# Run the application
CMD ["./gc-sidecar"]