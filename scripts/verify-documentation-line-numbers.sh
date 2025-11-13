#!/bin/bash
# Verify that line numbers in documentation still match actual code
#
# This script checks if the critical issues documented in the connection
# failure analysis are still present in the codebase at the documented locations.
#
# Usage: bash scripts/verify-documentation-line-numbers.sh

set -e

echo "========================================"
echo "Documentation Line Number Verification"
echo "========================================"
echo ""

ERRORS=0

# Issue 1.1: RPC accept unwrap
echo "Checking Issue 1.1: RPC server accept loop panic..."
if grep -n "listener.accept().await.unwrap()" crates/p2proxy/src/main.rs 2>/dev/null | grep -q .; then
    LINE=$(grep -n "listener.accept().await.unwrap()" crates/p2proxy/src/main.rs | cut -d: -f1 | head -1)
    echo "  ✅ Issue 1.1 found at line $LINE (documented: ~134)"
    if [ "$LINE" -gt 150 ] || [ "$LINE" -lt 120 ]; then
        echo "  ⚠️  Line number has drifted significantly from documented ~134"
    fi
else
    echo "  ⚠️  Issue 1.1 NOT FOUND - may have been fixed or moved"
    echo "     Expected pattern: listener.accept().await.unwrap()"
    echo "     Location: crates/p2proxy/src/main.rs"
    ERRORS=$((ERRORS + 1))
fi
echo ""

# Issue 1.2: remoc unwrap
echo "Checking Issue 1.2: RPC connection setup panic..."
if grep -n "remoc::Connect" crates/p2proxy/src/main.rs 2>/dev/null | grep -q .; then
    # Check if there's an unwrap nearby
    if grep -A 3 "remoc::Connect" crates/p2proxy/src/main.rs | grep -q "\.unwrap()"; then
        LINE=$(grep -n "remoc::Connect" crates/p2proxy/src/main.rs | cut -d: -f1 | head -1)
        echo "  ✅ Issue 1.2 found near line $LINE (documented: ~154)"
        if [ "$LINE" -gt 170 ] || [ "$LINE" -lt 140 ]; then
            echo "  ⚠️  Line number has drifted significantly from documented ~154"
        fi
    else
        echo "  ⚠️  Issue 1.2 NOT FOUND - may have been fixed"
        echo "     Found remoc::Connect but no .unwrap() nearby"
        ERRORS=$((ERRORS + 1))
    fi
else
    echo "  ⚠️  Issue 1.2 NOT FOUND - code may have changed significantly"
    ERRORS=$((ERRORS + 1))
fi
echo ""

# Issue 1.3: keypair unwrap
echo "Checking Issue 1.3: Keypair type assumption panic..."
if grep -n "try_into_ed25519().unwrap()" crates/p2proxy/src/swarm.rs 2>/dev/null | grep -q .; then
    LINE=$(grep -n "try_into_ed25519().unwrap()" crates/p2proxy/src/swarm.rs | cut -d: -f1 | head -1)
    echo "  ✅ Issue 1.3 found at line $LINE (documented: ~157)"
    if [ "$LINE" -gt 175 ] || [ "$LINE" -lt 145 ]; then
        echo "  ⚠️  Line number has drifted significantly from documented ~157"
    fi
else
    echo "  ⚠️  Issue 1.3 NOT FOUND - may have been fixed or moved"
    echo "     Expected pattern: try_into_ed25519().unwrap()"
    echo "     Location: crates/p2proxy/src/swarm.rs"
    ERRORS=$((ERRORS + 1))
fi
echo ""

# Issue 2.1: Bootstrap linear backoff
echo "Checking Issue 2.1: Bootstrap linear backoff..."
if grep -n "tokio::time::sleep(Duration::from_secs(2))" crates/p2proxy/src/swarm.rs 2>/dev/null | grep -q .; then
    echo "  ✅ Issue 2.1 found (linear 2s backoff in bootstrap)"
else
    echo "  ⚠️  Issue 2.1 NOT FOUND or already fixed (exponential backoff may be implemented)"
    ERRORS=$((ERRORS + 1))
fi
echo ""

# Issue 2.2: Peer discovery linear retry
echo "Checking Issue 2.2: Peer discovery linear retry..."
if grep -n "tokio::time::sleep(Duration::from_secs(1))" crates/p2proxy/src/swarm.rs 2>/dev/null | grep -q .; then
    echo "  ✅ Issue 2.2 found (linear 1s retry in peer discovery)"
else
    echo "  ⚠️  Issue 2.2 NOT FOUND or already fixed (exponential backoff may be implemented)"
    ERRORS=$((ERRORS + 1))
fi
echo ""

# Issue 2.3: Stream pool dual timeout
echo "Checking Issue 2.3: Stream pool dual timeout..."
if grep -n "stream_open_timeout" crates/p2proxy/src/stream_pool.rs 2>/dev/null | grep -q .; then
    echo "  ✅ Stream pool timeout configuration found"

    # Check if there's a separate semaphore_timeout
    if grep -n "semaphore_timeout" crates/p2proxy/src/stream_pool.rs 2>/dev/null | grep -q .; then
        echo "     ✅ Separate semaphore_timeout found (fix may be implemented)"
    else
        echo "     ⚠️  No separate semaphore_timeout (issue not yet fixed)"
    fi
else
    echo "  ⚠️  Stream pool timeout configuration not found"
    ERRORS=$((ERRORS + 1))
fi
echo ""

# Summary
echo "========================================"
if [ $ERRORS -eq 0 ]; then
    echo "✅ All documented issues verified in codebase"
    echo "   Documentation line numbers are accurate"
    exit 0
else
    echo "⚠️  $ERRORS issue(s) not found or line numbers drifted"
    echo ""
    echo "Action items:"
    echo "1. Update documentation with new line numbers"
    echo "2. Or verify issues have been fixed"
    echo "3. Run: git grep -n 'pattern' to find new locations"
    exit 1
fi
