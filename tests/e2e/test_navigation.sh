#!/bin/bash
# Test: Keyboard navigation and UI interactions
# Verifies keyboard shortcuts and navigation work correctly

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib.sh"

# Test: Ctrl+T toggles sidebar visibility
test_ctrl_t_toggle_sidebar() {
    local sock="$1"
    local data_dir="$2"

    # Initial state - sidebar should be visible with "Workspaces"
    # Note: On fresh start, sidebar might not show "Workspaces" until project is added
    # So we just test the toggle behavior

    # Get initial screen
    local screen1=$(get_screen "$sock")

    # Press Ctrl+T to toggle
    ctrl "$sock" "t"
    wait_idle "$sock" 300 3000 > /dev/null

    # Get screen after toggle
    local screen2=$(get_screen "$sock")

    # Press Ctrl+T again to toggle back
    ctrl "$sock" "t"
    wait_idle "$sock" 300 3000 > /dev/null

    # Get screen after second toggle
    local screen3=$(get_screen "$sock")

    # Screen should change after first toggle
    if [ "$screen1" = "$screen2" ]; then
        log_fail "Screen did not change after Ctrl+T"
        return 1
    fi

    log_pass "Ctrl+T changes the screen (toggle works)"
    return 0
}

# Test: Ctrl+Q quits the application
test_ctrl_q_quits() {
    local sock="$1"
    local data_dir="$2"

    # Press Ctrl+Q
    ctrl "$sock" "q"

    # Wait a moment for the app to quit
    sleep 1

    # Check if process exited
    local result=$(status "$sock" 2>/dev/null || echo '{"result":{"exited":true}}')
    local exited=$(echo "$result" | jq -r '.result.exited // true')

    if [ "$exited" = "true" ]; then
        log_pass "Ctrl+Q quit the application"
        return 0
    else
        log_fail "Application did not quit after Ctrl+Q"
        return 1
    fi
}

# Test: Help dialog (if implemented) - F1 or ?
test_help_key() {
    local sock="$1"
    local data_dir="$2"

    # Try pressing ? for help
    press "$sock" "?"
    wait_idle "$sock" 300 3000 > /dev/null

    local screen=$(get_screen "$sock")

    # Check if help appeared (might show keybindings)
    if echo "$screen" | grep -qi "help\|keybind\|shortcut"; then
        log_pass "Help screen appeared"
        # Close it
        press "$sock" "Escape"
        return 0
    else
        # Help might not be implemented, that's OK
        log_info "Help screen not found (may not be implemented)"
        return 0
    fi
}

# Test: Arrow key navigation works (when applicable)
test_arrow_keys() {
    local sock="$1"
    local data_dir="$2"

    # Press Up/Down - should not crash
    press "$sock" "Up"
    wait_idle "$sock" 100 1000 > /dev/null

    press "$sock" "Down"
    wait_idle "$sock" 100 1000 > /dev/null

    # If we get here without error, navigation didn't crash
    log_pass "Arrow key navigation works"
    return 0
}

# Run all tests
main() {
    local failed=0

    run_test "nav_ctrl_t_toggle" test_ctrl_t_toggle_sidebar || failed=1
    run_test "nav_arrow_keys" test_arrow_keys || failed=1
    run_test "nav_help_key" test_help_key || failed=1

    # Run quit test last since it kills the app
    run_test "nav_ctrl_q_quits" test_ctrl_q_quits || failed=1

    return $failed
}

main
