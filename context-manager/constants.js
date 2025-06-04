/**
 * Context Management Constants
 * Configuration values for conversation context window management
 */

// Context window limits and thresholds
export const CONTEXT_WINDOW_CONFIG = {
  // Maximum context window tokens (180K)
  MAX_TOKENS: 180000,

  // Compaction threshold (92% of max tokens)
  COMPACT_THRESHOLD: 0.92,

  // Auto-compact low water mark (60% of max tokens)
  AUTO_COMPACT_LOW_WATER_MARK: 0.6,

  // Auto-compact high water mark (80% of max tokens)
  AUTO_COMPACT_HIGH_WATER_MARK: 0.8,
};

// Minimum number of messages required before compaction can occur
export const MIN_MESSAGES_FOR_COMPACTION = 5;

// Error messages
export const ERROR_MESSAGES = {
  NOT_ENOUGH_MESSAGES: "Not enough messages to compact.",
  COMPACTION_FAILED:
    "Failed to generate conversation summary - response did not contain valid text content",
  API_ERROR: "API error occurred during compaction",
  PROMPT_TOO_LONG: "Prompt too long for compaction",
};

// Compaction reasons for telemetry
export const COMPACTION_REASONS = {
  NO_SUMMARY: "no_summary",
  API_ERROR: "api_error",
  PROMPT_TOO_LONG: "prompt_too_long",
};

// Default configuration
export const DEFAULT_CONFIG = {
  autoCompactEnabled: true,
  preserveLastNMessages: 2,
  includeFileStateRestoration: true,
  enableDetailedSummarization: true,
};
