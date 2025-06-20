#!/bin/bash

# Launch Zed with The Hub integrated
echo "🌟 Launching Zed with The Hub Integration"
echo "========================================"

# Build Zed with Hub integration if needed
if [ ! -f "target/debug/zed" ]; then
    echo "🔧 Building Zed with Hub integration..."
    cargo build
    if [ $? -ne 0 ]; then
        echo "❌ Failed to build Zed"
        exit 1
    fi
fi

# Build Hub CLI tool if needed
if [ ! -f "hub-cli/target/debug/hub-demo-cli" ]; then
    echo "🔧 Building Hub CLI demo tool..."
    cargo build --manifest-path hub-cli/Cargo.toml
    if [ $? -ne 0 ]; then
        echo "❌ Failed to build Hub CLI tool"
        exit 1
    fi
fi

echo ""
echo "🚀 Starting Zed with automatic Hub server..."
echo ""
echo "📝 What happens when you start Zed:"
echo "   • The Hub server starts automatically in the background"
echo "   • All terminals created in Zed have Hub capabilities"
echo "   • Hub terminal engine is initialized"
echo ""
echo "🧪 To test The Hub:"
echo "   1. Open a terminal in Zed (Terminal → New Terminal)"
echo "   2. Run: ./hub-cli/target/debug/hub-demo-cli progress"
echo "   3. Watch the Hub protocol communication in action!"
echo ""
echo "🎯 The Hub transforms CLI tools into rich, interactive applications"
echo "   while maintaining full backward compatibility."
echo ""

# Launch Zed
./target/debug/zed