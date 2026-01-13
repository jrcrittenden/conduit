#!/bin/bash
# Test: Conduit startup and main screen rendering
# Verifies that conduit starts correctly and displays the expected UI

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib.sh"

# Test: Main screen renders with logo and key hints
test_main_screen() {
    local sock="$1"
    local data_dir="$2"

    # Check for key hints at bottom (these contain actual searchable text)
    assert_contains "$sock" "C-n new project" "Shows Ctrl+N hint" || return 1
    assert_contains "$sock" "C-t sidebar" "Shows Ctrl+T hint" || return 1
    assert_contains "$sock" "C-q quit" "Shows Ctrl+Q hint" || return 1

    return 0
}

# Test: Fresh start shows "add first project" message
test_fresh_start_message() {
    local sock="$1"
    local data_dir="$2"

    assert_contains "$sock" "Add your first project" "Shows first project message" || \
    assert_contains "$sock" "Add a new project" "Shows add project message" || return 1

    return 0
}

# Test: App responds to handshake
test_handshake() {
    local sock="$1"
    local data_dir="$2"

    local result=$(tw_auto "$sock" "handshake" "null")
    local version=$(echo "$result" | jq -r '.result.termwright_version // empty')

    if [ -n "$version" ]; then
        log_pass "Handshake successful (termwright $version)"
        return 0
    else
        log_fail "Handshake failed"
        return 1
    fi
}

# Run all tests
main() {
    local failed=0

    run_test "startup_handshake" test_handshake || failed=1
    run_test "startup_main_screen" test_main_screen || failed=1
    run_test "startup_fresh_message" test_fresh_start_message || failed=1

    return $failed
}

main
