// Dynamic Context Manager - Advanced partitioned context window management
// Implements a 4-zone system: Files (20%), Terminal, Chat, and Task/Goal contexts

import { TokenCounter } from "./TokenCounter.js";
import { CONTEXT_WINDOW_CONFIG } from "./constants.js";

export class DynamicContextManager {
  constructor(options = {}) {
    this.config = {
      maxTokens: options.maxTokens || CONTEXT_WINDOW_CONFIG.MAX_TOKENS,
      fileZonePercentage: options.fileZonePercentage || 0.2,
      terminalZonePercentage: options.terminalZonePercentage || 0.15,
      chatZonePercentage: options.chatZonePercentage || 0.5,
      taskZonePercentage: options.taskZonePercentage || 0.15,
      maintainThreshold: options.maintainThreshold || 0.7,
      terminalCompressionAfter: options.terminalCompressionAfter || 10, // messages
      terminalCompressionLength: options.terminalCompressionLength || 200, // chars
      ...options,
    };

    this.tokenCounter = new TokenCounter();

    // Zone allocations
    this.zones = {
      files: Math.floor(this.config.maxTokens * this.config.fileZonePercentage),
      terminal: Math.floor(
        this.config.maxTokens * this.config.terminalZonePercentage
      ),
      chat: Math.floor(this.config.maxTokens * this.config.chatZonePercentage),
      task: Math.floor(this.config.maxTokens * this.config.taskZonePercentage),
    };

    // Context storage
    this.contexts = {
      files: new FileContextZone(this.zones.files),
      terminal: new TerminalContextZone(this.zones.terminal, this.config),
      chat: new ChatContextZone(this.zones.chat),
      task: new TaskContextZone(this.zones.task),
    };

    // Event handlers
    this.onZoneUpdate = options.onZoneUpdate || (() => {});
    this.onContextShift = options.onContextShift || (() => {});
  }

  /**
   * Add or update a file in the file context zone
   */
  updateFile(filePath, content, metadata = {}) {
    const result = this.contexts.files.updateFile(filePath, content, metadata);
    this.onZoneUpdate("files", this.contexts.files.getStatus());
    return result;
  }

  /**
   * Add terminal command and output
   */
  addTerminalEntry(command, output, timestamp = Date.now()) {
    const result = this.contexts.terminal.addEntry(command, output, timestamp);
    this.onZoneUpdate("terminal", this.contexts.terminal.getStatus());
    return result;
  }

  /**
   * Add chat message
   */
  addChatMessage(message) {
    const result = this.contexts.chat.addMessage(message);
    this.onZoneUpdate("chat", this.contexts.chat.getStatus());
    return result;
  }

  /**
   * Update task/goal context
   */
  updateTask(taskData) {
    const result = this.contexts.task.updateTask(taskData);
    this.onZoneUpdate("task", this.contexts.task.getStatus());
    return result;
  }

  /**
   * Get the current context for API calls
   */
  getCurrentContext() {
    return {
      files: this.contexts.files.getContext(),
      terminal: this.contexts.terminal.getContext(),
      chat: this.contexts.chat.getContext(),
      task: this.contexts.task.getContext(),
      metadata: this.getContextMetadata(),
    };
  }

  /**
   * Get context metadata and statistics
   */
  getContextMetadata() {
    const totalTokens = Object.values(this.contexts).reduce(
      (sum, zone) => sum + zone.getCurrentTokens(),
      0
    );

    return {
      totalTokens,
      maxTokens: this.config.maxTokens,
      utilization: totalTokens / this.config.maxTokens,
      zones: {
        files: {
          allocated: this.zones.files,
          used: this.contexts.files.getCurrentTokens(),
          utilization:
            this.contexts.files.getCurrentTokens() / this.zones.files,
        },
        terminal: {
          allocated: this.zones.terminal,
          used: this.contexts.terminal.getCurrentTokens(),
          utilization:
            this.contexts.terminal.getCurrentTokens() / this.zones.terminal,
        },
        chat: {
          allocated: this.zones.chat,
          used: this.contexts.chat.getCurrentTokens(),
          utilization: this.contexts.chat.getCurrentTokens() / this.zones.chat,
        },
        task: {
          allocated: this.zones.task,
          used: this.contexts.task.getCurrentTokens(),
          utilization: this.contexts.task.getCurrentTokens() / this.zones.task,
        },
      },
    };
  }

  /**
   * Check if context management is needed
   */
  shouldManageContext() {
    const metadata = this.getContextMetadata();
    return metadata.utilization >= this.config.maintainThreshold;
  }

  /**
   * Perform context management to maintain the threshold
   */
  manageContext() {
    const metadata = this.getContextMetadata();

    if (metadata.utilization < this.config.maintainThreshold) {
      return { managed: false, reason: "Below threshold" };
    }

    const actions = [];

    // Manage each zone that's over its allocation
    Object.entries(metadata.zones).forEach(([zoneName, zoneData]) => {
      if (zoneData.utilization > 1.0) {
        const zone = this.contexts[zoneName];
        const result = zone.compress();
        if (result.compressed) {
          actions.push({
            zone: zoneName,
            action: "compressed",
            tokensSaved: result.tokensSaved,
            details: result.details,
          });
        }
      }
    });

    this.onContextShift(actions);

    return {
      managed: true,
      actions,
      newUtilization: this.getContextMetadata().utilization,
    };
  }

  /**
   * Export the current state
   */
  exportState() {
    return {
      config: this.config,
      zones: this.zones,
      contexts: {
        files: this.contexts.files.exportState(),
        terminal: this.contexts.terminal.exportState(),
        chat: this.contexts.chat.exportState(),
        task: this.contexts.task.exportState(),
      },
      timestamp: Date.now(),
    };
  }

  /**
   * Import a previously exported state
   */
  importState(state) {
    this.config = { ...this.config, ...state.config };
    this.zones = state.zones;

    this.contexts.files.importState(state.contexts.files);
    this.contexts.terminal.importState(state.contexts.terminal);
    this.contexts.chat.importState(state.contexts.chat);
    this.contexts.task.importState(state.contexts.task);
  }
}

/**
 * File Context Zone - Manages file content with recency-based prioritization
 */
class FileContextZone {
  constructor(maxTokens) {
    this.maxTokens = maxTokens;
    this.files = new Map(); // filePath -> { content, tokens, lastAccessed, metadata }
    this.accessOrder = []; // Most recently accessed first
    this.editHistory = []; // Track all files that have been edited
    this.tokenCounter = new TokenCounter();
  }

  updateFile(filePath, content, metadata = {}) {
    const tokens = this.tokenCounter.calculateTokenCount(content);

    // Update or add file
    this.files.set(filePath, {
      content,
      tokens,
      lastAccessed: Date.now(),
      metadata: { ...metadata, isEdited: true },
    });

    // Update access order
    this.accessOrder = this.accessOrder.filter((path) => path !== filePath);
    this.accessOrder.unshift(filePath);

    // Track in edit history if not already there
    if (!this.editHistory.includes(filePath)) {
      this.editHistory.push(filePath);
    }

    // Manage space
    this.manageSpace();

    return { success: true, tokens };
  }

  manageSpace() {
    let currentTokens = this.getCurrentTokens();

    // Remove least recently accessed files until we're under the limit
    while (currentTokens > this.maxTokens && this.accessOrder.length > 1) {
      const leastRecentPath = this.accessOrder.pop();
      const fileData = this.files.get(leastRecentPath);

      if (fileData) {
        currentTokens -= fileData.tokens;
        this.files.delete(leastRecentPath);
      }
    }
  }

  getCurrentTokens() {
    return Array.from(this.files.values()).reduce(
      (sum, file) => sum + file.tokens,
      0
    );
  }

  getContext() {
    return {
      files: Array.from(this.files.entries()).map(([path, data]) => ({
        path,
        content: data.content,
        lastAccessed: data.lastAccessed,
        metadata: data.metadata,
      })),
      editHistory: this.editHistory,
      totalFiles: this.files.size,
      totalTokens: this.getCurrentTokens(),
    };
  }

  compress() {
    // For files, compression means removing least recently accessed
    const initialTokens = this.getCurrentTokens();
    this.manageSpace();
    const finalTokens = this.getCurrentTokens();

    return {
      compressed: initialTokens > finalTokens,
      tokensSaved: initialTokens - finalTokens,
      details: `Removed ${Math.max(
        0,
        initialTokens - finalTokens
      )} tokens from least recent files`,
    };
  }

  getStatus() {
    return {
      fileCount: this.files.size,
      totalTokens: this.getCurrentTokens(),
      maxTokens: this.maxTokens,
      utilization: this.getCurrentTokens() / this.maxTokens,
      editHistoryCount: this.editHistory.length,
    };
  }

  exportState() {
    return {
      files: Array.from(this.files.entries()),
      accessOrder: this.accessOrder,
      editHistory: this.editHistory,
      maxTokens: this.maxTokens,
    };
  }

  importState(state) {
    this.files = new Map(state.files);
    this.accessOrder = state.accessOrder;
    this.editHistory = state.editHistory;
    this.maxTokens = state.maxTokens;
  }
}

/**
 * Terminal Context Zone - Manages command history with compression
 */
class TerminalContextZone {
  constructor(maxTokens, config) {
    this.maxTokens = maxTokens;
    this.config = config;
    this.entries = []; // { command, output, timestamp, tokens }
    this.tokenCounter = new TokenCounter();
  }

  addEntry(command, output, timestamp = Date.now()) {
    const fullEntry = `$ ${command}\n${output}`;
    const tokens = this.tokenCounter.calculateTokenCount(fullEntry);

    this.entries.push({
      command,
      output,
      timestamp,
      tokens,
      compressed: false,
    });

    this.manageSpace();
    return { success: true, tokens };
  }

  manageSpace() {
    // Compress old entries first
    if (this.entries.length > this.config.terminalCompressionAfter) {
      const entriesToCompress = this.entries.slice(
        0,
        -this.config.terminalCompressionAfter
      );

      entriesToCompress.forEach((entry) => {
        if (!entry.compressed) {
          const shortOutput = entry.output.substring(
            0,
            this.config.terminalCompressionLength
          );
          const compressedContent = `$ ${entry.command}\n${shortOutput}${
            entry.output.length > this.config.terminalCompressionLength
              ? "..."
              : ""
          }`;
          entry.tokens =
            this.tokenCounter.calculateTokenCount(compressedContent);
          entry.output =
            shortOutput +
            (entry.output.length > this.config.terminalCompressionLength
              ? "..."
              : "");
          entry.compressed = true;
        }
      });
    }

    // Remove oldest entries if still over limit
    let currentTokens = this.getCurrentTokens();
    while (currentTokens > this.maxTokens && this.entries.length > 1) {
      const removed = this.entries.shift();
      currentTokens -= removed.tokens;
    }
  }

  getCurrentTokens() {
    return this.entries.reduce((sum, entry) => sum + entry.tokens, 0);
  }

  getContext() {
    return {
      entries: this.entries.map((entry) => ({
        command: entry.command,
        output: entry.output,
        timestamp: entry.timestamp,
        compressed: entry.compressed,
      })),
      totalEntries: this.entries.length,
      totalTokens: this.getCurrentTokens(),
    };
  }

  compress() {
    const initialTokens = this.getCurrentTokens();
    this.manageSpace();
    const finalTokens = this.getCurrentTokens();

    return {
      compressed: initialTokens > finalTokens,
      tokensSaved: initialTokens - finalTokens,
      details: `Compressed terminal history, saved ${
        initialTokens - finalTokens
      } tokens`,
    };
  }

  getStatus() {
    return {
      entryCount: this.entries.length,
      totalTokens: this.getCurrentTokens(),
      maxTokens: this.maxTokens,
      utilization: this.getCurrentTokens() / this.maxTokens,
      compressedEntries: this.entries.filter((e) => e.compressed).length,
    };
  }

  exportState() {
    return {
      entries: this.entries,
      maxTokens: this.maxTokens,
    };
  }

  importState(state) {
    this.entries = state.entries;
    this.maxTokens = state.maxTokens;
  }
}

/**
 * Chat Context Zone - Rolling window of conversation
 */
class ChatContextZone {
  constructor(maxTokens) {
    this.maxTokens = maxTokens;
    this.messages = [];
    this.tokenCounter = new TokenCounter();
  }

  addMessage(message) {
    const tokens = this.tokenCounter.calculateTokenCount(
      typeof message.content === "string"
        ? message.content
        : JSON.stringify(message.content)
    );

    this.messages.push({
      ...message,
      tokens,
      timestamp: Date.now(),
    });

    this.manageSpace();
    return { success: true, tokens };
  }

  manageSpace() {
    let currentTokens = this.getCurrentTokens();

    // Remove oldest messages (but keep at least 2)
    while (currentTokens > this.maxTokens && this.messages.length > 2) {
      const removed = this.messages.shift();
      currentTokens -= removed.tokens;
    }
  }

  getCurrentTokens() {
    return this.messages.reduce((sum, msg) => sum + msg.tokens, 0);
  }

  getContext() {
    return {
      messages: this.messages.map((msg) => ({
        role: msg.role,
        content: msg.content,
        timestamp: msg.timestamp,
      })),
      totalMessages: this.messages.length,
      totalTokens: this.getCurrentTokens(),
    };
  }

  compress() {
    const initialTokens = this.getCurrentTokens();
    this.manageSpace();
    const finalTokens = this.getCurrentTokens();

    return {
      compressed: initialTokens > finalTokens,
      tokensSaved: initialTokens - finalTokens,
      details: `Removed ${Math.max(
        0,
        (initialTokens - finalTokens) / this.tokenCounter.averageTokensPerChar
      )} characters from old messages`,
    };
  }

  getStatus() {
    return {
      messageCount: this.messages.length,
      totalTokens: this.getCurrentTokens(),
      maxTokens: this.maxTokens,
      utilization: this.getCurrentTokens() / this.maxTokens,
    };
  }

  exportState() {
    return {
      messages: this.messages,
      maxTokens: this.maxTokens,
    };
  }

  importState(state) {
    this.messages = state.messages;
    this.maxTokens = state.maxTokens;
  }
}

/**
 * Task Context Zone - Persistent project state and goals
 */
class TaskContextZone {
  constructor(maxTokens) {
    this.maxTokens = maxTokens;
    this.currentTask = null;
    this.goals = [];
    this.completedTasks = [];
    this.projectState = {};
    this.tokenCounter = new TokenCounter();
  }

  updateTask(taskData) {
    const { currentTask, goals, completedTasks, projectState } = taskData;

    if (currentTask) this.currentTask = currentTask;
    if (goals) this.goals = goals;
    if (completedTasks) this.completedTasks = completedTasks;
    if (projectState)
      this.projectState = { ...this.projectState, ...projectState };

    this.manageSpace();
    return { success: true, tokens: this.getCurrentTokens() };
  }

  manageSpace() {
    // For task context, we prioritize current task and goals
    // Compress completed tasks if needed
    let currentTokens = this.getCurrentTokens();

    while (currentTokens > this.maxTokens && this.completedTasks.length > 0) {
      // Remove oldest completed tasks
      const removed = this.completedTasks.shift();
      currentTokens = this.getCurrentTokens();
    }
  }

  getCurrentTokens() {
    const content = JSON.stringify({
      currentTask: this.currentTask,
      goals: this.goals,
      completedTasks: this.completedTasks,
      projectState: this.projectState,
    });
    return this.tokenCounter.calculateTokenCount(content);
  }

  getContext() {
    return {
      currentTask: this.currentTask,
      goals: this.goals,
      completedTasks: this.completedTasks,
      projectState: this.projectState,
      totalTokens: this.getCurrentTokens(),
    };
  }

  compress() {
    const initialTokens = this.getCurrentTokens();
    this.manageSpace();
    const finalTokens = this.getCurrentTokens();

    return {
      compressed: initialTokens > finalTokens,
      tokensSaved: initialTokens - finalTokens,
      details: `Compressed task history, removed ${this.completedTasks.length} old completed tasks`,
    };
  }

  getStatus() {
    return {
      hasCurrentTask: !!this.currentTask,
      goalCount: this.goals.length,
      completedTaskCount: this.completedTasks.length,
      totalTokens: this.getCurrentTokens(),
      maxTokens: this.maxTokens,
      utilization: this.getCurrentTokens() / this.maxTokens,
    };
  }

  exportState() {
    return {
      currentTask: this.currentTask,
      goals: this.goals,
      completedTasks: this.completedTasks,
      projectState: this.projectState,
      maxTokens: this.maxTokens,
    };
  }

  importState(state) {
    this.currentTask = state.currentTask;
    this.goals = state.goals;
    this.completedTasks = state.completedTasks;
    this.projectState = state.projectState;
    this.maxTokens = state.maxTokens;
  }
}

export default DynamicContextManager;
