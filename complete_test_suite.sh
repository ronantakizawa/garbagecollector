#!/bin/bash

# Complete Test Suite for Distributed GC Sidecar
# Combines all tests run during development and validation
# Tests: Basic functionality, comprehensive features, load testing, and real-world scenarios

set -e  # Exit on any error

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
PURPLE='\033[0;35m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Test configuration
SERVICE_HOST="localhost"
SERVICE_PORT="50051"
CLEANUP_PORT="8080"
PROTO_PATH="proto"
PROTO_FILE="gc_service.proto"

# Test counters
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0

# Performance tracking
declare -A PERFORMANCE_METRICS

# Helper functions
log_info() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

log_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
    ((PASSED_TESTS++))
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
    ((FAILED_TESTS++))
}

log_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

log_test() {
    echo -e "${PURPLE}[TEST]${NC} $1"
    ((TOTAL_TESTS++))
}

log_section() {
    echo ""
    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN} $1${NC}"
    echo -e "${CYAN}========================================${NC}"
}

log_subsection() {
    echo ""
    echo -e "${YELLOW}--- $1 ---${NC}"
}

# Check prerequisites
check_prerequisites() {
    log_section "Checking Prerequisites"
    
    # Check if grpcurl is installed
    if ! command -v grpcurl &> /dev/null; then
        log_error "grpcurl is not installed. Install with: brew install grpcurl"
        exit 1
    fi
    log_success "grpcurl is installed"
    
    # Check if bc is installed (for calculations)
    if ! command -v bc &> /dev/null; then
        log_error "bc is not installed. Install with: brew install bc"
        exit 1
    fi
    log_success "bc is installed for calculations"
    
    # Check if proto file exists
    if [ ! -f "$PROTO_PATH/$PROTO_FILE" ]; then
        log_error "Proto file not found at $PROTO_PATH/$PROTO_FILE"
        exit 1
    fi
    log_success "Proto file found"
    
    # Check if GC sidecar binary exists
    if [ ! -f "./target/release/gc-sidecar" ]; then
        log_error "GC sidecar binary not found. Build with: cargo build --release"
        exit 1
    fi
    log_success "GC sidecar binary found"
    
    # Test gRPC service connection
    log_info "Testing gRPC service connection..."
    if grpcurl -plaintext -import-path "$PROTO_PATH" -proto "$PROTO_FILE" "$SERVICE_HOST:$SERVICE_PORT" distributed_gc.DistributedGCService/HealthCheck > /dev/null 2>&1; then
        log_success "GC Sidecar service is running and responding"
    else
        log_error "GC Sidecar service is not responding on $SERVICE_HOST:$SERVICE_PORT"
        log_error "Make sure the service is running with: RUST_LOG=distributed_gc_sidecar=info ./target/release/gc-sidecar"
        exit 1
    fi
}

# Start cleanup server for testing
start_cleanup_server() {
    log_section "Starting Test Infrastructure"
    
    # Kill any existing cleanup server
    pkill -f "python.*cleanup.*8080" 2>/dev/null || true
    
    log_info "Starting test cleanup server on port $CLEANUP_PORT..."
    
    # Start cleanup server in background
    python3 -c "
import json
import threading
from http.server import HTTPServer, BaseHTTPRequestHandler
from datetime import datetime

cleanup_requests = []

class CleanupHandler(BaseHTTPRequestHandler):
    def log_message(self, format, *args):
        pass  # Suppress default logging
    
    def do_POST(self):
        try:
            content_length = int(self.headers['Content-Length'])
            post_data = self.rfile.read(content_length)
            cleanup_request = json.loads(post_data)
            
            # Log cleanup request
            timestamp = datetime.now().isoformat()
            print(f'🧹 [{timestamp}] Cleanup request: {cleanup_request}')
            cleanup_requests.append({'timestamp': timestamp, 'request': cleanup_request})
            
            # Always respond successfully
            self.send_response(200)
            self.send_header('Content-type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps({
                'success': True, 
                'message': 'Cleanup completed',
                'timestamp': timestamp
            }).encode())
        except Exception as e:
            self.send_response(500)
            self.send_header('Content-type', 'application/json')
            self.end_headers()
            self.wfile.write(json.dumps({'success': False, 'error': str(e)}).encode())
    
    def do_GET(self):
        # Health check endpoint
        self.send_response(200)
        self.send_header('Content-type', 'application/json')
        self.end_headers()
        self.wfile.write(json.dumps({
            'status': 'running',
            'cleanup_requests_received': len(cleanup_requests),
            'requests': cleanup_requests
        }).encode())

def run_server():
    httpd = HTTPServer(('localhost', $CLEANUP_PORT), CleanupHandler)
    print(f'🚀 Test cleanup server running on port $CLEANUP_PORT')
    httpd.serve_forever()

# Run in daemon thread
server_thread = threading.Thread(target=run_server, daemon=True)
server_thread.start()

# Keep the script running
try:
    server_thread.join()
except KeyboardInterrupt:
    print('\\n🛑 Cleanup server stopped')
" &
    
    CLEANUP_PID=$!
    sleep 2  # Give server time to start
    
    # Verify cleanup server is running
    if curl -s "http://localhost:$CLEANUP_PORT" > /dev/null 2>&1; then
        log_success "Test cleanup server started on port $CLEANUP_PORT (PID: $CLEANUP_PID)"
    else
        log_error "Failed to start test cleanup server"
        exit 1
    fi
}

# Stop cleanup server
stop_cleanup_server() {
    log_info "Stopping test cleanup server"
    pkill -f "python.*cleanup.*8080" 2>/dev/null || true
}

# Utility function to make gRPC calls
grpc_call() {
    local method="$1"
    local data="$2"
    
    if [ -n "$data" ]; then
        grpcurl -plaintext -import-path "$PROTO_PATH" -proto "$PROTO_FILE" -d "$data" \
            "$SERVICE_HOST:$SERVICE_PORT" "distributed_gc.DistributedGCService/$method" 2>/dev/null
    else
        grpcurl -plaintext -import-path "$PROTO_PATH" -proto "$PROTO_FILE" \
            "$SERVICE_HOST:$SERVICE_PORT" "distributed_gc.DistributedGCService/$method" 2>/dev/null
    fi
}

# Extract value from JSON response
extract_json_value() {
    local json="$1"
    local key="$2"
    echo "$json" | grep -o "\"$key\": *\"[^\"]*\"" | sed "s/\"$key\": *\"\([^\"]*\)\"/\1/"
}

# Track performance metrics
track_performance() {
    local test_name="$1"
    local start_time="$2"
    local end_time="$3"
    local operations="$4"
    
    local duration=$(echo "$end_time - $start_time" | bc -l)
    local ops_per_sec=$(echo "scale=2; $operations / $duration" | bc -l)
    
    PERFORMANCE_METRICS["${test_name}_duration"]="$duration"
    PERFORMANCE_METRICS["${test_name}_ops_per_sec"]="$ops_per_sec"
    PERFORMANCE_METRICS["${test_name}_operations"]="$operations"
}

# =============================================================================
# BASIC FUNCTIONALITY TESTS (From simple_test.sh)
# =============================================================================

test_basic_functionality() {
    log_section "Basic Functionality Tests"
    
    log_test "Health Check"
    local health_response=$(grpc_call "HealthCheck" "")
    
    if echo "$health_response" | grep -q "\"healthy\": true"; then
        local version=$(extract_json_value "$health_response" "version")
        log_success "Health check passed (version: $version)"
        log_info "Response: $health_response"
    else
        log_error "Health check failed"
        log_error "Response: $health_response"
    fi
    
    log_test "Basic Lease Creation"
    local create_response=$(grpc_call "CreateLease" '{
        "object_id": "basic-test-lease",
        "object_type": "DATABASE_ROW",
        "service_id": "basic-test-service",
        "lease_duration_seconds": 60,
        "metadata": {"test": "basic", "suite": "complete"}
    }')
    
    local lease_id=$(extract_json_value "$create_response" "leaseId")
    local success=$(echo "$create_response" | grep -o "\"success\": *[^,}]*" | sed 's/"success": *//')
    
    if [ "$success" = "true" ] && [ -n "$lease_id" ]; then
        log_success "Basic lease created successfully (ID: $lease_id)"
        echo "$lease_id" > /tmp/basic_lease_id
        echo "$create_response"
    else
        log_error "Failed to create basic lease"
        log_error "Response: $create_response"
    fi
}

# =============================================================================
# COMPREHENSIVE FUNCTIONALITY TESTS
# =============================================================================

test_comprehensive_functionality() {
    log_section "Comprehensive Functionality Tests"
    
    # Test different object types
    log_subsection "Object Type Testing"
    log_test "Different Object Types"
    
    local object_types=("DATABASE_ROW" "BLOB_STORAGE" "TEMPORARY_FILE" "WEBSOCKET_SESSION" "CACHE_ENTRY" "CUSTOM")
    local type_test_results=0
    
    for obj_type in "${object_types[@]}"; do
        local response=$(grpc_call "CreateLease" "{
            \"object_id\": \"test-${obj_type,,}-$(date +%s)\",
            \"object_type\": \"$obj_type\",
            \"service_id\": \"type-test-service\",
            \"lease_duration_seconds\": 120
        }")
        
        local success=$(echo "$response" | grep -o "\"success\": *[^,}]*" | sed 's/"success": *//')
        
        if [ "$success" = "true" ]; then
            log_success "Created lease for object type: $obj_type"
            ((type_test_results++))
        else
            log_error "Failed to create lease for object type: $obj_type"
        fi
    done
    
    log_info "Object type test results: $type_test_results/${#object_types[@]} successful"
    
    # Test lease with cleanup configuration
    log_subsection "Cleanup Configuration Testing"
    log_test "Lease with Cleanup Configuration"
    
    local cleanup_response=$(grpc_call "CreateLease" "{
        \"object_id\": \"cleanup-test-$(date +%s)\",
        \"object_type\": \"TEMPORARY_FILE\",
        \"service_id\": \"cleanup-test-service\",
        \"lease_duration_seconds\": 15,
        \"cleanup_config\": {
            \"cleanup_http_endpoint\": \"http://localhost:$CLEANUP_PORT/cleanup\",
            \"max_retries\": 3,
            \"retry_delay_seconds\": 2,
            \"cleanup_payload\": \"{\\\"action\\\":\\\"delete\\\",\\\"test\\\":true}\"
        }
    }")
    
    local cleanup_lease_id=$(extract_json_value "$cleanup_response" "leaseId")
    local cleanup_success=$(echo "$cleanup_response" | grep -o "\"success\": *[^,}]*" | sed 's/"success": *//')
    
    if [ "$cleanup_success" = "true" ] && [ -n "$cleanup_lease_id" ]; then
        log_success "Lease with cleanup config created (ID: $cleanup_lease_id)"
        echo "$cleanup_lease_id" > /tmp/cleanup_lease_id
        log_info "This lease will expire in 15 seconds and trigger cleanup"
    else
        log_error "Failed to create lease with cleanup config"
    fi
    
    # Test lease operations
    log_subsection "Lease Operations Testing"
    
    if [ -f "/tmp/basic_lease_id" ]; then
        local lease_id=$(cat /tmp/basic_lease_id)
        
        log_test "Get Lease Details"
        local get_response=$(grpc_call "GetLease" "{\"lease_id\": \"$lease_id\"}")
        
        if echo "$get_response" | grep -q "\"found\": true"; then
            log_success "Successfully retrieved lease details"
        else
            log_error "Failed to retrieve lease details"
        fi
        
        log_test "Lease Renewal"
        local renew_response=$(grpc_call "RenewLease" "{
            \"lease_id\": \"$lease_id\",
            \"service_id\": \"basic-test-service\",
            \"extend_duration_seconds\": 300
        }")
        
        local renew_success=$(echo "$renew_response" | grep -o "\"success\": *[^,}]*" | sed 's/"success": *//')
        
        if [ "$renew_success" = "true" ]; then
            log_success "Lease renewed successfully"
        else
            log_error "Failed to renew lease"
        fi
    fi
    
    # Test listing leases
    log_test "List Leases"
    local list_response=$(grpc_call "ListLeases" '{"limit": 20}')
    
    if echo "$list_response" | grep -q "\"leases\""; then
        local lease_count=$(echo "$list_response" | grep -o "\"leaseId\"" | wc -l)
        log_success "Successfully listed leases (count: $lease_count)"
    else
        log_error "Failed to list leases"
    fi
    
    # Test metrics
    log_test "Metrics Collection"
    local metrics_response=$(grpc_call "GetMetrics" "")
    
    if echo "$metrics_response" | grep -q "\"totalLeasesCreated\""; then
        local created=$(echo "$metrics_response" | grep -o "\"totalLeasesCreated\": *\"[^\"]*\"" | sed 's/"totalLeasesCreated": *"\([^"]*\)"/\1/')
        log_success "Metrics retrieved successfully (Total created: $created)"
    else
        log_error "Failed to retrieve metrics"
    fi
}

# =============================================================================
# ERROR HANDLING AND EDGE CASE TESTS
# =============================================================================

test_error_handling() {
    log_section "Error Handling and Edge Cases"
    
    log_test "Invalid Lease ID Handling"
    local invalid_response=$(grpc_call "GetLease" '{"lease_id": "invalid-lease-id-12345"}')
    if echo "$invalid_response" | grep -q "\"found\": false"; then
        log_success "Correctly handled invalid lease ID"
    else
        log_error "Failed to handle invalid lease ID correctly"
    fi
    
    log_test "Unauthorized Access Prevention"
    local unauth_response=$(grpc_call "RenewLease" '{
        "lease_id": "invalid-lease-id",
        "service_id": "wrong-service",
        "extend_duration_seconds": 60
    }')
    if echo "$unauth_response" | grep -q "\"success\": false"; then
        log_success "Correctly rejected unauthorized renewal"
    else
        log_error "Failed to reject unauthorized renewal"
    fi
    
    log_test "Invalid Duration Validation"
    local invalid_duration_response=$(grpc_call "CreateLease" '{
        "object_id": "test-invalid-duration",
        "object_type": "DATABASE_ROW", 
        "service_id": "test-service",
        "lease_duration_seconds": 999999
    }')
    if echo "$invalid_duration_response" | grep -q "\"success\": false"; then
        log_success "Correctly rejected invalid lease duration"
    else
        log_error "Failed to reject invalid lease duration"
    fi
    
    log_test "Empty Field Validation"
    local empty_fields_response=$(grpc_call "CreateLease" '{
        "object_id": "",
        "object_type": "DATABASE_ROW",
        "service_id": "",
        "lease_duration_seconds": 60
    }')
    if echo "$empty_fields_response" | grep -q "\"success\": false"; then
        log_success "Correctly rejected empty required fields"
    else
        log_error "Failed to reject empty required fields"
    fi
}

# =============================================================================
# PERFORMANCE AND LOAD TESTS
# =============================================================================

test_performance() {
    log_section "Performance and Load Testing"
    
    log_test "Lease Creation Performance"
    log_info "Creating 50 leases concurrently..."
    
    local start_time=$(date +%s.%N)
    local perf_operations=50
    local perf_success=0
    
    # Create leases in parallel
    for i in $(seq 1 $perf_operations); do
        (
            grpc_call "CreateLease" "{
                \"object_id\": \"perf-test-$i-$(date +%s)\",
                \"object_type\": \"DATABASE_ROW\",
                \"service_id\": \"perf-test-service\",
                \"lease_duration_seconds\": 180
            }" > /tmp/perf_result_$i
        ) &
    done
    
    wait  # Wait for all background jobs
    local end_time=$(date +%s.%N)
    
    # Count successful operations
    for i in $(seq 1 $perf_operations); do
        if [ -f "/tmp/perf_result_$i" ] && grep -q "\"success\": true" "/tmp/perf_result_$i"; then
            ((perf_success++))
        fi
        rm -f "/tmp/perf_result_$i"
    done
    
    # Calculate performance metrics
    track_performance "lease_creation" "$start_time" "$end_time" "$perf_success"
    
    local duration=${PERFORMANCE_METRICS["lease_creation_duration"]}
    local ops_per_sec=${PERFORMANCE_METRICS["lease_creation_ops_per_sec"]}
    
    log_info "Performance Results:"
    log_info "- Successful operations: $perf_success/$perf_operations"
    log_info "- Duration: ${duration}s"
    log_info "- Operations/sec: $ops_per_sec"
    
    if [ $perf_success -eq $perf_operations ]; then
        log_success "Performance test passed"
    else
        log_error "Performance test failed ($perf_success/$perf_operations succeeded)"
    fi
    
    log_test "Concurrent Operations Test"
    log_info "Testing concurrent lease operations..."
    
    local concurrent_start=$(date +%s.%N)
    local concurrent_ops=20
    local concurrent_success=0
    
    for i in $(seq 1 $concurrent_ops); do
        (
            # Create lease
            local lease_response=$(grpc_call "CreateLease" "{
                \"object_id\": \"concurrent-$i\",
                \"object_type\": \"CACHE_ENTRY\",
                \"service_id\": \"concurrent-service\",
                \"lease_duration_seconds\": 120
            }")
            
            local lease_id=$(extract_json_value "$lease_response" "leaseId")
            
            if [ -n "$lease_id" ]; then
                # Try to renew immediately
                local renew_response=$(grpc_call "RenewLease" "{
                    \"lease_id\": \"$lease_id\",
                    \"service_id\": \"concurrent-service\",
                    \"extend_duration_seconds\": 180
                }")
                
                if echo "$renew_response" | grep -q "\"success\": true"; then
                    echo "success" > /tmp/concurrent_result_$i
                fi
            fi
        ) &
    done
    
    wait
    local concurrent_end=$(date +%s.%N)
    
    # Count successful concurrent operations
    for i in $(seq 1 $concurrent_ops); do
        if [ -f "/tmp/concurrent_result_$i" ]; then
            ((concurrent_success++))
        fi
        rm -f "/tmp/concurrent_result_$i"
    done
    
    track_performance "concurrent_ops" "$concurrent_start" "$concurrent_end" "$concurrent_success"
    
    log_info "Concurrent Operations Results:"
    log_info "- Successful operations: $concurrent_success/$concurrent_ops"
    log_info "- Duration: ${PERFORMANCE_METRICS["concurrent_ops_duration"]}s"
    log_info "- Operations/sec: ${PERFORMANCE_METRICS["concurrent_ops_ops_per_sec"]}"
    
    if [ $concurrent_success -gt $((concurrent_ops / 2)) ]; then
        log_success "Concurrent operations test passed"
    else
        log_error "Concurrent operations test failed"
    fi
}

# =============================================================================
# LEASE EXPIRATION AND CLEANUP TESTS
# =============================================================================

test_lease_expiration() {
    log_section "Lease Expiration and Cleanup Testing"
    
    log_test "Lease Expiration"
    log_info "Creating short-lived lease for expiration testing..."
    
    # Create a very short-lived lease
    local expire_response=$(grpc_call "CreateLease" '{
        "object_id": "expiration-test-lease",
        "object_type": "WEBSOCKET_SESSION",
        "service_id": "expiration-test-service",
        "lease_duration_seconds": 8
    }')
    
    local expire_lease_id=$(extract_json_value "$expire_response" "leaseId")
    
    if [ -n "$expire_lease_id" ]; then
        log_info "Created short-lived lease (ID: $expire_lease_id), waiting for expiration..."
        sleep 10  # Wait longer than lease duration
        
        # Try to renew the expired lease
        local expired_renew_response=$(grpc_call "RenewLease" "{
            \"lease_id\": \"$expire_lease_id\",
            \"service_id\": \"expiration-test-service\",
            \"extend_duration_seconds\": 60
        }")
        
        if echo "$expired_renew_response" | grep -q "already expired"; then
            log_success "Lease correctly expired and renewal was rejected"
        else
            log_error "Lease expiration not working correctly"
        fi
    else
        log_error "Could not create lease for expiration testing"
    fi
    
    log_test "Cleanup Operations"
    log_info "Checking for cleanup operations..."
    
    # Wait for cleanup operations to potentially trigger
    sleep 10
    
    # Check cleanup server for received requests
    local cleanup_status=$(curl -s "http://localhost:$CLEANUP_PORT" 2>/dev/null || echo "{}")
    
    if echo "$cleanup_status" | grep -q "cleanup_requests_received"; then
        local request_count=$(echo "$cleanup_status" | grep -o "\"cleanup_requests_received\": *[0-9]*" | sed 's/"cleanup_requests_received": *//')
        if [ "$request_count" -gt 0 ]; then
            log_success "Cleanup operations triggered ($request_count requests received)"
            log_info "Cleanup server status: $cleanup_status"
        else
            log_warning "No cleanup requests received yet (may need more time)"
        fi
    else
        log_warning "Could not check cleanup server status"
    fi
}

# =============================================================================
# INTEGRATION TESTS
# =============================================================================

test_integration_scenarios() {
    log_section "Integration and Real-World Scenarios"
    
    log_test "Multi-Service Scenario"
    log_info "Simulating multiple services using the GC sidecar..."
    
    local services=("user-service" "file-service" "cache-service" "session-service")
    local integration_success=0
    local integration_total=0
    
    for service in "${services[@]}"; do
        for obj_num in {1..3}; do
            ((integration_total++))
            local integration_response=$(grpc_call "CreateLease" "{
                \"object_id\": \"${service}-object-${obj_num}\",
                \"object_type\": \"DATABASE_ROW\",
                \"service_id\": \"$service\",
                \"lease_duration_seconds\": 240,
                \"metadata\": {\"service\": \"$service\", \"object_num\": \"$obj_num\"}
            }")
            
            if echo "$integration_response" | grep -q "\"success\": true"; then
                ((integration_success++))
            fi
        done
    done
    
    log_info "Multi-service test: $integration_success/$integration_total leases created"
    
    if [ $integration_success -eq $integration_total ]; then
        log_success "Multi-service integration test passed"
    else
        log_error "Multi-service integration test failed"
    fi
    
    log_test "Service Isolation"
    log_info "Testing service isolation (services can't access each other's leases)..."
    
    # Create a lease with one service
    local isolation_response=$(grpc_call "CreateLease" '{
        "object_id": "isolation-test-object",
        "object_type": "CACHE_ENTRY",
        "service_id": "service-a",
        "lease_duration_seconds": 300
    }')
    
    local isolation_lease_id=$(extract_json_value "$isolation_response" "leaseId")
    
    if [ -n "$isolation_lease_id" ]; then
        # Try to renew with different service
        local isolation_renew_response=$(grpc_call "RenewLease" "{
            \"lease_id\": \"$isolation_lease_id\",
            \"service_id\": \"service-b\",
            \"extend_duration_seconds\": 300
        }")
        
        if echo "$isolation_renew_response" | grep -q "\"success\": false"; then
            log_success "Service isolation working correctly"
        else
            log_error "Service isolation failed - unauthorized access allowed"
        fi
    else
        log_error "Could not create lease for isolation testing"
    fi
}

# =============================================================================
# STRESS TESTING
# =============================================================================

test_stress_scenarios() {
    log_section "Stress Testing"
    
    log_test "High Volume Lease Creation"
    log_info "Creating large number of leases rapidly..."
    
    local stress_start=$(date +%s.%N)
    local stress_operations=200
    local stress_success=0
    local batch_size=20
    
    # Create leases in batches to avoid overwhelming the system
    for batch in $(seq 1 $((stress_operations / batch_size))); do
        for i in $(seq 1 $batch_size); do
            (
                local lease_num=$(((batch - 1) * batch_size + i))
                grpc_call "CreateLease" "{
                    \"object_id\": \"stress-test-$lease_num\",
                    \"object_type\": \"DATABASE_ROW\",
                    \"service_id\": \"stress-test-service\",
                    \"lease_duration_seconds\": 60
                }" > /tmp/stress_result_$lease_num
            ) &
        done
        wait  # Wait for each batch to complete
        sleep 0.1  # Small delay between batches
    done
    
    local stress_end=$(date +%s.%N)
    
    # Count successful operations
    for i in $(seq 1 $stress_operations); do
        if [ -f "/tmp/stress_result_$i" ] && grep -q "\"success\": true" "/tmp/stress_result_$i"; then
            ((stress_success++))
        fi
        rm -f "/tmp/stress_result_$i"
    done
    
    track_performance "stress_test" "$stress_start" "$stress_end" "$stress_success"
    
    log_info "Stress Test Results:"
    log_info "- Total operations: $stress_operations"
    log_info "- Successful: $stress_success"
    log_info "- Success rate: $(echo "scale=1; $stress_success * 100 / $stress_operations" | bc -l)%"
    log_info "- Duration: ${PERFORMANCE_METRICS["stress_test_duration"]}s"
    log_info "- Operations/sec: ${PERFORMANCE_METRICS["stress_test_ops_per_sec"]}"
    
    if [ $stress_success -gt $((stress_operations * 90 / 100)) ]; then
        log_success "Stress test passed (>90% success rate)"
    else
        log_error "Stress test failed (<90% success rate)"
    fi
}

# =============================================================================
# MEMORY AND RESOURCE MONITORING
# =============================================================================

test_resource_monitoring() {
    log_section "Resource Monitoring"
    
    log_test "Memory Usage Analysis"
    
    # Get process info
    local gc_process=$(ps aux | grep gc-sidecar | grep -v grep | head -1)
    
    if [ -n "$gc_process" ]; then
        local memory_percent=$(echo "$gc_process" | awk '{print $4}')
        local memory_kb=$(echo "$gc_process" | awk '{print $6}')
        local memory_mb=$(echo "scale=2; $memory_kb / 1024" | bc -l)
        
        log_success "Memory usage monitoring:"
        log_info "- Memory percentage: ${memory_percent}%"
        log_info "- Memory usage: ${memory_mb} MB (${memory_kb} KB)"
        
        if (( $(echo "$memory_mb < 500" | bc -l) )); then
            log_success "Memory usage is within acceptable limits (<500MB)"
        else
            log_warning "Memory usage is high (>${memory_mb}MB)"
        fi
    else
        log_warning "Could not find GC sidecar process for memory monitoring"