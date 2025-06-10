#!/bin/bash

# run_server.sh - Start GarbageTruck server with experiment-friendly configuration and persistent storage support

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
PURPLE='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

echo -e "${BLUE}🚀 Starting GarbageTruck Server (Experiment Mode with Persistent Storage)${NC}"
echo ""

# Parse command line arguments
BACKEND=""
ENABLE_WAL=""
ENABLE_RECOVERY=""
DATA_DIR=""
HELP=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --backend)
            BACKEND="$2"
            shift 2
            ;;
        --enable-wal)
            ENABLE_WAL="true"
            shift
            ;;
        --enable-recovery)
            ENABLE_RECOVERY="true"
            shift
            ;;
        --data-dir)
            DATA_DIR="$2"
            shift 2
            ;;
        --help|-h)
            HELP="true"
            shift
            ;;
        *)
            echo -e "${RED}❌ Unknown option: $1${NC}"
            HELP="true"
            shift
            ;;
    esac
done

if [[ "$HELP" == "true" ]]; then
    echo -e "${CYAN}📖 GarbageTruck Server Startup Script${NC}"
    echo ""
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --backend BACKEND       Storage backend: memory, persistent_file (default: memory)"
    echo "  --enable-wal           Enable Write-Ahead Logging (only with persistent_file)"
    echo "  --enable-recovery      Enable automatic recovery on startup"
    echo "  --data-dir DIR         Data directory for persistent storage (default: ./data)"
    echo "  --help, -h             Show this help message"
    echo ""
    echo "Examples:"
    echo "  $0                                    # Memory backend (fastest for experiments)"
    echo "  $0 --backend persistent_file          # Persistent storage without WAL"
    echo "  $0 --backend persistent_file --enable-wal --enable-recovery"
    echo "  $0 --backend persistent_file --enable-wal --data-dir /var/lib/garbagetruck"
    echo ""
    echo "Environment Variables:"
    echo "  All GC_* variables can be set to override defaults"
    echo "  See script source for complete list"
    exit 0
fi

# Set storage backend
if [[ -n "$BACKEND" ]]; then
    export GC_STORAGE_BACKEND="$BACKEND"
else
    export GC_STORAGE_BACKEND=${GC_STORAGE_BACKEND:-memory}
fi

# Set experiment-friendly environment variables if not already set
export GC_SERVER_HOST=${GC_SERVER_HOST:-0.0.0.0}
export GC_SERVER_PORT=${GC_SERVER_PORT:-50051}
export GC_DEFAULT_LEASE_DURATION=${GC_DEFAULT_LEASE_DURATION:-300}
export GC_MIN_LEASE_DURATION=${GC_MIN_LEASE_DURATION:-10}    # 10 seconds minimum
export GC_MAX_LEASE_DURATION=${GC_MAX_LEASE_DURATION:-3600}  # 1 hour maximum
export GC_CLEANUP_INTERVAL=${GC_CLEANUP_INTERVAL:-30}        # More frequent cleanup for experiments
export GC_CLEANUP_GRACE_PERIOD=${GC_CLEANUP_GRACE_PERIOD:-5} # Shorter grace period for experiments
export GC_MAX_LEASES_PER_SERVICE=${GC_MAX_LEASES_PER_SERVICE:-10000}
export GC_METRICS_ENABLED=${GC_METRICS_ENABLED:-true}
export GC_METRICS_PORT=${GC_METRICS_PORT:-9090}
export GC_INCLUDE_RECOVERY_METRICS=${GC_INCLUDE_RECOVERY_METRICS:-true}
export GC_INCLUDE_WAL_METRICS=${GC_INCLUDE_WAL_METRICS:-true}
export RUST_LOG=${RUST_LOG:-info}

# Configure persistent storage if enabled
if [[ "$GC_STORAGE_BACKEND" == "persistent_file" ]]; then
    echo -e "${PURPLE}💾 Configuring persistent file storage...${NC}"
    
    # Set data directory
    if [[ -n "$DATA_DIR" ]]; then
        export GC_DATA_DIRECTORY="$DATA_DIR"
    else
        export GC_DATA_DIRECTORY=${GC_DATA_DIRECTORY:-./data}
    fi
    
    # Set WAL and recovery options
    if [[ "$ENABLE_WAL" == "true" ]]; then
        export GC_ENABLE_WAL=true
        echo -e "${GREEN}✅ Write-Ahead Logging enabled${NC}"
    else
        export GC_ENABLE_WAL=${GC_ENABLE_WAL:-false}
    fi
    
    if [[ "$ENABLE_RECOVERY" == "true" ]]; then
        export GC_ENABLE_AUTO_RECOVERY=true
        echo -e "${GREEN}✅ Automatic recovery enabled${NC}"
    else
        export GC_ENABLE_AUTO_RECOVERY=${GC_ENABLE_AUTO_RECOVERY:-false}
    fi
    
    # Persistent storage configuration
    export GC_WAL_PATH=${GC_WAL_PATH:-"${GC_DATA_DIRECTORY}/garbagetruck.wal"}
    export GC_SNAPSHOT_PATH=${GC_SNAPSHOT_PATH:-"${GC_DATA_DIRECTORY}/garbagetruck.snapshot"}
    export GC_MAX_WAL_SIZE=${GC_MAX_WAL_SIZE:-10485760}  # 10MB for experiments
    export GC_WAL_SYNC_POLICY=${GC_WAL_SYNC_POLICY:-every:5}  # Sync every 5 operations
    export GC_COMPRESS_SNAPSHOTS=${GC_COMPRESS_SNAPSHOTS:-true}
    export GC_RECOVERY_STRATEGY=${GC_RECOVERY_STRATEGY:-conservative}
    export GC_SNAPSHOT_INTERVAL=${GC_SNAPSHOT_INTERVAL:-300}  # 5 minutes for experiments
    export GC_WAL_COMPACTION_THRESHOLD=${GC_WAL_COMPACTION_THRESHOLD:-1000}  # Lower threshold for experiments
    
    # Create data directory if it doesn't exist
    if [[ ! -d "$GC_DATA_DIRECTORY" ]]; then
        echo -e "${YELLOW}📁 Creating data directory: $GC_DATA_DIRECTORY${NC}"
        mkdir -p "$GC_DATA_DIRECTORY"
        if [[ $? -ne 0 ]]; then
            echo -e "${RED}❌ Failed to create data directory: $GC_DATA_DIRECTORY${NC}"
            exit 1
        fi
    fi
    
    # Check directory permissions
    if [[ ! -w "$GC_DATA_DIRECTORY" ]]; then
        echo -e "${RED}❌ Data directory is not writable: $GC_DATA_DIRECTORY${NC}"
        echo -e "${YELLOW}💡 Try: chmod 755 $GC_DATA_DIRECTORY${NC}"
        exit 1
    fi
fi

echo -e "${GREEN}📋 Server Configuration:${NC}"
echo "  Server: $GC_SERVER_HOST:$GC_SERVER_PORT"
echo "  Storage: $GC_STORAGE_BACKEND"

if [[ "$GC_STORAGE_BACKEND" == "persistent_file" ]]; then
    echo -e "${PURPLE}  💾 Persistent Storage:${NC}"
    echo "    Data directory: $GC_DATA_DIRECTORY"
    echo "    WAL enabled: $GC_ENABLE_WAL"
    echo "    Auto-recovery: $GC_ENABLE_AUTO_RECOVERY"
    
    if [[ "$GC_ENABLE_WAL" == "true" ]]; then
        echo "    WAL path: $GC_WAL_PATH"
        echo "    WAL sync policy: $GC_WAL_SYNC_POLICY"
        echo "    WAL max size: $(( GC_MAX_WAL_SIZE / 1024 / 1024 ))MB"
        echo "    WAL compaction threshold: $GC_WAL_COMPACTION_THRESHOLD entries"
    fi
    
    echo "    Snapshot path: $GC_SNAPSHOT_PATH"
    echo "    Snapshot interval: ${GC_SNAPSHOT_INTERVAL}s"
    echo "    Snapshot compression: $GC_COMPRESS_SNAPSHOTS"
    
    if [[ "$GC_ENABLE_AUTO_RECOVERY" == "true" ]]; then
        echo "    Recovery strategy: $GC_RECOVERY_STRATEGY"
    fi
fi

echo "  Lease duration: ${GC_MIN_LEASE_DURATION}s - ${GC_MAX_LEASE_DURATION}s (default: ${GC_DEFAULT_LEASE_DURATION}s)"
echo "  Cleanup interval: ${GC_CLEANUP_INTERVAL}s (grace: ${GC_CLEANUP_GRACE_PERIOD}s)"
echo "  Max leases per service: $GC_MAX_LEASES_PER_SERVICE"
echo "  Metrics: $GC_METRICS_ENABLED (port $GC_METRICS_PORT)"

if [[ "$GC_INCLUDE_RECOVERY_METRICS" == "true" ]] || [[ "$GC_INCLUDE_WAL_METRICS" == "true" ]]; then
    echo "  Enhanced metrics: Recovery=$GC_INCLUDE_RECOVERY_METRICS, WAL=$GC_INCLUDE_WAL_METRICS"
fi

echo ""

# Check for legacy PostgreSQL configuration and provide migration info
if [[ "$GC_STORAGE_BACKEND" == "postgres" ]]; then
    echo -e "${RED}❌ PostgreSQL backend is no longer supported${NC}"
    echo -e "${YELLOW}💡 Use persistent_file backend instead:${NC}"
    echo "   $0 --backend persistent_file --enable-wal --enable-recovery"
    echo ""
    exit 1
fi

# Validate WAL configuration
if [[ "$GC_ENABLE_WAL" == "true" ]] && [[ "$GC_STORAGE_BACKEND" != "persistent_file" ]]; then
    echo -e "${RED}❌ WAL can only be enabled with persistent_file backend${NC}"
    echo -e "${YELLOW}💡 Use: $0 --backend persistent_file --enable-wal${NC}"
    exit 1
fi

# Feature flag selection
FEATURES="server"
if [[ "$GC_STORAGE_BACKEND" == "persistent_file" ]]; then
    FEATURES="server,persistent"
fi

# Build the server
echo -e "${BLUE}🔨 Building GarbageTruck server (features: $FEATURES)...${NC}"
if ! cargo build --release --bin garbagetruck-server --features "$FEATURES"; then
    echo -e "${RED}❌ Build failed${NC}"
    echo -e "${YELLOW}💡 Try: cargo clean && cargo build --release --bin garbagetruck-server --features \"$FEATURES\"${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Build successful${NC}"
echo ""

# Display startup information
echo -e "${BLUE}🚀 Starting server...${NC}"
echo -e "${YELLOW}💡 Configured for experiments with:${NC}"
echo "   • Min lease duration: ${GC_MIN_LEASE_DURATION}s (allows short test leases)"
echo "   • Fast cleanup: every ${GC_CLEANUP_INTERVAL}s"
echo "   • Short grace period: ${GC_CLEANUP_GRACE_PERIOD}s"

if [[ "$GC_STORAGE_BACKEND" == "persistent_file" ]]; then
    echo -e "${PURPLE}   • Persistent storage in: $GC_DATA_DIRECTORY${NC}"
    if [[ "$GC_ENABLE_WAL" == "true" ]]; then
        echo -e "${PURPLE}   • WAL enabled with $GC_WAL_SYNC_POLICY sync policy${NC}"
    fi
    if [[ "$GC_ENABLE_AUTO_RECOVERY" == "true" ]]; then
        echo -e "${PURPLE}   • Auto-recovery enabled ($GC_RECOVERY_STRATEGY strategy)${NC}"
    fi
fi

echo ""
echo -e "${CYAN}📊 Monitoring:${NC}"
echo "   • Metrics: http://localhost:$GC_METRICS_PORT/metrics"
echo "   • Health: http://localhost:$GC_METRICS_PORT/health"
echo ""
echo -e "${CYAN}🧪 Quick Test Commands:${NC}"
echo "   • List leases: cargo run --bin garbagetruck -- lease list"
echo "   • Create test lease: cargo run --bin garbagetruck -- lease create --object-id test-$(date +%s) --duration 60"
echo "   • Health check: cargo run --bin garbagetruck -- health"
echo ""

if [[ "$GC_STORAGE_BACKEND" == "persistent_file" ]]; then
    echo -e "${CYAN}💾 Persistent Storage Commands:${NC}"
    echo "   • Check WAL integrity: ls -la $GC_DATA_DIRECTORY"
    echo "   • View WAL entries: tail -f $GC_WAL_PATH"
    echo "   • Manual recovery: cargo run --bin garbagetruck-recovery --features persistent"
    echo ""
fi

echo -e "${YELLOW}💡 Use Ctrl+C to stop the server${NC}"
echo ""

# Set up cleanup on exit
cleanup() {
    echo ""
    echo -e "${YELLOW}🛑 Shutting down server...${NC}"
    if [[ "$GC_STORAGE_BACKEND" == "persistent_file" ]] && [[ "$GC_ENABLE_WAL" == "true" ]]; then
        echo -e "${PURPLE}💾 WAL will be synced on shutdown${NC}"
    fi
    exit 0
}
trap cleanup SIGINT SIGTERM

# Run the server
cargo run --release --bin garbagetruck-server --features "$FEATURES"