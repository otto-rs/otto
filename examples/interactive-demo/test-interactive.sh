#!/bin/bash
# Test script to demonstrate interactive TTY support in Otto

set -e

echo "====================================="
echo "Otto Interactive TTY Demo"
echo "====================================="
echo ""

# Check if in a TTY
if [ ! -t 0 ]; then
    echo "ERROR: This script must be run from a terminal (TTY)"
    echo "       Don't pipe or redirect input to this script"
    exit 1
fi

OTTO="../../target/release/otto"

if [ ! -f "$OTTO" ]; then
    echo "ERROR: Otto binary not found at $OTTO"
    echo "       Run: cargo build --release"
    exit 1
fi

echo "This demo will show Otto's interactive TTY support."
echo "Each test requires your interaction."
echo ""

# Test 1: Simple read
echo "TEST 1: Simple Input"
echo "---------------------"
echo "This will ask for your name..."
sleep 1
$OTTO read-input
echo ""

# Test 2: Colored menu
echo "TEST 2: Colored Menu"
echo "--------------------"
echo "Choose an option from the menu..."
sleep 1
$OTTO colored-menu
echo ""

# Test 3: Check history
echo "TEST 3: Check History"
echo "---------------------"
echo "Viewing task history (including interactive flag)..."
$OTTO History read-input -n 5 || echo "Run this manually if you want to see history"
echo ""

# Test 4: Check logs
echo "TEST 4: Check Interactive Logs"
echo "-------------------------------"
echo "Looking for interactive session logs..."
LATEST_LOG=$(find ~/.otto -name "interactive.log" -type f 2>/dev/null | tail -1)
if [ -n "$LATEST_LOG" ]; then
    echo "Found log: $LATEST_LOG"
    echo "Contents:"
    cat "$LATEST_LOG"
else
    echo "No interactive logs found yet"
fi
echo ""

echo "====================================="
echo "Demo complete!"
echo ""
echo "Other interactive tasks to try:"
echo "  $OTTO shell           # Full bash shell"
echo "  $OTTO vim-edit        # Edit file with vim"
echo "  $OTTO python-interactive  # Python REPL"
echo "  $OTTO top-monitor     # System monitor"
echo ""
echo "All these tasks have 'interactive: true' in otto.yml"
echo "====================================="


