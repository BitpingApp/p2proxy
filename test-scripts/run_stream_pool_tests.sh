#!/bin/bash

# Stream Pool Test Suite Runner
# This script runs all automated tests for the stream pool implementation
# Can be executed by an autonomous agent for validation

set -euo pipefail

# Configuration
export PROXY_HOST="${PROXY_HOST:-127.0.0.1}"
export PROXY_PORT="${PROXY_PORT:-1080}"
export METRICS_URL="${METRICS_URL:-http://localhost:9091/metrics}"
export TEST_OUTPUT_DIR="${TEST_OUTPUT_DIR:-./test-results}"
export PROXY_BINARY="${PROXY_BINARY:-cargo run --bin p2proxy}"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Test counter
TOTAL_TESTS=0
PASSED_TESTS=0
FAILED_TESTS=0
SKIPPED_TESTS=0

# Logging functions
log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_test_pass() {
    echo -e "${GREEN}✓ PASS:${NC} $1"
    PASSED_TESTS=$((PASSED_TESTS + 1))
}

log_test_fail() {
    echo -e "${RED}✗ FAIL:${NC} $1"
    FAILED_TESTS=$((FAILED_TESTS + 1))
}

log_test_skip() {
    echo -e "${YELLOW}⊘ SKIP:${NC} $1"
    SKIPPED_TESTS=$((SKIPPED_TESTS + 1))
}

# Utility functions
wait_for_proxy() {
    local timeout=30
    local elapsed=0
    log_info "Waiting for proxy to be ready..."

    while [ $elapsed -lt $timeout ]; do
        if curl -s --socks5 $PROXY_HOST:$PROXY_PORT http://example.com > /dev/null 2>&1; then
            log_info "Proxy is ready"
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done

    log_error "Proxy failed to start within ${timeout}s"
    return 1
}

cleanup_proxy() {
    if [ ! -z "${PROXY_PID:-}" ]; then
        log_info "Stopping proxy (PID: $PROXY_PID)"
        kill $PROXY_PID 2>/dev/null || true
        wait $PROXY_PID 2>/dev/null || true
        sleep 2
    fi
}

get_metric_value() {
    local metric_name=$1
    local default_value=${2:-0}
    curl -s $METRICS_URL 2>/dev/null | grep "^${metric_name}" | awk '{print $2}' | head -1 || echo "$default_value"
}

# Test functions
test_1_simple_load() {
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    echo ""
    echo "==================================="
    echo "Test 1: Simple Page Load"
    echo "==================================="

    # Start proxy
    log_info "Starting proxy for Test 1..."
    $PROXY_BINARY > $TEST_OUTPUT_DIR/test1_proxy.log 2>&1 &
    PROXY_PID=$!

    if ! wait_for_proxy; then
        log_test_fail "Test 1: Proxy failed to start"
        echo "TEST 1: FAIL (proxy start)" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 1
    fi

    # Test with curl
    log_info "Making test request..."
    HTTP_CODE=$(curl -x socks5://$PROXY_HOST:$PROXY_PORT http://example.com \
                     -w "%{http_code}" -o /dev/null -s --max-time 10 2>/dev/null || echo "000")

    # Check result
    if [ "$HTTP_CODE" == "200" ]; then
        log_test_pass "Test 1: Simple page loaded successfully (HTTP $HTTP_CODE)"
        echo "TEST 1: PASS" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 0
    else
        log_test_fail "Test 1: Expected HTTP 200, got $HTTP_CODE"
        echo "TEST 1: FAIL (HTTP $HTTP_CODE)" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 1
    fi
}

test_2_concurrent_connections() {
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    echo ""
    echo "==================================="
    echo "Test 2: Concurrent Connections"
    echo "==================================="

    # Start proxy
    log_info "Starting proxy for Test 2..."
    $PROXY_BINARY > $TEST_OUTPUT_DIR/test2_proxy.log 2>&1 &
    PROXY_PID=$!

    if ! wait_for_proxy; then
        log_test_fail "Test 2: Proxy failed to start"
        echo "TEST 2: FAIL (proxy start)" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 1
    fi

    # Capture initial metrics
    INITIAL_OPENED=$(get_metric_value "p2proxy_stream_opened_total" 0)
    log_info "Initial streams opened: $INITIAL_OPENED"

    # Launch 50 concurrent requests
    log_info "Launching 50 concurrent requests..."
    for i in {1..50}; do
        curl -x socks5://$PROXY_HOST:$PROXY_PORT http://example.com \
             -o /dev/null -s --max-time 15 &
    done

    # Monitor active streams
    log_info "Monitoring concurrent streams..."
    MAX_CONCURRENT=0
    for j in {1..100}; do
        ACTIVE=$(get_metric_value "p2proxy_stream_pool_active_total" 0)
        if (( $(echo "$ACTIVE > $MAX_CONCURRENT" | bc -l 2>/dev/null || echo "0") )); then
            MAX_CONCURRENT=$ACTIVE
        fi
        sleep 0.1
    done

    # Wait for all requests to complete
    log_info "Waiting for all requests to complete..."
    wait
    sleep 2

    # Check final metrics
    FINAL_OPENED=$(get_metric_value "p2proxy_stream_opened_total" 0)
    TOTAL_OPENED=$((FINAL_OPENED - INITIAL_OPENED))

    log_info "Max concurrent streams observed: $MAX_CONCURRENT"
    log_info "Total streams opened: $TOTAL_OPENED"

    # Validate (allow some margin for retries/failures)
    if (( $(echo "$MAX_CONCURRENT <= 22" | bc -l 2>/dev/null || echo "0") )) && [ $TOTAL_OPENED -ge 45 ]; then
        log_test_pass "Test 2: Rate limiting working (Max: $MAX_CONCURRENT, Total: $TOTAL_OPENED)"
        echo "TEST 2: PASS (Max concurrent: $MAX_CONCURRENT, Total: $TOTAL_OPENED)" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 0
    else
        log_test_fail "Test 2: Rate limiting issue (Max: $MAX_CONCURRENT, Total: $TOTAL_OPENED)"
        echo "TEST 2: FAIL (Max: $MAX_CONCURRENT, Total: $TOTAL_OPENED)" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 1
    fi
}

test_3_metrics_accuracy() {
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    echo ""
    echo "==================================="
    echo "Test 3: Metrics Accuracy"
    echo "==================================="

    # Start fresh proxy
    log_info "Starting proxy for Test 3..."
    $PROXY_BINARY > $TEST_OUTPUT_DIR/test3_proxy.log 2>&1 &
    PROXY_PID=$!

    if ! wait_for_proxy; then
        log_test_fail "Test 3: Proxy failed to start"
        echo "TEST 3: FAIL (proxy start)" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 1
    fi

    # Get baseline
    INITIAL_OPENED=$(get_metric_value "p2proxy_stream_opened_total" 0)
    INITIAL_SESSIONS=$(get_metric_value "p2proxy_sessions_completed_total" 0)

    # Make exactly 10 requests
    log_info "Making 10 sequential requests..."
    for i in {1..10}; do
        curl -x socks5://$PROXY_HOST:$PROXY_PORT http://example.com \
             -o /dev/null -s --max-time 10 2>/dev/null || log_warn "Request $i may have failed"
        sleep 0.3
    done

    # Wait for metrics to settle
    sleep 2

    # Collect final metrics
    FINAL_OPENED=$(get_metric_value "p2proxy_stream_opened_total" 0)
    FINAL_SESSIONS=$(get_metric_value "p2proxy_sessions_completed_total" 0)
    ACQUIRE=$(get_metric_value "p2proxy_stream_pool_acquire_total" 0)

    OPENED_DELTA=$((FINAL_OPENED - INITIAL_OPENED))
    SESSIONS_DELTA=$((FINAL_SESSIONS - INITIAL_SESSIONS))

    log_info "Streams opened delta: $OPENED_DELTA"
    log_info "Sessions completed delta: $SESSIONS_DELTA"
    log_info "Pool acquires total: $ACQUIRE"

    # Validate (allow some tolerance for failures)
    if [ "$OPENED_DELTA" -ge 8 ] && [ "$SESSIONS_DELTA" -ge 8 ]; then
        log_test_pass "Test 3: Metrics are accurate (Opened: $OPENED_DELTA, Sessions: $SESSIONS_DELTA)"
        echo "TEST 3: PASS (Opened: $OPENED_DELTA, Sessions: $SESSIONS_DELTA)" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 0
    else
        log_test_fail "Test 3: Metrics inconsistent (Opened: $OPENED_DELTA, Sessions: $SESSIONS_DELTA)"
        echo "TEST 3: FAIL (Opened: $OPENED_DELTA, Sessions: $SESSIONS_DELTA)" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 1
    fi
}

test_4_pool_disabled() {
    TOTAL_TESTS=$((TOTAL_TESTS + 1))
    echo ""
    echo "==================================="
    echo "Test 4: Pool Disabled Fallback"
    echo "==================================="

    # Backup and modify config
    if [ ! -f Config.yaml ]; then
        log_test_skip "Test 4: Config.yaml not found"
        echo "TEST 4: SKIPPED (no config file)" >> $TEST_OUTPUT_DIR/test_report.txt
        return 0
    fi

    log_info "Backing up config and disabling pool..."
    cp Config.yaml Config.yaml.test4.backup
    sed -i.bak 's/enabled: true/enabled: false/' Config.yaml 2>/dev/null || \
        sed -i '' 's/enabled: true/enabled: false/' Config.yaml 2>/dev/null || {
            log_test_skip "Test 4: Could not modify config"
            mv Config.yaml.test4.backup Config.yaml 2>/dev/null
            echo "TEST 4: SKIPPED (config modify failed)" >> $TEST_OUTPUT_DIR/test_report.txt
            return 0
        }

    # Start proxy
    log_info "Starting proxy with pool disabled..."
    $PROXY_BINARY > $TEST_OUTPUT_DIR/test4_proxy.log 2>&1 &
    PROXY_PID=$!

    if ! wait_for_proxy; then
        log_test_fail "Test 4: Proxy failed to start with pool disabled"
        echo "TEST 4: FAIL (proxy start)" >> $TEST_OUTPUT_DIR/test_report.txt
        mv Config.yaml.test4.backup Config.yaml
        cleanup_proxy
        return 1
    fi

    # Test basic request
    HTTP_CODE=$(curl -x socks5://$PROXY_HOST:$PROXY_PORT http://example.com \
                     -w "%{http_code}" -o /dev/null -s --max-time 10 2>/dev/null || echo "000")

    # Restore config
    mv Config.yaml.test4.backup Config.yaml

    # Check result
    if [ "$HTTP_CODE" == "200" ]; then
        log_test_pass "Test 4: Proxy works with pool disabled (HTTP $HTTP_CODE)"
        echo "TEST 4: PASS" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 0
    else
        log_test_fail "Test 4: Proxy failed with pool disabled (HTTP $HTTP_CODE)"
        echo "TEST 4: FAIL (HTTP $HTTP_CODE)" >> $TEST_OUTPUT_DIR/test_report.txt
        cleanup_proxy
        return 1
    fi
}

# Main execution
main() {
    echo "========================================"
    echo "Stream Pool Test Suite"
    echo "========================================"
    echo "Date: $(date)"
    echo ""

    # Create output directory
    mkdir -p $TEST_OUTPUT_DIR
    echo "STREAM POOL TEST RESULTS" > $TEST_OUTPUT_DIR/test_report.txt
    echo "Date: $(date)" >> $TEST_OUTPUT_DIR/test_report.txt
    echo "Host: $PROXY_HOST:$PROXY_PORT" >> $TEST_OUTPUT_DIR/test_report.txt
    echo "" >> $TEST_OUTPUT_DIR/test_report.txt

    # Check prerequisites
    log_info "Checking prerequisites..."
    command -v curl >/dev/null 2>&1 || { log_error "curl is required but not installed"; exit 1; }
    command -v bc >/dev/null 2>&1 || log_warn "bc is not installed, some tests may be skipped"

    # Run tests
    test_1_simple_load || true
    test_2_concurrent_connections || true
    test_3_metrics_accuracy || true
    test_4_pool_disabled || true

    # Summary
    echo ""
    echo "========================================"
    echo "TEST SUMMARY"
    echo "========================================"
    cat $TEST_OUTPUT_DIR/test_report.txt

    echo ""
    echo "Results:"
    echo "  Total:   $TOTAL_TESTS"
    echo "  Passed:  ${GREEN}$PASSED_TESTS${NC}"
    echo "  Failed:  ${RED}$FAILED_TESTS${NC}"
    echo "  Skipped: ${YELLOW}$SKIPPED_TESTS${NC}"

    # Calculate percentage
    if [ $TOTAL_TESTS -gt 0 ]; then
        SUCCESS_RATE=$(( (PASSED_TESTS * 100) / TOTAL_TESTS ))
        echo "  Success: ${SUCCESS_RATE}%"
    fi

    echo ""
    echo "Detailed logs saved to: $TEST_OUTPUT_DIR/"

    # Exit with appropriate code
    if [ $FAILED_TESTS -gt 0 ]; then
        echo ""
        echo "${RED}❌ TEST SUITE FAILED${NC}"
        exit 1
    else
        echo ""
        echo "${GREEN}✅ TEST SUITE PASSED${NC}"
        exit 0
    fi
}

# Trap to ensure cleanup on exit
trap cleanup_proxy EXIT INT TERM

# Run main
main "$@"
