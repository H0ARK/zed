/**
 * Context Manager Module
 * Comprehensive conversation context management system
 *
 * This module provides sophisticated context window management including:
 * - Token counting and threshold monitoring
 * - Conversation summarization and compaction
 * - File state tracking and restoration
 * - Auto-compaction capabilities
 * - Pointer-based context management with hybrid edit strategies
 * - Dynamic zone-based context partitioning
 *
 * Based on the Claude Code CLI context management system.
 */

// Main classes
export { ContextManager } from "./ContextManager.js";
export { ConversationCompactor } from "./ConversationCompactor.js";
export { TokenCounter } from "./TokenCounter.js";
export { FileStateManager } from "./FileStateManager.js";
export { SummarizationPrompts } from "./SummarizationPrompts.js";

// Advanced context managers
export { PointerBasedContextManager } from "./PointerBasedContextManager.js";
export { DynamicContextManager } from "./DynamicContextManager.js";

// Constants and configuration
export {
  CONTEXT_WINDOW_CONFIG,
  MIN_MESSAGES_FOR_COMPACTION,
  ERROR_MESSAGES,
  COMPACTION_REASONS,
  DEFAULT_CONFIG,
} from "./constants.js";

// Context management strategies
export const CONTEXT_STRATEGIES = {
  TRADITIONAL: 'traditional',
  POINTER_BASED: 'pointer_based', 
  DYNAMIC_ZONES: 'dynamic_zones'
};

// Convenience factory function
export function createContextManager(strategy = CONTEXT_STRATEGIES.TRADITIONAL, options = {}) {
  switch (strategy) {
    case CONTEXT_STRATEGIES.POINTER_BASED:
      return new PointerBasedContextManager(options);
    case CONTEXT_STRATEGIES.DYNAMIC_ZONES:
      return new DynamicContextManager(options);
    case CONTEXT_STRATEGIES.TRADITIONAL:
    default:
      return new ContextManager(options);
  }
}

// Enhanced factory with automatic strategy selection
export function createOptimalContextManager(options = {}) {
  const {
    projectType = 'general',
    expectedFileCount = 10,
    expectedSessionLength = 'medium',
    memoryEfficiencyPriority = 'balanced',
    ...managerOptions
  } = options;

  // Auto-select strategy based on use case
  let strategy = CONTEXT_STRATEGIES.TRADITIONAL;

  if (memoryEfficiencyPriority === 'high' || expectedFileCount > 20) {
    strategy = CONTEXT_STRATEGIES.POINTER_BASED;
  } else if (projectType === 'terminal-heavy' || expectedSessionLength === 'long') {
    strategy = CONTEXT_STRATEGIES.DYNAMIC_ZONES;
  }

  return createContextManager(strategy, {
    ...managerOptions,
    // Add strategy-specific optimizations
    autoOptimize: true,
    selectedStrategy: strategy,
    selectionReason: `Auto-selected based on: projectType=${projectType}, fileCount=${expectedFileCount}, sessionLength=${expectedSessionLength}, memoryPriority=${memoryEfficiencyPriority}`
  });
}

// Default export is the factory function for easy usage
export default createContextManager;
