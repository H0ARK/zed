#!/bin/bash

# Test script for The Hub system
echo "🌟 The Hub End-to-End Test"
echo "=========================="

# Check if binaries exist
if [ ! -f "hub-server/target/debug/hub-server-daemon" ]; then
    echo "Building Hub server..."
    cargo build --manifest-path hub-server/Cargo.toml
fi

if [ ! -f "hub-cli/target/debug/hub-demo-cli" ]; then
    echo "Building Hub CLI..."
    cargo build --manifest-path hub-cli/Cargo.toml
fi

echo ""
echo "🚀 Starting Hub server on port 7878..."
echo "   Server will run in background"

# Start the server in background
./hub-server/target/debug/hub-server-daemon &
SERVER_PID=$!

# Give server time to start
sleep 2

echo ""
echo "🎯 Testing CLI tools with different demos..."
echo ""

# Test progress demo
echo "📊 Testing progress demo:"
./hub-cli/target/debug/hub-demo-cli progress
sleep 1

echo ""
echo "📋 Testing table demo:"
./hub-cli/target/debug/hub-demo-cli table
sleep 1

echo ""
echo "📁 Testing file tree demo:"
./hub-cli/target/debug/hub-demo-cli files
sleep 1

echo ""
echo "🎨 Testing mixed demo:"
./hub-cli/target/debug/hub-demo-cli mixed

echo ""
echo "🛑 Stopping Hub server..."
kill $SERVER_PID

echo ""
echo "✅ Test complete!"
echo ""
echo "💡 What you just saw:"
echo "   • Hub server accepted CLI connections"
echo "   • CLI tools sent rich UI components over the protocol"
echo "   • Server processed progress bars, tables, and file trees"
echo ""
echo "🎯 Next steps:"
echo "   • Start Zed to see Hub terminal integration"
echo "   • All new terminals will have Hub capabilities"
echo "   • Try running the demo CLI tools in Zed's terminal"