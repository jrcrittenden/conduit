#!/bin/bash
# Test: Adding a project via Ctrl+N
# Verifies the project addition workflow

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib.sh"

# Test: Ctrl+N opens the "Set Projects Directory" dialog
test_ctrl_n_opens_dialog() {
    local sock="$1"
    local data_dir="$2"

    # Press Ctrl+N
    ctrl "$sock" "n"
    wait_idle "$sock" 300 3000 > /dev/null

    # Should show the directory dialog
    assert_contains "$sock" "Set Projects Directory" "Ctrl+N opens directory dialog" || return 1
    assert_contains "$sock" "Where do you keep your projects" "Shows directory prompt" || return 1

    return 0
}

# Test: Directory dialog shows input field
test_directory_input() {
    local sock="$1"
    local data_dir="$2"

    # Press Ctrl+N to open dialog
    ctrl "$sock" "n"
    wait_idle "$sock" 300 3000 > /dev/null

    # Check for input field hints
    assert_contains "$sock" "Enter confirm" "Shows Enter hint" || return 1
    assert_contains "$sock" "Esc cancel" "Shows Escape hint" || return 1

    return 0
}

# Test: Escape cancels the dialog
test_escape_cancels() {
    local sock="$1"
    local data_dir="$2"

    # Open dialog with Ctrl+N
    ctrl "$sock" "n"
    wait_idle "$sock" 300 3000 > /dev/null

    # Verify dialog is open
    assert_contains "$sock" "Set Projects Directory" "Dialog is open" || return 1

    # Press Escape
    press "$sock" "Escape"
    wait_idle "$sock" 300 3000 > /dev/null

    # Dialog should be closed - back to main screen
    assert_not_contains "$sock" "Set Projects Directory" "Dialog closed after Escape" || return 1
    assert_contains "$sock" "C-n new project" "Back to main screen" || return 1

    return 0
}

# Test: Can type in the directory input
test_type_directory() {
    local sock="$1"
    local data_dir="$2"

    # Open dialog with Ctrl+N
    ctrl "$sock" "n"
    wait_idle "$sock" 300 3000 > /dev/null

    # Clear existing text and type new path
    # Use Ctrl+U to clear the line
    ctrl "$sock" "u"
    wait_idle "$sock" 100 1000 > /dev/null

    # Type a path
    type_text "$sock" "/tmp"
    wait_idle "$sock" 300 3000 > /dev/null

    # Should show what we typed
    assert_contains "$sock" "/tmp" "Input shows typed path" || return 1

    # Cancel to clean up
    press "$sock" "Escape"

    return 0
}

# Run all tests
main() {
    local failed=0

    run_test "project_ctrl_n_dialog" test_ctrl_n_opens_dialog || failed=1
    run_test "project_directory_input" test_directory_input || failed=1
    run_test "project_escape_cancels" test_escape_cancels || failed=1
    run_test "project_type_directory" test_type_directory || failed=1

    return $failed
}

main
