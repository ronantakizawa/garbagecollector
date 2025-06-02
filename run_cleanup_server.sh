#!/bin/bash

# run_cleanup_server.sh - Start the standalone cleanup server

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${BLUE}🧹 Starting GarbageTruck Cleanup Server${NC}"
echo ""

# Default configuration
PORT=${CLEANUP_PORT:-8080}
BASE_DIR=${CLEANUP_BASE_DIR:-./temp_experiment}
COST_PER_GB=${CLEANUP_COST_PER_GB:-0.023}
VERBOSE=${CLEANUP_VERBOSE:-false}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case $1 in
        -p|--port)
            PORT="$2"
            shift 2
            ;;
        -d|--directory)
            BASE_DIR="$2"
            shift 2
            ;;
        -c|--cost)
            COST_PER_GB="$2"
            shift 2
            ;;
        -v|--verbose)
            VERBOSE=true
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  -p, --port PORT          Port to listen on (default: 8080)"
            echo "  -d, --directory DIR      Base directory for cleanup (default: ./temp_experiment)"
            echo "  -c, --cost COST          Cost per GB/month for tracking (default: 0.023)"
            echo "  -v, --verbose            Enable verbose logging"
            echo "  -h, --help               Show this help message"
            echo ""
            echo "Environment variables:"
            echo "  CLEANUP_PORT             Same as --port"
            echo "  CLEANUP_BASE_DIR         Same as --directory"
            echo "  CLEANUP_COST_PER_GB      Same as --cost"
            echo "  CLEANUP_VERBOSE          Same as --verbose"
            echo ""
            echo "Examples:"
            echo "  $0                                    # Start with defaults"
            echo "  $0 -p 9000 -d /tmp/cleanup           # Custom port and directory"
            echo "  $0 --verbose                          # Enable debug logging"
            echo "  CLEANUP_PORT=9000 $0                  # Using environment variable"
            exit 0
            ;;
        *)
            echo -e "${RED}❌ Unknown option: $1${NC}"
            echo "Use -h or --help for usage information"
            exit 1
            ;;
    esac
done

echo -e "${GREEN}📋 Configuration:${NC}"
echo "  Port: $PORT"
echo "  Base directory: $BASE_DIR"
echo "  Cost tracking: \$${COST_PER_GB}/GB/month"
echo "  Verbose logging: $VERBOSE"
echo ""

# Check if port is available
if lsof -i :$PORT >/dev/null 2>&1; then
    echo -e "${RED}❌ Port $PORT is already in use${NC}"
    echo -e "${YELLOW}💡 Try a different port with: $0 --port 9000${NC}"
    exit 1
fi

# Create base directory if it doesn't exist
if [ ! -d "$BASE_DIR" ]; then
    mkdir -p "$BASE_DIR"
    echo -e "${GREEN}📁 Created base directory: $BASE_DIR${NC}"
fi

# Build the cleanup server
echo -e "${BLUE}🔨 Building cleanup server...${NC}"
if ! cargo build --release --bin garbagetruck-cleanup-server --features client; then
    echo -e "${RED}❌ Build failed${NC}"
    exit 1
fi

echo -e "${GREEN}✅ Build successful${NC}"
echo ""

# Prepare arguments
ARGS="--port $PORT --base-directory $BASE_DIR --cost-per-gb-month $COST_PER_GB"
if [ "$VERBOSE" = true ]; then
    ARGS="$ARGS --verbose"
fi

echo -e "${BLUE}🚀 Starting cleanup server...${NC}"
echo -e "${YELLOW}💡 Test endpoints:${NC}"
echo "  Health: curl http://localhost:$PORT/health"
echo "  Stats:  curl http://localhost:$PORT/stats"
echo "  Cleanup: curl -X POST http://localhost:$PORT/cleanup -H 'Content-Type: application/json' -d '{\"file_path\": \"$BASE_DIR/test.txt\"}'"
echo ""
echo -e "${YELLOW}🛑 Press Ctrl+C to stop the server${NC}"
echo ""

# Start the server
exec cargo run --release --bin garbagetruck-cleanup-server --features client -- $ARGS