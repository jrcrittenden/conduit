#!/bin/bash
# Shared helper functions for E2E tests
# Source this file in test scripts: source "$(dirname "$0")/lib.sh"

set -euo pipefail

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Global variables - use absolute path
SCRIPT_DIR_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR_LIB/../.." && pwd)"
CONDUIT_BINARY="${CONDUIT_BINARY:-$PROJECT_ROOT/target/release/conduit}"
TEST_TIMEOUT="${TEST_TIMEOUT:-10000}"
SCREENSHOT_DIR="$(dirname "$0")/screenshots"

# Ensure screenshot directory exists
mkdir -p "$SCREENSHOT_DIR"

# Generate unique identifiers for this test run
TEST_ID="$$-$(date +%s)"
REQUEST_ID=1

# Get next request ID
next_id() {
    local id=$REQUEST_ID
    REQUEST_ID=$((REQUEST_ID + 1))
    echo $id
}

# Log functions
log_info() {
    echo -e "${GREEN}[INFO]${NC} $*"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $*"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $*"
}

log_pass() {
    echo -e "${GREEN}✓${NC} $*"
}

log_fail() {
    echo -e "${RED}✗${NC} $*"
}

# Check dependencies
check_dependencies() {
    local missing=()

    if ! command -v socat &> /dev/null; then
        missing+=("socat")
    fi

    if ! command -v jq &> /dev/null; then
        missing+=("jq")
    fi

    if ! command -v termwright &> /dev/null; then
        missing+=("termwright")
    fi

    if [ ${#missing[@]} -ne 0 ]; then
        log_error "Missing dependencies: ${missing[*]}"
        log_info "Install with:"
        log_info "  brew install socat jq  # macOS"
        log_info "  sudo apt-get install socat jq  # Ubuntu"
        log_info "  cargo install termwright"
        exit 1
    fi
}

# Create a unique data directory for this test
create_data_dir() {
    local name="${1:-test}"
    local dir="/tmp/conduit-e2e-${name}-${TEST_ID}"
    mkdir -p "$dir"
    echo "$dir"
}

# Start conduit and return socket path
# Usage: SOCK=$(start_conduit "$DATA_DIR")
start_conduit() {
    local data_dir="$1"
    local cols="${2:-120}"
    local rows="${3:-40}"
    local sock="/tmp/conduit-e2e-${TEST_ID}.sock"

    # Remove old socket if exists
    rm -f "$sock"

    # Start termwright daemon (suppress its output)
    termwright daemon --socket "$sock" --cols "$cols" --rows "$rows" -- \
        "$CONDUIT_BINARY" --data-dir "$data_dir" >/dev/null 2>&1 &

    # Wait for socket to appear
    local tries=0
    while [ ! -S "$sock" ] && [ $tries -lt 50 ]; do
        sleep 0.1
        tries=$((tries + 1))
    done

    if [ ! -S "$sock" ]; then
        log_error "Failed to start conduit - socket not created" >&2
        return 1
    fi

    # Wait for app to fully initialize and render
    sleep 3

    echo "$sock"
}

# Send raw command to termwright and get response
# Usage: tw "$SOCK" '{"id":1,"method":"screen","params":{"format":"text"}}' [timeout]
tw() {
    local sock="$1"
    local cmd="$2"
    local timeout_secs="${3:-30}"

    # Simple approach: pipe command to socat with timeout
    # The echo + cat keep stdin open long enough for async responses
    echo "$cmd" | timeout "$timeout_secs" socat - UNIX-CONNECT:"$sock" 2>/dev/null
}

# Send command with auto-incrementing ID
# Usage: tw_auto "$SOCK" "screen" '{"format":"text"}'
tw_auto() {
    local sock="$1"
    local method="$2"
    local params="${3:-null}"
    local id=$(next_id)
    tw "$sock" "{\"id\":$id,\"method\":\"$method\",\"params\":$params}"
}

# Get screen text
get_screen() {
    local sock="$1"
    tw_auto "$sock" "screen" '{"format":"text"}' | jq -r '.result // empty'
}

# Get screen as JSON (includes colors)
get_screen_json() {
    local sock="$1"
    tw_auto "$sock" "screen" '{"format":"json"}'
}

# Wait for screen to stabilize
wait_idle() {
    local sock="$1"
    local idle_ms="${2:-500}"
    local timeout_ms="${3:-$TEST_TIMEOUT}"
    local id=$(next_id)
    local wait_secs=$(( (timeout_ms / 1000) + 2 ))
    tw "$sock" "{\"id\":$id,\"method\":\"wait_for_idle\",\"params\":{\"idle_ms\":$idle_ms,\"timeout_ms\":$timeout_ms}}" "$wait_secs"
}

# Wait for specific text to appear
wait_for_text() {
    local sock="$1"
    local text="$2"
    local timeout_ms="${3:-$TEST_TIMEOUT}"
    # Escape special JSON characters in text
    local escaped_text=$(echo "$text" | sed 's/\\/\\\\/g; s/"/\\"/g')
    local id=$(next_id)
    local wait_secs=$(( (timeout_ms / 1000) + 2 ))
    tw "$sock" "{\"id\":$id,\"method\":\"wait_for_text\",\"params\":{\"text\":\"$escaped_text\",\"timeout_ms\":$timeout_ms}}" "$wait_secs"
}

# Press a key
press() {
    local sock="$1"
    local key="$2"
    tw_auto "$sock" "press" "{\"key\":\"$key\"}" > /dev/null
}

# Type text
type_text() {
    local sock="$1"
    local text="$2"
    local escaped_text=$(echo "$text" | sed 's/\\/\\\\/g; s/"/\\"/g')
    tw_auto "$sock" "type" "{\"text\":\"$escaped_text\"}" > /dev/null
}

# Send Ctrl+key hotkey
ctrl() {
    local sock="$1"
    local ch="$2"
    tw_auto "$sock" "hotkey" "{\"ctrl\":true,\"ch\":\"$ch\"}" > /dev/null
}

# Send Alt+key hotkey
alt() {
    local sock="$1"
    local ch="$2"
    tw_auto "$sock" "hotkey" "{\"alt\":true,\"ch\":\"$ch\"}" > /dev/null
}

# Take screenshot and save to file
screenshot() {
    local sock="$1"
    local name="$2"
    local output="$SCREENSHOT_DIR/${name}.png"
    local result=$(tw_auto "$sock" "screenshot" '{}')
    local png_data=$(echo "$result" | jq -r '.result.png_base64 // empty')

    if [ -n "$png_data" ]; then
        echo "$png_data" | base64 -d > "$output"
        log_info "Screenshot saved: $output"
    else
        log_warn "Failed to capture screenshot"
    fi
}

# Check process status
status() {
    local sock="$1"
    tw_auto "$sock" "status" "null"
}

# Close the daemon
close_daemon() {
    local sock="$1"
    tw "$sock" '{"id":0,"method":"close","params":null}' > /dev/null 2>&1 || true
}

# Cleanup function - call at end of test or on error
cleanup() {
    local sock="$1"
    local data_dir="$2"

    if [ -n "$sock" ] && [ -S "$sock" ]; then
        close_daemon "$sock"
    fi

    if [ -n "$data_dir" ] && [ -d "$data_dir" ]; then
        rm -rf "$data_dir"
    fi
}

# Assert screen contains text
# Usage: assert_contains "$SOCK" "expected text" "Test description"
assert_contains() {
    local sock="$1"
    local expected="$2"
    local description="${3:-Screen contains '$expected'}"

    local screen=$(get_screen "$sock")

    if echo "$screen" | grep -q "$expected"; then
        log_pass "$description"
        return 0
    else
        log_fail "$description"
        log_error "Expected to find: $expected"
        log_error "Screen content:"
        echo "$screen" | head -30
        return 1
    fi
}

# Assert screen does NOT contain text
assert_not_contains() {
    local sock="$1"
    local unexpected="$2"
    local description="${3:-Screen does not contain '$unexpected'}"

    local screen=$(get_screen "$sock")

    if echo "$screen" | grep -q "$unexpected"; then
        log_fail "$description"
        log_error "Did not expect to find: $unexpected"
        return 1
    else
        log_pass "$description"
        return 0
    fi
}

# Run a test function with setup/teardown
# Usage: run_test "test_name" test_function
run_test() {
    local name="$1"
    local func="$2"

    log_info "Running: $name"

    local data_dir=$(create_data_dir "$name")
    local sock=""
    local result=0

    # Trap to ensure cleanup on error
    trap "cleanup '$sock' '$data_dir'" EXIT

    sock=$(start_conduit "$data_dir")
    if [ -z "$sock" ]; then
        log_fail "$name - Failed to start conduit"
        return 1
    fi

    # Wait for app to be ready
    wait_idle "$sock" 500 5000 > /dev/null

    # Run the test
    if $func "$sock" "$data_dir"; then
        log_pass "$name"
        result=0
    else
        log_fail "$name"
        # Take screenshot on failure
        screenshot "$sock" "failure-$name"
        result=1
    fi

    # Cleanup
    trap - EXIT
    cleanup "$sock" "$data_dir"

    return $result
}

# Initialize - check dependencies
check_dependencies
