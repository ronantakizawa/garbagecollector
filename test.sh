#!/bin/bash

echo "🧪 Testing Distributed GC Sidecar"

# Check if service is running
if ! curl -s --connect-timeout 2 localhost:50051 > /dev/null 2>&1; then
    echo "❌ Service is not running on port 50051"
    echo "Start it with: RUST_LOG=distributed_gc_sidecar=info ./target/release/gc-sidecar"
    exit 1
fi

echo "✅ Service is running on port 50051"

# Test with grpcurl using proto file
echo ""
echo "🔍 Testing Health Check..."
grpcurl -plaintext -import-path proto -proto gc_service.proto localhost:50051 distributed_gc.DistributedGCService/HealthCheck

echo ""
echo "📝 Creating a lease..."
LEASE_RESPONSE=$(grpcurl -plaintext -import-path proto -proto gc_service.proto -d '{
  "object_id": "session-12345",
  "object_type": "WEBSOCKET_SESSION",
  "service_id": "web-service",
  "lease_duration_seconds": 60,
  "metadata": {"user_id": "user123"}
}' localhost:50051 distributed_gc.DistributedGCService/CreateLease)

echo "$LEASE_RESPONSE"

# Extract lease_id for further testing
LEASE_ID=$(echo "$LEASE_RESPONSE" | grep -o '"lease_id": *"[^"]*"' | sed 's/"lease_id": *"\([^"]*\)"/\1/')

if [ ! -z "$LEASE_ID" ]; then
    echo ""
    echo "🔄 Renewing lease: $LEASE_ID"
    grpcurl -plaintext -import-path proto -proto gc_service.proto -d "{
      \"lease_id\": \"$LEASE_ID\",
      \"service_id\": \"web-service\",
      \"extend_duration_seconds\": 120
    }" localhost:50051 distributed_gc.DistributedGCService/RenewLease
    
    echo ""
    echo "🔍 Getting lease details..."
    grpcurl -plaintext -import-path proto -proto gc_service.proto -d "{
      \"lease_id\": \"$LEASE_ID\"
    }" localhost:50051 distributed_gc.DistributedGCService/GetLease
fi

echo ""
echo "📊 Listing all leases..."
grpcurl -plaintext -import-path proto -proto gc_service.proto -d '{"limit": 10}' localhost:50051 distributed_gc.DistributedGCService/ListLeases

echo ""
echo "📈 Getting metrics..."
grpcurl -plaintext -import-path proto -proto gc_service.proto localhost:50051 distributed_gc.DistributedGCService/GetMetrics

echo ""
echo "✅ All tests completed!"