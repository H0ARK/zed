# Token Calculation Fix

## Problem

The previous token calculation was severely underestimating actual token usage because it only counted the raw message content strings, ignoring all the JSON structure overhead that gets sent to the Anthropic API.

## What Was Missing

The old calculation didn't account for:
- Role information (`"role": "user"`, `"role": "assistant"`)
- Content type wrappers (`"type": "text"`)
- Tool use/result structures
- Cache control information
- System prompts and tool definitions
- Model parameters (temperature, max_tokens, etc.)
- JSON structure overhead

## Solution

Added new methods to `TokenCounter` that calculate tokens based on the actual serialized JSON request that gets sent to the API:

### New Methods

1. **`calculateTokensForRequestString(requestString)`**
   - Most accurate method - counts tokens on the actual request string
   - Use this when you have the serialized JSON request

2. **`calculateTokensForApiRequest(messages, options)`**
   - Constructs the full API request structure and counts tokens
   - Accounts for system prompts, tools, and all request parameters

3. **`convertToApiMessages(messages)`**
   - Converts internal message format to Anthropic API format
   - Handles text, images, tool use, and tool results

### Updated Behavior

- `calculateTokenCount(messages, options)` now uses the accurate method by default
- `calculateTokenCountLegacy(messages)` preserves the old content-only calculation
- Backward compatible - existing code will automatically get more accurate counts

## Impact

Test results show the difference:
- **Legacy method**: 145 tokens (content only)
- **New method**: 266 tokens (full API request)
- **Difference**: +121 tokens (83.4% increase!)

This explains why token usage was being severely underreported.

## Usage

```javascript
import { TokenCounter } from './TokenCounter.js';

const tokenCounter = new TokenCounter();

// Most accurate - use the actual request string
const tokens1 = tokenCounter.calculateTokensForRequestString(requestJsonString);

// Accurate - construct full API request
const tokens2 = tokenCounter.calculateTokensForApiRequest(messages, {
  system: 'You are a helpful assistant.',
  tools: [...],
  tool_choice: 'auto',
  temperature: 0.7
});

// Default method now uses accurate calculation
const tokens3 = tokenCounter.calculateTokenCount(messages, options);

// Legacy method for comparison
const legacyTokens = tokenCounter.calculateTokenCountLegacy(messages);
```

## Testing

Run the test suite to verify functionality:

```bash
cd context-manager
node TokenCounter.test.js
node test-token-calculation.js
```

## Files Changed

- `context-manager/TokenCounter.js` - Added new accurate calculation methods
- `context-manager/TokenCounter.test.js` - Comprehensive test suite
- `context-manager/test-token-calculation.js` - Demonstration script
- `TOKEN_CALCULATION_FIX.md` - This documentation

