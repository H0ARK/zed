#!/bin/bash

echo "Testing shell integration..."

# Test 1: Simple command that should work
echo "Test 1: Simple echo command"
echo "Hello from shell integration!"

# Test 2: Command that reads from stdin (this would fail with </dev/null)
echo "Test 2: Reading from stdin"
echo "This is a test" | while read line; do
    echo "Read: $line"
done

# Test 3: Interactive-style command (simulated)
echo "Test 3: Simulated interactive command"
echo "Would you like to continue? (y/n)"
echo "y" | {
    read answer
    if [ "$answer" = "y" ]; then
        echo "Continuing..."
    else
        echo "Stopping..."
    fi
}

# Test 4: Check if OSC sequences are present
echo "Test 4: Checking for OSC 133 sequences in output"
echo "If shell integration is working, you should see OSC sequences above this line"

echo "All tests completed!"
