#!/bin/bash
# Run all E2E tests
# Usage: ./tests/e2e/run_all.sh [test_pattern]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR/../.."

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

# Parse arguments
TEST_PATTERN="${1:-test_*.sh}"
VERBOSE="${VERBOSE:-0}"

echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Conduit E2E Tests${NC}"
echo -e "${BLUE}========================================${NC}"
echo

# Check if conduit binary exists
if [ ! -f "./target/release/conduit" ]; then
    echo -e "${YELLOW}Building conduit in release mode...${NC}"
    cargo build --release
fi

# Find all test files
TEST_FILES=$(find "$SCRIPT_DIR" -name "$TEST_PATTERN" -type f | sort)

if [ -z "$TEST_FILES" ]; then
    echo -e "${RED}No test files found matching: $TEST_PATTERN${NC}"
    exit 1
fi

# Count tests
TOTAL=$(echo "$TEST_FILES" | wc -l | tr -d ' ')
PASSED=0
FAILED=0
FAILED_TESTS=()

echo -e "Found ${TOTAL} test file(s)"
echo

# Run each test file
for test_file in $TEST_FILES; do
    test_name=$(basename "$test_file" .sh)
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${BLUE}Running: $test_name${NC}"
    echo -e "${BLUE}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"

    if bash "$test_file"; then
        PASSED=$((PASSED + 1))
        echo -e "${GREEN}✓ $test_name passed${NC}"
    else
        FAILED=$((FAILED + 1))
        FAILED_TESTS+=("$test_name")
        echo -e "${RED}✗ $test_name failed${NC}"
    fi
    echo
done

# Summary
echo -e "${BLUE}========================================${NC}"
echo -e "${BLUE}  Summary${NC}"
echo -e "${BLUE}========================================${NC}"
echo -e "Total:  $TOTAL"
echo -e "${GREEN}Passed: $PASSED${NC}"
echo -e "${RED}Failed: $FAILED${NC}"

if [ $FAILED -gt 0 ]; then
    echo
    echo -e "${RED}Failed tests:${NC}"
    for test in "${FAILED_TESTS[@]}"; do
        echo -e "  - $test"
    done
    echo
    echo -e "${YELLOW}Screenshots saved to: tests/e2e/screenshots/${NC}"
    exit 1
fi

echo
echo -e "${GREEN}All tests passed!${NC}"
exit 0
