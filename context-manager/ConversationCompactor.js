/**
 * Conversation Compactor
 * Main class that handles conversation compaction and summarization
 */

import {
  CONTEXT_WINDOW_CONFIG,
  MIN_MESSAGES_FOR_COMPACTION,
  ERROR_MESSAGES,
  COMPACTION_REASONS,
} from "./constants.js";
import { TokenCounter } from "./TokenCounter.js";
import { SummarizationPrompts } from "./SummarizationPrompts.js";
import { FileStateManager } from "./FileStateManager.js";

export class ConversationCompactor {
  constructor(options = {}) {
    this.tokenCounter = new TokenCounter();
    this.fileStateManager = new FileStateManager();
    this.config = { ...CONTEXT_WINDOW_CONFIG, ...options };

    // Callbacks for UI updates and AI calls
    this.onStatusUpdate = options.onStatusUpdate || (() => {});
    this.onError = options.onError || (() => {});
    this.callAI = options.callAI || this.defaultAICall.bind(this);
  }

  /**
   * Check if conversation needs compaction
   * @param {Array} messages - Current conversation messages
   * @param {boolean} forceCompact - Force compaction regardless of token count
   * @returns {Object} Compaction status
   */
  shouldCompact(messages, forceCompact = false) {
    if (messages.length < MIN_MESSAGES_FOR_COMPACTION && !forceCompact) {
      return {
        shouldCompact: false,
        reason: "insufficient_messages",
        messageCount: messages.length,
        minRequired: MIN_MESSAGES_FOR_COMPACTION,
      };
    }

    const tokenCount = this.tokenCounter.calculateTokenCount(messages);
    const thresholds = this.tokenCounter.checkThresholds(
      tokenCount,
      this.config.MAX_TOKENS
    );

    const shouldCompact = forceCompact || thresholds.isAboveCompactThreshold;

    return {
      shouldCompact,
      reason: forceCompact
        ? "forced"
        : shouldCompact
        ? "threshold_exceeded"
        : "within_limits",
      tokenCount,
      thresholds,
      messageCount: messages.length,
    };
  }

  /**
   * Check if auto-compaction should be triggered
   * @param {Array} messages - Current conversation messages
   * @returns {boolean} Whether auto-compaction should occur
   */
  shouldAutoCompact(messages) {
    const tokenCount = this.tokenCounter.calculateTokenCount(messages);
    const thresholds = this.tokenCounter.checkThresholds(
      tokenCount,
      this.config.MAX_TOKENS
    );
    return thresholds.isAboveAutoCompactThreshold;
  }

  /**
   * Perform conversation compaction
   * @param {Array} messages - Messages to compact
   * @param {Object} executionContext - Context for tool execution
   * @param {boolean} forceCompact - Force compaction
   * @param {string} customInstructions - Custom summarization instructions
   * @returns {Promise<Object>} Compaction result
   */
  async compactConversation(
    messages,
    executionContext,
    forceCompact = false,
    customInstructions = ""
  ) {
    try {
      // Check if compaction is needed
      const compactionStatus = this.shouldCompact(messages, forceCompact);
      if (!compactionStatus.shouldCompact) {
        return {
          success: false,
          reason: compactionStatus.reason,
          messages: messages,
          summaryMessage: null,
        };
      }

      this.onStatusUpdate("Compacting conversation...");

      // Prepare messages for summarization (exclude last 2 messages)
      const messagesToSummarize = messages.slice(0, -2);
      const messagesToKeep = messages.slice(-2);

      // Generate summarization prompt
      const summarizationPrompt =
        SummarizationPrompts.getSummarizationPrompt(customInstructions);

      // Create summarization request
      const summarizationMessage = {
        role: "user",
        content: summarizationPrompt,
      };

      // Call AI for summarization
      const summaryResponse = await this.callAI(
        [...messagesToSummarize, summarizationMessage],
        executionContext
      );

      if (!summaryResponse || !summaryResponse.content) {
        throw new Error(ERROR_MESSAGES.COMPACTION_FAILED);
      }

      // Format the summary
      const formattedSummary = SummarizationPrompts.formatCompactSummaryForUser(
        summaryResponse.content
      );

      // Create continuation system prompt
      const continuationPrompt =
        SummarizationPrompts.getCompactContinuationSystemPrompt(
          summaryResponse.content,
          forceCompact
        );

      // Create new message list with summary + recent messages
      const compactedMessages = [
        SummarizationPrompts.createSystemMessage(continuationPrompt),
        ...messagesToKeep,
      ];

      // Restore file state based on the summary
      const restoredFiles =
        await this.fileStateManager.restoreFileStateFromCompact(
          compactedMessages,
          summaryResponse.content
        );

      this.onStatusUpdate(null); // Clear status

      return {
        success: true,
        messages: compactedMessages,
        summaryMessage: {
          type: "system_summary",
          content: formattedSummary,
          timestamp: Date.now(),
        },
        originalMessageCount: messages.length,
        compactedMessageCount: compactedMessages.length,
        tokensSaved:
          compactionStatus.tokenCount -
          this.tokenCounter.calculateTokenCount(compactedMessages),
        restoredFiles,
      };
    } catch (error) {
      this.onStatusUpdate(null); // Clear status
      this.onError(`Error compacting conversation: ${error.message}`);

      return {
        success: false,
        error: error.message,
        messages: messages,
        summaryMessage: null,
      };
    }
  }

  /**
   * Get conversation status and warnings
   * @param {Array} messages - Current messages
   * @returns {Object} Status information
   */
  getConversationStatus(messages) {
    const tokenCount = this.tokenCounter.calculateTokenCount(messages);
    const thresholds = this.tokenCounter.checkThresholds(
      tokenCount,
      this.config.MAX_TOKENS
    );
    const fileStats = this.fileStateManager.getStatistics();

    return {
      tokenCount,
      thresholds,
      messageCount: messages.length,
      fileStats,
      warnings: this.generateWarnings(thresholds),
      recommendations: this.generateRecommendations(
        thresholds,
        messages.length
      ),
    };
  }

  /**
   * Generate warnings based on thresholds
   * @param {Object} thresholds - Threshold status
   * @returns {Array} Array of warning messages
   */
  generateWarnings(thresholds) {
    const warnings = [];

    if (thresholds.isAboveErrorThreshold) {
      warnings.push({
        level: "error",
        message: "Context window is nearly full. New requests may fail.",
        action: "Compact conversation immediately",
      });
    } else if (thresholds.isAboveAutoCompactThreshold) {
      warnings.push({
        level: "warning",
        message: "Context window is getting full. Auto-compaction recommended.",
        action: "Consider compacting conversation",
      });
    } else if (thresholds.isAboveWarningThreshold) {
      warnings.push({
        level: "info",
        message: "Context window usage is high.",
        action: "Monitor conversation length",
      });
    }

    return warnings;
  }

  /**
   * Generate recommendations based on current state
   * @param {Object} thresholds - Threshold status
   * @param {number} messageCount - Number of messages
   * @returns {Array} Array of recommendations
   */
  generateRecommendations(thresholds, messageCount) {
    const recommendations = [];

    if (
      thresholds.isAboveAutoCompactThreshold &&
      messageCount >= MIN_MESSAGES_FOR_COMPACTION
    ) {
      recommendations.push(
        "Enable auto-compaction to automatically manage context window"
      );
    }

    if (thresholds.percentUsed > 0.5) {
      recommendations.push(
        "Consider using more concise responses to preserve context"
      );
    }

    return recommendations;
  }

  /**
   * Default AI call implementation (should be overridden)
   * @param {Array} messages - Messages to send to AI
   * @param {Object} context - Execution context
   * @returns {Promise<Object>} AI response
   */
  async defaultAICall(messages, context) {
    throw new Error(
      "AI call function not implemented. Please provide a callAI function in options."
    );
  }

  /**
   * Update file state manager
   * @param {FileStateManager} fileStateManager - New file state manager
   */
  setFileStateManager(fileStateManager) {
    this.fileStateManager = fileStateManager;
  }

  /**
   * Get current file state manager
   * @returns {FileStateManager} Current file state manager
   */
  getFileStateManager() {
    return this.fileStateManager;
  }

  /**
   * Export compactor state for persistence
   * @returns {Object} Serializable state
   */
  exportState() {
    return {
      config: this.config,
      fileState: this.fileStateManager.exportState(),
      exportedAt: Date.now(),
    };
  }

  /**
   * Import compactor state from persistence
   * @param {Object} state - Previously exported state
   */
  importState(state) {
    if (state.config) {
      this.config = { ...this.config, ...state.config };
    }
    if (state.fileState) {
      this.fileStateManager.importState(state.fileState);
    }
  }
}
