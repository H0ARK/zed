# Test Context Management

This is a test file to verify that our context management integration is working properly.

## Features to Test

1. **Context Optimization**: The system should optimize context when approaching token limits
2. **FileDiff Context**: New context type that supports diff-based compression
3. **Smart Compression**: Keeps recent messages and compresses older ones
4. **Memory Efficiency**: Should show improved memory usage

## Test Scenarios

### Scenario 1: Large Context Window
When we have many files and messages, the system should:
- Detect when approaching 70% of token limit
- Apply context optimization strategies
- Compress older messages while preserving recent ones
- Use FileDiff context for file changes

### Scenario 2: Diff Generation
When files are modified, the system should:
- Generate smart diffs using KEEP_BOTH, REPLACE_WITH_DIFF, or DIFF_MARKER_ONLY strategies
- Store original content externally when beneficial
- Provide contextual analysis for changes

### Scenario 3: Dynamic Zone Management
The system should partition context windows:
- Files: 25%
- Terminal: 25% 
- Chat: 40%
- Tasks: 10%

This file contains enough content to test basic functionality. 