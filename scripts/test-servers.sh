#!/usr/bin/env bash
# Smoke-test every SOCKS5 listener p2proxy is supposed to be exposing.
#
# For each port we run a short suite:
#   1. TCP reachability (does the listener bind at all?)
#   2. HTTP through the proxy (confirms TCP-forwarding works end-to-end)
#   3. HTTPS through the proxy (confirms streams stay intact across TLS)
#   4. Exit-IP geolocation (sanity-check the country filter is doing its job)
#   5. Latency for a tiny request
#
# Then it dumps the relevant Prometheus counters from p2proxy's :9091
# endpoint so you can see — independent of the TUI — whether any actual
# bytes flowed and whether SOCKS sessions reached the `Initialized` stage.
#
# Usage:
#     apps/customer/p2proxy/scripts/test-servers.sh           # test default ports
#     apps/customer/p2proxy/scripts/test-servers.sh 1080 1081 # specific ports
#
# Requires: curl, dig (optional, for IP→country), bc (for latency math).

set -uo pipefail

DEFAULT_PORTS=(1080 1081 1082 1083)
PORTS=("${@:-${DEFAULT_PORTS[@]}}")

METRICS_HOST="${P2PROXY_METRICS_HOST:-127.0.0.1:9091}"
TIMEOUT="${P2PROXY_TEST_TIMEOUT:-10}"

# ANSI colours — kept in sync with tui-components/theme.rs by eye, not by
# code, because shell doesn't share the rust palette.
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
CYAN='\033[0;36m'
DIM='\033[0;90m'
BOLD='\033[1m'
RESET='\033[0m'

ok()    { echo -e "  ${GREEN}✓${RESET} $1"; }
fail()  { echo -e "  ${RED}✗${RESET} $1"; }
warn()  { echo -e "  ${YELLOW}!${RESET} $1"; }
info()  { echo -e "  ${DIM}·${RESET} $1"; }

# Test 1 — can we even open a TCP socket to the listener? Distinguishes
# "p2proxy didn't bind" from "p2proxy bound but the proxy session is
# failing", which are different bugs.
test_port_open() {
    local port=$1
    if timeout 2 bash -c "</dev/tcp/127.0.0.1/${port}" 2>/dev/null; then
        ok "TCP :${port} open"
        return 0
    else
        fail "TCP :${port} refused (listener not bound)"
        return 1
    fi
}

# Test 2 — HTTP through the SOCKS5 proxy. `--socks5-hostname` is
# important: resolves DNS on the proxy side rather than locally, which
# is what an exit-node proxy needs to do for any geolocation lookup to
# make sense.
#
# Status lines (ok/fail) go to STDERR so they print directly to the
# terminal even when the caller captures stdout via `$( ... )`. Only
# the exit IP is emitted on stdout. Previously both went to stdout and
# command-substitution swallowed the fail line — making "test_port"
# bail silently when HTTP timed out (e.g. while waiting for JIT peer
# discovery on a server with a small pool).
test_http() {
    local port=$1
    local response
    response=$(curl -sS --max-time "$TIMEOUT" --socks5-hostname "127.0.0.1:${port}" \
        "http://api.ipify.org" 2>&1)
    local rc=$?
    if [ $rc -eq 0 ] && [[ "$response" =~ ^[0-9.]+$ ]]; then
        echo -e "  ${GREEN}✓${RESET} HTTP ipify exit IP: ${BOLD}${response}${RESET}" >&2
        echo "$response"
        return 0
    else
        echo -e "  ${RED}✗${RESET} HTTP failed (rc=$rc): $response" >&2
        return 1
    fi
}

# Test 3 — HTTPS. Catches TLS protocol breaks (e.g. "tlsv1 alert
# protocol version" from earlier — bytes get truncated mid-handshake).
test_https() {
    local port=$1
    local response
    response=$(curl -sS --max-time "$TIMEOUT" --socks5-hostname "127.0.0.1:${port}" \
        "https://api.ipify.org" 2>&1)
    local rc=$?
    if [ $rc -eq 0 ] && [[ "$response" =~ ^[0-9.]+$ ]]; then
        ok "HTTPS ipify exit IP: ${BOLD}${response}${RESET}"
        return 0
    else
        fail "HTTPS failed (rc=$rc): $response"
        return 1
    fi
}

# Test 4 — figure out which country the exit IP is actually in. Uses
# ip-api.com which is free and doesn't need auth. If the proxy's
# `country` filter is configured for "AU" but the exit IP comes back
# "FR", the hub returned a wrong-country peer (or the country filter
# isn't reaching the hub correctly).
#
# Accepts the exit IP as `$2` to avoid paying a second proxy round-
# trip — the caller already learned it via `test_http`.
test_exit_country() {
    local port=$1
    local exit_ip=${2:-}
    if [ -z "$exit_ip" ]; then
        warn "skipping geolocation (no exit IP)"
        return 1
    fi
    # Geolocate from the local machine — going through the proxy here
    # would give the proxy's view of itself, not the exit's view.
    local country
    country=$(curl -sS --max-time 5 "http://ip-api.com/csv/${exit_ip}?fields=countryCode,country,isp" 2>/dev/null)
    if [ -n "$country" ]; then
        info "exit geo: $country"
    fi
}

# Test 5 — round-trip latency for a tiny request. Useful as a relative
# comparison between ports: if one country is consistently 5x slower
# than the others, the chosen peer there has a bad link to the
# customer.
test_latency() {
    local port=$1
    local time_total
    time_total=$(curl -sS --max-time "$TIMEOUT" --socks5-hostname "127.0.0.1:${port}" \
        -o /dev/null -w '%{time_total}\n' \
        "http://api.ipify.org" 2>/dev/null)
    if [ -n "$time_total" ]; then
        # Bash arithmetic can't handle floats; bc keeps us portable.
        local ms
        ms=$(echo "${time_total} * 1000" | bc -l 2>/dev/null | awk '{printf "%.0f", $1}')
        info "latency: ${ms}ms"
    fi
}

# Per-port driver. Returns 0 if at least the listener and HTTP worked,
# non-zero otherwise — so the script's exit code roughly tracks "is the
# proxy usable on this port".
test_port() {
    local port=$1
    echo
    echo -e "${BOLD}${CYAN}── port :${port} ──${RESET}"
    test_port_open "$port" || return 1
    # `test_http` prints its own status line to stderr so it's visible
    # even though stdout is captured. We use the captured IP for the
    # geolocation lookup below.
    local exit_ip
    if ! exit_ip=$(test_http "$port"); then
        return 1
    fi
    test_https "$port"
    # Pass the IP we already learned so test_exit_country doesn't
    # repeat the curl. Avoids paying the proxy round-trip twice and
    # gives consistent geolocation lookup even when HTTP+HTTPS exit
    # through different peers (which can happen mid-rotation).
    test_exit_country "$port" "$exit_ip"
    test_latency "$port"
}

# Pull the Prometheus counters that tell us whether the swarm layer
# actually saw any bytes / sessions. These increment regardless of the
# TUI being open, so they're the source of truth for "is the proxy
# doing work" when the dashboard looks empty.
dump_metrics() {
    echo
    echo -e "${BOLD}${CYAN}── metrics (${METRICS_HOST}) ──${RESET}"
    if ! curl -sS --max-time 3 "http://${METRICS_HOST}/metrics" >/tmp/p2proxy_metrics.$$ 2>&1; then
        warn "metrics endpoint unreachable — is p2proxy running with --no-ui or has the :9091 listener died?"
        return 1
    fi
    # Counters that show up only after some traffic has flowed; great
    # health-check signal because the values are interpretable directly.
    local interesting=(
        # Per-byte counters incremented in handle_proxy_events for every
        # DataTransferred message. Nonzero == proxy is forwarding bytes.
        p2proxy_upload_bytes_total
        p2proxy_download_bytes_total
        # Per-session lifecycle. Nonzero == SOCKS sessions got past
        # client_init (which is the gate before bandwidth would flow).
        p2proxy_sessions_initialized_total
        p2proxy_sessions_completed_total
        # SOCKS-server health.
        p2proxy_socks_connections_total
        p2proxy_socks_connections_established_total
        p2proxy_socks_rejected_no_peer_total
        # Stream-pool open failures — typically protocol-mismatch
        # rejections from an old tcp-forwarder on a node.
        p2proxy_stream_acquire_failed_total
        p2proxy_peer_terminal_error_total
        # Proactive peer-rediscovery triggered by ConnectionClosed.
        p2proxy_peer_proactive_rediscovery_total
        # Per-server visibility (BIT-???): pool size and whether the
        # active destination is set.
        p2proxy_server_pool_size
        p2proxy_server_active_destination_present
        p2proxy_sessions_active
    )
    local m
    for m in "${interesting[@]}"; do
        # `grep ^name ` (with trailing space) matches the bare metric
        # without picking up label-bearing variants of the same name.
        local line
        line=$(grep -E "^${m}( |\\{)" /tmp/p2proxy_metrics.$$ | head -n1)
        if [ -n "$line" ]; then
            info "$line"
        else
            warn "$m — not emitted yet"
        fi
    done
    rm -f /tmp/p2proxy_metrics.$$
}

main() {
    echo -e "${BOLD}p2proxy SOCKS5 smoke test${RESET}"
    echo -e "${DIM}ports: ${PORTS[*]} · timeout: ${TIMEOUT}s · metrics: ${METRICS_HOST}${RESET}"

    local failed=0
    for port in "${PORTS[@]}"; do
        test_port "$port" || failed=$((failed + 1))
    done

    dump_metrics

    echo
    if [ "$failed" -eq 0 ]; then
        echo -e "${GREEN}${BOLD}all ports healthy${RESET}"
        return 0
    else
        echo -e "${RED}${BOLD}${failed} of ${#PORTS[@]} ports failed${RESET}"
        return 1
    fi
}

main
