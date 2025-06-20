#!/bin/bash
set -e

echo "🚀 Zed Editor Build Setup Script"
echo "================================="

# Detect OS
if [[ "$OSTYPE" == "linux-gnu"* ]]; then
    OS="linux"
elif [[ "$OSTYPE" == "darwin"* ]]; then
    OS="macos"
elif [[ "$OSTYPE" == "msys" ]] || [[ "$OSTYPE" == "cygwin" ]]; then
    OS="windows"
else
    echo "❌ Unsupported OS: $OSTYPE"
    exit 1
fi

echo "📋 Detected OS: $OS"

# Install Rust toolchain
install_rust() {
    echo "🦀 Installing Rust toolchain..."
    if ! command -v rustup &> /dev/null; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source ~/.cargo/env
    fi
    
    # Set specific Rust version (from rust-toolchain.toml)
    rustup toolchain install 1.87
    rustup default 1.87
    
    # Add required components
    rustup component add rustfmt clippy
    
    # Add target platforms
    rustup target add wasm32-wasip2
    
    if [[ "$OS" == "macos" ]]; then
        rustup target add x86_64-apple-darwin aarch64-apple-darwin
    elif [[ "$OS" == "linux" ]]; then
        rustup target add x86_64-unknown-linux-gnu x86_64-unknown-linux-musl
    fi
    
    echo "✅ Rust toolchain installed"
}

# Linux dependencies
install_linux_deps() {
    echo "🐧 Installing Linux system dependencies..."
    
    if command -v apt &> /dev/null; then
        # Ubuntu/Debian/Mint
        sudo apt update
        sudo apt install -y \
            gcc g++ libasound2-dev libfontconfig-dev libwayland-dev \
            libx11-xcb-dev libxkbcommon-x11-dev libssl-dev libzstd-dev \
            libvulkan1 libgit2-dev make cmake clang jq git curl \
            gettext-base elfutils libsqlite3-dev musl-tools musl-dev \
            build-essential pkg-config
            
    elif command -v dnf &> /dev/null; then
        # Fedora/CentOS/RHEL
        sudo dnf install -y \
            musl-gcc gcc clang cmake alsa-lib-devel fontconfig-devel \
            wayland-devel libxcb-devel libxkbcommon-x11-devel \
            openssl-devel libzstd-devel vulkan-loader sqlite-devel \
            jq git tar pkgconfig
            
    elif command -v pacman &> /dev/null; then
        # Arch/Manjaro
        sudo pacman -S --needed \
            gcc clang musl cmake alsa-lib fontconfig wayland libgit2 \
            libxcb libxkbcommon-x11 openssl zstd pkgconf mold sqlite \
            jq git
            
    elif command -v zypper &> /dev/null; then
        # openSUSE
        sudo zypper install -y \
            alsa-devel clang cmake fontconfig-devel gcc gcc-c++ git \
            gzip jq libvulkan1 libxcb-devel libxkbcommon-devel \
            libxkbcommon-x11-devel libzstd-devel make mold openssl-devel \
            sqlite3-devel tar wayland-devel xcb-util-devel pkg-config
    else
        echo "❌ Unsupported Linux distribution. Please install dependencies manually."
        exit 1
    fi
    
    echo "✅ Linux dependencies installed"
}

# macOS dependencies
install_macos_deps() {
    echo "🍎 Installing macOS dependencies..."
    
    # Install Xcode Command Line Tools
    if ! xcode-select -p &> /dev/null; then
        echo "Installing Xcode Command Line Tools..."
        xcode-select --install
        echo "⚠️  Please complete the Xcode Command Line Tools installation and re-run this script"
        exit 1
    fi
    
    # Install Homebrew if not present
    if ! command -v brew &> /dev/null; then
        echo "Installing Homebrew..."
        /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
    fi
    
    # Install CMake
    brew install cmake
    
    # Configure Xcode
    if [[ -d "/Applications/Xcode.app" ]]; then
        sudo xcode-select --switch /Applications/Xcode.app/Contents/Developer
        sudo xcodebuild -license accept || true
        export BINDGEN_EXTRA_CLANG_ARGS="--sysroot=$(xcrun --show-sdk-path)"
    fi
    
    echo "✅ macOS dependencies installed"
}

# Install Node.js (required for some build processes)
install_nodejs() {
    echo "📦 Installing Node.js..."
    
    if ! command -v node &> /dev/null; then
        if [[ "$OS" == "linux" ]]; then
            # Install via NodeSource repository
            curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
            sudo apt-get install -y nodejs
        elif [[ "$OS" == "macos" ]]; then
            brew install node@22
        fi
    fi
    
    # Verify Node.js version
    NODE_VERSION=$(node --version | cut -d'v' -f2 | cut -d'.' -f1)
    if [[ $NODE_VERSION -lt 22 ]]; then
        echo "⚠️  Node.js version 22+ required, found version $NODE_VERSION"
    else
        echo "✅ Node.js $(node --version) installed"
    fi
}

# Install additional development tools
install_dev_tools() {
    echo "🔧 Installing development tools..."
    
    # Install sqlx-cli for database operations
    cargo install sqlx-cli --version 0.7.2
    
    echo "✅ Development tools installed"
}

# Setup project
setup_project() {
    echo "📁 Setting up project..."
    
    # Run bootstrap script if it exists
    if [[ -f "./script/bootstrap" ]]; then
        ./script/bootstrap
    fi
    
    # Run Linux setup script if on Linux
    if [[ "$OS" == "linux" && -f "./script/linux" ]]; then
        ./script/linux
    fi
    
    echo "✅ Project setup complete"
}

# Test build
test_build() {
    echo "🔨 Testing build..."
    
    # Check if we can compile
    echo "Running cargo check..."
    cargo check
    
    echo "Running cargo clippy..."
    ./script/clippy || cargo clippy --workspace
    
    echo "✅ Build test successful"
}

# Main execution
main() {
    echo "Starting setup for $OS..."
    
    # Install Rust
    install_rust
    
    # Install platform-specific dependencies
    case $OS in
        linux)
            install_linux_deps
            ;;
        macos)
            install_macos_deps
            ;;
        windows)
            echo "❌ Windows setup not implemented in this script"
            echo "Please install Visual Studio 2019/2022 with C++ tools and Windows SDK"
            exit 1
            ;;
    esac
    
    # Install Node.js
    install_nodejs
    
    # Install development tools
    install_dev_tools
    
    # Setup project
    setup_project
    
    # Test build
    test_build
    
    echo ""
    echo "🎉 Setup complete! You can now build Zed with:"
    echo "   cargo run                    # Debug build"
    echo "   cargo run --release          # Release build"
    echo "   cargo test --workspace       # Run tests"
    echo ""
    echo "💡 For development, also consider:"
    echo "   ./script/clippy              # Run linting"
    echo "   cargo run -p cli             # Build CLI tool"
}

# Run main function
main "$@"