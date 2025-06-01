#!/bin/bash
# scripts/demo-cli.sh - Demonstration script for GarbageTruck CLI

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Configuration
ENDPOINT="http://localhost:50051"
SERVICE_ID="demo-service"

echo -e "${BLUE}🚛 GarbageTruck CLI Demo${NC}"
echo "=================================="
echo ""

# Function to run CLI command with nice output
run_cli() {
    echo -e "${YELLOW}$ garbagetruck $@${NC}"
    cargo run --bin garbagetruck --features client -- \
        --endpoint "$ENDPOINT" \
        --service-id "$SERVICE_ID" \
        "$@"
    echo ""
}

# Function to check if GarbageTruck service is running
check_service() {
    echo "🔍 Checking if GarbageTruck service is available..."
    if cargo run --bin garbagetruck --features client -- \
        --endpoint "$ENDPOINT" health > /dev/null 2>&1; then
        echo -e "${GREEN}✅ Service is running${NC}"
        echo ""
    else
        echo -e "${RED}❌ GarbageTruck service not running at $ENDPOINT${NC}"
        echo "Please start the service first:"
        echo "  cargo run --bin garbagetruck-server"
        echo "Or:"
        echo "  make run-server"
        exit 1
    fi
}

check_service

echo -e "${BLUE}=== Health Check ===${NC}"
run_cli health

echo -e "${BLUE}=== Service Status ===${NC}"
run_cli status

echo -e "${BLUE}=== Creating Leases ===${NC}"

echo "Creating a WebSocket session lease..."
WEBSOCKET_OUTPUT=$(run_cli lease create \
    --object-id "session-demo-123" \
    --object-type websocket-session \
    --duration 1800 \
    --metadata user_id=demo_user \
    --metadata connection_type=websocket)
WEBSOCKET_LEASE=$(echo "$WEBSOCKET_OUTPUT" | grep "Lease ID:" | cut -d: -f2 | xargs)

echo "Creating a temporary file lease..."
TEMP_FILE_OUTPUT=$(run_cli lease create \
    --object-id "/tmp/demo-upload.jpg" \
    --object-type temporary-file \
    --duration 3600 \
    --metadata file_size=2048576 \
    --metadata mime_type=image/jpeg)
TEMP_FILE_LEASE=$(echo "$TEMP_FILE_OUTPUT" | grep "Lease ID:" | cut -d: -f2 | xargs)

echo "Creating a database row lease..."
DB_OUTPUT=$(run_cli lease create \
    --object-id "temp_uploads:row_456" \
    --object-type database-row \
    --duration 600 \
    --metadata table=temp_uploads \
    --metadata row_id=456)
DB_LEASE=$(echo "$DB_OUTPUT" | grep "Lease ID:" | cut -d: -f2 | xargs)

echo "Creating a cache entry lease..."
CACHE_OUTPUT=$(run_cli lease create \
    --object-id "user_profile_cache_demo_user" \
    --object-type cache-entry \
    --duration 1200 \
    --metadata cache_key=user_profile \
    --metadata user_id=demo_user)
CACHE_LEASE=$(echo "$CACHE_OUTPUT" | grep "Lease ID:" | cut -d: -f2 | xargs)

echo -e "${BLUE}=== Listing Leases ===${NC}"
run_cli lease list --limit 20

echo -e "${BLUE}=== Getting Lease Details ===${NC}"
echo "Getting details for WebSocket session lease..."
run_cli lease get "$WEBSOCKET_LEASE"

echo -e "${BLUE}=== Renewing Leases ===${NC}"
echo "Renewing WebSocket session lease by 30 minutes..."
run_cli lease renew "$WEBSOCKET_LEASE" --extend 1800

echo "Renewing temporary file lease by 1 hour..."
run_cli lease renew "$TEMP_FILE_LEASE" --extend 3600

echo -e "${BLUE}=== Updated Status ===${NC}"
run_cli status

echo -e "${BLUE}=== Filtering Leases ===${NC}"
echo "Listing only WebSocket session leases..."
run_cli lease list --object-type websocket-session

echo "Listing leases for our demo service..."
run_cli lease list --service "$SERVICE_ID"

echo -e "${BLUE}=== Releasing Leases ===${NC}"
echo "Releasing database row lease (shortest duration)..."
run_cli lease release "$DB_LEASE"

echo "Releasing cache entry lease..."
run_cli lease release "$CACHE_LEASE"

echo -e "${BLUE}=== Final Status ===${NC}"
run_cli status

echo -e "${BLUE}=== Cleanup Remaining Leases ===${NC}"
echo "Releasing remaining leases..."
run_cli lease release "$WEBSOCKET_LEASE"
run_cli lease release "$TEMP_FILE_LEASE"

echo -e "${GREEN}✅ Demo completed successfully!${NC}"
echo ""
echo -e "${BLUE}Summary of CLI capabilities demonstrated:${NC}"
echo "• Health checks and service status"
echo "• Creating leases with different object types"
echo "• Adding metadata and cleanup configuration"
echo "• Listing and filtering leases"
echo "• Getting detailed lease information"
echo "• Renewing lease durations"
echo "• Releasing leases for cleanup"
echo ""
echo -e "${YELLOW}💡 Try running commands manually:${NC}"
echo "  garbagetruck --help"
echo "  garbagetruck lease --help"
echo "  garbagetruck lease create --help"