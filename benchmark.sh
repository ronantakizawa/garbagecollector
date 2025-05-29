#!/bin/bash

echo "🚀 GC Sidecar Performance Benchmark"
echo "===================================="

# Test parameters
CONCURRENT_CLIENTS=10
OPERATIONS_PER_CLIENT=100
TOTAL_OPERATIONS=$((CONCURRENT_CLIENTS * OPERATIONS_PER_CLIENT))

echo "📊 Test Configuration:"
echo "- Concurrent clients: $CONCURRENT_CLIENTS"
echo "- Operations per client: $OPERATIONS_PER_CLIENT"
echo "- Total operations: $TOTAL_OPERATIONS"
echo ""

# Create lease creation benchmark
echo "⏱️ Benchmarking lease creation..."
START_TIME=$(date +%s.%N)

for client in $(seq 1 $CONCURRENT_CLIENTS); do
  {
    for op in $(seq 1 $OPERATIONS_PER_CLIENT); do
      grpcurl -plaintext -import-path proto -proto gc_service.proto -d "{
        \"object_id\": \"bench-${client}-${op}\",
        \"object_type\": \"DATABASE_ROW\",
        \"service_id\": \"benchmark-service-${client}\",
        \"lease_duration_seconds\": 300
      }" localhost:50051 distributed_gc.DistributedGCService/CreateLease > /dev/null 2>&1
    done
  } &
done

wait

END_TIME=$(date +%s.%N)
DURATION=$(echo "$END_TIME - $START_TIME" | bc -l)
OPS_PER_SEC=$(echo "scale=2; $TOTAL_OPERATIONS / $DURATION" | bc -l)

echo "✅ Lease Creation Results:"
echo "- Total time: ${DURATION}s"
echo "- Operations/sec: $OPS_PER_SEC"
echo "- Average latency: $(echo "scale=3; $DURATION / $TOTAL_OPERATIONS * 1000" | bc -l)ms"
echo ""

# List leases benchmark
echo "⏱️ Benchmarking lease listing..."
START_TIME=$(date +%s.%N)

for i in $(seq 1 50); do
  grpcurl -plaintext -import-path proto -proto gc_service.proto -d '{"limit": 100}' \
    localhost:50051 distributed_gc.DistributedGCService/ListLeases > /dev/null 2>&1 &
done

wait

END_TIME=$(date +%s.%N)
LIST_DURATION=$(echo "$END_TIME - $START_TIME" | bc -l)
LIST_OPS_PER_SEC=$(echo "scale=2; 50 / $LIST_DURATION" | bc -l)

echo "✅ Lease Listing Results:"
echo "- Total time: ${LIST_DURATION}s"
echo "- Operations/sec: $LIST_OPS_PER_SEC"
echo ""

# Memory usage check
echo "📊 Memory Usage:"
ps aux | grep gc-sidecar | grep -v grep | awk '{print "- Process: " $2 ", Memory: " $4 "%, RSS: " $6 "KB"}'
echo ""

# Get final metrics
echo "📈 Final Metrics:"
grpcurl -plaintext -import-path proto -proto gc_service.proto \
  localhost:50051 distributed_gc.DistributedGCService/GetMetrics 2>/dev/null | \
  grep -E "(totalLeasesCreated|activeLeases)" | head -5

echo ""
echo "🎯 Benchmark completed!"