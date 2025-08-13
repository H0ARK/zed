use crate::{
    context::{AgentContextHandle, AgentContextKey, ContextId},
    context_store::ContextStore,
    thread_store::ThreadStore,
};
use anyhow::Result;
use collections::{HashMap, VecDeque};
use gpui::{Context, Entity, EventEmitter, Task, WeakEntity};
use prompt_store::PromptStore;
use indexmap::IndexSet;
use project::Project;
use std::{
    path::PathBuf,
    sync::Arc,
    time::Instant,
};
use thiserror::Error;

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
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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
    pub text: Arc<str>,
    pub content_type: ContentType,
    pub representation_level: RepresentationLevel,
    pub token_count: usize,
    pub context_id: ContextId,
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
        Self { stats: HashMap::default(), recent_order: VecDeque::new(), capacity }
    }

    /// Recompute priorities using the current bounded history and use counts
    pub fn recompute_priorities(&mut self) {
        let max_count = self
            .stats
            .values()
            .map(|s| s.use_count)
            .max()
            .unwrap_or(1) as f64;

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
        self.stats.get(context_id).map(|s| s.priority).unwrap_or(0.0)
    }
    
    pub fn get_stats(&self, context_id: &ContextId) -> Option<&UsageStats> {
        self.stats.get(context_id)
    }

    pub fn recency_score(&self, context_id: &ContextId) -> f64 {
        if let Some(pos_from_front) = self.recent_order.iter().position(|id| id == context_id) {
            // Most recent at the back
            let pos_from_back = self.recent_order.len().saturating_sub(1).saturating_sub(pos_from_front);
            if self.capacity == 0 { return 0.0; }
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
}

/// Diff management system
pub struct DiffManager {
    active_diffs: HashMap<PathBuf, Vec<DiffEntry>>,
    max_per_strategy: HashMap<EditStrategy, usize>,
    default_limit: usize,
}

impl DiffManager {
    pub fn new(max_per_strategy: HashMap<EditStrategy, usize>, default_limit: usize) -> Self {
        Self { active_diffs: HashMap::default(), max_per_strategy: max_per_strategy, default_limit }
    }

    pub fn handle_file_edit(
        &mut self,
        file_path: PathBuf,
        old_content: &str,
        new_content: &str,
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
        };
        
        self.active_diffs.entry(file_path).or_default().push(diff_entry.clone());
        // Enforce per-strategy history limits
        self.trim_file_diffs(&diff_entry.file_path);
        
        Task::ready(Ok(Some(diff_entry)))
    }
    
    fn select_edit_strategy(&self, magnitude: &ChangeMagnitude) -> EditStrategy {
        if magnitude.contextual_factors.comment_only_changes 
            || magnitude.contextual_factors.formatting_only_changes {
            return EditStrategy::ReplaceWithDiff;
        }
        
        if magnitude.change_percentage < 0.05 
            && magnitude.contextual_factors.function_changes > 0 {
            return EditStrategy::KeepBoth;
        }
        
        if magnitude.change_percentage > 0.5 
            || magnitude.is_structural_change {
            return EditStrategy::DiffMarkerOnly;
        }
        
        EditStrategy::ReplaceWithDiff
    }

    fn generate_diff(&self, old_content: &str, new_content: &str) -> String {
        // Simple diff implementation - in practice, use a proper diff library
        if old_content == new_content {
            String::new()
        } else {
            format!("--- old\n+++ new\n@@ -1,{} +1,{} @@\n", 
                old_content.lines().count(), 
                new_content.lines().count())
        }
    }

    fn calculate_edit_magnitude(&self, diff_content: &str, old_content: &str) -> ChangeMagnitude {
        let old_lines = old_content.lines().count();
        let diff_lines = diff_content.lines().count();
        
        ChangeMagnitude {
            change_percentage: if old_lines > 0 { diff_lines as f64 / old_lines as f64 } else { 1.0 },
            token_impact: diff_lines as f64 * 4.0, // Rough token estimate
            change_count: diff_lines,
            is_structural_change: diff_lines > old_lines / 2,
            contextual_factors: ContextualFactors::default(),
        }
    }

    fn trim_file_diffs(&mut self, file_path: &PathBuf) {
        if let Some(diffs) = self.active_diffs.get_mut(file_path) {
            // For each strategy, enforce its limit by removing oldest entries of that strategy
            let strategies = [EditStrategy::KeepBoth, EditStrategy::ReplaceWithDiff, EditStrategy::DiffMarkerOnly];
            for strategy in strategies {
                let limit = self.max_per_strategy.get(&strategy).copied().unwrap_or(self.default_limit);
                if limit == 0 { continue; }

                // Count existing of this strategy
                let mut count = diffs.iter().filter(|d| d.strategy == strategy).count();
                if count <= limit { continue; }

                // Remove from oldest to newest
                let mut i = 0;
                while count > limit && i < diffs.len() {
                    if diffs[i].strategy == strategy {
                        diffs.remove(i);
                        count -= 1;
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

    pub fn add_entry(
        &mut self,
        command: String,
        output: String,
        is_error: bool,
    ) -> String {
        let command_id = format!("cmd_{}", Instant::now().elapsed().as_nanos());
        let original_length = output.len();
        
        let processed_output = if is_error {
            self.process_error_output(&output)
        } else {
            self.process_success_output(&output)
        };
        
        let is_compressed = processed_output.len() < original_length;
        
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
        let tail = lines.iter().rev().take(5).rev().copied().collect::<Vec<_>>().join("\n");
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
    BudgetExceeded { current: usize, limit: usize },
    DiffCreated { file_path: PathBuf, strategy: EditStrategy },
    TerminalCompressed { command_id: String, compression_ratio: f64 },
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
        candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        
        // Budgeted assembly with degradation
        cx.spawn(async move |this, cx| {
            let mut loaded_content = Vec::new();
            let threshold = (this.read_with(cx, |this, _| this.config.max_tokens)? as f32 
                * this.read_with(cx, |this, _| this.config.safety_threshold)?) as usize;
            
            for candidate in candidates {
                let level = candidate.level.clone();
                let content = this.update(cx, |this, cx| {
                    this.load_reference_with_level(candidate.context_id, level, cx)
                })?.await?;
                
                if let Some(content) = content {
                    let tokens = this.read_with(cx, |this, _| {
                        this.token_counter.count_tokens(&content.text)
                    })?;
                    
                    if this.read_with(cx, |this, _| this.current_token_count)? + tokens <= threshold {
                        this.update(cx, |this, _| {
                            this.current_token_count += tokens;
                            this.usage_tracker.update_usage(candidate.context_id, content.content_type.clone());
                        })?;
                        loaded_content.push(content);
                    } else {
                        // Try degrading the representation level
                        let level = candidate.level.clone();
                        if let Some(degraded_content) = this.update(cx, |this, cx| {
                            this.try_degrade_content(candidate.context_id, level, cx)
                        })?.await? {
                            loaded_content.push(degraded_content);
                        }
                        break;
                    }
                }
            }
            
            Ok(loaded_content)
        })
    }

    fn extract_references(&self, message: &str) -> Vec<AgentContextKey> {
        let refs = Vec::new();
        
        // Extract file paths (simple pattern matching)
        // Note: This is a simplified implementation. In practice, we would
        // need to match against existing context in the ContextStore
        for word in message.split_whitespace() {
            if word.contains('/') && (word.ends_with(".rs") || word.ends_with(".ts") || word.ends_with(".js")) {
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
                store.context().find(|handle| {
                    match handle {
                        AgentContextHandle::File(h) => h.context_id == context_id,
                        AgentContextHandle::Directory(h) => h.context_id == context_id,
                        AgentContextHandle::Symbol(h) => h.context_id == context_id,
                        AgentContextHandle::Selection(h) => h.context_id == context_id,
                        AgentContextHandle::FetchedUrl(h) => h.context_id == context_id,
                        AgentContextHandle::Thread(h) => h.context_id == context_id,
                        AgentContextHandle::TextThread(h) => h.context_id == context_id,
                        AgentContextHandle::Rules(h) => h.context_id == context_id,
                        AgentContextHandle::Image(h) => h.context_id == context_id,
                    }
                }).cloned()
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
                 AgentContextHandle::TextThread(_) => "[Text thread content placeholder]".to_string(),
                 AgentContextHandle::Rules(_) => "[Rules content placeholder]".to_string(),
                 AgentContextHandle::Image(_) => "[Image content]".to_string(),
             };
            
            // Apply representation level transformation
            let processed_content = this.read_with(cx, |this, _| {
                this.apply_representation_level(&raw_content, &level)
            })?;
            
            let content_type = this.read_with(cx, |this, _| {
                 this.determine_content_type_from_handle(&handle)
             })?;
            
            let token_count = this.read_with(cx, |this, _| {
                this.token_counter.count_tokens(&processed_content)
            })?;
            
            Ok(Some(LoadedContent {
                text: processed_content.into(),
                content_type,
                representation_level: level,
                token_count,
                context_id,
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

    fn emit_content_loaded(&mut self, context_id: ContextId, cx: &mut Context<Self>) {
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
    
    fn apply_representation_level(&self, text: &str, level: &RepresentationLevel) -> String {
        match level {
            RepresentationLevel::Full => text.to_string(),
            RepresentationLevel::Symbols => {
                // Extract function signatures, struct definitions, etc.
                text.lines()
                    .filter(|line| {
                        line.contains("fn ") || line.contains("struct ") || 
                        line.contains("impl ") || line.contains("trait ")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            RepresentationLevel::Headers => {
                // Extract comments and major structural elements
                text.lines()
                    .filter(|line| {
                        line.trim_start().starts_with("//") || 
                        line.contains("mod ") || line.contains("use ")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            RepresentationLevel::Diff => {
                // Show only recent changes if available
                format!("[DIFF] Recent changes in content ({}...)", 
                    text.chars().take(100).collect::<String>())
            },
            RepresentationLevel::Pointer => {
                // Minimal pointer representation
                format!("[POINTER] Content available ({} chars)", text.len())
            },
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
}

/// Scored candidate for context assembly
#[derive(Debug, Clone)]
struct ScoredCandidate {
    context_id: ContextId,
    level: RepresentationLevel,
    score: f64,
}