/**
 * Token Counter
 * Handles token counting for messages and conversation management
 */

export class TokenCounter {
  constructor() {
    // Simple token estimation - in a real implementation,
    // this would use the actual tokenizer for the model
    this.averageTokensPerChar = 0.25; // Rough estimate
  }

  /**
   * Calculate token count for the actual request string that gets sent to the agent
   * This is the most accurate method as it counts tokens on the full JSON structure
   * @param {string} requestString - The serialized JSON request string
   * @returns {number} Estimated token count
   */
  calculateTokensForRequestString(requestString) {
    if (!requestString || typeof requestString !== 'string') {
      return 0;
    }
    
    // Count tokens on the actual request string that gets sent to the API
    return Math.ceil(requestString.length * this.averageTokensPerChar);
  }

  /**
   * Calculate token count for a simulated Anthropic API request structure
   * This method constructs what the actual API request would look like
   * @param {Array} messages - Array of message objects
   * @param {Object} options - Additional request options (system, tools, etc.)
   * @returns {number} Estimated token count
   */
  calculateTokensForApiRequest(messages, options = {}) {
    if (!Array.isArray(messages)) {
      return 0;
    }

    // Construct a request structure similar to what gets sent to Anthropic
    const apiRequest = {
      model: options.model || "claude-3-5-sonnet-latest",
      max_tokens: options.max_tokens || 8192,
      messages: this.convertToApiMessages(messages),
      ...(options.system && { system: options.system }),
      ...(options.tools && options.tools.length > 0 && { tools: options.tools }),
      ...(options.tool_choice && { tool_choice: options.tool_choice }),
      ...(options.temperature && { temperature: options.temperature }),
    };

    // Serialize the request and count tokens on the actual JSON
    const requestString = JSON.stringify(apiRequest);
    return this.calculateTokensForRequestString(requestString);
  }

  /**
   * Convert messages to Anthropic API format
   * @param {Array} messages - Array of message objects
   * @returns {Array} Messages in Anthropic API format
   */
  convertToApiMessages(messages) {
    return messages.map(message => {
      const apiMessage = {
        role: message.role || 'user',
        content: []
      };

      if (typeof message.content === 'string') {
        apiMessage.content.push({
          type: 'text',
          text: message.content
        });
      } else if (Array.isArray(message.content)) {
        // Handle multi-part content (text + images, tool use, etc.)
        apiMessage.content = message.content.map(part => {
          if (typeof part === 'string') {
            return { type: 'text', text: part };
          } else if (part.type === 'text') {
            return { type: 'text', text: part.text || part.content || '' };
          } else if (part.type === 'image') {
            return {
              type: 'image',
              source: part.source || { type: 'base64', media_type: 'image/png', data: part.data || '' }
            };
          } else if (part.type === 'tool_use') {
            return {
              type: 'tool_use',
              id: part.id || '',
              name: part.name || '',
              input: part.input || {}
            };
          } else if (part.type === 'tool_result') {
            return {
              type: 'tool_result',
              tool_use_id: part.tool_use_id || '',
              content: part.content || '',
              is_error: part.is_error || false
            };
          }
          return part;
        });
      } else if (message.content && typeof message.content === 'object') {
        // Handle single content object
        if (message.content.type === 'text') {
          apiMessage.content.push({
            type: 'text',
            text: message.content.text || message.content.content || ''
          });
        } else {
          apiMessage.content.push(message.content);
        }
      }

      return apiMessage;
    });
  }

  /**
   * Calculate token count for a list of messages
   * @param {Array} messages - Array of message objects
   * @param {Object} options - Additional request options (system, tools, etc.)
   * @returns {number} Estimated token count
   */
  calculateTokenCount(messages, options = {}) {
    if (!Array.isArray(messages)) {
      return 0;
    }

    // Use the new accurate method that accounts for full API request structure
    return this.calculateTokensForApiRequest(messages, options);
  }

  /**
   * Legacy method: Calculate token count using simple content-based estimation
   * @param {Array} messages - Array of message objects
   * @returns {number} Estimated token count
   */
  calculateTokenCountLegacy(messages) {
    if (!Array.isArray(messages)) {
      return 0;
    }

    let totalTokens = 0;

    for (const message of messages) {
      totalTokens += this.calculateMessageTokens(message);
    }

    return Math.ceil(totalTokens);
  }

  /**
   * Calculate tokens for a single message
   * @param {Object} message - Message object
   * @returns {number} Estimated token count
   */
  calculateMessageTokens(message) {
    if (!message) return 0;

    let tokens = 0;

    // Count role tokens
    if (message.role) {
      tokens += message.role.length * this.averageTokensPerChar;
    }

    // Count content tokens
    if (message.content) {
      if (typeof message.content === "string") {
        tokens += message.content.length * this.averageTokensPerChar;
      } else if (Array.isArray(message.content)) {
        // Handle multi-part content (text + images, etc.)
        for (const part of message.content) {
          if (part.type === "text" && part.text) {
            tokens += part.text.length * this.averageTokensPerChar;
          } else if (part.type === "image") {
            // Images have a fixed token cost
            tokens += 85; // Approximate token cost for images
          }
        }
      }
    }

    // Add overhead for message structure
    tokens += 10; // Base overhead per message

    return tokens;
  }

  /**
   * Estimate tokens for a string
   * @param {string} text - Text to count
   * @returns {number} Estimated token count
   */
  estimateTokens(text) {
    if (!text || typeof text !== "string") {
      return 0;
    }
    return Math.ceil(text.length * this.averageTokensPerChar);
  }

  /**
   * Calculate token count for any content (compatibility with PointerBasedContextManager)
   * @param {string|Object|Array} content - Content to count
   * @returns {number} Estimated token count
   */
  calculateTokenCountForContent(content) {
    // If it's an array of messages, use the existing method
    if (Array.isArray(content) && content.length > 0 && content[0]?.role) {
      return this.calculateTokenCount(content);
    }
    
    // If it's a string, estimate directly
    if (typeof content === 'string') {
      return this.estimateTokens(content);
    }
    
    // If it's an object, try to extract content
    if (typeof content === 'object' && content !== null) {
      if (content.content) {
        return this.estimateTokens(content.content);
      }
      // Fallback: stringify the object
      return this.estimateTokens(JSON.stringify(content));
    }
    
    return 0;
  }

  /**
   * Check if token count is within limits
   * @param {number} tokenCount - Current token count
   * @param {number} maxTokens - Maximum allowed tokens
   * @returns {Object} Status object with various thresholds
   */
  checkThresholds(tokenCount, maxTokens) {
    const percentUsed = tokenCount / maxTokens;

    return {
      tokenCount,
      maxTokens,
      percentUsed,
      percentLeft: 1 - percentUsed,
      isAboveWarningThreshold: percentUsed > 0.7,
      isAboveErrorThreshold: percentUsed > 0.9,
      isAboveAutoCompactThreshold: percentUsed > 0.8,
      isAboveCompactThreshold: percentUsed > 0.92,
    };
  }
}
