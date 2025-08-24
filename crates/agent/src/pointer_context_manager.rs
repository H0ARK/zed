use crate::{
    context::{AgentContextHandle, AgentContextKey, ContextId},
    context_store::ContextStore,
    thread_store::ThreadStore,
};
use anyhow::Result;
use collections::{HashMap, VecDeque};
use gpui::{Context, Entity, EventEmitter, Task, WeakEntity};
use indexmap::IndexSet;
use project::Project;
use prompt_store::PromptStore;
use std::sync::atomic::AtomicUsize;
use std::{path::PathBuf, sync::Arc, time::Instant};
use thiserror::Error;
static DIFF_CREATED: AtomicUsize = AtomicUsize::new(0);
static DIFF_PRUNED: AtomicUsize = AtomicUsize::new(0);

/// Configuration for the PointerContextManager
#[derive(Clone, Debug)]
pub struct ContextManagerConfig {
    pub max_tokens: usize,
    pub safety_threshold: f32,
    /// Number of most-recent context references to retain for priority scoring
    pub usage_history_capacity: usize,
    pub terminal_compression_after: usize,
    pub per_type_budgets: HashMap<ContentType, f32>,
    /// How many diffs to retain per strategy (older diffs are evicted first)
    pub diff_history_limit: HashMap<EditStrategy, usize>,
    pub diff_ttl_secs: u64,
    pub rebalance_hysteresis_ratio: f32,
}

impl Default for ContextManagerConfig {
    fn default() -> Self {
        let mut per_type_budgets = HashMap::default();
        per_type_budgets.insert(ContentType::Files, 0.6);
        per_type_budgets.insert(ContentType::Terminal, 0.2);
        per_type_budgets.insert(ContentType::Diffs, 0.15);
        per_type_budgets.insert(ContentType::Tasks, 0.05);

        let mut diff_history_limit = HashMap::default();
        diff_history_limit.insert(EditStrategy::KeepBoth, 10);
        diff_history_limit.insert(EditStrategy::ReplaceWithDiff, 20);
        diff_history_limit.insert(EditStrategy::DiffMarkerOnly, 30);

        Self {
            max_tokens: 32000,
            safety_threshold: 0.85,
            usage_history_capacity: 200,
            terminal_compression_after: 50,
            per_type_budgets,
            diff_history_limit,
            diff_ttl_secs: 300,
            rebalance_hysteresis_ratio: 0.05,
        }
    }
}

/// Content types for budget allocation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ContentType {
    Files,
    Terminal,
    Diffs,
    Tasks,
}

/// Representation levels for content degradation
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RepresentationLevel {
    Full,
    Symbols,
    Headers,
    Diff,
    Pointer,
}

/// Edit strategies for diff management
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EditStrategy {
    KeepBoth,
    ReplaceWithDiff,
    DiffMarkerOnly,
}

/// Loaded content with metadata
#[derive(Debug, Clone)]
pub struct LoadedContent {
    // Always keep original full-fidelity content (if available) to derive lower representations deterministically.
    pub original_full: Arc<str>,
    pub text: Arc<str>,
    pub content_type: ContentType,
    pub representation_level: RepresentationLevel,
    pub token_count: usize,
    pub context_id: ContextId,
    // Cached representation variants (includes current + any degraded forms)
    pub cached_variants: HashMap<RepresentationLevel, Arc<str>>,
}

/// Usage statistics for exponential decay
#[derive(Debug, Clone)]
pub struct UsageStats {
    pub use_count: u32,
    pub priority: f64,
    pub content_type: ContentType,
}

/// Usage tracking with bounded history (recency + frequency)
pub struct UsageTracker {
    stats: HashMap<ContextId, UsageStats>,
    recent_order: VecDeque<ContextId>,
    capacity: usize,
}

impl UsageTracker {
    pub fn new(capacity: usize) -> Self {
        Self {
            stats: HashMap::default(),
            recent_order: VecDeque::new(),
            capacity,
        }
    }

    /// Recompute priorities using the current bounded history and use counts
    pub fn recompute_priorities(&mut self) {
        let max_count = self.stats.values().map(|s| s.use_count).max().unwrap_or(1) as f64;

        // Precompute recency scores without holding a mutable borrow of stats
        let mut recency_map: HashMap<ContextId, f64> = HashMap::default();
        for context_id in self.stats.keys() {
            recency_map.insert(*context_id, self.recency_score(context_id));
        }

        for (context_id, stats) in self.stats.iter_mut() {
            // Recency score: 1.0 for most recent, down to ~0.0 for least recent or missing
            let recency_score = *recency_map.get(context_id).unwrap_or(&0.0);

            // Frequency score: normalized log scale to dampen large counts
            let freq = stats.use_count as f64;
            let freq_score = if max_count > 1.0 {
                (1.0 + freq).ln() / (1.0 + max_count).ln()
            } else {
                0.0
            };

            // Blend recency and frequency
            stats.priority = recency_score * 0.6 + freq_score * 0.4;
        }
    }

    pub fn update_usage(&mut self, context_id: ContextId, content_type: ContentType) {
        let stats = self.stats.entry(context_id).or_insert_with(|| UsageStats {
            use_count: 0,
            priority: 0.0,
            content_type,
        });

        stats.use_count = stats.use_count.saturating_add(1);

        // Maintain bounded LRU-like order
        if let Some(pos) = self.recent_order.iter().position(|id| id == &context_id) {
            self.recent_order.remove(pos);
        }
        self.recent_order.push_back(context_id);
        while self.recent_order.len() > self.capacity {
            self.recent_order.pop_front();
        }
    }

    pub fn get_priority(&self, context_id: &ContextId) -> f64 {
        self.stats
            .get(context_id)
            .map(|s| s.priority)
            .unwrap_or(0.0)
    }

    pub fn get_stats(&self, context_id: &ContextId) -> Option<&UsageStats> {
        self.stats.get(context_id)
    }

    pub fn recency_score(&self, context_id: &ContextId) -> f64 {
        if let Some(pos_from_front) = self.recent_order.iter().position(|id| id == context_id) {
            // Most recent at the back
            let pos_from_back = self
                .recent_order
                .len()
                .saturating_sub(1)
                .saturating_sub(pos_from_front);
            if self.capacity == 0 {
                return 0.0;
            }
            1.0 - (pos_from_back as f64 / self.capacity as f64)
        } else {
            0.0
        }
    }
}

/// Unique identifier for diff entries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DiffId(u64);

impl DiffId {
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

/// Contextual factors for change analysis
#[derive(Debug, Clone, Default)]
pub struct ContextualFactors {
    pub function_changes: usize,
    pub dependency_changes: usize,
    pub type_changes: usize,
    pub config_changes: usize,
    pub comment_only_changes: bool,
    pub formatting_only_changes: bool,
}

/// Change magnitude analysis
#[derive(Debug, Clone)]
pub struct ChangeMagnitude {
    pub change_percentage: f64,
    pub token_impact: f64,
    pub change_count: usize,
    pub is_structural_change: bool,
    pub contextual_factors: ContextualFactors,
}

/// Diff entry with metadata
#[derive(Debug, Clone)]
pub struct DiffEntry {
    pub id: DiffId,
    pub file_path: PathBuf,
    pub diff_content: String,
    pub strategy: EditStrategy,
    pub created_at: Instant,
    pub change_magnitude: ChangeMagnitude,
    pub expires_at: Option<Instant>,
}

/// Diff management system
pub struct DiffManager {
    active_diffs: HashMap<PathBuf, Vec<DiffEntry>>,
    max_per_strategy: HashMap<EditStrategy, usize>,
    default_limit: usize,
}

impl DiffManager {
    pub fn new(max_per_strategy: HashMap<EditStrategy, usize>, default_limit: usize) -> Self {
        Self {
            active_diffs: HashMap::default(),
            max_per_strategy: max_per_strategy,
            default_limit,
        }
    }

    pub fn handle_file_edit(
        &mut self,
        file_path: PathBuf,
        old_content: &str,
        new_content: &str,
        ttl: std::time::Duration,
        _cx: &mut Context<PointerContextManager>,
    ) -> Task<Result<Option<DiffEntry>>> {
        let diff_content = self.generate_diff(old_content, new_content);

        if diff_content.is_empty() {
            return Task::ready(Ok(None));
        }

        let magnitude = self.calculate_edit_magnitude(&diff_content, old_content);
        let strategy = self.select_edit_strategy(&magnitude);

        let diff_entry = DiffEntry {
            id: DiffId::new(),
            file_path: file_path.clone(),
            diff_content,
            strategy: strategy.clone(),
            created_at: Instant::now(),
            change_magnitude: magnitude,
            expires_at: Some(Instant::now() + ttl),
        };

        self.active_diffs
            .entry(file_path)
            .or_default()
            .push(diff_entry.clone());
        // Metrics: diff created (manager-level atomics handled via callback in future; simple static)
        DIFF_CREATED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // Enforce per-strategy history limits
        self.trim_file_diffs(&diff_entry.file_path);

        Task::ready(Ok(Some(diff_entry)))
    }

    fn select_edit_strategy(&self, magnitude: &ChangeMagnitude) -> EditStrategy {
        if magnitude.contextual_factors.comment_only_changes
            || magnitude.contextual_factors.formatting_only_changes
        {
            return EditStrategy::ReplaceWithDiff;
        }

        if magnitude.change_percentage < 0.05 && magnitude.contextual_factors.function_changes > 0 {
            return EditStrategy::KeepBoth;
        }

        if magnitude.change_percentage > 0.5 || magnitude.is_structural_change {
            return EditStrategy::DiffMarkerOnly;
        }

        EditStrategy::ReplaceWithDiff
    }

    fn generate_diff(&self, old_content: &str, new_content: &str) -> String {
        if old_content == new_content {
            return String::new();
        }
        // LCS-based unified diff (minimal, O(n*m) for clarity; could optimize later)
        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();
        let n = old_lines.len();
        let m = new_lines.len();
        // DP table
        let mut lcs = vec![vec![0usize; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                if old_lines[i] == new_lines[j] {
                    lcs[i][j] = 1 + lcs[i + 1][j + 1];
                } else {
                    lcs[i][j] = lcs[i + 1][j].max(lcs[i][j + 1]);
                }
            }
        }
        // Backtrack to produce op stream
        #[derive(Clone, Debug)]
        enum Op<'a> {
            Keep(&'a str),
            Del(&'a str),
            Add(&'a str),
        }
        let mut ops: Vec<Op> = Vec::new();
        let (mut i, mut j) = (0usize, 0usize);
        while i < n && j < m {
            if old_lines[i] == new_lines[j] {
                ops.push(Op::Keep(old_lines[i]));
                i += 1;
                j += 1;
            } else if lcs[i + 1][j] >= lcs[i][j + 1] {
                ops.push(Op::Del(old_lines[i]));
                i += 1;
            } else {
                ops.push(Op::Add(new_lines[j]));
                j += 1;
            }
        }
        while i < n {
            ops.push(Op::Del(old_lines[i]));
            i += 1;
        }
        while j < m {
            ops.push(Op::Add(new_lines[j]));
            j += 1;
        }
        // Unified diff formatting with context window
        let context = 3usize;
        let mut out = String::new();
        out.push_str("--- old\n+++ new\n");
        // Identify change hunks
        let mut hunk: Vec<(isize, &Op)> = Vec::new();
        let mut last_change_index: Option<usize> = None;
        for (idx, op) in ops.iter().enumerate() {
            let is_change = !matches!(op, Op::Keep(_));
            if is_change {
                if let Some(last) = last_change_index {
                    if idx > last + (2 * context) + 1 {
                        // flush previous hunk
                        // inline flush of previous hunk
                        for (_i, (_idx, op)) in hunk.iter().enumerate() {
                            match op {
                                Op::Keep(line) => out.push_str(&format!(" {}\n", line)),
                                Op::Del(line) => out.push_str(&format!("-{}\n", line)),
                                Op::Add(line) => out.push_str(&format!("+{}\n", line)),
                            }
                        }
                        hunk.clear();
                    }
                }
                last_change_index = Some(idx);
            }
            hunk.push((idx as isize, op));
        }
        if !hunk.is_empty() {
            // inline flush of final hunk
            for (_i, (_idx, op)) in hunk.iter().enumerate() {
                match op {
                    Op::Keep(line) => out.push_str(&format!(" {}\n", line)),
                    Op::Del(line) => out.push_str(&format!("-{}\n", line)),
                    Op::Add(line) => out.push_str(&format!("+{}\n", line)),
                }
            }
        }
        out
    }

    fn calculate_edit_magnitude(&self, diff_content: &str, old_content: &str) -> ChangeMagnitude {
        let old_lines = old_content.lines().count();
        let diff_lines = diff_content.lines().count();

        ChangeMagnitude {
            change_percentage: if old_lines > 0 {
                diff_lines as f64 / old_lines as f64
            } else {
                1.0
            },
            token_impact: diff_lines as f64 * 4.0, // Rough token estimate
            change_count: diff_lines,
            is_structural_change: diff_lines > old_lines / 2,
            contextual_factors: ContextualFactors::default(),
        }
    }

    fn trim_file_diffs(&mut self, file_path: &PathBuf) {
        if let Some(diffs) = self.active_diffs.get_mut(file_path) {
            // For each strategy, enforce its limit by removing oldest entries of that strategy
            let strategies = [
                EditStrategy::KeepBoth,
                EditStrategy::ReplaceWithDiff,
                EditStrategy::DiffMarkerOnly,
            ];
            for strategy in strategies {
                let limit = self
                    .max_per_strategy
                    .get(&strategy)
                    .copied()
                    .unwrap_or(self.default_limit);
                if limit == 0 {
                    continue;
                }

                // Count existing of this strategy
                let mut count = diffs.iter().filter(|d| d.strategy == strategy).count();
                if count <= limit {
                    continue;
                }

                // Remove from oldest to newest
                let mut i = 0;
                while count > limit && i < diffs.len() {
                    if diffs[i].strategy == strategy {
                        diffs.remove(i);
                        count -= 1;
                        DIFF_PRUNED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        // Do not increment i; the next element shifted into i
                    } else {
                        i += 1;
                    }
                }
            }

            if diffs.is_empty() {
                self.active_diffs.remove(file_path);
            }
        }
    }
}

/// Terminal entry for compression
#[derive(Debug, Clone)]
pub struct TerminalEntry {
    pub command_id: String,
    pub command: String,
    pub output: String,
    pub is_error: bool,
    pub is_compressed: bool,
    pub timestamp: Instant,
    pub original_length: usize,
}

/// Terminal compression system
pub struct TerminalCompressor {
    entries: VecDeque<TerminalEntry>,
    compression_threshold: usize,
}

impl TerminalCompressor {
    pub fn new(compression_threshold: usize) -> Self {
        Self {
            entries: VecDeque::new(),
            compression_threshold,
        }
    }

    pub fn add_entry(&mut self, command: String, output: String, is_error: bool) -> String {
        // Stable ever-increasing command id to avoid collisions
        static COMMAND_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let command_id = format!(
            "cmd_{}",
            COMMAND_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let original_length = output.len();

        let processed_output = if is_error {
            self.process_error_output(&output)
        } else {
            self.process_success_output(&output)
        };

        let is_compressed = processed_output.len() < original_length;
        let compressed_length = processed_output.len();
        let _saved_chars = if is_compressed {
            original_length.saturating_sub(compressed_length)
        } else {
            0
        };

        let entry = TerminalEntry {
            command_id: command_id.clone(),
            command,
            output: processed_output,
            is_error,
            is_compressed,
            timestamp: Instant::now(),
            original_length,
        };

        self.entries.push_back(entry);
        self.compress_old_entries();

        // Return command id plus saved chars via side channel (not stored). Caller records metrics.
        command_id
    }

    fn process_error_output(&self, output: &str) -> String {
        let lines: Vec<&str> = output.lines().collect();
        if lines.len() <= 100 {
            return output.to_string();
        }

        let head: Vec<&str> = lines.iter().take(30).copied().collect();
        let tail: Vec<&str> = lines.iter().rev().take(50).rev().copied().collect();
        let omitted = lines.len() - 80;

        format!(
            "{}\n... ({} lines omitted) ...\n{}",
            head.join("\n"),
            omitted,
            tail.join("\n")
        )
    }

    fn process_success_output(&self, output: &str) -> String {
        if output.len() <= 200 {
            return output.to_string();
        }

        let lines: Vec<&str> = output.lines().collect();
        if lines.len() <= 10 {
            return output.to_string();
        }

        let head = lines.iter().take(5).copied().collect::<Vec<_>>().join("\n");
        let tail = lines
            .iter()
            .rev()
            .take(5)
            .rev()
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        let omitted = lines.len() - 10;

        format!(
            "{}\n... ({} lines omitted; success) ...\n{}",
            head, omitted, tail
        )
    }

    fn compress_old_entries(&mut self) {
        while self.entries.len() > self.compression_threshold {
            self.entries.pop_front();
        }
    }

    // Helper returning command id and (original_len, compressed_len) for metrics.
    pub fn add_entry_with_stats(
        &mut self,
        command: String,
        output: String,
        is_error: bool,
    ) -> (String, usize, usize) {
        let original_len = output.len();
        let command_id = self.add_entry(command, output, is_error);
        let compressed_len = self
            .entries
            .back()
            .map(|e| e.output.len())
            .unwrap_or(original_len);
        (command_id, original_len, compressed_len)
    }
}

/// Token counting system
pub struct TokenCounter {
    // Simple token counting - in practice, integrate with actual tokenizer
}

impl TokenCounter {
    pub fn new() -> Self {
        Self {}
    }

    pub fn count_tokens(&self, text: &str) -> usize {
        // Rough approximation: 1 token per 4 characters
        (text.len() + 3) / 4
    }
}

/// Context manager errors
#[derive(Error, Debug)]
pub enum ContextManagerError {
    #[error("Context not found: {context_id:?}")]
    ContextNotFound { context_id: ContextId },

    #[error("Budget exceeded: {current} > {limit}")]
    BudgetExceeded { current: usize, limit: usize },

    #[error("Failed to load content: {source}")]
    ContentLoadError { source: anyhow::Error },

    #[error("Diff generation failed for {file_path}")]
    DiffGenerationError { file_path: PathBuf },
}

/// Events emitted by the context manager
#[derive(Debug, Clone)]
pub enum ContextManagerEvent {
    ContentLoaded(ContextId),
    ContentEvicted(ContextId),
    BudgetExceeded {
        current: usize,
        limit: usize,
    },
    DiffCreated {
        file_path: PathBuf,
        strategy: EditStrategy,
    },
    RepresentationChanged {
        context_id: ContextId,
        from: RepresentationLevel,
        to: RepresentationLevel,
        token_delta: isize,
    },
    TerminalCompressed {
        command_id: String,
        compression_ratio: f64,
    },
}

#[derive(Default, Debug, Clone)]
struct InternalMetrics {
    loads: usize,
    candidate_attempts: usize,
    degradations: usize,
    promotions: usize,
    evictions: usize,
    diff_created: usize,
    diff_pruned: usize,
    compression_savings_tokens: usize,
    rebalance_iterations: usize,
}

/// Main pointer context manager
pub struct PointerContextManager {
    // Configuration
    config: ContextManagerConfig,

    // Core storage (existing Zed components)
    context_store: Entity<ContextStore>,
    thread_store: Option<WeakEntity<ThreadStore>>,
    project: WeakEntity<Project>,
    prompt_store: Option<Entity<PromptStore>>,

    // Memory management
    usage_tracker: UsageTracker,
    token_counter: TokenCounter,

    // Content management
    diff_manager: DiffManager,
    terminal_compressor: TerminalCompressor,

    // Active context tracking
    active_refs: IndexSet<AgentContextKey>,
    loaded_content: HashMap<ContextId, LoadedContent>,

    // Budget tracking
    current_token_count: usize,
    budget_allocations: HashMap<ContentType, usize>,

    // Metrics
    metrics: InternalMetrics,
}

impl EventEmitter<ContextManagerEvent> for PointerContextManager {}

impl PointerContextManager {
    pub fn new(
        context_store: Entity<ContextStore>,
        thread_store: Option<WeakEntity<ThreadStore>>,
        project: WeakEntity<Project>,
        prompt_store: Option<Entity<PromptStore>>,
        config: ContextManagerConfig,
    ) -> Self {
        Self {
            config: config.clone(),
            context_store,
            thread_store,
            project,
            prompt_store,
            usage_tracker: UsageTracker::new(config.usage_history_capacity),
            token_counter: TokenCounter::new(),
            diff_manager: DiffManager::new(config.diff_history_limit.clone(), 20),
            terminal_compressor: TerminalCompressor::new(config.terminal_compression_after),
            active_refs: IndexSet::new(),
            loaded_content: HashMap::default(),
            current_token_count: 0,
            budget_allocations: HashMap::default(),
            metrics: InternalMetrics::default(),
        }
    }

    pub fn assemble_context(
        &mut self,
        user_message: &str,
        intent: Option<&str>,
        _hard_pins: Vec<ContextId>,
        cx: &mut Context<Self>,
    ) -> Task<Result<Vec<LoadedContent>>> {
        // Recompute usage priorities based on bounded history
        self.usage_tracker.recompute_priorities();

        // Extract references from message
        let refs = self.extract_references(user_message);

        // Score and prioritize references
        let mut candidates = self.score_references(&refs, intent.unwrap_or(""));
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Budgeted assembly with degradation
        cx.spawn(async move |this, cx| {
            let mut loaded_content = Vec::new();
            let threshold = (this.read_with(cx, |this, _| this.config.max_tokens)? as f32
                * this.read_with(cx, |this, _| this.config.safety_threshold)?)
                as usize;

            for candidate in candidates {
                // Track attempt
                this.update(cx, |this, _| {
                    this.metrics.candidate_attempts += 1;
                })?;
                // Skip if already loaded at some level
                let already_loaded = this.read_with(cx, |this, _| {
                    this.loaded_content.contains_key(&candidate.context_id)
                })?;
                if already_loaded {
                    continue;
                }

                // Iteratively attempt from requested level downward until fits or pointer exhausted
                let mut attempt_level = candidate.level.clone();
                let mut accepted = false;
                while !accepted {
                    let opt_content = this
                        .update(cx, |this, cx| {
                            this.load_reference_with_level(
                                candidate.context_id,
                                attempt_level.clone(),
                                cx,
                            )
                        })?
                        .await?;

                    let Some(content) = opt_content else {
                        break;
                    };

                    let tokens = this
                        .read_with(cx, |this, _| this.token_counter.count_tokens(&content.text))?;
                    let projected =
                        this.read_with(cx, |this, _| this.current_token_count)? + tokens;

                    if projected <= threshold {
                        // Accept
                        this.update(cx, |this, cx| {
                            this.current_token_count += tokens;
                            this.loaded_content
                                .entry(content.context_id)
                                .and_modify(|entry| {
                                    entry.cached_variants.insert(
                                        content.representation_level.clone(),
                                        content.text.clone(),
                                    );
                                    if content.representation_level < entry.representation_level {
                                        let from = entry.representation_level.clone();
                                        let from_tokens = entry.token_count;
                                        entry.representation_level =
                                            content.representation_level.clone();
                                        entry.text = content.text.clone();
                                        entry.token_count = content.token_count;
                                        let delta =
                                            entry.token_count as isize - from_tokens as isize;
                                        cx.emit(ContextManagerEvent::RepresentationChanged {
                                            context_id: content.context_id,
                                            from,
                                            to: entry.representation_level.clone(),
                                            token_delta: delta,
                                        });
                                    }
                                })
                                .or_insert_with(|| {
                                    let mut cloned = content.clone();
                                    cloned.original_full = content.original_full.clone();
                                    cloned
                                });
                            this.metrics.loads += 1;
                            this.usage_tracker
                                .update_usage(candidate.context_id, content.content_type.clone());
                            this.emit_content_loaded(candidate.context_id, cx);
                        })?;
                        loaded_content.push(content);
                        accepted = true;
                    } else {
                        // Emit budget exceeded event only once per candidate
                        this.update(cx, |this, cx| {
                            if projected > this.config.max_tokens {
                                this.emit_budget_exceeded(projected, this.config.max_tokens, cx);
                            }
                        })?;

                        // Degrade level
                        attempt_level = match attempt_level {
                            RepresentationLevel::Full => RepresentationLevel::Symbols,
                            RepresentationLevel::Symbols => RepresentationLevel::Headers,
                            RepresentationLevel::Headers => RepresentationLevel::Diff,
                            RepresentationLevel::Diff => RepresentationLevel::Pointer,
                            RepresentationLevel::Pointer => {
                                // Cannot degrade further; give up on this candidate
                                break;
                            }
                        };
                        // Continue loop to try degraded representation
                    }
                }
            }

            // Debug invariant (non-fatal) – ensure token tally matches sum
            #[cfg(debug_assertions)]
            {
                let (sum, max) = this.read_with(cx, |this, _| {
                    let sum: usize = this.loaded_content.values().map(|c| c.token_count).sum();
                    (sum, this.current_token_count)
                })?;
                debug_assert_eq!(
                    sum, max,
                    "Token ledger drift: computed_sum={} ledger={}",
                    sum, max
                );
            }

            // Post-assembly maintenance (rebalance / per-type budgets / diff pruning)
            this.update(cx, |this, cx2| {
                // First perform maintenance (may degrade / evict) so metrics reflect raw adjustments
                this.post_assembly_maintenance(cx2);
                // Then attempt promotions to utilize any recovered headroom
                this.promote_high_value_content(cx2);
                this.prune_expired_diffs();
                #[cfg(debug_assertions)]
                this.assert_invariants();
            })?;
            Ok(loaded_content)
        })
    }

    fn extract_references(&self, message: &str) -> Vec<AgentContextKey> {
        let refs = Vec::new();

        // Extract file paths (simple pattern matching)
        // Note: This is a simplified implementation. In practice, we would
        // need to match against existing context in the ContextStore
        for word in message.split_whitespace() {
            if word.contains('/')
                && (word.ends_with(".rs") || word.ends_with(".ts") || word.ends_with(".js"))
            {
                // For now, we'll skip this since we need actual AgentContextHandle instances
                // This would need to be implemented by querying the ContextStore
            }
        }

        refs
    }

    fn score_references(&self, refs: &[AgentContextKey], intent: &str) -> Vec<ScoredCandidate> {
        let mut candidates = Vec::new();

        for context_key in refs {
            if let Some(context_id) = self.get_context_id_for_key(context_key) {
                let usage_score = self.usage_tracker.get_priority(&context_id);
                let intent_score = self.calculate_intent_relevance(context_key, intent);
                let recency_score = self.calculate_recency_score(&context_id);

                let total_score = usage_score * 0.4 + intent_score * 0.4 + recency_score * 0.2;

                candidates.push(ScoredCandidate {
                    context_id,
                    level: RepresentationLevel::Full,
                    score: total_score,
                });
            }
        }

        candidates
    }

    fn load_reference_with_level(
        &mut self,
        context_id: ContextId,
        level: RepresentationLevel,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<LoadedContent>>> {
        let context_store = self.context_store.clone();
        let _project = self.project.clone();

        cx.spawn(async move |this, cx| {
            // Find the context handle by ID in the ContextStore
            let context_handle = context_store.read_with(cx, |store, _| {
                store
                    .context()
                    .find(|handle| match handle {
                        AgentContextHandle::File(h) => h.context_id == context_id,
                        AgentContextHandle::Directory(h) => h.context_id == context_id,
                        AgentContextHandle::Symbol(h) => h.context_id == context_id,
                        AgentContextHandle::Selection(h) => h.context_id == context_id,
                        AgentContextHandle::FetchedUrl(h) => h.context_id == context_id,
                        AgentContextHandle::Thread(h) => h.context_id == context_id,
                        AgentContextHandle::TextThread(h) => h.context_id == context_id,
                        AgentContextHandle::Rules(h) => h.context_id == context_id,
                        AgentContextHandle::Image(h) => h.context_id == context_id,
                    })
                    .cloned()
            })?;

            let Some(handle) = context_handle else {
                return Ok(None);
            };

            // For now, we'll use a simplified approach since we can't easily convert
            // WeakEntity<Project> to Entity<Project> and AsyncApp to App in this context
            let raw_content = match &handle {
                AgentContextHandle::File(_) => "[File content placeholder]".to_string(),
                AgentContextHandle::Directory(_) => "[Directory content placeholder]".to_string(),
                AgentContextHandle::Symbol(_) => "[Symbol content placeholder]".to_string(),
                AgentContextHandle::Selection(_) => "[Selection content placeholder]".to_string(),
                AgentContextHandle::FetchedUrl(url) => url.text.to_string(),
                AgentContextHandle::Thread(_) => "[Thread content placeholder]".to_string(),
                AgentContextHandle::TextThread(_) => {
                    "[Text thread content placeholder]".to_string()
                }
                AgentContextHandle::Rules(_) => "[Rules content placeholder]".to_string(),
                AgentContextHandle::Image(_) => "[Image content]".to_string(),
            };

            // Apply representation level transformation
            let processed_content = this.read_with(cx, |_this, _| {
                PointerContextManager::apply_representation_level(&raw_content, &level, None)
            })?;

            let content_type = this.read_with(cx, |this, _| {
                this.determine_content_type_from_handle(&handle)
            })?;

            let token_count = this.read_with(cx, |this, _| {
                this.token_counter.count_tokens(&processed_content)
            })?;

            Ok(Some(LoadedContent {
                original_full: Arc::from(raw_content),
                text: processed_content.clone().into(),
                content_type,
                representation_level: level.clone(),
                token_count,
                context_id,
                cached_variants: {
                    let mut m = HashMap::new();
                    m.insert(level.clone(), processed_content.into());
                    m
                },
            }))
        })
    }

    fn try_degrade_content(
        &mut self,
        context_id: ContextId,
        current_level: RepresentationLevel,
        cx: &mut Context<Self>,
    ) -> Task<Result<Option<LoadedContent>>> {
        let next_level = match current_level {
            RepresentationLevel::Full => RepresentationLevel::Symbols,
            RepresentationLevel::Symbols => RepresentationLevel::Headers,
            RepresentationLevel::Headers => RepresentationLevel::Diff,
            RepresentationLevel::Diff => RepresentationLevel::Pointer,
            RepresentationLevel::Pointer => return Task::ready(Ok(None)),
        };

        self.load_reference_with_level(context_id, next_level, cx)
    }

    pub fn emit_content_loaded(&mut self, context_id: ContextId, cx: &mut Context<Self>) {
        cx.emit(ContextManagerEvent::ContentLoaded(context_id));
    }

    fn emit_budget_exceeded(&mut self, current: usize, limit: usize, cx: &mut Context<Self>) {
        cx.emit(ContextManagerEvent::BudgetExceeded { current, limit });
    }

    // Helper methods
    fn get_context_id_for_key(&self, _key: &AgentContextKey) -> Option<ContextId> {
        // TODO: Map AgentContextKey to ContextId using context store
        None
    }

    fn calculate_intent_relevance(&self, _key: &AgentContextKey, intent: &str) -> f64 {
        // Simple keyword matching for intent relevance
        let keywords = intent.to_lowercase();
        if keywords.contains("debug") || keywords.contains("error") {
            0.8
        } else if keywords.contains("implement") || keywords.contains("add") {
            0.6
        } else {
            0.3
        }
    }

    fn calculate_recency_score(&self, context_id: &ContextId) -> f64 {
        // Use bounded history position as recency proxy
        let score = self.usage_tracker.recency_score(context_id);
        if score > 0.0 { score } else { 0.1 }
    }

    fn apply_representation_level(
        text: &str,
        level: &RepresentationLevel,
        content_type: Option<&ContentType>,
    ) -> String {
        // Detect task-list style content to enable compression even if content_type is not provided.
        let looks_like_task_list = {
            let mut total = 0usize;
            let mut taskish = 0usize;
            for line in text.lines().take(1000) {
                total += 1;
                let t = line.trim_start();
                if t.starts_with("- [ ]")
                    || t.starts_with("[ ]")
                    || t.starts_with("- [x]")
                    || t.starts_with("[x]")
                    || t.starts_with("- [X]")
                    || t.starts_with("[X]")
                {
                    taskish += 1;
                }
            }
            total > 5 && taskish * 3 >= total // ~>= 33% of lines are task-ish
        };

        let is_tasks = matches!(content_type, Some(&ContentType::Tasks)) || looks_like_task_list;

        if is_tasks {
            // Compress typical TODO task lists by prioritizing open items and summarizing completed ones.
            let mut open: Vec<&str> = Vec::new();
            let mut completed: usize = 0;
            let mut headings: Vec<&str> = Vec::new();

            for line in text.lines() {
                let t = line.trim_start();
                let is_completed = t.starts_with("- [x]")
                    || t.starts_with("[x]")
                    || t.starts_with("- [X]")
                    || t.starts_with("[X]");
                let is_unchecked = t.starts_with("- [ ]") || t.starts_with("[ ]");
                let is_heading = t.starts_with('#');

                if is_completed {
                    completed += 1;
                    continue;
                }
                if is_unchecked {
                    open.push(line);
                    continue;
                }
                if is_heading {
                    headings.push(line);
                    continue;
                }
            }

            let open_count = open.len();

            return match level {
                RepresentationLevel::Full => {
                    // Preserve original content; append a summary if the list is large to hint at compression.
                    if completed > 0 && open_count + completed > 50 {
                        let mut s = String::with_capacity(text.len() + 64);
                        s.push_str(text);
                        s.push_str("\n... (");
                        s.push_str(&completed.to_string());
                        s.push_str(" completed tasks omitted in context) ...");
                        s
                    } else {
                        text.to_string()
                    }
                }
                RepresentationLevel::Symbols => {
                    // Keep headings and all open tasks; summarize completed tasks.
                    let mut out = String::new();
                    if !headings.is_empty() {
                        out.push_str(&headings.join("\n"));
                        out.push('\n');
                    }
                    if !open.is_empty() {
                        out.push_str(&open.join("\n"));
                    }
                    if completed > 0 {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(&format!("... ({} completed tasks omitted) ...", completed));
                    }
                    out
                }
                RepresentationLevel::Headers => {
                    // Keep headings and a limited number of open tasks; summarize the rest.
                    let limit = 50usize;
                    let mut out = String::new();
                    if !headings.is_empty() {
                        out.push_str(&headings.join("\n"));
                        out.push('\n');
                    }
                    let take_n = open_count.min(limit);
                    if take_n > 0 {
                        out.push_str(&open[..take_n].join("\n"));
                    }
                    if open_count > limit {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(&format!("... ({} more open tasks) ...", open_count - limit));
                    }
                    if completed > 0 {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(&format!("... ({} completed tasks omitted) ...", completed));
                    }
                    out
                }
                RepresentationLevel::Diff => {
                    // Provide a compact summary with a few recent open tasks.
                    let recent = open.iter().take(10).copied().collect::<Vec<_>>().join("\n");
                    let header = if open_count > 0 {
                        format!(
                            "[TASKS SUMMARY] open: {}, completed: {}. Recent open items:",
                            open_count, completed
                        )
                    } else {
                        format!(
                            "[TASKS SUMMARY] open: {}, completed: {}",
                            open_count, completed
                        )
                    };
                    if recent.is_empty() {
                        header
                    } else {
                        format!("{header}\n{recent}")
                    }
                }
                RepresentationLevel::Pointer => {
                    // Minimal pointer representation for tasks.
                    format!(
                        "[POINTER] Tasks (open: {}, completed: {})",
                        open_count, completed
                    )
                }
            };
        }

        // Default behavior for non-task content.
        match level {
            RepresentationLevel::Full => text.to_string(),
            RepresentationLevel::Symbols => text
                .lines()
                .filter(|line| {
                    line.contains("fn ")
                        || line.contains("struct ")
                        || line.contains("impl ")
                        || line.contains("trait ")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            RepresentationLevel::Headers => text
                .lines()
                .filter(|line| {
                    line.trim_start().starts_with("//")
                        || line.contains("mod ")
                        || line.contains("use ")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            RepresentationLevel::Diff => {
                format!(
                    "[DIFF] Recent changes in content ({}...)",
                    text.chars().take(100).collect::<String>()
                )
            }
            RepresentationLevel::Pointer => {
                format!("[POINTER] Content available ({} chars)", text.len())
            }
        }
    }

    fn determine_content_type(&self, context: &crate::context::AgentContext) -> ContentType {
        match context {
            crate::context::AgentContext::File(_) => ContentType::Files,
            crate::context::AgentContext::Directory(_) => ContentType::Files,
            crate::context::AgentContext::Symbol(_) => ContentType::Files,
            crate::context::AgentContext::Thread(_) => ContentType::Tasks,
            crate::context::AgentContext::Selection(_) => ContentType::Files,
            crate::context::AgentContext::FetchedUrl(_) => ContentType::Files,
            crate::context::AgentContext::TextThread(_) => ContentType::Tasks,
            crate::context::AgentContext::Rules(_) => ContentType::Files,
            crate::context::AgentContext::Image(_) => ContentType::Files,
        }
    }

    fn determine_content_type_from_handle(&self, handle: &AgentContextHandle) -> ContentType {
        match handle {
            AgentContextHandle::File(_) => ContentType::Files,
            AgentContextHandle::Directory(_) => ContentType::Files,
            AgentContextHandle::Symbol(_) => ContentType::Files,
            AgentContextHandle::Thread(_) => ContentType::Tasks,
            AgentContextHandle::Selection(_) => ContentType::Files,
            AgentContextHandle::FetchedUrl(_) => ContentType::Files,
            AgentContextHandle::TextThread(_) => ContentType::Tasks,
            AgentContextHandle::Rules(_) => ContentType::Files,
            AgentContextHandle::Image(_) => ContentType::Files,
        }
    }

    // Enforce per-type soft budgets. If a type exceeds its allocated share, trigger global rebalance.
    fn enforce_per_type_budgets(&mut self) {
        if self.config.max_tokens == 0 || self.loaded_content.is_empty() {
            return;
        }
        // Aggregate usage by type
        let mut usage: HashMap<ContentType, usize> = HashMap::default();
        for c in self.loaded_content.values() {
            *usage.entry(c.content_type.clone()).or_default() += c.token_count;
        }
        // Determine violations
        let mut over_types = Vec::new();
        for (ty, share) in &self.config.per_type_budgets {
            let allowed = (*share * self.config.max_tokens as f32) as usize;
            // Per-type hysteresis margin to reduce churn
            let margin = (allowed as f32 * self.config.rebalance_hysteresis_ratio) as usize;
            if let Some(used) = usage.get(ty) {
                if *used > allowed + margin {
                    over_types.push(ty.clone());
                }
            }
        }
        if !over_types.is_empty() {
            self.global_rebalance(Some(over_types), None);
        }
    }

    pub fn allocation_stats(&self) -> AllocationStats {
        let mut per_type: HashMap<ContentType, usize> = HashMap::default();
        let mut by_level: HashMap<RepresentationLevel, usize> = HashMap::default();
        for c in self.loaded_content.values() {
            *per_type.entry(c.content_type.clone()).or_default() += c.token_count;
            *by_level.entry(c.representation_level.clone()).or_default() += c.token_count;
        }
        AllocationStats {
            total_tokens: self.current_token_count,
            per_type,
            by_level,
            count: self.loaded_content.len(),
        }
    }

    pub fn metrics_snapshot(&self) -> InternalMetrics {
        self.metrics.clone()
    }

    pub fn status(&self) -> ManagerStatus {
        ManagerStatus {
            allocation: self.allocation_stats(),
            metrics: self.metrics_snapshot(),
        }
    }

    pub fn record_terminal_output(
        &mut self,
        command: String,
        output: String,
        is_error: bool,
        cx: &mut Context<Self>,
    ) -> String {
        let _original_chars = output.len();
        // Use helper that returns stats (added in TerminalCompressor impl)
        let (command_id, original_len, compressed_len) = self
            .terminal_compressor
            .add_entry_with_stats(command, output, is_error);
        let saved_chars = original_len.saturating_sub(compressed_len);
        if saved_chars > 0 {
            let saved_tokens = saved_chars / 4;
            if saved_tokens > 0 {
                self.metrics.compression_savings_tokens += saved_tokens;
            }
            let ratio = if original_len > 0 {
                compressed_len as f64 / original_len as f64
            } else {
                1.0
            };
            cx.emit(ContextManagerEvent::TerminalCompressed {
                command_id: command_id.clone(),
                compression_ratio: ratio,
            });
        }
        command_id
    }

    fn next_higher(level: &RepresentationLevel) -> Option<RepresentationLevel> {
        match level {
            RepresentationLevel::Pointer => Some(RepresentationLevel::Diff),
            RepresentationLevel::Diff => Some(RepresentationLevel::Headers),
            RepresentationLevel::Headers => Some(RepresentationLevel::Symbols),
            RepresentationLevel::Symbols => Some(RepresentationLevel::Full),
            RepresentationLevel::Full => None,
        }
    }

    fn promote_high_value_content(&mut self, cx: &mut Context<Self>) {
        let safety_cap = (self.config.max_tokens as f32 * self.config.safety_threshold) as usize;
        let hysteresis_margin =
            (self.config.max_tokens as f32 * self.config.rebalance_hysteresis_ratio) as usize;
        if self.current_token_count + hysteresis_margin >= safety_cap {
            return;
        }
        // Collect promotable items
        let mut items: Vec<(ContextId, f64, RepresentationLevel)> = self
            .loaded_content
            .values()
            .filter(|c| Self::next_higher(&c.representation_level).is_some())
            .map(|c| {
                (
                    c.context_id,
                    self.usage_tracker.get_priority(&c.context_id),
                    c.representation_level.clone(),
                )
            })
            .collect();
        // High priority first, lower representation first
        items.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.2.cmp(&b.2))
        });
        for (id, _prio, level) in items {
            if self.current_token_count + hysteresis_margin >= safety_cap {
                break;
            }
            let Some(entry) = self.loaded_content.get_mut(&id) else {
                continue;
            };
            let Some(target) = Self::next_higher(&level) else {
                continue;
            };
            // Obtain or generate variant from original_full
            let variant_text = if let Some(cached) = entry.cached_variants.get(&target) {
                cached.clone()
            } else {
                let generated: Arc<str> = Arc::from(Self::apply_representation_level(
                    &entry.original_full,
                    &target,
                    Some(&entry.content_type),
                ));
                entry
                    .cached_variants
                    .insert(target.clone(), generated.clone());
                generated
            };
            let new_tokens = self.token_counter.count_tokens(&variant_text);
            if self.current_token_count + (new_tokens.saturating_sub(entry.token_count))
                > safety_cap
            {
                continue;
            }

            // (Removed embedded test code from production method)
            // (End of removed test code)
            if new_tokens > entry.token_count {
                let from = entry.representation_level.clone();
                let from_tokens = entry.token_count;
                entry.text = variant_text;
                entry.representation_level = target.clone();
                entry.token_count = new_tokens;
                self.current_token_count += new_tokens - from_tokens;
                cx.emit(ContextManagerEvent::RepresentationChanged {
                    context_id: entry.context_id,
                    from,
                    to: target,
                    token_delta: entry.token_count as isize - from_tokens as isize,
                });
                self.metrics.promotions += 1;
            }
        }
    }
}

// Global degradation / eviction pass. If specific types provided, focus them first.
impl PointerContextManager {
    fn global_rebalance(
        &mut self,
        focus_types: Option<Vec<ContentType>>,
        mut cx: Option<&mut Context<Self>>,
    ) {
        if self.loaded_content.is_empty() {
            return;
        }
        // Collect candidates sorted by (priority ascending, representation descending)
        // We only degrade when above safety threshold or per-type violation.
        let safety_cap = (self.config.max_tokens as f32 * self.config.safety_threshold) as usize;
        let hysteresis_margin =
            (self.config.max_tokens as f32 * self.config.rebalance_hysteresis_ratio) as usize;
        if self.current_token_count <= safety_cap && focus_types.is_none() {
            return;
        }

        // Build a working list (ContextId, priority, current_level)
        let mut items: Vec<(ContextId, f64, RepresentationLevel)> = self
            .loaded_content
            .values()
            .map(|c| {
                (
                    c.context_id,
                    self.usage_tracker.get_priority(&c.context_id),
                    c.representation_level.clone(),
                )
            })
            .collect();

        // Sort lowest priority first, and degrade the richest (highest level) first for those
        items.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.2.cmp(&a.2))
        });

        let is_focus = |ty: &ContentType| {
            if let Some(ref focus) = focus_types {
                focus.contains(ty)
            } else {
                true
            }
        };

        for (id, _prio, level) in items {
            self.metrics.rebalance_iterations += 1;
            if self.current_token_count + hysteresis_margin <= safety_cap && focus_types.is_none() {
                break;
            }
            // Attempt to degrade one step
            if let Some(entry) = self.loaded_content.get_mut(&id) {
                if !is_focus(&entry.content_type) {
                    continue;
                }
                let next = match level {
                    RepresentationLevel::Full => Some(RepresentationLevel::Symbols),
                    RepresentationLevel::Symbols => Some(RepresentationLevel::Headers),
                    RepresentationLevel::Headers => Some(RepresentationLevel::Diff),
                    RepresentationLevel::Diff => Some(RepresentationLevel::Pointer),
                    RepresentationLevel::Pointer => None,
                };
                if let Some(next_level) = next {
                    if next_level > entry.representation_level {
                        continue; // ordering guard; shouldn't happen
                    }
                    // Generate or fetch cached variant
                    let variant_text = if let Some(cached) = entry.cached_variants.get(&next_level)
                    {
                        cached.clone()
                    } else {
                        let generated: Arc<str> = Arc::from(Self::apply_representation_level(
                            &entry.original_full,
                            &next_level,
                            Some(&entry.content_type),
                        ));
                        entry
                            .cached_variants
                            .insert(next_level.clone(), generated.clone());
                        generated
                    };
                    // Recount tokens
                    let new_tokens = self.token_counter.count_tokens(&variant_text);
                    if new_tokens < entry.token_count {
                        let diff_tokens_before = entry.token_count;
                        let from = entry.representation_level.clone();
                        entry.text = variant_text;
                        entry.representation_level = next_level.clone();
                        entry.token_count = new_tokens;
                        self.current_token_count = self
                            .current_token_count
                            .saturating_sub(diff_tokens_before - new_tokens);
                        if let Some(cx) = &mut cx {
                            cx.emit(ContextManagerEvent::RepresentationChanged {
                                context_id: entry.context_id,
                                from,
                                to: next_level.clone(),
                                token_delta: new_tokens as isize - diff_tokens_before as isize,
                            });
                        }
                        self.metrics.degradations += 1;
                    }
                } else {
                    // Already at pointer; consider eviction
                    let tok = entry.token_count;
                    self.current_token_count = self.current_token_count.saturating_sub(tok);
                    if let Some(cx) = &mut cx {
                        cx.emit(ContextManagerEvent::ContentEvicted(id));
                    }
                    self.metrics.evictions += 1;
                    self.loaded_content.remove(&id);
                }
            }
        }
    }

    // Called after assembly to perform rebalancing & per-type enforcement
    pub fn post_assembly_maintenance(&mut self, cx: &mut Context<Self>) {
        self.enforce_per_type_budgets();
        if self.current_token_count > self.config.max_tokens {
            self.global_rebalance(None, Some(cx));
        } else {
            self.global_rebalance(None, Some(cx));
        }
        // After rebalance, try promotions if headroom
        self.promote_high_value_content(cx);
    }
    // Remove expired diffs (TTL)
    fn prune_expired_diffs(&mut self) {
        let now = Instant::now();
        for (_path, diffs) in self.diff_manager.active_diffs.iter_mut() {
            let before = diffs.len();
            diffs.retain(|d| {
                if let Some(exp) = d.expires_at {
                    exp > now
                } else {
                    true
                }
            });
            let removed = before.saturating_sub(diffs.len());
            if removed > 0 {
                self.metrics.diff_pruned += removed;
            }
        }
        self.diff_manager.active_diffs.retain(|_, v| !v.is_empty());
    }
    #[cfg(debug_assertions)]
    fn assert_invariants(&self) {
        let sum: usize = self.loaded_content.values().map(|c| c.token_count).sum();
        debug_assert_eq!(
            sum, self.current_token_count,
            "Invariant breached: token sum mismatch"
        );
        for (_id, c) in &self.loaded_content {
            debug_assert!(
                c.cached_variants.contains_key(&c.representation_level),
                "Missing cache for current level"
            );
            if let Some(full) = c.cached_variants.get(&RepresentationLevel::Full) {
                if c.representation_level != RepresentationLevel::Full {
                    debug_assert!(
                        full.len() >= c.text.len(),
                        "Full should not be shorter than degraded"
                    );
                }
            }
        }
    }
}
/// Scored candidate for context assembly
#[derive(Debug, Clone)]
struct ScoredCandidate {
    context_id: ContextId,
    level: RepresentationLevel,
    score: f64,
}

/// Allocation statistics (moved out of impl block)
pub struct AllocationStats {
    pub total_tokens: usize,
    pub per_type: HashMap<ContentType, usize>,
    pub by_level: HashMap<RepresentationLevel, usize>,
    pub count: usize,
}

/// Public manager status snapshot (moved out of impl block)
pub struct ManagerStatus {
    pub allocation: AllocationStats,
    pub metrics: InternalMetrics,
}
