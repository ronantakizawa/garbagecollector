#!/usr/bin/env bash

# run_default_experiment.sh – Run the DEFAULT GarbageTruck experiment only

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
PURPLE='\033[0;35m'
NC='\033[0m'

echo -e "${PURPLE}🧪 GarbageTruck DEFAULT Experiment${NC}"
echo "=============================================="
echo ""

# Check if server is running
check_server() {
    echo -e "${BLUE}🔍 Checking if GarbageTruck server is running...${NC}"
    if cargo run --bin garbagetruck --features client -- health &>/dev/null; then
        echo -e "${GREEN}✅ Server is running${NC}"
        return 0
    else
        echo -e "${RED}❌ Server is not running${NC}"
        echo -e "${YELLOW}💡 Start the server first:${NC}"
        echo "   ./run_server.sh"
        return 1
    fi
}

main() {
    # Ensure GarbageTruck gRPC server is live
    if ! check_server; then
        exit 1
    fi

    # Check for 'bc' (used in original script for arithmetic, optional here)
    if ! command -v bc &> /dev/null; then
        echo -e "${YELLOW}⚠️  'bc' not found; numeric summaries may be limited.${NC}"
    fi

    echo ""
    echo -e "${PURPLE}🚀 Running DEFAULT experiment (100MB, 200 jobs, 5% failure)${NC}"
    echo "================================================"
    EXPERIMENT_MODE=default cargo run --bin garbagetruck-experiment --features client

    sleep 120
}

# If this script is being executed (not sourced), run main
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
    main "$@"
fi
