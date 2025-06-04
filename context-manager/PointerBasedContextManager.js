// Pointer-Based Context Manager - External content management with dynamic insertion
// Implements the user's superior approach: external tracking + pointers + dynamic loading

import { TokenCounter } from "./TokenCounter.js";
import { CONTEXT_WINDOW_CONFIG } from "./constants.js";

export class PointerBasedContextManager {
  constructor(options = {}) {
    this.config = {
      maxTokens: options.maxTokens || CONTEXT_WINDOW_CONFIG.MAX_TOKENS,
      safetyThreshold: options.safetyThreshold || 0.7, // User's 70% safety zone
      terminalCompressionAfter: options.terminalCompressionAfter || 3,
      functionHeadersOnly: options.functionHeadersOnly || true,
      ...options,
    };

    this.tokenCounter = new TokenCounter();

    // External content storage - the "database" of everything
    this.contentStore = {
      files: new Map(), // filePath -> { content, headers, metadata }
      terminal: new Map(), // commandId -> { command, output, compressed }
      tasks: new Map(), // taskId -> { description, status, context }
      references: new Map(), // refId -> content
    };

    // Active context window - what's actually sent to the API
    this.activeContext = [];

    // Reference tracking - what's currently loaded
    this.loadedRefs = new Set();

    // Usage tracking for smart eviction
    this.usageStats = new Map(); // refId -> { lastUsed, useCount, priority }
  }

  /**
   * Add a message to the context, processing any references
   */
  addMessage(message) {
    // Extract references from the message
    const refs = this.extractReferences(message);

    // Add the message to active context
    this.activeContext.push({
      ...message,
      refs: refs,
      timestamp: Date.now(),
    });

    // Load referenced content
    refs.forEach((ref) => this.loadReference(ref));

    // Manage context size
    this.manageContextSize();

    return { success: true, refs: refs.length };
  }

  /**
   * Update file content in the store
   */
  updateFile(filePath, content, metadata = {}) {
    const headers = this.extractFunctionHeaders(content);
    const tokens = this.tokenCounter.calculateTokenCountForContent(content);

    const existingFile = this.contentStore.files.get(filePath);
    let diff = null;

    // Generate diff if file already exists
    if (existingFile) {
      diff = this.generateDiff(existingFile.content, content);
    }

    this.contentStore.files.set(filePath, {
      content,
      headers,
      tokens,
      lastModified: Date.now(),
      metadata: { ...metadata, isEdited: true },
      previousVersion: existingFile ? existingFile.content : null,
      diff: diff,
    });

    // Handle file reference updates in context
    const fileRef = `file:${filePath}`;
    if (this.loadedRefs.has(fileRef)) {
      this.handleFileEdit(filePath, diff, existingFile);
    }

    return { success: true, tokens, headers: headers.length, hasDiff: !!diff };
  }

  /**
   * Handle file edits in active context - implements hybrid strategy
   */
  handleFileEdit(filePath, diff, previousFile) {
    const fileRef = `file:${filePath}`;
    const diffRef = `diff:${filePath}:${Date.now()}`;

    if (!diff || !diff.hasChanges) {
      this.refreshReference(fileRef);
      return;
    }

    // Calculate edit magnitude to decide strategy
    const editMagnitude = this.calculateEditMagnitude(diff, previousFile);
    const strategy = this.selectEditStrategy(editMagnitude, diff);

    switch (strategy) {
      case "KEEP_BOTH":
        this.handleSmallEdit(fileRef, diffRef, diff, filePath);
        break;
      case "REPLACE_WITH_DIFF":
        this.handleMediumEdit(fileRef, diffRef, diff, filePath);
        break;
      case "DIFF_MARKER_ONLY":
        this.handleLargeEdit(fileRef, diffRef, diff, filePath);
        break;
    }
  }

  /**
   * Calculate how significant an edit is
   */
  calculateEditMagnitude(diff, previousFile) {
    const totalLines = previousFile
      ? previousFile.content.split("\n").length
      : 100;
    const changePercentage = diff.changeCount / totalLines;
    const tokenImpact = diff.diffText.length / (previousFile?.tokens || 1000);

    // Enhanced analysis based on conversation insights
    const contextualFactors = this.analyzeContextualFactors(diff, previousFile);
    
    return {
      changePercentage,
      tokenImpact,
      changeCount: diff.changeCount,
      isStructuralChange: this.isStructuralChange(diff.diffText),
      contextualFactors,
      // Memory efficiency metrics
      memoryEfficiency: this.calculateMemoryEfficiency(diff, previousFile),
      contextPreservation: this.estimateContextPreservation(diff, previousFile),
    };
  }

  /**
   * Analyze contextual factors that influence edit strategy selection
   */
  analyzeContextualFactors(diff, previousFile) {
    const diffText = diff.diffText;
    
    return {
      // Function-level changes are more important to preserve
      functionChanges: (diffText.match(/[+-]\s*function\s+\w+/g) || []).length,
      
      // Import/export changes affect file relationships
      dependencyChanges: (diffText.match(/[+-]\s*(import|export)/g) || []).length,
      
      // Type/interface changes in TypeScript
      typeChanges: (diffText.match(/[+-]\s*(interface|type|class)\s+\w+/g) || []).length,
      
      // Configuration or constant changes
      configChanges: (diffText.match(/[+-]\s*(const|let|var)\s+[A-Z_]+/g) || []).length,
      
      // Comment changes (less critical)
      commentOnlyChanges: this.isCommentOnlyChange(diffText),
      
      // Whitespace/formatting changes (least critical)
      formattingOnlyChanges: this.isFormattingOnlyChange(diffText, previousFile),
    };
  }

  /**
   * Calculate memory efficiency for different strategies
   */
  calculateMemoryEfficiency(diff, previousFile) {
    const originalTokens = previousFile?.tokens || 1000;
    const diffTokens = this.tokenCounter.calculateTokenCountForContent(diff.diffText);
    const summaryTokens = Math.min(200, diffTokens * 0.1); // Estimated summary size
    
    return {
      keepBoth: {
        tokens: originalTokens + diffTokens + originalTokens, // V1 + diff + V2
        efficiency: 1.0, // Baseline
        memoryOverhead: (diffTokens + originalTokens) / originalTokens,
      },
      replaceWithDiff: {
        tokens: diffTokens + originalTokens, // diff + V2
        efficiency: (originalTokens + diffTokens + originalTokens) / (diffTokens + originalTokens),
        memoryOverhead: diffTokens / originalTokens,
      },
      diffMarkerOnly: {
        tokens: summaryTokens + originalTokens, // summary + V2
        efficiency: (originalTokens + diffTokens + originalTokens) / (summaryTokens + originalTokens),
        memoryOverhead: summaryTokens / originalTokens,
      },
    };
  }

  /**
   * Estimate context preservation percentage for each strategy
   */
  estimateContextPreservation(diff, previousFile) {
    const contextualFactors = this.analyzeContextualFactors(diff, previousFile);
    
    // Base preservation rates from conversation insights
    let keepBothPreservation = 1.0; // 100%
    let replaceWithDiffPreservation = 0.875; // 87.5% (conversation estimate: 85-90%)
    let diffMarkerPreservation = 0.3; // 30% (summary only)
    
    // Adjust based on contextual factors
    if (contextualFactors.functionChanges > 0) {
      replaceWithDiffPreservation += 0.05; // Function changes preserve well in diffs
    }
    
    if (contextualFactors.dependencyChanges > 0) {
      replaceWithDiffPreservation += 0.03; // Import/export changes are clear in diffs
    }
    
    if (contextualFactors.commentOnlyChanges) {
      replaceWithDiffPreservation += 0.1; // Comment changes are very clear in diffs
      diffMarkerPreservation += 0.2; // Even summaries can capture comment changes well
    }
    
    if (contextualFactors.formattingOnlyChanges) {
      replaceWithDiffPreservation += 0.05; // Formatting changes are obvious
      diffMarkerPreservation += 0.1;
    }
    
    return {
      keepBoth: Math.min(1.0, keepBothPreservation),
      replaceWithDiff: Math.min(1.0, replaceWithDiffPreservation),
      diffMarkerOnly: Math.min(1.0, diffMarkerPreservation),
    };
  }

  /**
   * Enhanced strategy selection based on conversation insights
   */
  selectEditStrategy(magnitude, diff) {
    const { memoryEfficiency, contextPreservation, contextualFactors } = magnitude;
    
    // Special cases based on conversation insights
    
    // 1. Comment or formatting only changes - always use diff (high preservation, low cost)
    if (contextualFactors.commentOnlyChanges || contextualFactors.formattingOnlyChanges) {
      return "REPLACE_WITH_DIFF";
    }
    
    // 2. Very small changes with high context importance
    if (magnitude.changePercentage < 0.05 && contextualFactors.functionChanges > 0) {
      return "KEEP_BOTH"; // Function changes need full context
    }
    
    // 3. Memory-conscious thresholds from conversation
    
    // Small edits: < 10% change AND < 500 tokens AND good memory efficiency
    if (magnitude.changePercentage < 0.1 && 
        diff.diffText.length < 500 && 
        memoryEfficiency.keepBoth.memoryOverhead < 0.2) {
      return "KEEP_BOTH";
    }
    
    // Large structural changes: > 50% change OR structural OR poor context preservation
    if (magnitude.changePercentage > 0.5 || 
        magnitude.isStructuralChange ||
        contextPreservation.replaceWithDiff < 0.6) {
      return "DIFF_MARKER_ONLY";
    }
    
    // Medium edits: The "sweet spot" from conversation - 85-90% context, major memory savings
    // This is where the conversation insight really shines
    if (magnitude.changePercentage >= 0.1 && magnitude.changePercentage <= 0.5) {
      // Additional check: ensure diff strategy actually provides good value
      const efficiencyGain = memoryEfficiency.replaceWithDiff.efficiency;
      const contextLoss = 1 - contextPreservation.replaceWithDiff;
      
      // If we get > 1.5x efficiency with < 20% context loss, use diff strategy
      if (efficiencyGain > 1.5 && contextLoss < 0.2) {
        return "REPLACE_WITH_DIFF";
      }
    }
    
    // Default to replace with diff for most medium cases
    return "REPLACE_WITH_DIFF";
  }

  /**
   * Check if changes are comment-only
   */
  isCommentOnlyChange(diffText) {
    const meaningfulLines = diffText.split('\n').filter(line => {
      const trimmed = line.trim();
      if (!trimmed.startsWith('+') && !trimmed.startsWith('-')) return false;
      
      const content = trimmed.substring(1).trim();
      // Check if it's a comment line
      return content.startsWith('//') || 
             content.startsWith('/*') || 
             content.startsWith('*') ||
             content.startsWith('<!--') ||
             content.startsWith('#');
    });
    
    const totalChanges = diffText.split('\n').filter(line => 
      line.trim().startsWith('+') || line.trim().startsWith('-')
    ).length;
    
    return meaningfulLines.length > 0 && meaningfulLines.length === totalChanges;
  }

  /**
   * Check if changes are formatting/whitespace only
   */
  isFormattingOnlyChange(diffText, previousFile) {
    if (!previousFile) return false;
    
    // Remove all whitespace and compare
    const oldContentNormalized = previousFile.content.replace(/\s+/g, ' ').trim();
    const newContentNormalized = this.contentStore.files.get(previousFile.filePath)?.content
      ?.replace(/\s+/g, ' ').trim();
    
    return oldContentNormalized === newContentNormalized;
  }

  /**
   * Handle small edits - keep both versions temporarily
   */
  handleSmallEdit(fileRef, diffRef, diff, filePath) {
    // Add compact diff to context
    this.activeContext.push({
      role: "system",
      content: `Small Edit: ${filePath}\n\`\`\`diff\n${diff.diffText}\n\`\`\``,
      ref: diffRef,
      timestamp: Date.now(),
      isReference: true,
      isDiff: true,
      relatedFile: filePath,
      editStrategy: "KEEP_BOTH",
    });

    this.loadedRefs.add(diffRef);
    this.refreshReference(fileRef);

    // Auto-cleanup after 2 minutes for small edits
    setTimeout(() => this.cleanupStaleDiff(diffRef, filePath), 2 * 60 * 1000);
  }

  /**
   * Handle medium edits - replace original with diff marker
   */
  handleMediumEdit(fileRef, diffRef, diff, filePath) {
    // Remove original file from context
    this.unloadReference(fileRef);

    // Add diff marker in place of original
    this.activeContext.push({
      role: "system",
      content: `File Edit: ${filePath}\n\`\`\`diff\n${diff.diffText}\n\`\`\`\n\n*Original file replaced with diff for memory efficiency*`,
      ref: diffRef,
      timestamp: Date.now(),
      isReference: true,
      isDiff: true,
      relatedFile: filePath,
      editStrategy: "REPLACE_WITH_DIFF",
      replacesOriginal: true,
    });

    this.loadedRefs.add(diffRef);

    // Add new version
    this.refreshReference(fileRef);

    // Longer cleanup time for medium edits
    setTimeout(() => this.cleanupStaleDiff(diffRef, filePath), 5 * 60 * 1000);
  }

  /**
   * Handle large edits - use minimal diff marker only
   */
  handleLargeEdit(fileRef, diffRef, diff, filePath) {
    // Remove original file from context
    this.unloadReference(fileRef);

    // Add minimal edit marker
    const summary = this.createEditSummary(diff);
    this.activeContext.push({
      role: "system",
      content: `Major Edit: ${filePath}\n${summary}\n\n*File significantly changed - see current version for details*`,
      ref: diffRef,
      timestamp: Date.now(),
      isReference: true,
      isDiff: true,
      relatedFile: filePath,
      editStrategy: "DIFF_MARKER_ONLY",
      replacesOriginal: true,
    });

    this.loadedRefs.add(diffRef);

    // Add new version
    this.refreshReference(fileRef);

    // Quick cleanup for large edits (they're less likely to be referenced)
    setTimeout(() => this.cleanupStaleDiff(diffRef, filePath), 1 * 60 * 1000);
  }

  /**
   * Check if this is a structural change (functions added/removed, etc.)
   */
  isStructuralChange(diffText) {
    const structuralPatterns = [
      /[+-]\s*function\s+\w+/, // Function additions/removals
      /[+-]\s*class\s+\w+/, // Class additions/removals
      /[+-]\s*import\s+/, // Import changes
      /[+-]\s*export\s+/, // Export changes
      /[+-]\s*const\s+\w+\s*=/, // Major constant definitions
    ];

    return structuralPatterns.some((pattern) => pattern.test(diffText));
  }

  /**
   * Create a concise summary of major edits
   */
  createEditSummary(diff) {
    const lines = diff.diffText.split("\n");
    const additions = lines.filter((line) => line.startsWith("+")).length;
    const deletions = lines.filter((line) => line.startsWith("-")).length;

    let summary = `${additions} additions, ${deletions} deletions`;

    // Add context about what changed
    if (diff.diffText.includes("function")) {
      summary += " (functions modified)";
    }
    if (diff.diffText.includes("import") || diff.diffText.includes("export")) {
      summary += " (imports/exports changed)";
    }
    if (diff.diffText.includes("class")) {
      summary += " (class structure changed)";
    }

    return summary;
  }

  /**
   * Generate a meaningful diff between file versions - Enhanced based on conversation insights
   */
  generateDiff(oldContent, newContent) {
    if (oldContent === newContent) {
      return { hasChanges: false, diffText: "", changeCount: 0 };
    }

    const oldLines = oldContent.split("\n");
    const newLines = newContent.split("\n");

    // Enhanced diff algorithm with better context preservation
    const diffResult = this.generateSmartDiff(oldLines, newLines);
    
    return {
      hasChanges: true,
      diffText: diffResult.diffText,
      changeCount: diffResult.changeCount,
      oldLineCount: oldLines.length,
      newLineCount: newLines.length,
      // Additional metadata from conversation insights
      changeType: diffResult.changeType,
      contextQuality: diffResult.contextQuality,
      memoryEfficiency: diffResult.memoryEfficiency,
    };
  }

  /**
   * Smart diff generation that preserves context better
   */
  generateSmartDiff(oldLines, newLines) {
    const diffLines = [];
    let changeCount = 0;
    let contextLines = 0;
    
    // Use a more sophisticated diff algorithm
    const changes = this.computeLineChanges(oldLines, newLines);
    
    // Group changes and add appropriate context
    const changeGroups = this.groupChanges(changes);
    
    for (const group of changeGroups) {
      // Add context before the change group
      const contextBefore = this.getContextLines(oldLines, group.startLine - 3, group.startLine - 1);
      diffLines.push(...contextBefore.map(line => `  ${line}`));
      contextLines += contextBefore.length;
      
      // Add the actual changes
      for (const change of group.changes) {
        if (change.type === 'delete') {
          diffLines.push(`- ${change.line}`);
          changeCount++;
        } else if (change.type === 'add') {
          diffLines.push(`+ ${change.line}`);
          changeCount++;
        } else if (change.type === 'modify') {
          diffLines.push(`- ${change.oldLine}`);
          diffLines.push(`+ ${change.newLine}`);
          changeCount++;
        }
      }
      
      // Add context after the change group
      const contextAfter = this.getContextLines(oldLines, group.endLine + 1, group.endLine + 3);
      diffLines.push(...contextAfter.map(line => `  ${line}`));
      contextLines += contextAfter.length;
      
      // Add separator between change groups
      if (changeGroups.indexOf(group) < changeGroups.length - 1) {
        diffLines.push('  ...');
      }
    }
    
    // Analyze the type of changes for better strategy selection
    const changeType = this.analyzeChangeType(changes);
    
    return {
      diffText: diffLines.join('\n'),
      changeCount,
      changeType,
      contextQuality: this.calculateContextQuality(changeCount, contextLines, diffLines.length),
      memoryEfficiency: this.calculateDiffMemoryEfficiency(diffLines.length, oldLines.length),
    };
  }

  /**
   * Compute line-by-line changes using a simple LCS-based approach
   */
  computeLineChanges(oldLines, newLines) {
    const changes = [];
    let oldIndex = 0;
    let newIndex = 0;
    
    while (oldIndex < oldLines.length || newIndex < newLines.length) {
      const oldLine = oldLines[oldIndex];
      const newLine = newLines[newIndex];
      
      if (oldIndex >= oldLines.length) {
        // Only new lines left
        changes.push({ type: 'add', line: newLine, newIndex });
        newIndex++;
      } else if (newIndex >= newLines.length) {
        // Only old lines left
        changes.push({ type: 'delete', line: oldLine, oldIndex });
        oldIndex++;
      } else if (oldLine === newLine) {
        // Lines match
        oldIndex++;
        newIndex++;
      } else {
        // Lines differ - check if it's a modification or insertion/deletion
        const nextOldMatch = this.findNextMatch(oldLines, oldIndex + 1, newLine);
        const nextNewMatch = this.findNextMatch(newLines, newIndex + 1, oldLine);
        
        if (nextOldMatch !== -1 && (nextNewMatch === -1 || nextOldMatch < nextNewMatch)) {
          // Old line was deleted
          changes.push({ type: 'delete', line: oldLine, oldIndex });
          oldIndex++;
        } else if (nextNewMatch !== -1) {
          // New line was inserted
          changes.push({ type: 'add', line: newLine, newIndex });
          newIndex++;
        } else {
          // Line was modified
          changes.push({ type: 'modify', oldLine, newLine, oldIndex, newIndex });
          oldIndex++;
          newIndex++;
        }
      }
    }
    
    return changes;
  }

  /**
   * Find the next matching line in an array
   */
  findNextMatch(lines, startIndex, targetLine) {
    for (let i = startIndex; i < Math.min(lines.length, startIndex + 5); i++) {
      if (lines[i] === targetLine) {
        return i;
      }
    }
    return -1;
  }

  /**
   * Group nearby changes together for better context
   */
  groupChanges(changes) {
    if (changes.length === 0) return [];
    
    const groups = [];
    let currentGroup = {
      startLine: changes[0].oldIndex || changes[0].newIndex || 0,
      endLine: changes[0].oldIndex || changes[0].newIndex || 0,
      changes: [changes[0]]
    };
    
    for (let i = 1; i < changes.length; i++) {
      const change = changes[i];
      const lineNumber = change.oldIndex || change.newIndex || 0;
      
      // If changes are within 3 lines of each other, group them
      if (lineNumber - currentGroup.endLine <= 3) {
        currentGroup.changes.push(change);
        currentGroup.endLine = lineNumber;
      } else {
        groups.push(currentGroup);
        currentGroup = {
          startLine: lineNumber,
          endLine: lineNumber,
          changes: [change]
        };
      }
    }
    
    groups.push(currentGroup);
    return groups;
  }

  /**
   * Get context lines around changes
   */
  getContextLines(lines, startIndex, endIndex) {
    const start = Math.max(0, startIndex);
    const end = Math.min(lines.length - 1, endIndex);
    
    const contextLines = [];
    for (let i = start; i <= end; i++) {
      if (lines[i] !== undefined) {
        contextLines.push(lines[i]);
      }
    }
    
    return contextLines;
  }

  /**
   * Analyze the type of changes for strategy selection
   */
  analyzeChangeType(changes) {
    const types = {
      additions: changes.filter(c => c.type === 'add').length,
      deletions: changes.filter(c => c.type === 'delete').length,
      modifications: changes.filter(c => c.type === 'modify').length,
    };
    
    const total = types.additions + types.deletions + types.modifications;
    
    if (total === 0) return 'none';
    if (types.modifications / total > 0.7) return 'refactor';
    if (types.additions / total > 0.7) return 'expansion';
    if (types.deletions / total > 0.7) return 'reduction';
    return 'mixed';
  }

  /**
   * Calculate context quality score
   */
  calculateContextQuality(changeCount, contextLines, totalLines) {
    if (totalLines === 0) return 0;
    
    const contextRatio = contextLines / totalLines;
    const changeRatio = changeCount / totalLines;
    
    // Higher context ratio and balanced change ratio = better quality
    return Math.min(1.0, contextRatio * 0.7 + (1 - changeRatio) * 0.3);
  }

  /**
   * Calculate memory efficiency of the diff
   */
  calculateDiffMemoryEfficiency(diffLines, originalLines) {
    if (originalLines === 0) return 1.0;
    
    const compressionRatio = diffLines / originalLines;
    
    // Lower ratio = better efficiency
    return Math.max(0.1, 1.0 - compressionRatio);
  }

  /**
   * Mark diff references for cleanup when they become stale
   */
  markForDiffCleanup(filePath, diffRef) {
    // Set up automatic cleanup of diff after it's not actively referenced
    setTimeout(() => {
      this.cleanupStaleDiff(diffRef, filePath);
    }, 5 * 60 * 1000); // 5 minutes
  }

  /**
   * Clean up stale diffs that are no longer actively referenced
   */
  cleanupStaleDiff(diffRef, filePath) {
    const stats = this.usageStats.get(diffRef);
    const now = Date.now();

    // If diff hasn't been accessed in the last 5 minutes, remove it
    if (!stats || now - stats.lastUsed > 5 * 60 * 1000) {
      this.unloadReference(diffRef);

      // Also clean up the diff from file storage
      const fileData = this.contentStore.files.get(filePath);
      if (fileData && fileData.diff) {
        delete fileData.diff;
        delete fileData.previousVersion;
      }
    }
  }

  /**
   * Enhanced context size management that considers diff importance
   */
  manageContextSize() {
    const currentTokens = this.getCurrentTokens();
    const threshold = this.config.maxTokens * this.config.safetyThreshold;

    if (currentTokens <= threshold) return;

    // Smart prioritization based on edit strategies
    const referencesToRemove = Array.from(this.loadedRefs)
      .map((ref) => {
        const stats = this.usageStats.get(ref) || { lastUsed: 0, useCount: 0 };
        const msg = this.activeContext.find((m) => m.ref === ref);

        let priority = stats.useCount * 0.3 + (stats.lastUsed / 1000000) * 0.7;

        // Adjust priority based on edit strategy
        if (msg?.isDiff) {
          switch (msg.editStrategy) {
            case "KEEP_BOTH":
              priority *= 0.6; // Medium priority - recent small changes
              break;
            case "REPLACE_WITH_DIFF":
              priority *= 0.4; // Lower priority - already replaced original
              break;
            case "DIFF_MARKER_ONLY":
              priority *= 0.2; // Lowest priority - minimal info
              break;
          }
        } else if (ref.startsWith("file:")) {
          priority *= 1.2; // Files are important
        } else if (ref.startsWith("terminal:")) {
          priority *= 0.7; // Terminal is moderately important
        }

        return { ref, priority, editStrategy: msg?.editStrategy };
      })
      .sort((a, b) => a.priority - b.priority);

    for (const { ref } of referencesToRemove) {
      this.unloadReference(ref);

      if (this.getCurrentTokens() <= threshold) break;
    }
  }

  /**
   * Remove a reference from active context
   */
  unloadReference(ref) {
    this.activeContext = this.activeContext.filter((msg) => msg.ref !== ref);
    this.loadedRefs.delete(ref);
  }

  /**
   * Add terminal command and output
   */
  addTerminalEntry(command, output, metadata = {}) {
    const commandId = `cmd_${Date.now()}`;
    const isError =
      output.includes("Error") ||
      output.includes("Failed") ||
      metadata.exitCode !== 0;

    // Compress output based on success/failure and length
    const processedOutput = this.processTerminalOutput(output, isError);

    this.contentStore.terminal.set(commandId, {
      command,
      output: processedOutput,
      originalLength: output.length,
      compressed: processedOutput.length < output.length,
      isError,
      timestamp: Date.now(),
      metadata,
    });

    // Auto-compress old terminal entries
    this.compressOldTerminalEntries();

    return {
      success: true,
      commandId,
      compressed: processedOutput.length < output.length,
    };
  }

  /**
   * Update task/goal information
   */
  updateTask(taskId, taskData) {
    this.contentStore.tasks.set(taskId, {
      ...taskData,
      lastUpdated: Date.now(),
    });

    // If task is currently referenced, refresh it
    const taskRef = `task:${taskId}`;
    if (this.loadedRefs.has(taskRef)) {
      this.refreshReference(taskRef);
    }

    return { success: true, taskId };
  }

  /**
   * Get the current context for API calls - Enhanced with conversation insights
   */
  getCurrentContext() {
    // Calculate total tokens
    const totalTokens = this.activeContext.reduce((sum, msg) => {
      const content =
        typeof msg.content === "string"
          ? msg.content
          : JSON.stringify(msg.content);
      return sum + this.tokenCounter.calculateTokenCountForContent(content);
    }, 0);

    // Enhanced metadata based on conversation insights
    const memoryAnalysis = this.analyzeMemoryEfficiency();
    const editStrategies = this.getEditStrategyBreakdown();

    return {
      messages: this.activeContext.map((msg) => ({
        role: msg.role,
        content: msg.content,
        timestamp: msg.timestamp,
      })),
      metadata: {
        totalTokens,
        maxTokens: this.config.maxTokens,
        utilization: totalTokens / this.config.maxTokens,
        loadedRefs: Array.from(this.loadedRefs),
        storeStats: this.getStoreStatistics(),
        // New insights from conversation
        memoryAnalysis,
        editStrategies,
        contextQuality: this.calculateOverallContextQuality(),
        efficiencyGains: this.calculateEfficiencyGains(),
      },
    };
  }

  /**
   * Extract references from message content
   */
  extractReferences(message) {
    const refs = [];
    const content =
      typeof message.content === "string"
        ? message.content
        : JSON.stringify(message.content);

    // Look for file references
    const fileMatches = content.match(/(?:file:|@)([^\s,]+\.[a-zA-Z]+)/g);
    if (fileMatches) {
      fileMatches.forEach((match) => {
        const filePath = match.replace(/^(?:file:|@)/, "");
        refs.push(`file:${filePath}`);

        // If this is an edit operation, also load recent diff
        if (
          content.includes("edit") ||
          content.includes("change") ||
          content.includes("modify")
        ) {
          const fileData = this.contentStore.files.get(filePath);
          if (fileData && fileData.diff) {
            // Find the most recent diff for this file
            const diffRefs = Array.from(this.loadedRefs).filter((ref) =>
              ref.startsWith(`diff:${filePath}:`)
            );
            if (diffRefs.length > 0) {
              // Get the most recent diff
              const latestDiff = diffRefs.sort().pop();
              refs.push(latestDiff);
            }
          }
        }
      });
    }

    // Look for function references
    const functionMatches = content.match(/function\s+(\w+)|(\w+)\s*\(/g);
    if (functionMatches) {
      functionMatches.forEach((match) => {
        const funcName = match.replace(/function\s+|[\s\(]/g, "");
        if (funcName) refs.push(`function:${funcName}`);
      });
    }

    // Look for terminal references
    if (
      content.includes("run") ||
      content.includes("command") ||
      content.includes("terminal")
    ) {
      refs.push("terminal:recent");
    }

    // Look for task references
    const taskMatches = content.match(/task:(\w+)/g);
    if (taskMatches) {
      taskMatches.forEach((match) => refs.push(match));
    }

    // Look for diff references
    if (
      content.includes("diff") ||
      content.includes("change") ||
      content.includes("what changed")
    ) {
      // Load recent diffs
      const diffRefs = Array.from(this.loadedRefs).filter((ref) =>
        ref.startsWith("diff:")
      );
      refs.push(...diffRefs.slice(-3)); // Last 3 diffs
    }

    return refs;
  }

  /**
   * Load a reference into the active context
   */
  loadReference(ref) {
    if (this.loadedRefs.has(ref)) {
      // Update usage stats
      this.updateUsageStats(ref);
      return;
    }

    const content = this.resolveReference(ref);
    if (!content) return;

    // Add to active context
    this.activeContext.push({
      role: "system",
      content: content,
      ref: ref,
      timestamp: Date.now(),
      isReference: true,
    });

    this.loadedRefs.add(ref);
    this.updateUsageStats(ref);
  }

  /**
   * Resolve a reference to actual content
   */
  resolveReference(ref) {
    const [type, identifier] = ref.split(":", 2);

    switch (type) {
      case "file":
        return this.resolveFileReference(identifier);
      case "function":
        return this.resolveFunctionReference(identifier);
      case "terminal":
        return this.resolveTerminalReference(identifier);
      case "task":
        return this.resolveTaskReference(identifier);
      default:
        return null;
    }
  }

  /**
   * Enhanced reference resolution that understands edit strategies
   */
  resolveFileReference(filePath) {
    const fileData = this.contentStore.files.get(filePath);
    if (!fileData) return null;

    // Check if there's an active diff that replaces the original
    const replacingDiff = Array.from(this.loadedRefs).find((ref) => {
      if (!ref.startsWith(`diff:${filePath}:`)) return false;
      const diffMsg = this.activeContext.find((msg) => msg.ref === ref);
      return diffMsg?.replacesOriginal;
    });

    if (replacingDiff) {
      // Original is replaced by diff, just return current version
      return `File: ${filePath} (current version)\n\`\`\`\n${fileData.content}\n\`\`\``;
    }

    // Normal file resolution logic
    const recentlyUsed = this.isRecentlyUsed(`file:${filePath}`);

    if (recentlyUsed || !this.config.functionHeadersOnly) {
      return `File: ${filePath}\n\`\`\`\n${fileData.content}\n\`\`\``;
    } else {
      return `File: ${filePath} (headers only)\n\`\`\`\n${fileData.headers.join(
        "\n"
      )}\n\`\`\``;
    }
  }

  /**
   * Resolve function reference across all files
   */
  resolveFunctionReference(functionName) {
    for (const [filePath, fileData] of this.contentStore.files) {
      if (
        fileData.content.includes(`function ${functionName}`) ||
        fileData.content.includes(`${functionName} =`) ||
        fileData.content.includes(`${functionName}:`)
      ) {
        // Extract just the function
        const functionCode = this.extractFunction(
          fileData.content,
          functionName
        );
        return `Function: ${functionName} from ${filePath}\n\`\`\`\n${functionCode}\n\`\`\``;
      }
    }
    return null;
  }

  /**
   * Resolve terminal reference
   */
  resolveTerminalReference(identifier) {
    if (identifier === "recent") {
      // Get last few terminal entries
      const recentEntries = Array.from(this.contentStore.terminal.values())
        .sort((a, b) => b.timestamp - a.timestamp)
        .slice(0, 5);

      return recentEntries
        .map(
          (entry) =>
            `$ ${entry.command}\n${entry.output}${
              entry.compressed ? " ..." : ""
            }`
        )
        .join("\n\n");
    }

    const entry = this.contentStore.terminal.get(identifier);
    return entry ? `$ ${entry.command}\n${entry.output}` : null;
  }

  /**
   * Resolve task reference
   */
  resolveTaskReference(taskId) {
    const task = this.contentStore.tasks.get(taskId);
    return task ? `Task: ${taskId}\n${JSON.stringify(task, null, 2)}` : null;
  }

  /**
   * Process terminal output for compression
   */
  processTerminalOutput(output, isError) {
    // Keep errors mostly intact
    if (isError) {
      return output.length > 1000
        ? output.substring(0, 800) + "\n... (truncated)"
        : output;
    }

    // Compress successful output more aggressively
    if (output.length > 200) {
      const lines = output.split("\n");
      if (lines.length > 10) {
        return (
          lines.slice(0, 3).join("\n") +
          "\n... (success, " +
          (lines.length - 3) +
          " more lines)"
        );
      }
    }

    return output;
  }

  /**
   * Extract function headers from code
   */
  extractFunctionHeaders(content) {
    const headers = [];

    // Match various function patterns
    const patterns = [
      /(?:export\s+)?(?:async\s+)?function\s+(\w+)\s*\([^)]*\)/g,
      /(?:export\s+)?const\s+(\w+)\s*=\s*(?:async\s+)?\([^)]*\)\s*=>/g,
      /(\w+)\s*:\s*(?:async\s+)?function\s*\([^)]*\)/g,
      /(\w+)\s*:\s*\([^)]*\)\s*=>/g,
    ];

    patterns.forEach((pattern) => {
      let match;
      while ((match = pattern.exec(content)) !== null) {
        headers.push(match[0]);
      }
    });

    return headers;
  }

  /**
   * Extract a specific function from code
   */
  extractFunction(content, functionName) {
    // Simple extraction - could be made more sophisticated
    const lines = content.split("\n");
    let startLine = -1;
    let braceCount = 0;
    let inFunction = false;

    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];

      if (
        line.includes(`function ${functionName}`) ||
        line.includes(`${functionName} =`) ||
        line.includes(`${functionName}:`)
      ) {
        startLine = i;
        inFunction = true;
      }

      if (inFunction) {
        braceCount += (line.match(/\{/g) || []).length;
        braceCount -= (line.match(/\}/g) || []).length;

        if (braceCount === 0 && startLine !== -1) {
          return lines.slice(startLine, i + 1).join("\n");
        }
      }
    }

    return `// Function ${functionName} not found or incomplete`;
  }

  /**
   * Update usage statistics for smart eviction
   */
  updateUsageStats(ref) {
    const stats = this.usageStats.get(ref) || {
      lastUsed: 0,
      useCount: 0,
      priority: 0.5,
    };
    stats.lastUsed = Date.now();
    stats.useCount++;
    this.usageStats.set(ref, stats);
  }

  /**
   * Check if a reference was recently used
   */
  isRecentlyUsed(ref, thresholdMinutes = 30) {
    const stats = this.usageStats.get(ref);
    if (!stats) return false;

    const threshold = Date.now() - thresholdMinutes * 60 * 1000;
    return stats.lastUsed > threshold;
  }

  /**
   * Compress old terminal entries
   */
  compressOldTerminalEntries() {
    const entries = Array.from(this.contentStore.terminal.entries()).sort(
      ([, a], [, b]) => b.timestamp - a.timestamp
    );

    // Keep recent entries uncompressed
    entries
      .slice(this.config.terminalCompressionAfter)
      .forEach(([id, entry]) => {
        if (!entry.compressed && !entry.isError) {
          entry.output = entry.isError
            ? entry.output
            : `${entry.command} completed successfully`;
          entry.compressed = true;
        }
      });
  }

  /**
   * Get current token count
   */
  getCurrentTokens() {
    return this.activeContext.reduce((sum, msg) => {
      const content =
        typeof msg.content === "string"
          ? msg.content
          : JSON.stringify(msg.content);
              return sum + this.tokenCounter.calculateTokenCountForContent(content);
    }, 0);
  }

  /**
   * Get storage statistics
   */
  getStoreStatistics() {
    return {
      files: this.contentStore.files.size,
      terminal: this.contentStore.terminal.size,
      tasks: this.contentStore.tasks.size,
      loadedRefs: this.loadedRefs.size,
      totalTokens: this.getCurrentTokens(),
      utilization: this.getCurrentTokens() / this.config.maxTokens,
    };
  }

  /**
   * Refresh a loaded reference
   */
  refreshReference(ref) {
    if (!this.loadedRefs.has(ref)) return;

    // Remove old version
    this.unloadReference(ref);

    // Load new version
    this.loadReference(ref);
  }

  /**
   * Export current state
   */
  exportState() {
    return {
      config: this.config,
      contentStore: {
        files: Array.from(this.contentStore.files.entries()),
        terminal: Array.from(this.contentStore.terminal.entries()),
        tasks: Array.from(this.contentStore.tasks.entries()),
      },
      usageStats: Array.from(this.usageStats.entries()),
      timestamp: Date.now(),
    };
  }

  /**
   * Import previously exported state
   */
  importState(state) {
    this.config = { ...this.config, ...state.config };

    this.contentStore.files = new Map(state.contentStore.files);
    this.contentStore.terminal = new Map(state.contentStore.terminal);
    this.contentStore.tasks = new Map(state.contentStore.tasks);

    this.usageStats = new Map(state.usageStats);
  }

  /**
   * Analyze memory efficiency of current context
   */
  analyzeMemoryEfficiency() {
    const diffMessages = this.activeContext.filter(msg => msg.isDiff);
    const fileMessages = this.activeContext.filter(msg => msg.ref?.startsWith('file:'));
    
    let totalSavings = 0;
    let totalOriginalSize = 0;
    
    diffMessages.forEach(msg => {
      const filePath = msg.relatedFile;
      const fileData = this.contentStore.files.get(filePath);
      
      if (fileData) {
        const originalTokens = fileData.tokens;
        const diffTokens = this.tokenCounter.calculateTokenCountForContent(msg.content);
        
        totalOriginalSize += originalTokens;
        
        switch (msg.editStrategy) {
          case 'REPLACE_WITH_DIFF':
            totalSavings += originalTokens - diffTokens; // Saved by not keeping original
            break;
          case 'DIFF_MARKER_ONLY':
            totalSavings += originalTokens - (diffTokens * 0.1); // Saved by using summary
            break;
        }
      }
    });
    
    return {
      totalSavings,
      totalOriginalSize,
      efficiencyRatio: totalOriginalSize > 0 ? totalSavings / totalOriginalSize : 0,
      memoryFootprint: this.getCurrentTokens(),
      projectedWithoutOptimization: this.getCurrentTokens() + totalSavings,
    };
  }

  /**
   * Get breakdown of edit strategies currently in use
   */
  getEditStrategyBreakdown() {
    const strategies = {
      KEEP_BOTH: 0,
      REPLACE_WITH_DIFF: 0,
      DIFF_MARKER_ONLY: 0,
    };
    
    const contextPreservation = {
      KEEP_BOTH: 0,
      REPLACE_WITH_DIFF: 0,
      DIFF_MARKER_ONLY: 0,
    };
    
    this.activeContext.filter(msg => msg.isDiff).forEach(msg => {
      if (msg.editStrategy) {
        strategies[msg.editStrategy]++;
        
        // Estimate context preservation based on conversation insights
        switch (msg.editStrategy) {
          case 'KEEP_BOTH':
            contextPreservation[msg.editStrategy] += 1.0;
            break;
          case 'REPLACE_WITH_DIFF':
            contextPreservation[msg.editStrategy] += 0.875; // 87.5% from conversation
            break;
          case 'DIFF_MARKER_ONLY':
            contextPreservation[msg.editStrategy] += 0.3; // 30% from conversation
            break;
        }
      }
    });
    
    const totalEdits = Object.values(strategies).reduce((sum, count) => sum + count, 0);
    const avgContextPreservation = totalEdits > 0 
      ? Object.values(contextPreservation).reduce((sum, val) => sum + val, 0) / totalEdits
      : 1.0;
    
    return {
      strategies,
      totalEdits,
      averageContextPreservation: avgContextPreservation,
      // Insights from conversation
      memoryEfficiencyScore: this.calculateMemoryEfficiencyScore(strategies),
      contextQualityScore: avgContextPreservation,
    };
  }

  /**
   * Calculate overall context quality
   */
  calculateOverallContextQuality() {
    const totalMessages = this.activeContext.length;
    if (totalMessages === 0) return 1.0;
    
    let qualitySum = 0;
    
    this.activeContext.forEach(msg => {
      if (msg.isDiff) {
        // Use conversation insights for diff quality
        switch (msg.editStrategy) {
          case 'KEEP_BOTH':
            qualitySum += 1.0;
            break;
          case 'REPLACE_WITH_DIFF':
            qualitySum += 0.875; // 85-90% from conversation
            break;
          case 'DIFF_MARKER_ONLY':
            qualitySum += 0.3;
            break;
          default:
            qualitySum += 0.8;
        }
      } else {
        qualitySum += 1.0; // Full quality for non-diff messages
      }
    });
    
    return qualitySum / totalMessages;
  }

  /**
   * Calculate efficiency gains from hybrid approach
   */
  calculateEfficiencyGains() {
    const memoryAnalysis = this.analyzeMemoryEfficiency();
    const editBreakdown = this.getEditStrategyBreakdown();
    
    // Compare to traditional approach (keeping all versions)
    const traditionalMemory = memoryAnalysis.projectedWithoutOptimization;
    const currentMemory = memoryAnalysis.memoryFootprint;
    
    const memorySavings = traditionalMemory > 0 
      ? (traditionalMemory - currentMemory) / traditionalMemory 
      : 0;
    
    return {
      memorySavingsPercentage: memorySavings * 100,
      contextPreservationPercentage: editBreakdown.averageContextPreservation * 100,
      // The key insight from conversation: 85-90% context with major memory savings
      isOptimalRange: editBreakdown.averageContextPreservation >= 0.85 && memorySavings >= 0.4,
      efficiencyScore: (memorySavings * 0.6) + (editBreakdown.averageContextPreservation * 0.4),
    };
  }

  /**
   * Calculate memory efficiency score based on strategy distribution
   */
  calculateMemoryEfficiencyScore(strategies) {
    const total = Object.values(strategies).reduce((sum, count) => sum + count, 0);
    if (total === 0) return 1.0;
    
    // Weight strategies by their efficiency (from conversation insights)
    const efficiencyWeights = {
      KEEP_BOTH: 0.5,        // Low efficiency (2x memory usage)
      REPLACE_WITH_DIFF: 0.9, // High efficiency (major savings, good context)
      DIFF_MARKER_ONLY: 0.8, // Good efficiency (max savings, minimal context)
    };
    
    let weightedScore = 0;
    Object.entries(strategies).forEach(([strategy, count]) => {
      weightedScore += (count / total) * efficiencyWeights[strategy];
    });
    
    return weightedScore;
  }
}

export default PointerBasedContextManager;
