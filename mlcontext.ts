// Pointer-Based Context Manager — Pseudocode (agent memory & context assembly)
// Purpose: External content store + pointer loading + dynamic degradation under budget
// Key features added:
// - Multi-level representations (full | symbols | headers | diff | pointer)
// - Exponential decay of usage priority (time/turn-based)
// - Budgeted assembly by score with degrade-before-drop
// - Terminal compression (error-biased, head/tail, ring-buffer)
// - Diff lifecycle (TTL + supersede)
// - Memory efficiency + context quality analytics

// class PointerBasedContextManager {
//     constructor(options = {}) {
//       this.config = {
//         maxTokens: options.maxTokens || CONTEXT_WINDOW_CONFIG.MAX_TOKENS,
//         safetyThreshold: options.safetyThreshold || 0.7, // 70% safety zone
//         terminalCompressionAfter: options.terminalCompressionAfter || 5,
//         functionHeadersOnly: options.functionHeadersOnly ?? true,

//         // New
//         halfLifeMinutes: options.halfLifeMinutes || 45, // usage decay
//         perTypeBudgets: {
//           files: 0.50,
//           terminal: 0.15,
//           diffs: 0.15,
//           tasks: 0.20,
//           ...(options.perTypeBudgets || {}),
//         },
//         diffTTL: {
//           KEEP_BOTH: 120_000,        // 2m
//           REPLACE_WITH_DIFF: 300_000,// 5m
//           DIFF_MARKER_ONLY: 60_000,  // 1m
//           ...(options.diffTTL || {}),
//         },
//       };

//       this.tokenCounter = new TokenCounter();

//       // External content store
//       this.contentStore = {
//         files:    new Map(), // filePath -> { content, headers[], tokens, lastModified, metadata, previousVersion?, diff? }
//         terminal: new Map(), // commandId -> { command, output, compressed, isError, timestamp, metadata }
//         tasks:    new Map(), // taskId -> { description, status, context, lastUpdated }
//         references: new Map(), // refId -> any
//       };

//       // Live context and tracking
//       this.activeContext = [];      // array of { role, content, ref?, isReference?, level?, timestamp, ... }
//       this.loadedRefs    = new Set(); // Set<refId>
//       this.usageStats    = new Map(); // refId -> { lastUsed, useCount, priority }
//     }

//     // ---- Turn lifecycle ------------------------------------------------------

//     onTurnStart() {
//       this.decayUsageStats(this.config.halfLifeMinutes);
//     }

//     addMessage(message) {
//       const refs = this.extractReferences(message);
//       this.activeContext.push({ ...message, refs, timestamp: now() });

//       for (const ref of refs) this.loadReference(ref);
//       // Instead of naive size trim, assemble explicitly for this turn:
//       this.assembleContext({ userMessage: message, intent: this.inferIntent(message) });

//       return { success: true, refs: refs.length };
//     }

//     // ---- Files ---------------------------------------------------------------

//     updateFile(filePath, content, metadata = {}) {
//       const headers = this.extractFunctionHeaders(content);
//       const tokens  = this.tokenCounter.calculateTokenCountForContent(content);
//       const existing = this.contentStore.files.get(filePath);

//       const diff = existing ? this.generateDiff(existing.content, content) : null;

//       this.contentStore.files.set(filePath, {
//         content,
//         headers,
//         tokens,
//         lastModified: now(),
//         metadata: { ...metadata, isEdited: true },
//         previousVersion: existing ? existing.content : null,
//         diff,
//       });

//       // If referenced, apply hybrid update strategy
//       if (this.loadedRefs.has(`file:${filePath}`)) {
//         this.handleFileEdit(filePath, diff, existing);
//       }

//       return { success: true, tokens, headers: headers.length, hasDiff: !!diff?.hasChanges };
//     }

//     handleFileEdit(filePath, diff, previousFile) {
//       const fileRef = `file:${filePath}`;
//       const diffRef = `diff:${filePath}:${now()}`;

//       if (!diff || !diff.hasChanges) {
//         this.refreshReference(fileRef);
//         return;
//       }

//       const magnitude = this.calculateEditMagnitude(diff, previousFile);
//       const strategy  = this.selectEditStrategy(magnitude, diff);

//       if (strategy === 'KEEP_BOTH') {
//         this.handleSmallEdit(fileRef, diffRef, diff, filePath);
//       } else if (strategy === 'REPLACE_WITH_DIFF') {
//         this.handleMediumEdit(fileRef, diffRef, diff, filePath);
//       } else { // 'DIFF_MARKER_ONLY'
//         this.handleLargeEdit(fileRef, diffRef, diff, filePath);
//       }

//       // TTL + supersede
//       setTimeout(() => {
//         const newerExists = Array.from(this.loadedRefs).some(r => r.startsWith(`diff:${filePath}:`) && r > diffRef);
//         if (!newerExists) this.cleanupStaleDiff(diffRef, filePath);
//       }, this.config.diffTTL[strategy]);
//     }

//     calculateEditMagnitude(diff, previousFile) {
//       const totalLines = previousFile ? previousFile.content.split('\n').length : 100;
//       const changePercentage = diff.changeCount / totalLines;
//       const tokenImpact      = this.tokenCounter.calculateTokenCountForContent(diff.diffText) / (previousFile?.tokens || 1000);
//       const contextualFactors = this.analyzeContextualFactors(diff, previousFile);

//       return {
//         changePercentage,
//         tokenImpact,
//         changeCount: diff.changeCount,
//         isStructuralChange: this.isStructuralChange(diff.diffText),
//         contextualFactors,
//         memoryEfficiency: this.calculateMemoryEfficiency(diff, previousFile),
//         contextPreservation: this.estimateContextPreservation(diff, previousFile),
//       };
//     }

//     analyzeContextualFactors(diff, previousFile) {
//       const t = diff.diffText;
//       return {
//         functionChanges: (t.match(/[+-]\s*function\s+\w+/g) || []).length,
//         dependencyChanges: (t.match(/[+-]\s*(import|export)/g) || []).length,
//         typeChanges: (t.match(/[+-]\s*(interface|type|class)\s+\w+/g) || []).length,
//         configChanges: (t.match(/[+-]\s*(const|let|var)\s+[A-Z_]+/g) || []).length,
//         commentOnlyChanges: this.isCommentOnlyChange(t),
//         formattingOnlyChanges: this.isFormattingOnlyChange(t, previousFile),
//       };
//     }

//     calculateMemoryEfficiency(diff, previousFile) { /* compute tokens for keepBoth/replace/diffMarker */ }
//     estimateContextPreservation(diff, previousFile) { /* use heuristics incl. function/dependency changes */ }

//     selectEditStrategy(magnitude, diff) {
//       const { memoryEfficiency, contextPreservation, contextualFactors } = magnitude;

//       if (contextualFactors.commentOnlyChanges || contextualFactors.formattingOnlyChanges) return 'REPLACE_WITH_DIFF';
//       if (magnitude.changePercentage < 0.05 && contextualFactors.functionChanges > 0) return 'KEEP_BOTH';
//       if (magnitude.changePercentage > 0.5 || magnitude.isStructuralChange || contextPreservation.replaceWithDiff < 0.6) return 'DIFF_MARKER_ONLY';

//       if (magnitude.changePercentage >= 0.1 && magnitude.changePercentage <= 0.5) {
//         const efficiencyGain = memoryEfficiency.replaceWithDiff.efficiency;
//         const contextLoss = 1 - contextPreservation.replaceWithDiff;
//         if (efficiencyGain > 1.5 && contextLoss < 0.2) return 'REPLACE_WITH_DIFF';
//       }

//       return 'REPLACE_WITH_DIFF';
//     }

//     handleSmallEdit(fileRef, diffRef, diff, filePath) {
//       this.activeContext.push({
//         role: 'system',
//         content: '```diff\n' + diff.diffText + '\n```',
//         ref: diffRef,
//         timestamp: now(),
//         isReference: true,
//         isDiff: true,
//         relatedFile: filePath,
//         editStrategy: 'KEEP_BOTH',
//       });
//       this.loadedRefs.add(diffRef);
//       this.refreshReference(fileRef);
//     }

//     handleMediumEdit(fileRef, diffRef, diff, filePath) {
//       this.unloadReference(fileRef);
//       this.activeContext.push({
//         role: 'system',
//         content: '```diff\n' + diff.diffText + '\n```\n(Original replaced with diff)',
//         ref: diffRef,
//         timestamp: now(),
//         isReference: true,
//         isDiff: true,
//         relatedFile: filePath,
//         editStrategy: 'REPLACE_WITH_DIFF',
//         replacesOriginal: true,
//       });
//       this.loadedRefs.add(diffRef);
//       this.refreshReference(fileRef);
//     }

//     handleLargeEdit(fileRef, diffRef, diff, filePath) {
//       this.unloadReference(fileRef);
//       const summary = this.createEditSummary(diff);
//       this.activeContext.push({
//         role: 'system',
//         content: `Major Edit: ${filePath}\n${summary}`,
//         ref: diffRef,
//         timestamp: now(),
//         isReference: true,
//         isDiff: true,
//         relatedFile: filePath,
//         editStrategy: 'DIFF_MARKER_ONLY',
//         replacesOriginal: true,
//       });
//       this.loadedRefs.add(diffRef);
//       this.refreshReference(fileRef);
//     }

//     isCommentOnlyChange(diffText) { /* only +/- lines that are comments */ }
//     isFormattingOnlyChange(diffText, previousFile) { /* whitespace normalized equality */ }
//     isStructuralChange(diffText) { /* function/class/import/export/const patterns */ }
//     createEditSummary(diff) { /* counts + hints (functions/imports/classes) */ }

//     generateDiff(oldContent, newContent) {
//       if (oldContent === newContent) return { hasChanges: false, diffText: '', changeCount: 0 };
//       // Use LCS-like changes + grouped context before/after
//       const oldLines = oldContent.split('\n');
//       const newLines = newContent.split('\n');
//       const result = this.generateSmartDiff(oldLines, newLines);
//       return { hasChanges: true, diffText: result.diffText, changeCount: result.changeCount, oldLineCount: oldLines.length, newLineCount: newLines.length, changeType: result.changeType, contextQuality: result.contextQuality, memoryEfficiency: result.memoryEfficiency };
//     }

//     generateSmartDiff(oldLines, newLines) { /* computeLineChanges + groupChanges + context lines */ }
//     computeLineChanges(oldLines, newLines) { /* LCS-ish */ }
//     groupChanges(changes) { /* cluster within window */ }
//     getContextLines(lines, start, end) { /* safe slice */ }
//     analyzeChangeType(changes) { /* expansion/reduction/refactor/mixed */ }
//     calculateContextQuality(changeCount, contextLines, totalLines) { /* score 0..1 */ }
//     calculateDiffMemoryEfficiency(diffLinesCount, originalLinesCount) { /* compression ratio */ }

//     cleanupStaleDiff(diffRef, filePath) {
//       const stats = this.usageStats.get(diffRef);
//       const stale = !stats || now() - stats.lastUsed > 5 * 60_000;
//       if (!stale) return;

//       this.unloadReference(diffRef);
//       const f = this.contentStore.files.get(filePath);
//       if (f && f.diff) { delete f.diff; delete f.previousVersion; }
//     }

//     // ---- Terminal ------------------------------------------------------------

//     addTerminalEntry(command, output, metadata = {}) {
//       const commandId = `cmd_${now()}`;
//       const isError = output.includes('Error') || output.includes('Failed') || metadata.exitCode !== 0;

//       const processedOutput = this.processTerminalOutput(output, isError);

//       this.contentStore.terminal.set(commandId, {
//         command,
//         output: processedOutput,
//         originalLength: output.length,
//         compressed: processedOutput.length < output.length,
//         isError,
//         timestamp: now(),
//         metadata,
//       });

//       this.compressOldTerminalEntries();
//       return { success: true, commandId, compressed: processedOutput.length < output.length };
//     }

//     processTerminalOutput(output, isError) {
//       if (isError) {
//         const lines = output.split('\n');
//         const head = lines.slice(0, 30);
//         const tail = lines.slice(-50);
//         return lines.length > 100
//           ? [...head, `... (${lines.length - 80} lines omitted) ...`, ...tail].join('\n')
//           : output;
//       }
//       if (output.length <= 200) return output;
//       const lines = output.split('\n');
//       const head = lines.slice(0, 5).join('\n');
//       const tail = lines.slice(-5).join('\n');
//       return `${head}\n... (${lines.length - 10} lines omitted; success) ...\n${tail}`;
//     }

//     compressOldTerminalEntries() {
//       const entries = Array.from(this.contentStore.terminal.entries())
//         .sort(([, a], [, b]) => b.timestamp - a.timestamp);

//       entries.slice(this.config.terminalCompressionAfter).forEach(([id, entry]) => {
//         if (!entry.compressed && !entry.isError) {
//           entry.output = `${entry.command} completed successfully`;
//           entry.compressed = true;
//         }
//       });
//     }

//     // ---- Tasks ---------------------------------------------------------------

//     updateTask(taskId, taskData) {
//       this.contentStore.tasks.set(taskId, { ...taskData, lastUpdated: now() });
//       const taskRef = `task:${taskId}`;
//       if (this.loadedRefs.has(taskRef)) this.refreshReference(taskRef);
//       return { success: true, taskId };
//     }

//     // ---- Assembly under budget (multi-level + degrade) -----------------------

//     decayUsageStats(halfLifeMinutes = 30) {
//       const halfLifeMs = halfLifeMinutes * 60_000;
//       for (const [ref, stats] of this.usageStats.entries()) {
//         const ageMs = now() - (stats.lastUsed || 0);
//         const decay = Math.pow(0.5, ageMs / halfLifeMs);
//         const use = Math.max(0, (stats.useCount || 0) - 1);
//         const recencyBoost = stats.lastUsed ? 1 : 0;
//         stats.priority = (use * 0.4 + recencyBoost * 0.6) * decay;
//         this.usageStats.set(ref, stats);
//       }
//     }

//     representRef(ref, level /* 'full'|'symbols'|'headers'|'diff'|'pointer' */) {
//       const [type, id] = ref.split(':', 2);
//       if (type === 'file') {
//         const f = this.contentStore.files.get(id);
//         if (!f) return null;

//         // If a replacing diff exists, prefer current version text (acts as "full")
//         const replacingDiff = Array.from(this.loadedRefs).find(r => r.startsWith(`diff:${id}:`) && (this.activeContext.find(m => m.ref === r)?.replacesOriginal));
//         if (replacingDiff) return `File: ${id} (current)\n\`\`\`\n${f.content}\n\`\`\``;

//         if (level === 'pointer') return `@${id}`;
//         if (level === 'diff' && f.diff?.diffText) return '```diff\n' + f.diff.diffText + '\n```';
//         if (level === 'symbols') return f.headers.slice(0, 50).join('\n');
//         if (level === 'headers') return f.headers.join('\n');
//         return f.content; // full
//       }
//       if (type === 'terminal') {
//         if (id === 'recent') {
//           const recent = Array.from(this.contentStore.terminal.values()).sort((a,b)=>b.timestamp - a.timestamp).slice(0,5);
//           return recent.map(e => `$ ${e.command}\n${e.output}${e.compressed ? ' ...' : ''}`).join('\n\n');
//         }
//         const e = this.contentStore.terminal.get(id);
//         return e ? `$ ${e.command}\n${e.output}` : null;
//       }
//       if (type === 'task') return JSON.stringify(this.contentStore.tasks.get(id) || {}, null, 2);
//       return null;
//     }

//     estimateTokensForRef(ref, level) {
//       const c = this.representRef(ref, level);
//       return c ? this.tokenCounter.calculateTokenCountForContent(c) : 0;
//     }

//     scoreReference(ref, intent = '') {
//       const stats = this.usageStats.get(ref) || { priority: 0, useCount: 0, lastUsed: 0 };
//       const [type] = ref.split(':', 2);
//       const typeWeight = type === 'file' ? 1.0 : type === 'terminal' ? 0.7 : type === 'task' ? 0.9 : 0.6;
//       const recency = 1 / (1 + Math.exp(-(now() - stats.lastUsed) / 120_000)); // 0..1
//       const freq    = Math.tanh((stats.useCount || 0) / 5); // 0..1
//       const relevance = intent && ref.includes(intent) ? 0.6 : 0; // replace with IR
//       return 0.45 * recency + 0.35 * freq + 0.2 * typeWeight + relevance;
//     }

//     assembleContext({ userMessage, intent = '', hardPins = [] } = {}) {
//       const threshold = this.config.maxTokens * this.config.safetyThreshold;

//       // Candidate refs: from message + already loaded (consider refresh)
//       const refs = new Set(this.extractReferences(userMessage));
//       for (const r of Array.from(this.loadedRefs)) refs.add(r);

//       const candidates = Array.from(refs).map(ref => {
//         const score = this.scoreReference(ref, intent);
//         let preferred = 'full';
//         if (ref.startsWith('file:'))     preferred = 'headers';
//         if (ref.startsWith('terminal:')) preferred = 'pointer';
//         return { ref, score, level: preferred };
//       }).sort((a, b) => b.score - a.score);

//       for (const c of candidates) {
//         if (hardPins.includes(c.ref)) c.level = 'full';
//         let tokens = this.estimateTokensForRef(c.ref, c.level);
//         const degrade = ['full','symbols','headers','diff','pointer'];
//         let i = degrade.indexOf(c.level);

//         while (this.getCurrentTokens() + tokens > threshold && i < degrade.length - 1) {
//           i += 1;
//           c.level = degrade[i];
//           tokens = this.estimateTokensForRef(c.ref, c.level);
//         }
//         if (this.getCurrentTokens() + tokens <= threshold && c.level !== 'pointer') {
//           this.loadRefAsLevel(c.ref, c.level);
//         }
//         if (this.getCurrentTokens() > threshold) break;
//       }

//       this.evictUntilUnderBudget(threshold);
//     }

//     loadRefAsLevel(ref, level) {
//       const content = this.representRef(ref, level);
//       if (!content) return;
//       this.activeContext.push({ role: 'system', content, ref, timestamp: now(), isReference: true, level });
//       this.loadedRefs.add(ref);
//       this.updateUsageStats(ref);
//     }

//     evictUntilUnderBudget(threshold) {
//       if (this.getCurrentTokens() <= threshold) return;

//       // 1) Downgrade representation levels first
//       const order = ['full','symbols','headers','diff','pointer'];
//       for (let pass = 0; pass < order.length - 1; pass++) {
//         for (const msg of this.activeContext.filter(m => m.isReference)) {
//           const idx = order.indexOf(msg.level || 'full');
//           if (idx >= 0 && idx < order.length - 1) {
//             const nextLevel = order[idx + 1];
//             const newContent = this.representRef(msg.ref, nextLevel);
//             if (newContent) { msg.content = newContent; msg.level = nextLevel; }
//             if (this.getCurrentTokens() <= threshold) return;
//           }
//         }
//       }

//       // 2) Evict lowest-scoring references
//       const scored = this.activeContext
//         .filter(m => m.isReference)
//         .map(m => ({ msg: m, score: this.scoreReference(m.ref) }))
//         .sort((a, b) => a.score - b.score);

//       for (const { msg } of scored) {
//         this.unloadReference(msg.ref);
//         if (this.getCurrentTokens() <= threshold) break;
//       }
//     }

//     // Backward-compat trim hook
//     manageContextSize() {
//       const threshold = this.config.maxTokens * this.config.safetyThreshold;
//       this.evictUntilUnderBudget(threshold);
//     }

//     // ---- References ----------------------------------------------------------

//     extractReferences(message) {
//       const refs = [];
//       const content = typeof message?.content === 'string' ? message.content : JSON.stringify(message?.content || '');

//       // file refs: file: or @path.ext
//       (content.match(/(?:file:|@)([^\s,]+\.[a-zA-Z0-9]+)/g) || []).forEach(m => {
//         const filePath = m.replace(/^(?:file:|@)/, '');
//         refs.push(`file:${filePath}`);

//         if (/(edit|change|modify)/i.test(content)) {
//           // attach most recent diff if exists
//           const diffRefs = Array.from(this.loadedRefs).filter(r => r.startsWith(`diff:${filePath}:`));
//           if (diffRefs.length) refs.push(diffRefs.sort().pop());
//         }
//       });

//       // function refs: function foo or foo()
//       (content.match(/function\s+(\w+)|(\w+)\s*\(/g) || []).forEach(m => {
//         const fn = m.replace(/function\s+|[\s\(]/g, '');
//         if (fn) refs.push(`function:${fn}`);
//       });

//       if (/(run|command|terminal)/i.test(content)) refs.push('terminal:recent');
//       (content.match(/task:(\w+)/g) || []).forEach(m => refs.push(m));

//       if (/(diff|change|what changed)/i.test(content)) {
//         const recentDiffs = Array.from(this.loadedRefs).filter(r => r.startsWith('diff:')).slice(-3);
//         refs.push(...recentDiffs);
//       }

//       return refs;
//     }

//     loadReference(ref) {
//       if (this.loadedRefs.has(ref)) { this.updateUsageStats(ref); return; }
//       const content = this.resolveReference(ref);
//       if (!content) return;

//       this.activeContext.push({ role: 'system', content, ref, timestamp: now(), isReference: true, level: 'full' });
//       this.loadedRefs.add(ref);
//       this.updateUsageStats(ref);
//     }

//     resolveReference(ref) {
//       const [type, identifier] = ref.split(':', 2);
//       if (type === 'file')     return this.resolveFileReference(identifier);
//       if (type === 'function') return this.resolveFunctionReference(identifier);
//       if (type === 'terminal') return this.resolveTerminalReference(identifier);
//       if (type === 'task')     return this.resolveTaskReference(identifier);
//       return null;
//     }

//     resolveFileReference(filePath) {
//       const f = this.contentStore.files.get(filePath);
//       if (!f) return null;

//       const replacingDiff = Array.from(this.loadedRefs).find(r => r.startsWith(`diff:${filePath}:`) && (this.activeContext.find(m => m.ref === r)?.replacesOriginal));
//       if (replacingDiff) return `File: ${filePath} (current)\n\`\`\`\n${f.content}\n\`\`\``;

//       const recentlyUsed = this.isRecentlyUsed(`file:${filePath}`);
//       if (recentlyUsed || !this.config.functionHeadersOnly) {
//         return `File: ${filePath}\n\`\`\`\n${f.content}\n\`\`\``;
//       }
//       return `File: ${filePath} (headers)\n${f.headers.join('\n')}`;
//     }

//     resolveFunctionReference(functionName) {
//       for (const [filePath, f] of this.contentStore.files) {
//         if (f.content.includes(`function ${functionName}`) || f.content.includes(`${functionName} =`) || f.content.includes(`${functionName}:`)) {
//           const code = this.extractFunction(f.content, functionName);
//           return `Function: ${functionName} from ${filePath}\n\`\`\`\n${code}\n\`\`\``;
//         }
//       }
//       return null;
//     }

//     resolveTerminalReference(identifier) {
//       if (identifier === 'recent') {
//         const recent = Array.from(this.contentStore.terminal.values()).sort((a,b)=>b.timestamp - a.timestamp).slice(0,5);
//         return recent.map(e => `$ ${e.command}\n${e.output}${e.compressed ? ' ...' : ''}`).join('\n\n');
//       }
//       const e = this.contentStore.terminal.get(identifier);
//       return e ? `$ ${e.command}\n${e.output}` : null;
//     }

//     resolveTaskReference(taskId) {
//       const t = this.contentStore.tasks.get(taskId);
//       return t ? `Task: ${taskId}\n${JSON.stringify(t, null, 2)}` : null;
//     }

//     unloadReference(ref) {
//       this.activeContext = this.activeContext.filter(m => m.ref !== ref);
//       this.loadedRefs.delete(ref);
//     }

//     refreshReference(ref) {
//       if (!this.loadedRefs.has(ref)) return;
//       this.unloadReference(ref);
//       this.loadReference(ref);
//     }

//     // ---- Small helpers -------------------------------------------------------

//     extractFunctionHeaders(content) { /* regex capture for common function forms */ }
//     extractFunction(content, functionName) { /* naive balanced braces scan from decl line */ }

//     updateUsageStats(ref) {
//       const stats = this.usageStats.get(ref) || { lastUsed: 0, useCount: 0, priority: 0.5 };
//       stats.lastUsed = now();
//       stats.useCount += 1;
//       this.usageStats.set(ref, stats);
//     }

//     isRecentlyUsed(ref, thresholdMinutes = 30) {
//       const stats = this.usageStats.get(ref);
//       if (!stats) return false;
//       return stats.lastUsed > now() - thresholdMinutes * 60_000;
//     }

//     getCurrentTokens() {
//       return this.activeContext.reduce((sum, m) => {
//         const c = typeof m.content === 'string' ? m.content : JSON.stringify(m.content);
//         return sum + this.tokenCounter.calculateTokenCountForContent(c);
//       }, 0);
//     }

//     getStoreStatistics() {
//       return {
//         files: this.contentStore.files.size,
//         terminal: this.contentStore.terminal.size,
//         tasks: this.contentStore.tasks.size,
//         loadedRefs: this.loadedRefs.size,
//         totalTokens: this.getCurrentTokens(),
//         utilization: this.getCurrentTokens() / this.config.maxTokens,
//       };
//     }

//     // ---- Context export + analytics -----------------------------------------

//     getCurrentContext() {
//       const totalTokens = this.getCurrentTokens();
//       return {
//         messages: this.activeContext.map(({ role, content, timestamp }) => ({ role, content, timestamp })),
//         metadata: {
//           totalTokens,
//           maxTokens: this.config.maxTokens,
//           utilization: totalTokens / this.config.maxTokens,
//           loadedRefs: Array.from(this.loadedRefs),
//           storeStats: this.getStoreStatistics(),
//           memoryAnalysis: this.analyzeMemoryEfficiency(),
//           editStrategies: this.getEditStrategyBreakdown(),
//           contextQuality: this.calculateOverallContextQuality(),
//           efficiencyGains: this.calculateEfficiencyGains(),
//         },
//       };
//     }

//     analyzeMemoryEfficiency() { /* compute savings from diffs vs originals */ }
//     getEditStrategyBreakdown() { /* counts + avg preservation */ }
//     calculateOverallContextQuality() { /* average per-message quality */ }
//     calculateEfficiencyGains() { /* compare to naive full-context baseline */ }
//     calculateMemoryEfficiencyScore(strategies) { /* weighted by strategy */ }

//     exportState() {
//       return {
//         config: this.config,
//         contentStore: {
//           files: Array.from(this.contentStore.files.entries()),
//           terminal: Array.from(this.contentStore.terminal.entries()),
//           tasks: Array.from(this.contentStore.tasks.entries()),
//         },
//         usageStats: Array.from(this.usageStats.entries()),
//         timestamp: now(),
//       };
//     }

//     importState(state) {
//       this.config = { ...this.config, ...state.config };
//       this.contentStore.files    = new Map(state.contentStore.files);
//       this.contentStore.terminal = new Map(state.contentStore.terminal);
//       this.contentStore.tasks    = new Map(state.contentStore.tasks);
//       this.usageStats            = new Map(state.usageStats);
//     }

//     // ---- Utilities -----------------------------------------------------------

//   }

//   function now() { return Date.now(); }
