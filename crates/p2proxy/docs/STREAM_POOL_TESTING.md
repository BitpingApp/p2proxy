# Stream Pool Testing Methodology

## Overview

This document provides a comprehensive testing methodology for the Stream Pool implementation. The Stream Pool manages P2P stream connections with rate limiting and timeout control to prevent peer overload when browsers open many concurrent connections.

**Testing Goal:** Verify that Firefox can load complex web pages through the proxy without requiring page refreshes, with >99% resource loading success rate.

---

## Pre-Test Setup

### 1. Environment Configuration

**Required:**
- p2proxy built and ready to run (`cargo build --bin p2proxy`)
- Firefox installed and configured
- Access to Prometheus metrics endpoint (port 9091)
- `jq` installed for JSON processing
- `curl` installed for HTTP testing

**Environment Variables:**
```bash
export PROXY_HOST="127.0.0.1"
export PROXY_PORT="1080"
export METRICS_URL="http://localhost:9091/metrics"
export TEST_OUTPUT_DIR="./test-results"
```

### 2. Baseline Metrics Collection

Before starting tests, collect baseline metrics:

```bash
# Create output directory
mkdir -p $TEST_OUTPUT_DIR

# Capture initial state
curl -s $METRICS_URL > $TEST_OUTPUT_DIR/baseline_metrics.txt

# Record baseline values
echo "BASELINE METRICS:" > $TEST_OUTPUT_DIR/test_report.txt
grep -E "p2proxy_(stream|socks|session)" $TEST_OUTPUT_DIR/baseline_metrics.txt >> $TEST_OUTPUT_DIR/test_report.txt
```

### 3. Configuration Verification

Verify Config.yaml has stream pool enabled:

```bash
# Check if pool is enabled
if grep -q "enabled: true" Config.yaml; then
    echo "✓ Stream pool is enabled"
else
    echo "✗ ERROR: Stream pool is not enabled"
    exit 1
fi

# Display pool configuration
echo "Pool Configuration:"
grep -A 5 "pool:" Config.yaml
```

---

## Test Scenarios

### Test 1: Simple Page Load (Baseline Functionality)

**Objective:** Verify basic proxy functionality works with pool enabled.

**Procedure:**
1. Start p2proxy with debug logging
2. Configure Firefox to use SOCKS5 proxy
3. Load a simple page (http://example.com)
4. Verify successful load

**Automated Test:**
```bash
#!/bin/bash
# test_simple_load.sh

echo "=== Test 1: Simple Page Load ==="

# Start proxy in background
cargo run --bin p2proxy > $TEST_OUTPUT_DIR/test1_proxy.log 2>&1 &
PROXY_PID=$!
sleep 5

# Test with curl
RESULT=$(curl -x socks5://$PROXY_HOST:$PROXY_PORT http://example.com -w "%{http_code}" -o /dev/null -s)

# Check result
if [ "$RESULT" == "200" ]; then
    echo "✓ PASS: Simple page loaded successfully"
    echo "TEST 1: PASS" >> $TEST_OUTPUT_DIR/test_report.txt
else
    echo "✗ FAIL: Expected 200, got $RESULT"
    echo "TEST 1: FAIL (HTTP $RESULT)" >> $TEST_OUTPUT_DIR/test_report.txt
fi

# Cleanup
kill $PROXY_PID
sleep 2
```

**Success Criteria:**
- HTTP 200 response received
- No errors in proxy logs
- Metrics show: `p2proxy_stream_opened_total` incremented by 1

---

### Test 2: Concurrent Connections (Rate Limiting)

**Objective:** Verify pool correctly limits concurrent stream opens.

**Procedure:**
1. Configure pool with `max_total: 20`
2. Simultaneously request 50 URLs through the proxy
3. Verify no more than 20 streams open concurrently
4. Verify all requests eventually complete

**Automated Test:**
```bash
#!/bin/bash
# test_concurrent.sh

echo "=== Test 2: Concurrent Connections ==="

# Start proxy
cargo run --bin p2proxy > $TEST_OUTPUT_DIR/test2_proxy.log 2>&1 &
PROXY_PID=$!
sleep 5

# Capture initial metrics
INITIAL_OPENED=$(curl -s $METRICS_URL | grep "p2proxy_stream_opened_total" | awk '{print $2}' | head -1)

# Launch 50 concurrent requests
for i in {1..50}; do
    curl -x socks5://$PROXY_HOST:$PROXY_PORT http://example.com -o /dev/null -s &
done

# Monitor active streams every 100ms for 10 seconds
MAX_CONCURRENT=0
for j in {1..100}; do
    ACTIVE=$(curl -s $METRICS_URL | grep "p2proxy_stream_pool_active_total" | awk '{print $2}' | head -1)
    if (( $(echo "$ACTIVE > $MAX_CONCURRENT" | bc -l) )); then
        MAX_CONCURRENT=$ACTIVE
    fi
    sleep 0.1
done

# Wait for all requests to complete
wait

# Check final metrics
FINAL_OPENED=$(curl -s $METRICS_URL | grep "p2proxy_stream_opened_total" | awk '{print $2}' | head -1)
TOTAL_OPENED=$((FINAL_OPENED - INITIAL_OPENED))

# Validate
echo "Max concurrent streams observed: $MAX_CONCURRENT"
echo "Total streams opened: $TOTAL_OPENED"

if (( $(echo "$MAX_CONCURRENT <= 20" | bc -l) )) && [ $TOTAL_OPENED -eq 50 ]; then
    echo "✓ PASS: Rate limiting working correctly"
    echo "TEST 2: PASS (Max concurrent: $MAX_CONCURRENT, Total: $TOTAL_OPENED)" >> $TEST_OUTPUT_DIR/test_report.txt
else
    echo "✗ FAIL: Rate limiting issue (Max: $MAX_CONCURRENT, Total: $TOTAL_OPENED)"
    echo "TEST 2: FAIL" >> $TEST_OUTPUT_DIR/test_report.txt
fi

kill $PROXY_PID
sleep 2
```

**Success Criteria:**
- Max concurrent streams ≤ 20 (configured limit)
- All 50 requests complete successfully
- No timeout errors in logs

---

### Test 3: Complex Page Load (Firefox Integration)

**Objective:** Verify Firefox can load resource-heavy pages on first try.

**Procedure:**
1. Start proxy with default pool config
2. Use Firefox to load BBC News (typically 80+ resources)
3. Monitor network inspector
4. Count failed resources

**Manual Test Procedure:**
```
1. Start proxy: cargo run --bin p2proxy
2. Configure Firefox:
   - Settings → Network Settings → Manual proxy configuration
   - SOCKS Host: 127.0.0.1, Port: 1080, SOCKS v5
3. Open Firefox DevTools (F12) → Network tab
4. Clear cache (Ctrl+Shift+Delete → Everything)
5. Navigate to https://www.bbc.com
6. Wait for page to fully load
7. Count resources:
   - Total requests
   - Failed requests (red)
   - Success rate = (Total - Failed) / Total * 100
```

**Automated Test (headless Firefox):**
```bash
#!/bin/bash
# test_complex_page.sh

echo "=== Test 3: Complex Page Load ==="

# Start proxy
cargo run --bin p2proxy > $TEST_OUTPUT_DIR/test3_proxy.log 2>&1 &
PROXY_PID=$!
sleep 5

# Capture before metrics
BEFORE_ERRORS=$(curl -s $METRICS_URL | grep "p2proxy_session_errors_total" | awk '{print $2}')
BEFORE_SESSIONS=$(curl -s $METRICS_URL | grep "p2proxy_sessions_completed_total" | awk '{print $2}')

# Use headless Firefox with proxy (requires geckodriver)
# Note: This requires geckodriver and selenium setup
cat > $TEST_OUTPUT_DIR/firefox_test.py << 'EOF'
from selenium import webdriver
from selenium.webdriver.firefox.options import Options
from selenium.webdriver.common.proxy import Proxy, ProxyType
import time
import json

options = Options()
options.headless = True
proxy = Proxy()
proxy.proxy_type = ProxyType.MANUAL
proxy.socks_proxy = "127.0.0.1:1080"
proxy.socks_version = 5
options.proxy = proxy

driver = webdriver.Firefox(options=options)
driver.set_page_load_timeout(30)

try:
    driver.get("https://www.bbc.com")
    time.sleep(10)  # Wait for all resources

    # Get performance data
    nav_timing = driver.execute_script("return window.performance.getEntriesByType('navigation')[0]")
    resource_timing = driver.execute_script("return window.performance.getEntriesByType('resource')")

    total_resources = len(resource_timing)
    failed_resources = len([r for r in resource_timing if r['transferSize'] == 0])
    success_rate = ((total_resources - failed_resources) / total_resources * 100) if total_resources > 0 else 0

    results = {
        "total_resources": total_resources,
        "failed_resources": failed_resources,
        "success_rate": success_rate,
        "load_time": nav_timing['loadEventEnd'] - nav_timing['fetchStart']
    }

    with open('test-results/firefox_results.json', 'w') as f:
        json.dump(results, f)

    print(f"Total resources: {total_resources}")
    print(f"Failed resources: {failed_resources}")
    print(f"Success rate: {success_rate:.1f}%")

except Exception as e:
    print(f"Error: {e}")
    with open('test-results/firefox_results.json', 'w') as f:
        json.dump({"error": str(e)}, f)
finally:
    driver.quit()
EOF

# Run Firefox test if geckodriver available
if command -v geckodriver &> /dev/null; then
    python3 $TEST_OUTPUT_DIR/firefox_test.py

    # Check results
    if [ -f $TEST_OUTPUT_DIR/firefox_results.json ]; then
        SUCCESS_RATE=$(jq -r '.success_rate' $TEST_OUTPUT_DIR/firefox_results.json)
        if (( $(echo "$SUCCESS_RATE >= 99" | bc -l) )); then
            echo "✓ PASS: Firefox loaded page successfully ($SUCCESS_RATE% success rate)"
            echo "TEST 3: PASS ($SUCCESS_RATE%)" >> $TEST_OUTPUT_DIR/test_report.txt
        else
            echo "✗ FAIL: Too many failed resources ($SUCCESS_RATE% success rate)"
            echo "TEST 3: FAIL ($SUCCESS_RATE%)" >> $TEST_OUTPUT_DIR/test_report.txt
        fi
    fi
else
    echo "⚠ SKIP: geckodriver not found, run manual test"
    echo "TEST 3: SKIPPED (manual test required)" >> $TEST_OUTPUT_DIR/test_report.txt
fi

kill $PROXY_PID
sleep 2
```

**Success Criteria:**
- Success rate ≥ 99%
- Page loads in < 10 seconds
- No manual refresh required
- `p2proxy_session_errors_total` increase < 2%

---

### Test 4: Timeout Handling

**Objective:** Verify stream open timeouts are handled correctly.

**Procedure:**
1. Configure pool with `open_timeout_secs: 1` (very short)
2. Attempt to connect to slow/unresponsive peer
3. Verify timeout is enforced and error is handled

**Automated Test:**
```bash
#!/bin/bash
# test_timeout.sh

echo "=== Test 4: Timeout Handling ==="

# Temporarily modify config for short timeout
cp Config.yaml Config.yaml.backup
sed -i.bak 's/open_timeout_secs: 10/open_timeout_secs: 1/' Config.yaml

# Start proxy
cargo run --bin p2proxy > $TEST_OUTPUT_DIR/test4_proxy.log 2>&1 &
PROXY_PID=$!
sleep 5

# Record initial timeout count
INITIAL_TIMEOUTS=$(curl -s $METRICS_URL | grep "p2proxy_stream_acquire_timeout_total" | awk '{print $2}' | head -1 || echo "0")

# Attempt request that might timeout
timeout 5 curl -x socks5://$PROXY_HOST:$PROXY_PORT http://example.com -o /dev/null -s
CURL_RESULT=$?

# Check timeout metric
sleep 1
FINAL_TIMEOUTS=$(curl -s $METRICS_URL | grep "p2proxy_stream_acquire_timeout_total" | awk '{print $2}' | head -1 || echo "0")

# Restore config
mv Config.yaml.backup Config.yaml

# Validate
if grep -q "Timeout" $TEST_OUTPUT_DIR/test4_proxy.log; then
    echo "✓ PASS: Timeout handled correctly"
    echo "TEST 4: PASS" >> $TEST_OUTPUT_DIR/test_report.txt
else
    echo "✓ PASS: No timeout occurred (peer responded quickly)"
    echo "TEST 4: PASS (no timeout needed)" >> $TEST_OUTPUT_DIR/test_report.txt
fi

kill $PROXY_PID
sleep 2
```

**Success Criteria:**
- Timeout enforced within configured time
- Error logged with "Timeout" message
- No panic or crash
- `p2proxy_stream_acquire_timeout_total` incremented

---

### Test 5: Pool Disabled Fallback

**Objective:** Verify system works with pool disabled (rollback scenario).

**Procedure:**
1. Set `pool.enabled: false` in Config.yaml
2. Run basic functionality tests
3. Verify proxy still works

**Automated Test:**
```bash
#!/bin/bash
# test_pool_disabled.sh

echo "=== Test 5: Pool Disabled Fallback ==="

# Disable pool
cp Config.yaml Config.yaml.backup
sed -i.bak 's/enabled: true/enabled: false/' Config.yaml

# Start proxy
cargo run --bin p2proxy > $TEST_OUTPUT_DIR/test5_proxy.log 2>&1 &
PROXY_PID=$!
sleep 5

# Test basic request
RESULT=$(curl -x socks5://$PROXY_HOST:$PROXY_PORT http://example.com -w "%{http_code}" -o /dev/null -s)

# Restore config
mv Config.yaml.backup Config.yaml

# Validate
if [ "$RESULT" == "200" ]; then
    echo "✓ PASS: Proxy works with pool disabled"
    echo "TEST 5: PASS" >> $TEST_OUTPUT_DIR/test_report.txt
else
    echo "✗ FAIL: Proxy failed with pool disabled (HTTP $RESULT)"
    echo "TEST 5: FAIL" >> $TEST_OUTPUT_DIR/test_report.txt
fi

kill $PROXY_PID
sleep 2
```

**Success Criteria:**
- Proxy functions normally
- No pool metrics are recorded
- Request completes successfully

---

### Test 6: Metrics Accuracy

**Objective:** Verify all pool metrics are tracked correctly.

**Procedure:**
1. Clear all metrics (restart proxy)
2. Make exactly 10 requests
3. Verify metrics match expected values

**Automated Test:**
```bash
#!/bin/bash
# test_metrics.sh

echo "=== Test 6: Metrics Accuracy ==="

# Start fresh proxy
cargo run --bin p2proxy > $TEST_OUTPUT_DIR/test6_proxy.log 2>&1 &
PROXY_PID=$!
sleep 5

# Make exactly 10 requests
for i in {1..10}; do
    curl -x socks5://$PROXY_HOST:$PROXY_PORT http://example.com -o /dev/null -s
    sleep 0.5
done

# Wait for metrics to settle
sleep 2

# Collect metrics
METRICS=$(curl -s $METRICS_URL)
OPENED=$(echo "$METRICS" | grep "p2proxy_stream_opened_total" | awk '{print $2}' | head -1)
ACQUIRE=$(echo "$METRICS" | grep "p2proxy_stream_pool_acquire_total" | awk '{print $2}' | head -1)
SESSIONS=$(echo "$METRICS" | grep "p2proxy_sessions_completed_total" | awk '{print $2}' | head -1)

echo "Streams opened: $OPENED"
echo "Pool acquires: $ACQUIRE"
echo "Sessions completed: $SESSIONS"

# Validate
ERRORS=0
[ "$OPENED" -lt 10 ] && echo "✗ stream_opened_total too low: $OPENED" && ERRORS=$((ERRORS+1))
[ "$ACQUIRE" -lt 10 ] && echo "✗ stream_pool_acquire_total too low: $ACQUIRE" && ERRORS=$((ERRORS+1))
[ "$SESSIONS" -lt 10 ] && echo "✗ sessions_completed_total too low: $SESSIONS" && ERRORS=$((ERRORS+1))

if [ $ERRORS -eq 0 ]; then
    echo "✓ PASS: All metrics accurate"
    echo "TEST 6: PASS" >> $TEST_OUTPUT_DIR/test_report.txt
else
    echo "✗ FAIL: $ERRORS metric(s) inaccurate"
    echo "TEST 6: FAIL ($ERRORS errors)" >> $TEST_OUTPUT_DIR/test_report.txt
fi

kill $PROXY_PID
sleep 2
```

**Success Criteria:**
- `p2proxy_stream_opened_total` ≥ 10
- `p2proxy_stream_pool_acquire_total` ≥ 10
- `p2proxy_sessions_completed_total` ≥ 10
- All metrics consistent with each other

---

## Metrics Reference

### Key Metrics to Monitor

| Metric | Type | Description | Expected Behavior |
|--------|------|-------------|-------------------|
| `p2proxy_stream_pool_active_total` | Gauge | Current active streams per peer | Should not exceed `max_total` config |
| `p2proxy_stream_opened_total` | Counter | Total streams successfully opened | Should increase with each request |
| `p2proxy_stream_opened_success_total` | Counter | Successful stream opens per peer | Should be close to `stream_opened_total` |
| `p2proxy_stream_opened_failed_total` | Counter | Failed stream opens per peer | Should be minimal (< 1% of total) |
| `p2proxy_stream_acquire_timeout_total` | Counter | Timeouts waiting for stream slot | Should be 0 under normal load |
| `p2proxy_stream_acquire_duration_seconds` | Histogram | Time to acquire stream | p50 < 1s, p99 < 5s |
| `p2proxy_session_errors_total` | Counter | Total session errors | Should increase < 2% per session |
| `p2proxy_sessions_completed_total` | Counter | Successfully completed sessions | Should match number of requests |

### Interpreting Metrics

**Healthy System:**
```
p2proxy_stream_pool_active_total{peer="..."} 5
p2proxy_stream_opened_total 150
p2proxy_stream_opened_success_total{peer="..."} 150
p2proxy_stream_opened_failed_total{peer="..."} 0
p2proxy_stream_acquire_timeout_total 0
```

**Overloaded Peer:**
```
p2proxy_stream_pool_active_total{peer="..."} 20  # At max
p2proxy_stream_acquire_timeout_total 5            # Timeouts occurring
```

**Configuration Issue:**
```
p2proxy_stream_opened_failed_total{peer="..."} 50  # High failure rate
p2proxy_session_errors_total 50                     # Many session errors
```

---

## Success Criteria Summary

### Overall Test Suite

**PASS Criteria:**
- All 6 tests pass
- No crashes or panics
- Firefox loads complex pages with >99% success rate on first try

**FAIL Criteria:**
- Any test fails
- Crash or panic occurs
- Firefox requires page refresh to load resources

### Performance Benchmarks

| Metric | Target | Minimum Acceptable |
|--------|--------|-------------------|
| Complex page load success rate | >99% | >95% |
| Page load time (BBC News) | < 5s | < 10s |
| Stream acquire duration (p50) | < 500ms | < 1s |
| Stream acquire duration (p99) | < 2s | < 5s |
| Session error rate | < 1% | < 5% |
| Max concurrent streams | ≤ configured limit | ≤ configured limit + 1 |

---

## Troubleshooting Guide

### Issue: High Timeout Rate

**Symptoms:**
- `p2proxy_stream_acquire_timeout_total` increasing rapidly
- Slow page loads
- Many failed requests

**Diagnosis:**
```bash
# Check current active streams
curl -s $METRICS_URL | grep "p2proxy_stream_pool_active_total"

# Check if at limit
# If value == max_total consistently, peer is slow or config too restrictive
```

**Solutions:**
1. Increase `max_total` in Config.yaml (try 30-40)
2. Increase `open_timeout_secs` (try 15-20)
3. Check peer quality (switch to different country/peer)

### Issue: High Failure Rate

**Symptoms:**
- `p2proxy_stream_opened_failed_total` > 10% of total
- Many session errors
- Inconsistent page loading

**Diagnosis:**
```bash
# Check error logs
grep -i "error\|fail" $TEST_OUTPUT_DIR/proxy.log | tail -20

# Check peer connection status
curl -s $METRICS_URL | grep "p2proxy_peer_connections_total"
```

**Solutions:**
1. Verify peer is reachable: Check `p2proxy_peer_connections_total`
2. Check authentication: Look for "auth" errors in logs
3. Try different peer (remove country filter temporarily)

### Issue: Metrics Not Updating

**Symptoms:**
- Metrics endpoint returns old values
- Pool metrics always show 0

**Diagnosis:**
```bash
# Verify pool is enabled
grep -A 5 "pool:" Config.yaml

# Check if metrics exporter is running
curl -s http://localhost:9091/metrics | head -5
```

**Solutions:**
1. Ensure `pool.enabled: true` in Config.yaml
2. Restart proxy to apply config changes
3. Verify Prometheus exporter port not blocked

### Issue: Firefox Still Needs Refresh

**Symptoms:**
- Page loads partially
- Some images/CSS missing
- F5 refresh fixes it

**Diagnosis:**
```bash
# Check concurrent connection limits
curl -s $METRICS_URL | grep "p2proxy_stream_pool_active_total"

# Check Firefox connection settings
# about:config -> network.http.max-persistent-connections-per-proxy
```

**Solutions:**
1. Increase pool `max_total` to match Firefox's connection limit (default 32)
2. Check if timeout is too aggressive (increase `open_timeout_secs`)
3. Monitor logs during page load to identify specific failures

---

## Automated Full Test Suite

```bash
#!/bin/bash
# run_all_tests.sh

set -e

export PROXY_HOST="127.0.0.1"
export PROXY_PORT="1080"
export METRICS_URL="http://localhost:9091/metrics"
export TEST_OUTPUT_DIR="./test-results"

echo "========================================"
echo "Stream Pool Test Suite"
echo "========================================"
echo ""

# Create output directory
mkdir -p $TEST_OUTPUT_DIR
echo "STREAM POOL TEST RESULTS" > $TEST_OUTPUT_DIR/test_report.txt
echo "Date: $(date)" >> $TEST_OUTPUT_DIR/test_report.txt
echo "" >> $TEST_OUTPUT_DIR/test_report.txt

# Run all tests
./test_simple_load.sh
echo ""
./test_concurrent.sh
echo ""
./test_complex_page.sh
echo ""
./test_timeout.sh
echo ""
./test_pool_disabled.sh
echo ""
./test_metrics.sh
echo ""

# Generate summary
echo "========================================"
echo "TEST SUMMARY"
echo "========================================"
cat $TEST_OUTPUT_DIR/test_report.txt

# Count results
PASSED=$(grep -c "PASS" $TEST_OUTPUT_DIR/test_report.txt || echo "0")
FAILED=$(grep -c "FAIL" $TEST_OUTPUT_DIR/test_report.txt || echo "0")
SKIPPED=$(grep -c "SKIP" $TEST_OUTPUT_DIR/test_report.txt || echo "0")

echo ""
echo "Passed: $PASSED"
echo "Failed: $FAILED"
echo "Skipped: $SKIPPED"

# Exit code
if [ $FAILED -gt 0 ]; then
    echo ""
    echo "❌ TEST SUITE FAILED"
    exit 1
else
    echo ""
    echo "✅ TEST SUITE PASSED"
    exit 0
fi
```

---

## Quick Reference Commands

```bash
# Start proxy with debug logging
RUST_LOG=debug cargo run --bin p2proxy

# Monitor metrics in real-time
watch -n 1 'curl -s http://localhost:9091/metrics | grep -E "stream_pool|stream_opened"'

# Count active sessions
curl -s http://localhost:9091/metrics | grep "stream_pool_active_total"

# Check error rates
curl -s http://localhost:9091/metrics | grep -E "error|fail"

# Test single request
curl -x socks5://127.0.0.1:1080 https://example.com -v

# Test with timing
curl -x socks5://127.0.0.1:1080 https://example.com -w "\nTotal time: %{time_total}s\n"

# Monitor logs for errors
tail -f target/debug/p2proxy.log | grep -i error
```

---

## Agent Execution Instructions

For autonomous agent testing, execute in this order:

1. **Setup:** Run pre-test setup commands to create environment
2. **Baseline:** Collect baseline metrics before any tests
3. **Execute Tests:** Run each test script in sequence (Test 1-6)
4. **Collect Results:** Parse test_report.txt for PASS/FAIL status
5. **Analyze:** Compare metrics before/after, calculate success rates
6. **Report:** Generate final report with all metrics and test results

**Expected Output Format:**
```
TEST_SUITE_RESULT: PASS
TESTS_PASSED: 6/6
SUCCESS_RATE: 100%
PERFORMANCE: ACCEPTABLE (p99 < 5s)
RECOMMENDATION: Deploy to production
```

**Failure Handling:**
- If any test fails, include full error details
- Attach relevant log snippets
- Provide specific troubleshooting steps from guide above
- Suggest configuration changes if applicable
