# Zed Agent CLI

The Zed CLI now supports an agent mode that allows you to interact with AI assistants directly from the command line in a headless environment.

## Usage

```bash
zed --agent --message "Your message here" [OPTIONS] [FILES...]
```

## Options

- `--agent`: Enable agent mode for AI assistance
- `--message <MESSAGE>`: Message to send to the agent (optional - defaults to a help request)
- `--provider <PROVIDER>`: Specify the AI provider (e.g., anthropic, openai)
- `--model <MODEL>`: Specify the model to use (e.g., claude-3-sonnet, gpt-4)

## Examples

### Basic Usage

```bash
# Simple help request
zed --agent --message "Hello, can you help me?"

# Ask about code
zed --agent --message "Can you help me understand this code?" src/main.rs

# Use specific provider and model
zed --agent --message "Review this function" --provider anthropic --model claude-3-sonnet src/lib.rs
```

### Working with Files

```bash
# Analyze files in a directory
zed --agent --message "What files are in this project?" .

# Get help with a specific file
zed --agent --message "Explain this code" path/to/file.rs

# Working directory detection
zed --agent --message "Review this code" src/main.rs
# The agent will use 'src' as the working directory
```

### Default Behavior

```bash
# Without a message, defaults to general help
zed --agent

# With file paths but no message, offers file assistance
zed --agent src/main.rs
```

## Features

- **Headless Operation**: Runs without a GUI, perfect for terminal environments
- **Working Directory Detection**: Automatically detects working directory from file paths
- **Provider Selection**: Support for multiple AI providers
- **Model Configuration**: Specify which AI model to use
- **File Context**: Understands file paths and project structure

## Response Format

The agent provides formatted responses with emojis for better readability:

- 🤖 Agent responses
- 📁 Working directory information
- 🧠 Model/provider information
- 💭 Processing status
- ✅ Completion status
- ⚠️ Warnings or fallback information

## Integration

The agent CLI integrates with Zed's existing infrastructure:

- Uses the same language model registry as the GUI
- Supports all configured providers and models
- Inherits project understanding capabilities
- Works with Zed's file system and project detection

## Development

For development builds:

```bash
# Build the CLI
cargo build --package cli

# Build Zed
cargo build --package zed --no-default-features

# Test with development binaries
target/debug/cli --zed target/debug/zed --agent --message "Hello"
```

## Limitations

- Currently provides simple responses (full agent capabilities to be implemented)
- Requires a Zed binary to be available
- Language model providers must be configured in Zed settings

## Future Enhancements

- Full agent conversation capabilities
- File reading and analysis
- Code generation and modification
- Integration with language servers
- Streaming responses for long operations
- Interactive mode for multi-turn conversations