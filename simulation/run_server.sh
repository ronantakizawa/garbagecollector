#!/bin/bash

# run_server.sh - Start GarbageTruck server with experiment-friendly configuration

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${BLUE}🚀 Starting GarbageTruck Server (Experiment Mode)${NC}"
echo ""

# Set experiment-friendly environment variables if not already set
export GC_SERVER_HOST=${GC_SERVER_HOST:-0.0.0.0}
export GC_SERVER_PORT=${GC_SERVER_PORT:-50051}
export GC_STORAGE_BACKEND=${GC_STORAGE_BACKEND:-memory}
export GC_DEFAULT_LEASE_DURATION=${GC_DEFAULT_LEASE_DURATION:-300}
export GC_MIN_LEASE_DURATION=${GC_MIN_LEASE_DURATION:-10}    # 10 seconds minimum
export GC_MAX_LEASE_DURATION=${GC_MAX_LEASE_DURATION:-3600}  # 1 hour maximum
export GC_CLEANUP_INTERVAL=${GC_CLEANUP_INTERVAL:-30}        # More frequent cleanup for experiments
export GC_CLEANUP_GRACE_PERIOD=${GC_CLEANUP_GRACE_PERIOD:-5} # Shorter grace period for experiments
export GC_MAX_LEASES_PER_SERVICE=${GC_MAX_LEASES_PER_SERVICE:-10000}
export GC_METRICS_ENABLED=${GC_METRICS_ENABLED:-true}
export GC_METRICS_PORT=${GC_METRICS_PORT:-9090}
export RUST_LOG=${RUST_LOG:-info}

echo -e "${GREEN}📋 Experiment-Friendly Configuration:${NC}"
echo "  Server: $GC_SERVER_HOST:$GC_SERVER_PORT"
echo "  Storage: $GC_STORAGE_BACKEND"
echo "  Lease duration: ${GC_MIN_LEASE_DURATION}s - ${GC_MAX_LEASE_DURATION}s (default: ${GC_DEFAULT_LEASE_DURATION}s)"
echo "  Cleanup interval: ${GC_CLEANUP_INTERVAL}s (grace: ${GC_CLEANUP_GRACE_PERIOD}s)"
echo "  Max leases per service: $GC_MAX_LEASES_PER_SERVICE"
echo "  Metrics: $GC_METRICS_ENABLED (port $GC_METRICS_PORT)"
echo ""

# Check if using PostgreSQL and warn about DATABASE_URL
if [ "$GC_STORAGE_BACKEND" = "postgres" ]; then
    if [ -z "$DATABASE_URL" ]; then
        echo -e "${RED}❌ Error: PostgreSQL backend selected but DATABASE_URL not set${NC}"
        echo -e "${YELLOW}💡 Set DATABASE_URL environment variable:${NC}"
        echo "   export DATABASE_URL='postgresql://user:password@localhost:5432/garbagetruck'"
        echo ""
        exit 1
    else
        echo -e "${GREEN}✅ PostgreSQL configuration detected${NC}"
        echo "  Database URL: ${DATABASE_URL%@*}@***"
        echo ""
    fi
fi

# Build the server
echo -e "${BLUE}🔨 Building GarbageTruck server...${NC}"
if ! cargo build --release --bin garbagetruck-server; then
    echo -e "${RED}❌ Build failed${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Build successful${NC}"
echo ""

# Start the server
echo -e "${BLUE}🚀 Starting server...${NC}"
echo -e "${YELLOW}💡 Configured for experiments with:${NC}"
echo "   • Min lease duration: ${GC_MIN_LEASE_DURATION}s (allows short test leases)"
echo "   • Fast cleanup: every ${GC_CLEANUP_INTERVAL}s"
echo "   • Short grace period: ${GC_CLEANUP_GRACE_PERIOD}s"
echo ""
echo -e "${YELLOW}💡 Use Ctrl+C to stop the server${NC}"
echo ""

# Run the server
cargo run --release --bin garbagetruck-server