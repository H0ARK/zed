# Context Manager

A sophisticated conversation context management system with **three distinct strategies** optimized for different use cases, providing intelligent conversation summarization, advanced file edit tracking, and memory-efficient context preservation.

## Features

### Core Features (All Strategies)
- **Token Counting**: Accurate token estimation for conversation management
- **Conversation Compaction**: Intelligent summarization when approaching context limits
- **File State Management**: Track and restore file context across conversations
- **Auto-Compaction**: Automatic conversation management when thresholds are exceeded
- **Threshold Monitoring**: Multiple warning levels for context window usage
- **Detailed Summarization**: Preserves technical details, code snippets, and architectural decisions

### Advanced Features (Strategy-Specific)
- **🚀 Pointer-Based Strategy**: 40-80% memory savings with 85-90% context preservation
- **🎯 Hybrid Edit Tracking**: Smart diff strategies (KEEP_BOTH, REPLACE_WITH_DIFF, DIFF_MARKER_ONLY)
- **📊 Real-time Analytics**: Memory efficiency and context quality metrics
- **🏗️ Dynamic Zone Management**: Partitioned context windows for multi-context workflows
- **⚡ Smart Compression**: Context-aware compression based on usage patterns

## Quick Start

### Automatic Strategy Selection (Recommended)

```javascript
import { createOptimalContextManager } from "./context-manager/index.js";

// Let the system choose the best strategy for your use case
const contextManager = createOptimalContextManager({
  projectType: 'web-development',     // 'general', 'web-development', 'terminal-heavy'
  expectedFileCount: 25,              // Number of files you'll be working with
  expectedSessionLength: 'long',      // 'short', 'medium', 'long'
  memoryEfficiencyPriority: 'high',   // 'low', 'balanced', 'high'
  
  // Standard options
  autoCompactEnabled: true,
  onStatusUpdate: (status) => console.log("Status:", status),
  onError: (error) => console.error("Error:", error),
  callAI: async (messages, context) => {
    return await yourAIService.call(messages);
  },
});

console.log("Selected strategy:", contextManager.config.selectedStrategy);
console.log("Reason:", contextManager.config.selectionReason);
```

### Manual Strategy Selection

```javascript
import { createContextManager, CONTEXT_STRATEGIES } from "./context-manager/index.js";

// For heavy file editing (recommended for development)
const pointerBased = createContextManager(CONTEXT_STRATEGIES.POINTER_BASED, {
  maxTokens: 180000,
  safetyThreshold: 0.7,
  functionHeadersOnly: true
});

// For terminal-heavy workflows
const dynamicZones = createContextManager(CONTEXT_STRATEGIES.DYNAMIC_ZONES, {
  fileZonePercentage: 0.25,
  terminalZonePercentage: 0.25,
  chatZonePercentage: 0.4,
  taskZonePercentage: 0.1
});

// For simple conversations
const traditional = createContextManager(CONTEXT_STRATEGIES.TRADITIONAL, {
  autoCompactEnabled: true
});
```

### Basic Usage Example

```javascript
// Using the pointer-based strategy for file editing
const contextManager = createContextManager(CONTEXT_STRATEGIES.POINTER_BASED);

// Add a file
contextManager.updateFile('src/auth.js', `
function login(username, password) {
  return bcrypt.compare(password, user.hashedPassword);
}
`, { type: 'javascript' });

// Add a message
contextManager.addMessage({
  role: 'user',
  content: 'Update the login function to use async/await'
});

// Update the file (automatically triggers smart diff strategy)
contextManager.updateFile('src/auth.js', `
async function login(username, password) {
  return await bcrypt.compare(password, user.hashedPassword);
}
`);

// Get context with efficiency metrics
const context = contextManager.getCurrentContext();
console.log("Memory savings:", context.metadata.efficiencyGains.memorySavingsPercentage + "%");
console.log("Context quality:", context.metadata.contextQuality);
```

## Architecture

### Core Components

#### ContextManager

The main facade class that provides a unified interface for all context management operations.

#### ConversationCompactor

Handles the core compaction logic, including summarization and message reconstruction.

#### TokenCounter

Provides token counting and threshold checking functionality.

#### FileStateManager

Manages file state tracking and restoration during compaction.

#### SummarizationPrompts

Generates and formats prompts for conversation summarization.

### Configuration

```javascript
const config = {
  // Context window settings
  MAX_TOKENS: 180000,
  COMPACT_THRESHOLD: 0.92,
  AUTO_COMPACT_LOW_WATER_MARK: 0.6,
  AUTO_COMPACT_HIGH_WATER_MARK: 0.8,

  // Behavior settings
  autoCompactEnabled: true,
  preserveLastNMessages: 2,
  includeFileStateRestoration: true,
  enableDetailedSummarization: true,
};
```

## API Reference

### ContextManager

#### Constructor

```javascript
new ContextManager(options);
```

Options:

- `autoCompactEnabled`: Enable automatic compaction (default: true)
- `onStatusUpdate`: Callback for status updates
- `onError`: Callback for errors
- `onWarning`: Callback for warnings
- `callAI`: Function to call AI service for summarization

#### Methods

##### analyzeConversation(messages)

Analyzes the current conversation state and returns comprehensive information.

```javascript
const analysis = contextManager.analyzeConversation(messages);
// Returns: { tokenCount, thresholds, messageCount, fileStats, warnings, recommendations }
```

##### shouldAutoCompact(messages)

Checks if auto-compaction should be triggered.

```javascript
const shouldCompact = contextManager.shouldAutoCompact(messages);
```

##### compactConversation(messages, executionContext, options)

Performs conversation compaction with summarization.

```javascript
const result = await contextManager.compactConversation(messages, context, {
  forceCompact: false,
  customInstructions: "Focus on code changes",
});
```

##### File Management

```javascript
// Add file to state
contextManager.addFile("/path/to/file.js", {
  content: "...",
  type: "javascript",
});

// Get file from state
const file = contextManager.getFile("/path/to/file.js");

// Get all files
const allFiles = contextManager.getAllFiles();

// Clear file state
contextManager.clearFileState();
```

### Token Counting

```javascript
import { TokenCounter } from "./context-manager/index.js";

const tokenCounter = new TokenCounter();
const count = tokenCounter.calculateTokenCount(messages);
const thresholds = tokenCounter.checkThresholds(count, 180000);
```

### File State Management

```javascript
import { FileStateManager } from "./context-manager/index.js";

const fileManager = new FileStateManager();
fileManager.setFile("/path/to/file.js", {
  content: 'console.log("hello");',
  type: "javascript",
  lastModified: Date.now(),
});

const stats = fileManager.getStatistics();
```

## Summarization System

The summarization system creates detailed summaries that preserve:

1. **Primary Request and Intent**: User's explicit requests
2. **Key Technical Concepts**: Technologies and frameworks discussed
3. **Files and Code Sections**: Specific files with code snippets
4. **Problem Solving**: Issues resolved and troubleshooting
5. **Pending Tasks**: Outstanding work items
6. **Current Work**: Precise description of recent work
7. **Next Steps**: Recommended actions

### Custom Summarization Instructions

```javascript
const result = await contextManager.compactConversation(messages, context, {
  customInstructions: `
    ## Custom Instructions
    Focus on TypeScript code changes and remember any mistakes made.
    Include test output and file reads verbatim.
  `,
});
```

## Thresholds and Warnings

The system provides multiple warning levels:

- **70% (Warning)**: High usage notification
- **80% (Auto-compact)**: Triggers automatic compaction
- **90% (Error)**: Critical - new requests may fail
- **92% (Compact)**: Forces compaction

## File State Restoration

After compaction, the system automatically:

1. Analyzes the summary for mentioned files
2. Checks recent messages for file references
3. Restores relevant files to maintain context
4. Provides restoration report

## Error Handling

The system includes comprehensive error handling:

```javascript
const contextManager = new ContextManager({
  onError: (error) => {
    console.error("Context Manager Error:", error);
    // Handle error appropriately
  },
});
```

## Integration Example

```javascript
// Complete integration example
import { ContextManager } from "./context-manager/index.js";

class ChatApplication {
  constructor() {
    this.messages = [];
    this.contextManager = new ContextManager({
      autoCompactEnabled: true,
      onStatusUpdate: this.handleStatusUpdate.bind(this),
      onError: this.handleError.bind(this),
      onWarning: this.handleWarning.bind(this),
      callAI: this.callAI.bind(this),
    });
  }

  async sendMessage(userMessage) {
    this.messages.push({ role: "user", content: userMessage });

    // Check if compaction is needed
    if (this.contextManager.shouldAutoCompact(this.messages)) {
      const result = await this.contextManager.compactConversation(
        this.messages,
        this.getExecutionContext()
      );

      if (result.success) {
        this.messages = result.messages;
        this.showSummary(result.summaryMessage);
      }
    }

    // Get AI response
    const response = await this.callAI(
      this.messages,
      this.getExecutionContext()
    );
    this.messages.push({ role: "assistant", content: response.content });

    return response;
  }

  async callAI(messages, context) {
    // Your AI service integration
    return await yourAIService.chat(messages);
  }

  handleStatusUpdate(status) {
    if (status) {
      this.showStatus(status);
    } else {
      this.hideStatus();
    }
  }

  handleError(error) {
    this.showError(error);
  }

  handleWarning(warning) {
    this.showWarning(warning);
  }

  getExecutionContext() {
    return {
      // Your execution context
    };
  }
}
```

## License

Based on the Claude Code CLI context management system. See original license for details.
