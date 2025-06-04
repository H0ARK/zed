#!/usr/bin/env node

/**
 * Test script to demonstrate the difference between old and new token calculation methods
 */

import { TokenCounter } from './TokenCounter.js';

const tokenCounter = new TokenCounter();

// Sample messages that might be sent to an agent
const sampleMessages = [
  {
    role: 'user',
    content: 'Hello, can you help me write a function to calculate the factorial of a number?'
  },
  {
    role: 'assistant',
    content: 'I\'d be happy to help you write a factorial function! Here\'s a simple recursive implementation:\n\n```javascript\nfunction factorial(n) {\n  if (n <= 1) return 1;\n  return n * factorial(n - 1);\n}\n```\n\nThis function works by recursively multiplying the number by the factorial of (n-1) until it reaches the base case.'
  },
  {
    role: 'user',
    content: 'Thanks! Can you also show me an iterative version?'
  }
];

// Sample system prompt and tools that might be included
const sampleOptions = {
  system: 'You are a helpful programming assistant. Always provide clear, well-commented code examples.',
  tools: [
    {
      name: 'execute_code',
      description: 'Execute JavaScript code and return the result',
      input_schema: {
        type: 'object',
        properties: {
          code: { type: 'string', description: 'The JavaScript code to execute' }
        },
        required: ['code']
      }
    }
  ],
  tool_choice: 'auto',
  temperature: 0.7
};

console.log('=== Token Calculation Comparison ===\n');

// Calculate using legacy method (content-only)
const legacyTokens = tokenCounter.calculateTokenCountLegacy(sampleMessages);
console.log(`Legacy method (content-only): ${legacyTokens} tokens`);

// Calculate using new method (full API request structure)
const newTokens = tokenCounter.calculateTokenCount(sampleMessages, sampleOptions);
console.log(`New method (full API request): ${newTokens} tokens`);

// Show the difference
const difference = newTokens - legacyTokens;
const percentageIncrease = ((difference / legacyTokens) * 100).toFixed(1);
console.log(`\nDifference: +${difference} tokens (${percentageIncrease}% increase)`);

console.log('\n=== What the new method accounts for ===');
console.log('✓ Role information ("role": "user", "role": "assistant")');
console.log('✓ Content type wrappers ("type": "text")');
console.log('✓ System prompts');
console.log('✓ Tool definitions and schemas');
console.log('✓ Tool choice settings');
console.log('✓ Model parameters (temperature, max_tokens, etc.)');
console.log('✓ JSON structure overhead');

// Show a sample of what the actual API request looks like
console.log('\n=== Sample API Request Structure ===');
const apiRequest = {
  model: "claude-3-5-sonnet-latest",
  max_tokens: 8192,
  messages: tokenCounter.convertToApiMessages(sampleMessages),
  system: sampleOptions.system,
  tools: sampleOptions.tools,
  tool_choice: sampleOptions.tool_choice,
  temperature: sampleOptions.temperature
};

console.log(JSON.stringify(apiRequest, null, 2));

// Test with just the request string
console.log('\n=== Direct Request String Calculation ===');
const requestString = JSON.stringify(apiRequest);
const directTokens = tokenCounter.calculateTokensForRequestString(requestString);
console.log(`Request string length: ${requestString.length} characters`);
console.log(`Direct calculation: ${directTokens} tokens`);
console.log(`Matches new method: ${directTokens === newTokens ? '✓' : '✗'}`);

