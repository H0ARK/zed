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
   * Calculate token count for a list of messages
   * @param {Array} messages - Array of message objects
   * @returns {number} Estimated token count
   */
  calculateTokenCount(messages) {
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
