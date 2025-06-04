/**
 * Context Manager
 * Main facade class that provides a unified interface for context management
 */

import { ConversationCompactor } from "./ConversationCompactor.js";
import { TokenCounter } from "./TokenCounter.js";
import { FileStateManager } from "./FileStateManager.js";
import { CONTEXT_WINDOW_CONFIG, DEFAULT_CONFIG } from "./constants.js";

export class ContextManager {
  constructor(options = {}) {
    this.config = { ...DEFAULT_CONFIG, ...options };

    // Initialize components
    this.tokenCounter = new TokenCounter();
    this.fileStateManager = new FileStateManager();
    this.compactor = new ConversationCompactor({
      ...this.config,
      onStatusUpdate: options.onStatusUpdate,
      onError: options.onError,
      callAI: options.callAI,
    });

    // Set the file state manager in the compactor
    this.compactor.setFileStateManager(this.fileStateManager);

    // Event handlers
    this.eventHandlers = {
      statusUpdate: options.onStatusUpdate || (() => {}),
      error: options.onError || (() => {}),
      warning: options.onWarning || (() => {}),
      compactionComplete: options.onCompactionComplete || (() => {}),
    };
  }

  /**
   * Analyze current conversation state
   * @param {Array} messages - Current conversation messages
   * @returns {Object} Comprehensive analysis of conversation state
   */
  analyzeConversation(messages) {
    const tokenCount = this.tokenCounter.calculateTokenCount(messages);
    const thresholds = this.tokenCounter.checkThresholds(
      tokenCount,
      CONTEXT_WINDOW_CONFIG.MAX_TOKENS
    );
    const fileStats = this.fileStateManager.getStatistics();
    const compactionStatus = this.compactor.shouldCompact(messages);

    return {
      tokenCount,
      thresholds,
      messageCount: messages.length,
      fileStats,
      compactionStatus,
      autoCompactRecommended: this.compactor.shouldAutoCompact(messages),
      warnings: this.generateWarnings(thresholds),
      recommendations: this.generateRecommendations(
        thresholds,
        messages.length
      ),
      timestamp: Date.now(),
    };
  }

  /**
   * Check if auto-compaction should be triggered
   * @param {Array} messages - Current conversation messages
   * @returns {boolean} Whether auto-compaction should occur
   */
  shouldAutoCompact(messages) {
    return (
      this.config.autoCompactEnabled &&
      this.compactor.shouldAutoCompact(messages)
    );
  }

  /**
   * Perform conversation compaction
   * @param {Array} messages - Messages to compact
   * @param {Object} executionContext - Context for tool execution
   * @param {Object} options - Compaction options
   * @returns {Promise<Object>} Compaction result
   */
  async compactConversation(messages, executionContext, options = {}) {
    const {
      forceCompact = false,
      customInstructions = "",
      preserveLastNMessages = this.config.preserveLastNMessages,
    } = options;

    try {
      const result = await this.compactor.compactConversation(
        messages,
        executionContext,
        forceCompact,
        customInstructions
      );

      if (result.success) {
        this.eventHandlers.compactionComplete(result);
      }

      return result;
    } catch (error) {
      this.eventHandlers.error(`Compaction failed: ${error.message}`);
      throw error;
    }
  }

  /**
   * Add a file to the file state
   * @param {string} filePath - Path to the file
   * @param {Object} fileData - File data object
   */
  addFile(filePath, fileData) {
    this.fileStateManager.setFile(filePath, fileData);
  }

  /**
   * Get a file from the file state
   * @param {string} filePath - Path to the file
   * @returns {Object|null} File data or null if not found
   */
  getFile(filePath) {
    return this.fileStateManager.getFile(filePath);
  }

  /**
   * Remove a file from the file state
   * @param {string} filePath - Path to the file
   */
  removeFile(filePath) {
    this.fileStateManager.removeFile(filePath);
  }

  /**
   * Get all files currently in state
   * @returns {Array<string>} Array of file paths
   */
  getAllFiles() {
    return this.fileStateManager.getAllFilePaths();
  }

  /**
   * Clear all file state
   */
  clearFileState() {
    this.fileStateManager.clearAll();
  }

  /**
   * Get file state statistics
   * @returns {Object} File state statistics
   */
  getFileStatistics() {
    return this.fileStateManager.getStatistics();
  }

  /**
   * Calculate token count for messages
   * @param {Array} messages - Messages to count
   * @returns {number} Token count
   */
  calculateTokenCount(messages) {
    return this.tokenCounter.calculateTokenCount(messages);
  }

  /**
   * Check token thresholds
   * @param {number} tokenCount - Current token count
   * @returns {Object} Threshold status
   */
  checkThresholds(tokenCount) {
    return this.tokenCounter.checkThresholds(
      tokenCount,
      CONTEXT_WINDOW_CONFIG.MAX_TOKENS
    );
  }

  /**
   * Get conversation status with warnings and recommendations
   * @param {Array} messages - Current messages
   * @returns {Object} Status information
   */
  getConversationStatus(messages) {
    const analysis = this.analyzeConversation(messages);

    // Emit warnings if needed
    for (const warning of analysis.warnings) {
      this.eventHandlers.warning(warning);
    }

    return analysis;
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
        threshold: "error",
      });
    } else if (thresholds.isAboveAutoCompactThreshold) {
      warnings.push({
        level: "warning",
        message: "Context window is getting full. Auto-compaction recommended.",
        action: "Consider compacting conversation",
        threshold: "auto_compact",
      });
    } else if (thresholds.isAboveWarningThreshold) {
      warnings.push({
        level: "info",
        message: "Context window usage is high.",
        action: "Monitor conversation length",
        threshold: "warning",
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

    if (thresholds.isAboveAutoCompactThreshold && messageCount >= 5) {
      recommendations.push({
        type: "auto_compact",
        message:
          "Enable auto-compaction to automatically manage context window",
        priority: "high",
      });
    }

    if (thresholds.percentUsed > 0.5) {
      recommendations.push({
        type: "response_length",
        message: "Consider using more concise responses to preserve context",
        priority: "medium",
      });
    }

    if (this.fileStateManager.getStatistics().fileCount > 20) {
      recommendations.push({
        type: "file_cleanup",
        message: "Consider clearing unused files from context",
        priority: "low",
      });
    }

    return recommendations;
  }

  /**
   * Update configuration
   * @param {Object} newConfig - New configuration options
   */
  updateConfig(newConfig) {
    this.config = { ...this.config, ...newConfig };
    this.compactor.config = { ...this.compactor.config, ...newConfig };
  }

  /**
   * Get current configuration
   * @returns {Object} Current configuration
   */
  getConfig() {
    return { ...this.config };
  }

  /**
   * Export complete state for persistence
   * @returns {Object} Serializable state
   */
  exportState() {
    return {
      config: this.config,
      compactorState: this.compactor.exportState(),
      exportedAt: Date.now(),
      version: "1.0.0",
    };
  }

  /**
   * Import complete state from persistence
   * @param {Object} state - Previously exported state
   */
  importState(state) {
    if (state.config) {
      this.updateConfig(state.config);
    }
    if (state.compactorState) {
      this.compactor.importState(state.compactorState);
    }
  }

  /**
   * Reset all state
   */
  reset() {
    this.fileStateManager.clearAll();
    this.config = { ...DEFAULT_CONFIG };
    this.compactor.config = { ...CONTEXT_WINDOW_CONFIG };
  }

  /**
   * Get component instances for advanced usage
   * @returns {Object} Component instances
   */
  getComponents() {
    return {
      tokenCounter: this.tokenCounter,
      fileStateManager: this.fileStateManager,
      compactor: this.compactor,
    };
  }
}
