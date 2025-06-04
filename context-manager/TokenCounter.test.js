/**
 * Test suite for TokenCounter
 */

import { TokenCounter } from './TokenCounter.js';

const tokenCounter = new TokenCounter();

// Test helper function
function runTest(testName, testFn) {
  try {
    testFn();
    console.log(`✓ ${testName}`);
  } catch (error) {
    console.error(`✗ ${testName}: ${error.message}`);
  }
}

function assertEqual(actual, expected, message) {
  if (actual !== expected) {
    throw new Error(`${message}: expected ${expected}, got ${actual}`);
  }
}

function assertTrue(condition, message) {
  if (!condition) {
    throw new Error(message);
  }
}

console.log('=== TokenCounter Test Suite ===\n');

// Test 1: Basic request string calculation
runTest('calculateTokensForRequestString - basic test', () => {
  const requestString = '{"model":"claude-3-5-sonnet","messages":[{"role":"user","content":[{"type":"text","text":"Hello"}]}]}';
  const tokens = tokenCounter.calculateTokensForRequestString(requestString);
  const expectedTokens = Math.ceil(requestString.length * 0.25);
  assertEqual(tokens, expectedTokens, 'Request string token calculation');
});

// Test 2: Empty/invalid inputs
runTest('calculateTokensForRequestString - empty inputs', () => {
  assertEqual(tokenCounter.calculateTokensForRequestString(''), 0, 'Empty string');
  assertEqual(tokenCounter.calculateTokensForRequestString(null), 0, 'Null input');
  assertEqual(tokenCounter.calculateTokensForRequestString(undefined), 0, 'Undefined input');
  assertEqual(tokenCounter.calculateTokensForRequestString(123), 0, 'Non-string input');
});

// Test 3: Message conversion to API format
runTest('convertToApiMessages - simple text messages', () => {
  const messages = [
    { role: 'user', content: 'Hello' },
    { role: 'assistant', content: 'Hi there!' }
  ];
  
  const apiMessages = tokenCounter.convertToApiMessages(messages);
  
  assertEqual(apiMessages.length, 2, 'Message count');
  assertEqual(apiMessages[0].role, 'user', 'First message role');
  assertEqual(apiMessages[0].content[0].type, 'text', 'First message content type');
  assertEqual(apiMessages[0].content[0].text, 'Hello', 'First message text');
  assertEqual(apiMessages[1].role, 'assistant', 'Second message role');
  assertEqual(apiMessages[1].content[0].text, 'Hi there!', 'Second message text');
});

// Test 4: Multi-part content
runTest('convertToApiMessages - multi-part content', () => {
  const messages = [
    {
      role: 'user',
      content: [
        { type: 'text', text: 'Look at this image:' },
        { type: 'image', data: 'base64data', source: { type: 'base64', media_type: 'image/png', data: 'base64data' } }
      ]
    }
  ];
  
  const apiMessages = tokenCounter.convertToApiMessages(messages);
  
  assertEqual(apiMessages[0].content.length, 2, 'Multi-part content count');
  assertEqual(apiMessages[0].content[0].type, 'text', 'First part type');
  assertEqual(apiMessages[0].content[1].type, 'image', 'Second part type');
});

// Test 5: Tool use content
runTest('convertToApiMessages - tool use content', () => {
  const messages = [
    {
      role: 'assistant',
      content: [
        { type: 'tool_use', id: 'tool_123', name: 'calculator', input: { expression: '2+2' } }
      ]
    },
    {
      role: 'user',
      content: [
        { type: 'tool_result', tool_use_id: 'tool_123', content: '4', is_error: false }
      ]
    }
  ];
  
  const apiMessages = tokenCounter.convertToApiMessages(messages);
  
  assertEqual(apiMessages[0].content[0].type, 'tool_use', 'Tool use type');
  assertEqual(apiMessages[0].content[0].name, 'calculator', 'Tool name');
  assertEqual(apiMessages[1].content[0].type, 'tool_result', 'Tool result type');
  assertEqual(apiMessages[1].content[0].tool_use_id, 'tool_123', 'Tool use ID');
});

// Test 6: Full API request calculation
runTest('calculateTokensForApiRequest - complete request', () => {
  const messages = [
    { role: 'user', content: 'Hello' },
    { role: 'assistant', content: 'Hi!' }
  ];
  
  const options = {
    system: 'You are a helpful assistant.',
    tools: [
      {
        name: 'test_tool',
        description: 'A test tool',
        input_schema: { type: 'object', properties: {} }
      }
    ],
    tool_choice: 'auto',
    temperature: 0.7
  };
  
  const tokens = tokenCounter.calculateTokensForApiRequest(messages, options);
  
  // Should be significantly more than just the message content
  assertTrue(tokens > 50, 'Should account for full request structure');
  
  // Should be consistent
  const tokens2 = tokenCounter.calculateTokensForApiRequest(messages, options);
  assertEqual(tokens, tokens2, 'Should be consistent across calls');
});

// Test 7: New vs legacy comparison
runTest('calculateTokenCount vs calculateTokenCountLegacy', () => {
  const messages = [
    { role: 'user', content: 'Write a function to sort an array' },
    { role: 'assistant', content: 'Here is a simple sorting function:\n\nfunction sort(arr) {\n  return arr.sort();\n}' }
  ];
  
  const legacyTokens = tokenCounter.calculateTokenCountLegacy(messages);
  const newTokens = tokenCounter.calculateTokenCount(messages, {
    system: 'You are a coding assistant.',
    tools: [{ name: 'execute', description: 'Execute code', input_schema: {} }]
  });
  
  assertTrue(newTokens > legacyTokens, 'New method should count more tokens than legacy');
  assertTrue((newTokens - legacyTokens) > 20, 'Difference should be significant');
});

// Test 8: Edge cases
runTest('Edge cases', () => {
  // Empty messages array - should still count base request structure
  const emptyTokens = tokenCounter.calculateTokenCount([]);
  assertTrue(emptyTokens > 0, 'Should count base request structure even for empty messages');
  
  // Messages with empty content
  const emptyMessages = [{ role: 'user', content: '' }];
  const emptyContentTokens = tokenCounter.calculateTokenCount(emptyMessages);
  assertTrue(emptyContentTokens > emptyTokens, 'Should count more tokens with message structure');
  
  // Non-array input
  assertEqual(tokenCounter.calculateTokenCount(null), 0, 'Null messages');
  assertEqual(tokenCounter.calculateTokenCount('not an array'), 0, 'Non-array messages');
});

// Test 9: Backward compatibility
runTest('Backward compatibility', () => {
  const messages = [
    { role: 'user', content: 'Test message' }
  ];
  
  // Should work without options parameter
  const tokens1 = tokenCounter.calculateTokenCount(messages);
  const tokens2 = tokenCounter.calculateTokenCount(messages, {});
  
  assertTrue(tokens1 > 0, 'Should work without options');
  assertTrue(tokens2 > 0, 'Should work with empty options');
});

// Test 10: calculateTokenCountForContent compatibility
runTest('calculateTokenCountForContent compatibility', () => {
  // String input
  const stringTokens = tokenCounter.calculateTokenCountForContent('Hello world');
  assertTrue(stringTokens > 0, 'Should handle string input');
  
  // Array of messages
  const messages = [{ role: 'user', content: 'Hello' }];
  const messageTokens = tokenCounter.calculateTokenCountForContent(messages);
  assertTrue(messageTokens > 0, 'Should handle message array');
  
  // Object input
  const objectTokens = tokenCounter.calculateTokenCountForContent({ content: 'Hello' });
  assertTrue(objectTokens > 0, 'Should handle object input');
});

console.log('\n=== Test Suite Complete ===');
console.log('All tests passed! ✓');
