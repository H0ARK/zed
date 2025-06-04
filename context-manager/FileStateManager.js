/**
 * File State Manager
 * Manages file state tracking and restoration during context compaction
 */

export class FileStateManager {
  constructor() {
    this.fileState = new Map();
  }

  /**
   * Add or update a file in the state
   * @param {string} filePath - Path to the file
   * @param {Object} fileData - File data object
   */
  setFile(filePath, fileData) {
    this.fileState.set(filePath, {
      ...fileData,
      lastAccessed: Date.now(),
      accessCount: (this.fileState.get(filePath)?.accessCount || 0) + 1,
    });
  }

  /**
   * Get file data from state
   * @param {string} filePath - Path to the file
   * @returns {Object|null} File data or null if not found
   */
  getFile(filePath) {
    const fileData = this.fileState.get(filePath);
    if (fileData) {
      // Update access time
      fileData.lastAccessed = Date.now();
      fileData.accessCount++;
    }
    return fileData || null;
  }

  /**
   * Remove a file from state
   * @param {string} filePath - Path to the file
   */
  removeFile(filePath) {
    this.fileState.delete(filePath);
  }

  /**
   * Clear all file state
   */
  clearAll() {
    this.fileState.clear();
  }

  /**
   * Get all file paths currently in state
   * @returns {Array<string>} Array of file paths
   */
  getAllFilePaths() {
    return Array.from(this.fileState.keys());
  }

  /**
   * Get file state statistics
   * @returns {Object} Statistics about the file state
   */
  getStatistics() {
    const files = Array.from(this.fileState.values());
    const totalSize = files.reduce(
      (sum, file) => sum + (file.content?.length || 0),
      0
    );

    return {
      fileCount: this.fileState.size,
      totalContentLength: totalSize,
      averageFileSize: files.length > 0 ? totalSize / files.length : 0,
      mostAccessed: this.getMostAccessedFiles(5),
      recentlyAccessed: this.getRecentlyAccessedFiles(5),
    };
  }

  /**
   * Get most frequently accessed files
   * @param {number} limit - Number of files to return
   * @returns {Array} Array of file objects sorted by access count
   */
  getMostAccessedFiles(limit = 10) {
    return Array.from(this.fileState.entries())
      .map(([path, data]) => ({ path, ...data }))
      .sort((a, b) => b.accessCount - a.accessCount)
      .slice(0, limit);
  }

  /**
   * Get recently accessed files
   * @param {number} limit - Number of files to return
   * @returns {Array} Array of file objects sorted by last access time
   */
  getRecentlyAccessedFiles(limit = 10) {
    return Array.from(this.fileState.entries())
      .map(([path, data]) => ({ path, ...data }))
      .sort((a, b) => b.lastAccessed - a.lastAccessed)
      .slice(0, limit);
  }

  /**
   * Restore file state from a compacted conversation
   * This analyzes the summary and recent messages to determine which files
   * should be restored to maintain context
   * @param {Array} messages - The compacted messages
   * @param {string} summary - The conversation summary
   * @returns {Promise<Array>} Array of files that were restored
   */
  async restoreFileStateFromCompact(messages, summary) {
    const restoredFiles = [];
    const mentionedFiles = this.extractFilePathsFromText(summary);

    // Also check recent messages for file references
    for (const message of messages.slice(-3)) {
      if (message.content) {
        const messageFiles = this.extractFilePathsFromText(
          typeof message.content === "string"
            ? message.content
            : JSON.stringify(message.content)
        );
        mentionedFiles.push(...messageFiles);
      }
    }

    // Remove duplicates
    const uniqueFiles = [...new Set(mentionedFiles)];

    // For each mentioned file, try to restore it if we have it in our state
    for (const filePath of uniqueFiles) {
      const fileData = this.getFile(filePath);
      if (fileData) {
        restoredFiles.push({
          path: filePath,
          restored: true,
          reason: "mentioned_in_summary",
        });
      } else {
        // File was mentioned but not in our state - might need to be re-read
        restoredFiles.push({
          path: filePath,
          restored: false,
          reason: "needs_reread",
        });
      }
    }

    return restoredFiles;
  }

  /**
   * Extract file paths from text using common patterns
   * @param {string} text - Text to analyze
   * @returns {Array<string>} Array of potential file paths
   */
  extractFilePathsFromText(text) {
    if (!text || typeof text !== "string") return [];

    const filePaths = [];

    // Common file path patterns
    const patterns = [
      // Absolute paths
      /\/[a-zA-Z0-9_\-\.\/]+\.[a-zA-Z0-9]+/g,
      // Relative paths
      /[a-zA-Z0-9_\-\.\/]+\.[a-zA-Z0-9]+/g,
      // Paths in quotes
      /"([^"]+\.[a-zA-Z0-9]+)"/g,
      /'([^']+\.[a-zA-Z0-9]+)'/g,
      // Paths in backticks
      /`([^`]+\.[a-zA-Z0-9]+)`/g,
    ];

    for (const pattern of patterns) {
      const matches = text.match(pattern);
      if (matches) {
        filePaths.push(...matches);
      }
    }

    // Clean up and filter
    return filePaths
      .map((path) => path.replace(/['"`,]/g, "").trim())
      .filter((path) => path.length > 0 && path.includes("."))
      .filter((path) => !path.includes(" ")) // Remove paths with spaces (likely false positives)
      .slice(0, 20); // Limit to prevent excessive file operations
  }

  /**
   * Export file state for persistence
   * @returns {Object} Serializable file state
   */
  exportState() {
    return {
      files: Object.fromEntries(this.fileState),
      exportedAt: Date.now(),
    };
  }

  /**
   * Import file state from persistence
   * @param {Object} stateData - Previously exported state
   */
  importState(stateData) {
    if (stateData && stateData.files) {
      this.fileState = new Map(Object.entries(stateData.files));
    }
  }
}
