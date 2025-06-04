use std::fmt::Write as _;
use std::io::Write;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

use agent_settings::{AgentSettings, CompletionMode};
use anyhow::{Result, anyhow};
use assistant_tool::{ActionLog, AnyToolCard, Tool, ToolWorkingSet};
use chrono::{DateTime, Utc};
use collections::HashMap;
use editor::display_map::CreaseMetadata;
use feature_flags::{self, FeatureFlagAppExt};
use futures::future::Shared;
use futures::{FutureExt, StreamExt as _};
use git::repository::DiffType;
use gpui::{
    AnyWindowHandle, App, AppContext, AsyncApp, Context, Entity, EventEmitter, SharedString, Task,
    WeakEntity,
};
use language_model::{
    ConfiguredModel, LanguageModel, LanguageModelCompletionError, LanguageModelCompletionEvent,
    LanguageModelId, LanguageModelKnownError, LanguageModelRegistry, LanguageModelRequest,
    LanguageModelRequestMessage, LanguageModelRequestTool, LanguageModelToolResult,
    LanguageModelToolResultContent, LanguageModelToolUseId, MessageContent,
    ModelRequestLimitReachedError, PaymentRequiredError, RequestUsage, Role, SelectedModel,
    StopReason, TokenUsage,
};
use postage::stream::Stream as _;
use project::Project;
use project::git_store::{GitStore, GitStoreCheckpoint, RepositoryState};
use prompt_store::{ModelContext, PromptBuilder};
use proto::Plan;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings::Settings;
use thiserror::Error;
use ui::Window;
use util::{ResultExt as _, post_inc};
use uuid::Uuid;
use zed_llm_client::{CompletionIntent, CompletionRequestStatus};

use crate::ThreadStore;
use crate::context::{AgentContext, AgentContextHandle, ContextLoadResult, LoadedContext};
use crate::thread_store::{
    SerializedCrease, SerializedLanguageModel, SerializedMessage, SerializedMessageSegment,
    SerializedThread, SerializedToolResult, SerializedToolUse, SharedProjectContext,
};
use crate::tool_use::{PendingToolUse, ToolUse, ToolUseMetadata, ToolUseState};

/// Result of context optimization with metrics
#[derive(Debug, Clone)]
pub struct OptimizedContext {
    pub messages: Vec<Message>,
    pub strategy_used: ContextStrategy,
    pub memory_savings: f32,
    pub context_preservation: f32,
    pub optimization_metrics: ContextOptimizationMetrics,
}

/// Detailed metrics about context optimization
#[derive(Debug, Clone)]
pub struct ContextOptimizationMetrics {
    pub original_message_count: usize,
    pub optimized_message_count: usize,
    pub original_token_count: usize,
    pub optimized_token_count: usize,
    pub compression_ratio: f32,
    pub messages_compressed: usize,
    pub messages_kept_full: usize,
    pub context_zones: ContextZoneBreakdown,
    pub optimization_time_ms: f32,
}

/// Breakdown of how messages were distributed across priority zones
#[derive(Debug, Clone)]
pub struct ContextZoneBreakdown {
    pub recent_zone_messages: usize,      // Always kept (highest priority)
    pub compressed_zone_messages: usize,  // Smart compression applied
    pub dropped_zone_messages: usize,     // Completely dropped (lowest priority)
    pub recent_zone_tokens: usize,
    pub compressed_zone_tokens: usize,
    pub dropped_zone_tokens: usize,
}

/// Comprehensive analytics about context optimization
#[derive(Debug, Clone)]
pub struct ContextOptimizationAnalytics {
    pub current_strategy: ContextStrategy,
    pub efficiency_score: f32,           // 0.0-1.0, higher is better
    pub memory_pressure: MemoryPressure,
    pub optimization_frequency: OptimizationFrequency,
    pub performance_metrics: ContextOptimizationMetrics,
    pub recommendations: Vec<OptimizationRecommendation>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemoryPressure {
    Low,    // < 80% of token limit
    Medium, // 80-95% of token limit  
    High,   // > 95% of token limit
}

#[derive(Debug, Clone, PartialEq)]
pub enum OptimizationFrequency {
    Rare,       // 0-5 messages
    Occasional, // 6-15 messages
    Frequent,   // 16-30 messages
    Constant,   // 30+ messages
}

#[derive(Debug, Clone, PartialEq)]
pub enum OptimizationRecommendation {
    IncreaseCompression,      // Memory usage too high
    PreserveMoreContext,      // Context preservation too low
    EnableSmartCompression,   // Should switch from Full strategy
    OptimizePerformance,      // Optimization taking too long
    ConsiderPointerStrategy,  // For very large contexts (future)
    EnableDynamicZones,       // For complex conversations (future)
}

/// Different context optimization strategies
#[derive(Debug, Clone, PartialEq)]
pub enum ContextStrategy {
    /// Use all messages without optimization
    Full,
    /// Smart compression with diff-based strategies
    SmartCompression,
    /// Pointer-based external storage (future)
    PointerBased,
    /// Dynamic zone management (future)
    DynamicZones,
}

#[derive(
    Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Serialize, Deserialize, JsonSchema,
)]
pub struct ThreadId(Arc<str>);

impl ThreadId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string().into())
    }
}

impl std::fmt::Display for ThreadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ThreadId {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}

/// The ID of the user prompt that initiated a request.
///
/// This equates to the user physically submitting a message to the model (e.g., by pressing the Enter key).
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Serialize, Deserialize)]
pub struct PromptId(Arc<str>);

impl PromptId {
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string().into())
    }
}

impl std::fmt::Display for PromptId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Serialize, Deserialize)]
pub struct MessageId(pub(crate) usize);

impl MessageId {
    fn post_inc(&mut self) -> Self {
        Self(post_inc(&mut self.0))
    }
}

/// Stored information that can be used to resurrect a context crease when creating an editor for a past message.
#[derive(Clone, Debug)]
pub struct MessageCrease {
    pub range: Range<usize>,
    pub metadata: CreaseMetadata,
    /// None for a deserialized message, Some otherwise.
    pub context: Option<AgentContextHandle>,
}

/// A message in a [`Thread`].
#[derive(Debug, Clone)]
pub struct Message {
    pub id: MessageId,
    pub role: Role,
    pub segments: Vec<MessageSegment>,
    pub loaded_context: LoadedContext,
    pub creases: Vec<MessageCrease>,
    pub is_hidden: bool,
}

impl Message {
    /// Returns whether the message contains any meaningful text that should be displayed
    /// The model sometimes runs tool without producing any text or just a marker ([`USING_TOOL_MARKER`])
    pub fn should_display_content(&self) -> bool {
        self.segments.iter().all(|segment| segment.should_display())
    }

    pub fn push_thinking(&mut self, text: &str, signature: Option<String>) {
        if let Some(MessageSegment::Thinking {
            text: segment,
            signature: current_signature,
        }) = self.segments.last_mut()
        {
            if let Some(signature) = signature {
                *current_signature = Some(signature);
            }
            segment.push_str(text);
        } else {
            self.segments.push(MessageSegment::Thinking {
                text: text.to_string(),
                signature,
            });
        }
    }

    pub fn push_text(&mut self, text: &str) {
        if let Some(MessageSegment::Text(segment)) = self.segments.last_mut() {
            segment.push_str(text);
        } else {
            self.segments.push(MessageSegment::Text(text.to_string()));
        }
    }

    pub fn to_string(&self) -> String {
        let mut result = String::new();

        if !self.loaded_context.text.is_empty() {
            result.push_str(&self.loaded_context.text);
        }

        for segment in &self.segments {
            match segment {
                MessageSegment::Text(text) => result.push_str(text),
                MessageSegment::Thinking { text, .. } => {
                    result.push_str("<think>\n");
                    result.push_str(text);
                    result.push_str("\n</think>");
                }
                MessageSegment::RedactedThinking(_) => {}
            }
        }

        result
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageSegment {
    Text(String),
    Thinking {
        text: String,
        signature: Option<String>,
    },
    RedactedThinking(Vec<u8>),
}

impl MessageSegment {
    pub fn should_display(&self) -> bool {
        match self {
            Self::Text(text) => text.is_empty(),
            Self::Thinking { text, .. } => text.is_empty(),
            Self::RedactedThinking(_) => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectSnapshot {
    pub worktree_snapshots: Vec<WorktreeSnapshot>,
    pub unsaved_buffer_paths: Vec<String>,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeSnapshot {
    pub worktree_path: String,
    pub git_state: Option<GitState>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitState {
    pub remote_url: Option<String>,
    pub head_sha: Option<String>,
    pub current_branch: Option<String>,
    pub diff: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ThreadCheckpoint {
    message_id: MessageId,
    git_checkpoint: GitStoreCheckpoint,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ThreadFeedback {
    Positive,
    Negative,
}

pub enum LastRestoreCheckpoint {
    Pending {
        message_id: MessageId,
    },
    Error {
        message_id: MessageId,
        error: String,
    },
}

impl LastRestoreCheckpoint {
    pub fn message_id(&self) -> MessageId {
        match self {
            LastRestoreCheckpoint::Pending { message_id } => *message_id,
            LastRestoreCheckpoint::Error { message_id, .. } => *message_id,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub enum DetailedSummaryState {
    #[default]
    NotGenerated,
    Generating {
        message_id: MessageId,
    },
    Generated {
        text: SharedString,
        message_id: MessageId,
    },
}

impl DetailedSummaryState {
    fn text(&self) -> Option<SharedString> {
        if let Self::Generated { text, .. } = self {
            Some(text.clone())
        } else {
            None
        }
    }
}

#[derive(Default, Debug)]
pub struct TotalTokenUsage {
    pub total: usize,
    pub max: usize,
}

impl TotalTokenUsage {
    pub fn ratio(&self) -> TokenUsageRatio {
        #[cfg(debug_assertions)]
        let warning_threshold: f32 = std::env::var("ZED_THREAD_WARNING_THRESHOLD")
            .unwrap_or("0.8".to_string())
            .parse()
            .unwrap();
        #[cfg(not(debug_assertions))]
        let warning_threshold: f32 = 0.8;

        // When the maximum is unknown because there is no selected model,
        // avoid showing the token limit warning.
        if self.max == 0 {
            TokenUsageRatio::Normal
        } else if self.total >= self.max {
            TokenUsageRatio::Exceeded
        } else if self.total as f32 / self.max as f32 >= warning_threshold {
            TokenUsageRatio::Warning
        } else {
            TokenUsageRatio::Normal
        }
    }

    pub fn add(&self, tokens: usize) -> TotalTokenUsage {
        TotalTokenUsage {
            total: self.total + tokens,
            max: self.max,
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum TokenUsageRatio {
    #[default]
    Normal,
    Warning,
    Exceeded,
}

#[derive(Debug, Clone, Copy)]
pub enum QueueState {
    Sending,
    Queued { position: usize },
    Started,
}

/// A thread of conversation with the LLM.
pub struct Thread {
    id: ThreadId,
    updated_at: DateTime<Utc>,
    summary: ThreadSummary,
    pending_summary: Task<Option<()>>,
    detailed_summary_task: Task<Option<()>>,
    detailed_summary_tx: postage::watch::Sender<DetailedSummaryState>,
    detailed_summary_rx: postage::watch::Receiver<DetailedSummaryState>,
    completion_mode: agent_settings::CompletionMode,
    messages: Vec<Message>,
    next_message_id: MessageId,
    last_prompt_id: PromptId,
    project_context: SharedProjectContext,
    checkpoints_by_message: HashMap<MessageId, ThreadCheckpoint>,
    completion_count: usize,
    pending_completions: Vec<PendingCompletion>,
    project: Entity<Project>,
    prompt_builder: Arc<PromptBuilder>,
    tools: Entity<ToolWorkingSet>,
    tool_use: ToolUseState,
    action_log: Entity<ActionLog>,
    last_restore_checkpoint: Option<LastRestoreCheckpoint>,
    pending_checkpoint: Option<ThreadCheckpoint>,
    initial_project_snapshot: Shared<Task<Option<Arc<ProjectSnapshot>>>>,
    request_token_usage: Vec<TokenUsage>,
    cumulative_token_usage: TokenUsage,
    exceeded_window_error: Option<ExceededWindowError>,
    last_usage: Option<RequestUsage>,
    tool_use_limit_reached: bool,
    feedback: Option<ThreadFeedback>,
    message_feedback: HashMap<MessageId, ThreadFeedback>,
    last_auto_capture_at: Option<Instant>,
    last_received_chunk_at: Option<Instant>,
    request_callback: Option<
        Box<dyn FnMut(&LanguageModelRequest, &[Result<LanguageModelCompletionEvent, String>])>,
    >,
    remaining_turns: u32,
    configured_model: Option<ConfiguredModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadSummary {
    Pending,
    Generating,
    Ready(SharedString),
    Error,
}

impl ThreadSummary {
    pub const DEFAULT: SharedString = SharedString::new_static("New Thread");

    pub fn or_default(&self) -> SharedString {
        self.unwrap_or(Self::DEFAULT)
    }

    pub fn unwrap_or(&self, message: impl Into<SharedString>) -> SharedString {
        self.ready().unwrap_or_else(|| message.into())
    }

    pub fn ready(&self) -> Option<SharedString> {
        match self {
            ThreadSummary::Ready(summary) => Some(summary.clone()),
            ThreadSummary::Pending | ThreadSummary::Generating | ThreadSummary::Error => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExceededWindowError {
    /// Model used when last message exceeded context window
    model_id: LanguageModelId,
    /// Token count including last message
    token_count: usize,
}

impl Thread {
    pub fn new(
        project: Entity<Project>,
        tools: Entity<ToolWorkingSet>,
        prompt_builder: Arc<PromptBuilder>,
        system_prompt: SharedProjectContext,
        cx: &mut Context<Self>,
    ) -> Self {
        let (detailed_summary_tx, detailed_summary_rx) = postage::watch::channel();
        let configured_model = LanguageModelRegistry::read_global(cx).default_model();

        Self {
            id: ThreadId::new(),
            updated_at: Utc::now(),
            summary: ThreadSummary::Pending,
            pending_summary: Task::ready(None),
            detailed_summary_task: Task::ready(None),
            detailed_summary_tx,
            detailed_summary_rx,
            completion_mode: AgentSettings::get_global(cx).preferred_completion_mode,
            messages: Vec::new(),
            next_message_id: MessageId(0),
            last_prompt_id: PromptId::new(),
            project_context: system_prompt,
            checkpoints_by_message: HashMap::default(),
            completion_count: 0,
            pending_completions: Vec::new(),
            project: project.clone(),
            prompt_builder,
            tools: tools.clone(),
            last_restore_checkpoint: None,
            pending_checkpoint: None,
            tool_use: ToolUseState::new(tools.clone()),
            action_log: cx.new(|_| ActionLog::new(project.clone())),
            initial_project_snapshot: {
                let project_snapshot = Self::project_snapshot(project, cx);
                cx.foreground_executor()
                    .spawn(async move { Some(project_snapshot.await) })
                    .shared()
            },
            request_token_usage: Vec::new(),
            cumulative_token_usage: TokenUsage::default(),
            exceeded_window_error: None,
            last_usage: None,
            tool_use_limit_reached: false,
            feedback: None,
            message_feedback: HashMap::default(),
            last_auto_capture_at: None,
            last_received_chunk_at: None,
            request_callback: None,
            remaining_turns: u32::MAX,
            configured_model,
        }
    }

    pub fn deserialize(
        id: ThreadId,
        serialized: SerializedThread,
        project: Entity<Project>,
        tools: Entity<ToolWorkingSet>,
        prompt_builder: Arc<PromptBuilder>,
        project_context: SharedProjectContext,
        window: Option<&mut Window>, // None in headless mode
        cx: &mut Context<Self>,
    ) -> Self {
        let next_message_id = MessageId(
            serialized
                .messages
                .last()
                .map(|message| message.id.0 + 1)
                .unwrap_or(0),
        );
        let tool_use = ToolUseState::from_serialized_messages(
            tools.clone(),
            &serialized.messages,
            project.clone(),
            window,
            cx,
        );
        let (detailed_summary_tx, detailed_summary_rx) =
            postage::watch::channel_with(serialized.detailed_summary_state);

        let configured_model = LanguageModelRegistry::global(cx).update(cx, |registry, cx| {
            serialized
                .model
                .and_then(|model| {
                    let model = SelectedModel {
                        provider: model.provider.clone().into(),
                        model: model.model.clone().into(),
                    };
                    registry.select_model(&model, cx)
                })
                .or_else(|| registry.default_model())
        });

        let completion_mode = serialized
            .completion_mode
            .unwrap_or_else(|| AgentSettings::get_global(cx).preferred_completion_mode);

        Self {
            id,
            updated_at: serialized.updated_at,
            summary: ThreadSummary::Ready(serialized.summary),
            pending_summary: Task::ready(None),
            detailed_summary_task: Task::ready(None),
            detailed_summary_tx,
            detailed_summary_rx,
            completion_mode,
            messages: serialized
                .messages
                .into_iter()
                .map(|message| Message {
                    id: message.id,
                    role: message.role,
                    segments: message
                        .segments
                        .into_iter()
                        .map(|segment| match segment {
                            SerializedMessageSegment::Text { text } => MessageSegment::Text(text),
                            SerializedMessageSegment::Thinking { text, signature } => {
                                MessageSegment::Thinking { text, signature }
                            }
                            SerializedMessageSegment::RedactedThinking { data } => {
                                MessageSegment::RedactedThinking(data)
                            }
                        })
                        .collect(),
                    loaded_context: LoadedContext {
                        contexts: Vec::new(),
                        text: message.context,
                        images: Vec::new(),
                    },
                    creases: message
                        .creases
                        .into_iter()
                        .map(|crease| MessageCrease {
                            range: crease.start..crease.end,
                            metadata: CreaseMetadata {
                                icon_path: crease.icon_path,
                                label: crease.label,
                            },
                            context: None,
                        })
                        .collect(),
                    is_hidden: message.is_hidden,
                })
                .collect(),
            next_message_id,
            last_prompt_id: PromptId::new(),
            project_context,
            checkpoints_by_message: HashMap::default(),
            completion_count: 0,
            pending_completions: Vec::new(),
            last_restore_checkpoint: None,
            pending_checkpoint: None,
            project: project.clone(),
            prompt_builder,
            tools,
            tool_use,
            action_log: cx.new(|_| ActionLog::new(project)),
            initial_project_snapshot: Task::ready(serialized.initial_project_snapshot).shared(),
            request_token_usage: serialized.request_token_usage,
            cumulative_token_usage: serialized.cumulative_token_usage,
            exceeded_window_error: None,
            last_usage: None,
            tool_use_limit_reached: serialized.tool_use_limit_reached,
            feedback: None,
            message_feedback: HashMap::default(),
            last_auto_capture_at: None,
            last_received_chunk_at: None,
            request_callback: None,
            remaining_turns: u32::MAX,
            configured_model,
        }
    }

    pub fn set_request_callback(
        &mut self,
        callback: impl 'static
        + FnMut(&LanguageModelRequest, &[Result<LanguageModelCompletionEvent, String>]),
    ) {
        self.request_callback = Some(Box::new(callback));
    }

    pub fn id(&self) -> &ThreadId {
        &self.id
    }

    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    pub fn touch_updated_at(&mut self) {
        self.updated_at = Utc::now();
    }

    pub fn advance_prompt_id(&mut self) {
        self.last_prompt_id = PromptId::new();
    }

    pub fn project_context(&self) -> SharedProjectContext {
        self.project_context.clone()
    }

    pub fn get_or_init_configured_model(&mut self, cx: &App) -> Option<ConfiguredModel> {
        if self.configured_model.is_none() {
            self.configured_model = LanguageModelRegistry::read_global(cx).default_model();
        }
        self.configured_model.clone()
    }

    pub fn configured_model(&self) -> Option<ConfiguredModel> {
        self.configured_model.clone()
    }

    pub fn set_configured_model(&mut self, model: Option<ConfiguredModel>, cx: &mut Context<Self>) {
        self.configured_model = model;
        cx.notify();
    }

    pub fn summary(&self) -> &ThreadSummary {
        &self.summary
    }

    pub fn set_summary(&mut self, new_summary: impl Into<SharedString>, cx: &mut Context<Self>) {
        let current_summary = match &self.summary {
            ThreadSummary::Pending | ThreadSummary::Generating => return,
            ThreadSummary::Ready(summary) => summary,
            ThreadSummary::Error => &ThreadSummary::DEFAULT,
        };

        let mut new_summary = new_summary.into();

        if new_summary.is_empty() {
            new_summary = ThreadSummary::DEFAULT;
        }

        if current_summary != &new_summary {
            self.summary = ThreadSummary::Ready(new_summary);
            cx.emit(ThreadEvent::SummaryChanged);
        }
    }

    pub fn completion_mode(&self) -> CompletionMode {
        self.completion_mode
    }

    pub fn set_completion_mode(&mut self, mode: CompletionMode) {
        self.completion_mode = mode;
    }

    pub fn message(&self, id: MessageId) -> Option<&Message> {
        let index = self
            .messages
            .binary_search_by(|message| message.id.cmp(&id))
            .ok()?;

        self.messages.get(index)
    }

    pub fn messages(&self) -> impl ExactSizeIterator<Item = &Message> {
        self.messages.iter()
    }

    pub fn is_generating(&self) -> bool {
        !self.pending_completions.is_empty() || !self.all_tools_finished()
    }

    /// Indicates whether streaming of language model events is stale.
    /// When `is_generating()` is false, this method returns `None`.
    pub fn is_generation_stale(&self) -> Option<bool> {
        const STALE_THRESHOLD: u128 = 250;

        self.last_received_chunk_at
            .map(|instant| instant.elapsed().as_millis() > STALE_THRESHOLD)
    }

    fn received_chunk(&mut self) {
        self.last_received_chunk_at = Some(Instant::now());
    }

    pub fn queue_state(&self) -> Option<QueueState> {
        self.pending_completions
            .first()
            .map(|pending_completion| pending_completion.queue_state)
    }

    pub fn tools(&self) -> &Entity<ToolWorkingSet> {
        &self.tools
    }

    pub fn pending_tool(&self, id: &LanguageModelToolUseId) -> Option<&PendingToolUse> {
        self.tool_use
            .pending_tool_uses()
            .into_iter()
            .find(|tool_use| &tool_use.id == id)
    }

    pub fn tools_needing_confirmation(&self) -> impl Iterator<Item = &PendingToolUse> {
        self.tool_use
            .pending_tool_uses()
            .into_iter()
            .filter(|tool_use| tool_use.status.needs_confirmation())
    }

    pub fn has_pending_tool_uses(&self) -> bool {
        !self.tool_use.pending_tool_uses().is_empty()
    }

    pub fn checkpoint_for_message(&self, id: MessageId) -> Option<ThreadCheckpoint> {
        self.checkpoints_by_message.get(&id).cloned()
    }

    pub fn restore_checkpoint(
        &mut self,
        checkpoint: ThreadCheckpoint,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        self.last_restore_checkpoint = Some(LastRestoreCheckpoint::Pending {
            message_id: checkpoint.message_id,
        });
        cx.emit(ThreadEvent::CheckpointChanged);
        cx.notify();

        let git_store = self.project().read(cx).git_store().clone();
        let restore = git_store.update(cx, |git_store, cx| {
            git_store.restore_checkpoint(checkpoint.git_checkpoint.clone(), cx)
        });

        cx.spawn(async move |this, cx| {
            let result = restore.await;
            this.update(cx, |this, cx| {
                if let Err(err) = result.as_ref() {
                    this.last_restore_checkpoint = Some(LastRestoreCheckpoint::Error {
                        message_id: checkpoint.message_id,
                        error: err.to_string(),
                    });
                } else {
                    this.truncate(checkpoint.message_id, cx);
                    this.last_restore_checkpoint = None;
                }
                this.pending_checkpoint = None;
                cx.emit(ThreadEvent::CheckpointChanged);
                cx.notify();
            })?;
            result
        })
    }

    fn finalize_pending_checkpoint(&mut self, cx: &mut Context<Self>) {
        let pending_checkpoint = if self.is_generating() {
            return;
        } else if let Some(checkpoint) = self.pending_checkpoint.take() {
            checkpoint
        } else {
            return;
        };

        self.finalize_checkpoint(pending_checkpoint, cx);
    }

    fn finalize_checkpoint(
        &mut self,
        pending_checkpoint: ThreadCheckpoint,
        cx: &mut Context<Self>,
    ) {
        let git_store = self.project.read(cx).git_store().clone();
        let final_checkpoint = git_store.update(cx, |git_store, cx| git_store.checkpoint(cx));
        cx.spawn(async move |this, cx| match final_checkpoint.await {
            Ok(final_checkpoint) => {
                let equal = git_store
                    .update(cx, |store, cx| {
                        store.compare_checkpoints(
                            pending_checkpoint.git_checkpoint.clone(),
                            final_checkpoint.clone(),
                            cx,
                        )
                    })?
                    .await
                    .unwrap_or(false);

                if !equal {
                    this.update(cx, |this, cx| {
                        this.insert_checkpoint(pending_checkpoint, cx)
                    })?;
                }

                Ok(())
            }
            Err(_) => this.update(cx, |this, cx| {
                this.insert_checkpoint(pending_checkpoint, cx)
            }),
        })
        .detach();
    }

    fn insert_checkpoint(&mut self, checkpoint: ThreadCheckpoint, cx: &mut Context<Self>) {
        self.checkpoints_by_message
            .insert(checkpoint.message_id, checkpoint);
        cx.emit(ThreadEvent::CheckpointChanged);
        cx.notify();
    }

    pub fn last_restore_checkpoint(&self) -> Option<&LastRestoreCheckpoint> {
        self.last_restore_checkpoint.as_ref()
    }

    pub fn truncate(&mut self, message_id: MessageId, cx: &mut Context<Self>) {
        let Some(message_ix) = self
            .messages
            .iter()
            .rposition(|message| message.id == message_id)
        else {
            return;
        };
        for deleted_message in self.messages.drain(message_ix..) {
            self.checkpoints_by_message.remove(&deleted_message.id);
        }
        cx.notify();
    }

    pub fn context_for_message(&self, id: MessageId) -> impl Iterator<Item = &AgentContext> {
        self.messages
            .iter()
            .find(|message| message.id == id)
            .into_iter()
            .flat_map(|message| message.loaded_context.contexts.iter())
    }

    pub fn is_turn_end(&self, ix: usize) -> bool {
        if self.messages.is_empty() {
            return false;
        }

        if !self.is_generating() && ix == self.messages.len() - 1 {
            return true;
        }

        let Some(message) = self.messages.get(ix) else {
            return false;
        };

        if message.role != Role::Assistant {
            return false;
        }

        self.messages
            .get(ix + 1)
            .and_then(|message| {
                self.message(message.id)
                    .map(|next_message| next_message.role == Role::User && !next_message.is_hidden)
            })
            .unwrap_or(false)
    }

    pub fn last_usage(&self) -> Option<RequestUsage> {
        self.last_usage
    }

    pub fn tool_use_limit_reached(&self) -> bool {
        self.tool_use_limit_reached
    }

    /// Returns whether all of the tool uses have finished running.
    pub fn all_tools_finished(&self) -> bool {
        // If the only pending tool uses left are the ones with errors, then
        // that means that we've finished running all of the pending tools.
        self.tool_use
            .pending_tool_uses()
            .iter()
            .all(|tool_use| tool_use.status.is_error())
    }

    pub fn tool_uses_for_message(&self, id: MessageId, cx: &App) -> Vec<ToolUse> {
        self.tool_use.tool_uses_for_message(id, cx)
    }

    pub fn tool_results_for_message(
        &self,
        assistant_message_id: MessageId,
    ) -> Vec<&LanguageModelToolResult> {
        self.tool_use.tool_results_for_message(assistant_message_id)
    }

    pub fn tool_result(&self, id: &LanguageModelToolUseId) -> Option<&LanguageModelToolResult> {
        self.tool_use.tool_result(id)
    }

    pub fn output_for_tool(&self, id: &LanguageModelToolUseId) -> Option<&Arc<str>> {
        match &self.tool_use.tool_result(id)?.content {
            LanguageModelToolResultContent::Text(text) => Some(text),
            LanguageModelToolResultContent::Image(_) => {
                // TODO: We should display image
                None
            }
        }
    }

    pub fn card_for_tool(&self, id: &LanguageModelToolUseId) -> Option<AnyToolCard> {
        self.tool_use.tool_result_card(id).cloned()
    }

    /// Return tools that are both enabled and supported by the model
    pub fn available_tools(
        &self,
        cx: &App,
        model: Arc<dyn LanguageModel>,
    ) -> Vec<LanguageModelRequestTool> {
        if model.supports_tools() {
            self.tools()
                .read(cx)
                .enabled_tools(cx)
                .into_iter()
                .filter_map(|tool| {
                    // Skip tools that cannot be supported
                    let input_schema = tool.input_schema(model.tool_input_format()).ok()?;
                    Some(LanguageModelRequestTool {
                        name: tool.name(),
                        description: tool.description(),
                        input_schema,
                    })
                })
                .collect()
        } else {
            Vec::default()
        }
    }

    pub fn insert_user_message(
        &mut self,
        text: impl Into<String>,
        loaded_context: ContextLoadResult,
        git_checkpoint: Option<GitStoreCheckpoint>,
        creases: Vec<MessageCrease>,
        cx: &mut Context<Self>,
    ) -> MessageId {
        if !loaded_context.referenced_buffers.is_empty() {
            self.action_log.update(cx, |log, cx| {
                for buffer in loaded_context.referenced_buffers {
                    log.buffer_read(buffer, cx);
                }
            });
        }

        let message_id = self.insert_message(
            Role::User,
            vec![MessageSegment::Text(text.into())],
            loaded_context.loaded_context,
            creases,
            false,
            cx,
        );

        if let Some(git_checkpoint) = git_checkpoint {
            self.pending_checkpoint = Some(ThreadCheckpoint {
                message_id,
                git_checkpoint,
            });
        }

        self.auto_capture_telemetry(cx);

        message_id
    }

    pub fn insert_invisible_continue_message(&mut self, cx: &mut Context<Self>) -> MessageId {
        let id = self.insert_message(
            Role::User,
            vec![MessageSegment::Text("Continue where you left off".into())],
            LoadedContext::default(),
            vec![],
            true,
            cx,
        );
        self.pending_checkpoint = None;

        id
    }

    pub fn insert_assistant_message(
        &mut self,
        segments: Vec<MessageSegment>,
        cx: &mut Context<Self>,
    ) -> MessageId {
        self.insert_message(
            Role::Assistant,
            segments,
            LoadedContext::default(),
            Vec::new(),
            false,
            cx,
        )
    }

    pub fn insert_message(
        &mut self,
        role: Role,
        segments: Vec<MessageSegment>,
        loaded_context: LoadedContext,
        creases: Vec<MessageCrease>,
        is_hidden: bool,
        cx: &mut Context<Self>,
    ) -> MessageId {
        let id = self.next_message_id.post_inc();
        self.messages.push(Message {
            id,
            role,
            segments,
            loaded_context,
            creases,
            is_hidden,
        });
        self.touch_updated_at();
        cx.emit(ThreadEvent::MessageAdded(id));
        id
    }

    pub fn edit_message(
        &mut self,
        id: MessageId,
        new_role: Role,
        new_segments: Vec<MessageSegment>,
        loaded_context: Option<LoadedContext>,
        checkpoint: Option<GitStoreCheckpoint>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(message) = self.messages.iter_mut().find(|message| message.id == id) else {
            return false;
        };
        message.role = new_role;
        message.segments = new_segments;
        if let Some(context) = loaded_context {
            message.loaded_context = context;
        }
        if let Some(git_checkpoint) = checkpoint {
            self.checkpoints_by_message.insert(
                id,
                ThreadCheckpoint {
                    message_id: id,
                    git_checkpoint,
                },
            );
        }
        self.touch_updated_at();
        cx.emit(ThreadEvent::MessageEdited(id));
        true
    }

    pub fn delete_message(&mut self, id: MessageId, cx: &mut Context<Self>) -> bool {
        let Some(index) = self.messages.iter().position(|message| message.id == id) else {
            return false;
        };
        self.messages.remove(index);
        self.touch_updated_at();
        cx.emit(ThreadEvent::MessageDeleted(id));
        true
    }

    /// Returns the representation of this [`Thread`] in a textual form.
    ///
    /// This is the representation we use when attaching a thread as context to another thread.
    pub fn text(&self) -> String {
        let mut text = String::new();

        for message in &self.messages {
            text.push_str(match message.role {
                language_model::Role::User => "User:",
                language_model::Role::Assistant => "Agent:",
                language_model::Role::System => "System:",
            });
            text.push('\n');

            for segment in &message.segments {
                match segment {
                    MessageSegment::Text(content) => text.push_str(content),
                    MessageSegment::Thinking { text: content, .. } => {
                        text.push_str(&format!("<think>{}</think>", content))
                    }
                    MessageSegment::RedactedThinking(_) => {}
                }
            }
            text.push('\n');
        }

        text
    }

    /// Serializes this thread into a format for storage or telemetry.
    pub fn serialize(&self, cx: &mut Context<Self>) -> Task<Result<SerializedThread>> {
        let initial_project_snapshot = self.initial_project_snapshot.clone();
        cx.spawn(async move |this, cx| {
            let initial_project_snapshot = initial_project_snapshot.await;
            this.read_with(cx, |this, cx| SerializedThread {
                version: SerializedThread::VERSION.to_string(),
                summary: this.summary().or_default(),
                updated_at: this.updated_at(),
                messages: this
                    .messages()
                    .map(|message| SerializedMessage {
                        id: message.id,
                        role: message.role,
                        segments: message
                            .segments
                            .iter()
                            .map(|segment| match segment {
                                MessageSegment::Text(text) => {
                                    SerializedMessageSegment::Text { text: text.clone() }
                                }
                                MessageSegment::Thinking { text, signature } => {
                                    SerializedMessageSegment::Thinking {
                                        text: text.clone(),
                                        signature: signature.clone(),
                                    }
                                }
                                MessageSegment::RedactedThinking(data) => {
                                    SerializedMessageSegment::RedactedThinking {
                                        data: data.clone(),
                                    }
                                }
                            })
                            .collect(),
                        tool_uses: this
                            .tool_uses_for_message(message.id, cx)
                            .into_iter()
                            .map(|tool_use| SerializedToolUse {
                                id: tool_use.id,
                                name: tool_use.name,
                                input: tool_use.input,
                            })
                            .collect(),
                        tool_results: this
                            .tool_results_for_message(message.id)
                            .into_iter()
                            .map(|tool_result| SerializedToolResult {
                                tool_use_id: tool_result.tool_use_id.clone(),
                                is_error: tool_result.is_error,
                                content: tool_result.content.clone(),
                                output: tool_result.output.clone(),
                            })
                            .collect(),
                        context: message.loaded_context.text.clone(),
                        creases: message
                            .creases
                            .iter()
                            .map(|crease| SerializedCrease {
                                start: crease.range.start,
                                end: crease.range.end,
                                icon_path: crease.metadata.icon_path.clone(),
                                label: crease.metadata.label.clone(),
                            })
                            .collect(),
                        is_hidden: message.is_hidden,
                    })
                    .collect(),
                initial_project_snapshot,
                cumulative_token_usage: this.cumulative_token_usage,
                request_token_usage: this.request_token_usage.clone(),
                detailed_summary_state: this.detailed_summary_rx.borrow().clone(),
                exceeded_window_error: this.exceeded_window_error.clone(),
                model: this
                    .configured_model
                    .as_ref()
                    .map(|model| SerializedLanguageModel {
                        provider: model.provider.id().0.to_string(),
                        model: model.model.id().0.to_string(),
                    }),
                completion_mode: Some(this.completion_mode),
                tool_use_limit_reached: this.tool_use_limit_reached,
            })
        })
    }

    pub fn remaining_turns(&self) -> u32 {
        self.remaining_turns
    }

    pub fn set_remaining_turns(&mut self, remaining_turns: u32) {
        self.remaining_turns = remaining_turns;
    }

    pub fn send_to_model(
        &mut self,
        model: Arc<dyn LanguageModel>,
        intent: CompletionIntent,
        window: Option<AnyWindowHandle>,
        cx: &mut Context<Self>,
    ) {
        if self.remaining_turns == 0 {
            return;
        }

        self.remaining_turns -= 1;

        let request = self.to_completion_request(model.clone(), intent, cx);

        self.stream_completion(request, model, window, cx);
    }

    pub fn used_tools_since_last_user_message(&self) -> bool {
        for message in self.messages.iter().rev() {
            if self.tool_use.message_has_tool_results(message.id) {
                return true;
            } else if message.role == Role::User {
                return false;
            }
        }

        false
    }

    pub fn to_completion_request(
        &self,
        model: Arc<dyn LanguageModel>,
        intent: CompletionIntent,
        cx: &mut Context<Self>,
    ) -> LanguageModelRequest {
        // First, optimize the context using smart strategies
        let optimized_context = self.optimize_context_for_request(&model, cx);
        
        let mut request = LanguageModelRequest {
            thread_id: Some(self.id.to_string()),
            prompt_id: Some(self.last_prompt_id.to_string()),
            intent: Some(intent),
            mode: None,
            messages: vec![],
            tools: Vec::new(),
            tool_choice: None,
            stop: Vec::new(),
            temperature: AgentSettings::temperature_for_model(&model, cx),
        };

        let available_tools = self.available_tools(cx, model.clone());
        let available_tool_names = available_tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect();

        let model_context = &ModelContext {
            available_tools: available_tool_names,
        };

        if let Some(project_context) = self.project_context.borrow().as_ref() {
            match self
                .prompt_builder
                .generate_assistant_system_prompt(project_context, model_context)
            {
                Err(err) => {
                    let message = format!("{err:?}").into();
                    log::error!("{message}");
                    cx.emit(ThreadEvent::ShowError(ThreadError::Message {
                        header: "Error generating system prompt".into(),
                        message,
                    }));
                }
                Ok(system_prompt) => {
                    request.messages.push(LanguageModelRequestMessage {
                        role: Role::System,
                        content: vec![MessageContent::Text(system_prompt)],
                        cache: true,
                    });
                }
            }
        } else {
            let message = "Context for system prompt unexpectedly not ready.".into();
            log::error!("{message}");
            cx.emit(ThreadEvent::ShowError(ThreadError::Message {
                header: "Error generating system prompt".into(),
                message,
            }));
        }

        // Use optimized context instead of all messages
        let mut message_ix_to_cache = None;
        for optimized_message in &optimized_context.messages {
            let mut request_message = LanguageModelRequestMessage {
                role: optimized_message.role,
                content: Vec::new(),
                cache: false,
            };

            optimized_message
                .loaded_context
                .add_to_request_message(&mut request_message);

            for segment in &optimized_message.segments {
                match segment {
                    MessageSegment::Text(text) => {
                        if !text.is_empty() {
                            request_message
                                .content
                                .push(MessageContent::Text(text.into()));
                        }
                    }
                    MessageSegment::Thinking { text, signature } => {
                        if !text.is_empty() {
                            request_message.content.push(MessageContent::Thinking {
                                text: text.into(),
                                signature: signature.clone(),
                            });
                        }
                    }
                    MessageSegment::RedactedThinking(data) => {
                        request_message
                            .content
                            .push(MessageContent::RedactedThinking(data.clone()));
                    }
                };
            }

            let mut cache_message = true;
            let mut tool_results_message = LanguageModelRequestMessage {
                role: Role::User,
                content: Vec::new(),
                cache: false,
            };
            
            // Handle tool results for this message
            if let Some(original_message) = self.messages.iter().find(|m| m.id == optimized_message.id) {
                for (tool_use, tool_result) in self.tool_use.tool_results(original_message.id) {
                    if let Some(tool_result) = tool_result {
                        request_message
                            .content
                            .push(MessageContent::ToolUse(tool_use.clone()));
                        tool_results_message
                            .content
                            .push(MessageContent::ToolResult(LanguageModelToolResult {
                                tool_use_id: tool_use.id.clone(),
                                tool_name: tool_result.tool_name.clone(),
                                is_error: tool_result.is_error,
                                content: if tool_result.content.is_empty() {
                                    "<Tool returned an empty string>".into()
                                } else {
                                    tool_result.content.clone()
                                },
                                output: None,
                            }));
                    } else {
                        cache_message = false;
                        log::debug!(
                            "skipped tool use {:?} because it is still pending",
                            tool_use
                        );
                    }
                }
            }

            if cache_message {
                message_ix_to_cache = Some(request.messages.len());
            }
            request.messages.push(request_message);

            if !tool_results_message.content.is_empty() {
                if cache_message {
                    message_ix_to_cache = Some(request.messages.len());
                }
                request.messages.push(tool_results_message);
            }
        }

        // https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching
        if let Some(message_ix_to_cache) = message_ix_to_cache {
            request.messages[message_ix_to_cache].cache = true;
        }

        self.attached_tracked_files_state(&mut request.messages, cx);

        request.tools = available_tools;
        request.mode = if model.supports_max_mode() {
            Some(self.completion_mode.into())
        } else {
            Some(CompletionMode::Normal.into())
        };

        request
    }

    /// Optimize context using smart strategies before sending to model
    fn optimize_context_for_request(
        &self,
        model: &Arc<dyn LanguageModel>,
        cx: &App,
    ) -> OptimizedContext {
        let max_tokens = model.max_token_count();
        let current_usage = self.total_token_usage().map(|u| u.total).unwrap_or(0);
        
        // If we're under 70% of token limit, use all messages
        let safety_threshold = (max_tokens as f32 * 0.7) as usize;
        if current_usage < safety_threshold {
            return OptimizedContext {
                messages: self.messages.clone(),
                strategy_used: ContextStrategy::Full,
                memory_savings: 0.0,
                context_preservation: 1.0,
                optimization_metrics: ContextOptimizationMetrics {
                    original_message_count: self.messages.len(),
                    optimized_message_count: self.messages.len(),
                    original_token_count: current_usage,
                    optimized_token_count: current_usage,
                    compression_ratio: 1.0,
                    messages_compressed: 0,
                    messages_kept_full: self.messages.len(),
                    context_zones: ContextZoneBreakdown {
                        recent_zone_messages: self.messages.len(),
                        compressed_zone_messages: 0,
                        dropped_zone_messages: 0,
                        recent_zone_tokens: current_usage,
                        compressed_zone_tokens: 0,
                        dropped_zone_tokens: 0,
                    },
                    optimization_time_ms: 0.0,
                },
            };
        }

        // Apply smart optimization strategies
        self.apply_context_optimization_strategies(max_tokens, cx)
    }

    /// Apply your JavaScript context management strategies in Rust
    /// This implements the core logic from your PointerBasedContextManager.js
    fn apply_context_optimization_strategies(
        &self,
        max_tokens: usize,
        cx: &App,
    ) -> OptimizedContext {
        let start_time = std::time::Instant::now();
        let target_tokens = (max_tokens as f32 * 0.7) as usize; // 70% safety threshold
        let current_usage = self.total_token_usage().map(|u| u.total).unwrap_or(0);
        
        // If we're well under the limit, use full context
        if current_usage < target_tokens {
            let optimization_time = start_time.elapsed().as_secs_f32() * 1000.0;
            return OptimizedContext {
                messages: self.messages.clone(),
                strategy_used: ContextStrategy::Full,
                memory_savings: 0.0,
                context_preservation: 1.0,
                optimization_metrics: ContextOptimizationMetrics {
                    original_message_count: self.messages.len(),
                    optimized_message_count: self.messages.len(),
                    original_token_count: current_usage,
                    optimized_token_count: current_usage,
                    compression_ratio: 0.0,
                    messages_compressed: 0,
                    messages_kept_full: self.messages.len(),
                    context_zones: self.calculate_context_zones(&self.messages),
                    optimization_time_ms: optimization_time,
                },
            };
        }

        // Apply smart compression strategies similar to your JS implementation
        let (optimized_messages, strategy_used) = self.apply_smart_compression_strategy(target_tokens, cx);
        
        let optimized_tokens: usize = optimized_messages.iter()
            .map(|msg| self.estimate_message_tokens(msg))
            .sum();
            
        let memory_savings = if current_usage > 0 {
            1.0 - (optimized_tokens as f32 / current_usage as f32)
        } else {
            0.0
        };
        
        // Calculate context preservation based on strategy and actual compression
        let context_preservation = match strategy_used {
            ContextStrategy::Full => 1.0,
            ContextStrategy::SmartCompression => {
                // Dynamic calculation based on how much we actually compressed
                let compression_factor = optimized_tokens as f32 / current_usage as f32;
                0.70 + (compression_factor * 0.25) // Range: 70-95% preservation
            },
            ContextStrategy::PointerBased => 0.90, // Future implementation
            ContextStrategy::DynamicZones => 0.80, // Future implementation
        };
        
        let optimization_time = start_time.elapsed().as_secs_f32() * 1000.0;
        
        OptimizedContext {
            messages: optimized_messages.clone(),
            strategy_used,
            memory_savings,
            context_preservation,
            optimization_metrics: ContextOptimizationMetrics {
                original_message_count: self.messages.len(),
                optimized_message_count: optimized_messages.len(),
                original_token_count: current_usage,
                optimized_token_count: optimized_tokens,
                compression_ratio: memory_savings,
                messages_compressed: self.messages.len() - optimized_messages.len(),
                messages_kept_full: optimized_messages.len(),
                context_zones: self.calculate_context_zones(&optimized_messages),
                optimization_time_ms: optimization_time,
            },
        }
    }

    /// Smart compression strategy implementing your JS logic
    fn apply_smart_compression_strategy(
        &self,
        target_tokens: usize,
        cx: &App,
    ) -> (Vec<Message>, ContextStrategy) {
        let mut optimized_messages = Vec::new();
        let mut total_tokens = 0;
        
        // Strategy: Keep recent messages, compress older ones
        // This mirrors your JavaScript "priority zones" concept
        
        // Zone 1: Always keep the most recent messages (highest priority)
        let recent_zone_size = 3;
        let recent_messages: Vec<_> = self.messages.iter()
            .rev()
            .take(recent_zone_size)
            .collect();
            
        // Add recent messages (in reverse order to maintain chronology)
        for message in recent_messages.iter().rev() {
            optimized_messages.push((*message).clone());
            total_tokens += self.estimate_message_tokens(message);
        }
        
        // Zone 2: Compress older messages using smart strategies
        let older_messages: Vec<_> = self.messages.iter()
            .rev()
            .skip(recent_zone_size)
            .collect();
            
        for message in older_messages.iter().rev() {
            let message_tokens = self.estimate_message_tokens(message);
            
            if total_tokens + message_tokens > target_tokens {
                // Apply compression - this is where your diff-based strategies would go
                if let Some(compressed) = self.compress_message_using_smart_strategy(message, cx) {
                    let compressed_tokens = self.estimate_message_tokens(&compressed);
                    if total_tokens + compressed_tokens <= target_tokens {
                        optimized_messages.insert(optimized_messages.len() - recent_zone_size, compressed);
                        // Update token count for tracking (used in optimization metrics)
                        #[allow(unused_assignments)]
                        { total_tokens += compressed_tokens; }
                    }
                }
                break; // Stop adding more messages
            } else {
                // Message fits, add it as-is
                optimized_messages.insert(optimized_messages.len() - recent_zone_size, (*message).clone());
                total_tokens += message_tokens;
            }
        }
        
        (optimized_messages, ContextStrategy::SmartCompression)
    }

    /// Compress a message using smart strategies (implementing your JS diff logic)
    fn compress_message_using_smart_strategy(&self, message: &Message, _cx: &App) -> Option<Message> {
        // This implements the core compression logic from your JavaScript system
        let mut compressed = message.clone();
        
        // Strategy 1: Compress large context using diff-like approach
        if !message.loaded_context.text.is_empty() && message.loaded_context.text.len() > 2000 {
            compressed.loaded_context.text = self.create_smart_context_summary(&message.loaded_context.text);
        }
        
        // Strategy 2: Compress long message content
        if let Some(MessageSegment::Text(text)) = message.segments.first() {
            if text.len() > 1000 {
                compressed.segments = vec![MessageSegment::Text(self.create_message_summary(text))];
            }
        }
        
        Some(compressed)
    }

    /// Create smart context summary (implementing your diff-based approach)
    fn create_smart_context_summary(&self, context_text: &str) -> String {
        // This mirrors your JavaScript diff generation logic
        let lines: Vec<&str> = context_text.lines().collect();
        
        if lines.len() <= 20 {
            return context_text.to_string();
        }
        
        // Keep important parts: beginning, end, and structure
        let header_lines = 5;
        let footer_lines = 5;
        let structure_lines = self.extract_structure_lines(&lines);
        
        let mut summary = String::new();
        
        // Add header
        summary.push_str(&lines.iter().take(header_lines).cloned().collect::<Vec<_>>().join("\n"));
        summary.push_str("\n\n");
        
        // Add structure (function definitions, class declarations, etc.)
        if !structure_lines.is_empty() {
            summary.push_str("... [Key structure] ...\n");
            summary.push_str(&structure_lines.join("\n"));
            summary.push_str("\n");
        }
        
        // Add compression indicator
        let compressed_lines = lines.len() - header_lines - footer_lines - structure_lines.len();
        if compressed_lines > 0 {
            summary.push_str(&format!("... [compressed {} lines] ...\n", compressed_lines));
        }
        
        // Add footer
        summary.push_str(&lines.iter().rev().take(footer_lines).rev().cloned().collect::<Vec<_>>().join("\n"));
        
        summary
    }

    /// Extract important structural lines (functions, classes, etc.)
    fn extract_structure_lines(&self, lines: &[&str]) -> Vec<String> {
        lines.iter()
            .filter(|line| {
                let trimmed = line.trim();
                // Look for function definitions, class declarations, etc.
                trimmed.starts_with("fn ") ||
                trimmed.starts_with("class ") ||
                trimmed.starts_with("function ") ||
                trimmed.starts_with("def ") ||
                trimmed.starts_with("impl ") ||
                trimmed.starts_with("struct ") ||
                trimmed.starts_with("enum ") ||
                trimmed.starts_with("interface ") ||
                trimmed.starts_with("type ")
            })
            .take(10) // Limit to avoid too much structure
            .map(|s| s.to_string())
            .collect()
    }

    /// Create message summary
    fn create_message_summary(&self, text: &str) -> String {
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= 10 {
            return text.to_string();
        }
        
        let start = lines.iter().take(5).cloned().collect::<Vec<_>>().join("\n");
        let end = lines.iter().rev().take(3).rev().cloned().collect::<Vec<_>>().join("\n");
        
        format!("{}\n... [compressed {} lines] ...\n{}", start, lines.len() - 8, end)
    }



    /// Estimate token count for a message
    fn estimate_message_tokens(&self, message: &Message) -> usize {
        // Simplified estimation for backwards compatibility
        let mut total_tokens = 0;

        // Basic message text content
        let text_content = message.to_string();
        total_tokens += text_content.len() / 4;

        // Loaded context (files, directories, symbols, etc.)
        total_tokens += message.loaded_context.text.len() / 4;

        // Images in loaded context (images are token-heavy)
        total_tokens += message.loaded_context.images.len() * 765;

        total_tokens
    }





    fn to_summarize_request(
        &self,
        model: &Arc<dyn LanguageModel>,
        intent: CompletionIntent,
        added_user_message: String,
        cx: &App,
    ) -> LanguageModelRequest {
        let mut request = LanguageModelRequest {
            thread_id: None,
            prompt_id: None,
            intent: Some(intent),
            mode: None,
            messages: vec![],
            tools: Vec::new(),
            tool_choice: None,
            stop: Vec::new(),
            temperature: AgentSettings::temperature_for_model(model, cx),
        };

        for message in &self.messages {
            let mut request_message = LanguageModelRequestMessage {
                role: message.role,
                content: Vec::new(),
                cache: false,
            };

            for segment in &message.segments {
                match segment {
                    MessageSegment::Text(text) => request_message
                        .content
                        .push(MessageContent::Text(text.clone())),
                    MessageSegment::Thinking { .. } => {}
                    MessageSegment::RedactedThinking(_) => {}
                }
            }

            if request_message.content.is_empty() {
                continue;
            }

            request.messages.push(request_message);
        }

        request.messages.push(LanguageModelRequestMessage {
            role: Role::User,
            content: vec![MessageContent::Text(added_user_message)],
            cache: false,
        });

        request
    }

    fn attached_tracked_files_state(
        &self,
        messages: &mut Vec<LanguageModelRequestMessage>,
        cx: &App,
    ) {
        const STALE_FILES_HEADER: &str = include_str!("./prompts/stale_files_prompt_header.txt");

        let mut stale_message = String::new();

        let action_log = self.action_log.read(cx);

        for stale_file in action_log.stale_buffers(cx) {
            let Some(file) = stale_file.read(cx).file() else {
                continue;
            };

            if stale_message.is_empty() {
                write!(&mut stale_message, "{}\n", STALE_FILES_HEADER.trim()).ok();
            }

            writeln!(&mut stale_message, "- {}", file.path().display()).ok();
        }

        let mut content = Vec::with_capacity(2);

        if !stale_message.is_empty() {
            content.push(stale_message.into());
        }

        if !content.is_empty() {
            let context_message = LanguageModelRequestMessage {
                role: Role::User,
                content,
                cache: false,
            };

            messages.push(context_message);
        }
    }

    pub fn stream_completion(
        &mut self,
        request: LanguageModelRequest,
        model: Arc<dyn LanguageModel>,
        window: Option<AnyWindowHandle>,
        cx: &mut Context<Self>,
    ) {
        self.tool_use_limit_reached = false;

        let pending_completion_id = post_inc(&mut self.completion_count);
        let mut request_callback_parameters = if self.request_callback.is_some() {
            Some((request.clone(), Vec::new()))
        } else {
            None
        };
        let prompt_id = self.last_prompt_id.clone();
        let tool_use_metadata = ToolUseMetadata {
            model: model.clone(),
            thread_id: self.id.clone(),
            prompt_id: prompt_id.clone(),
        };

        self.last_received_chunk_at = Some(Instant::now());

        let task = cx.spawn(async move |thread, cx| {
            let stream_completion_future = model.stream_completion(request, &cx);
            let initial_token_usage =
                thread.read_with(cx, |thread, _cx| thread.cumulative_token_usage);
            let stream_completion = async {
                let mut events = stream_completion_future.await?;

                let mut stop_reason = StopReason::EndTurn;
                let mut current_token_usage = TokenUsage::default();

                thread
                    .update(cx, |_thread, cx| {
                        cx.emit(ThreadEvent::NewRequest);
                    })
                    .ok();

                let mut request_assistant_message_id = None;

                while let Some(event) = events.next().await {
                    if let Some((_, response_events)) = request_callback_parameters.as_mut() {
                        response_events
                            .push(event.as_ref().map_err(|error| error.to_string()).cloned());
                    }

                    thread.update(cx, |thread, cx| {
                        let event = match event {
                            Ok(event) => event,
                            Err(LanguageModelCompletionError::BadInputJson {
                                id,
                                tool_name,
                                raw_input: invalid_input_json,
                                json_parse_error,
                            }) => {
                                thread.receive_invalid_tool_json(
                                    id,
                                    tool_name,
                                    invalid_input_json,
                                    json_parse_error,
                                    window,
                                    cx,
                                );
                                return Ok(());
                            }
                            Err(LanguageModelCompletionError::Other(error)) => {
                                return Err(error);
                            }
                        };

                        match event {
                            LanguageModelCompletionEvent::StartMessage { .. } => {
                                request_assistant_message_id =
                                    Some(thread.insert_assistant_message(
                                        vec![MessageSegment::Text(String::new())],
                                        cx,
                                    ));
                            }
                            LanguageModelCompletionEvent::Stop(reason) => {
                                stop_reason = reason;
                            }
                            LanguageModelCompletionEvent::UsageUpdate(token_usage) => {
                                thread.update_token_usage_at_last_message(token_usage);
                                thread.cumulative_token_usage = thread.cumulative_token_usage
                                    + token_usage
                                    - current_token_usage;
                                current_token_usage = token_usage;
                                // Notify UI to update token count display
                                cx.notify();
                            }
                            LanguageModelCompletionEvent::Text(chunk) => {
                                thread.received_chunk();

                                cx.emit(ThreadEvent::ReceivedTextChunk);
                                if let Some(last_message) = thread.messages.last_mut() {
                                    if last_message.role == Role::Assistant
                                        && !thread.tool_use.has_tool_results(last_message.id)
                                    {
                                        last_message.push_text(&chunk);
                                        cx.emit(ThreadEvent::StreamedAssistantText(
                                            last_message.id,
                                            chunk,
                                        ));
                                    } else {
                                        // If we won't have an Assistant message yet, assume this chunk marks the beginning
                                        // of a new Assistant response.
                                        //
                                        // Importantly: We do *not* want to emit a `StreamedAssistantText` event here, as it
                                        // will result in duplicating the text of the chunk in the rendered Markdown.
                                        request_assistant_message_id =
                                            Some(thread.insert_assistant_message(
                                                vec![MessageSegment::Text(chunk.to_string())],
                                                cx,
                                            ));
                                    };
                                }
                            }
                            LanguageModelCompletionEvent::Thinking {
                                text: chunk,
                                signature,
                            } => {
                                thread.received_chunk();

                                if let Some(last_message) = thread.messages.last_mut() {
                                    if last_message.role == Role::Assistant
                                        && !thread.tool_use.has_tool_results(last_message.id)
                                    {
                                        last_message.push_thinking(&chunk, signature);
                                        cx.emit(ThreadEvent::StreamedAssistantThinking(
                                            last_message.id,
                                            chunk,
                                        ));
                                    } else {
                                        // If we won't have an Assistant message yet, assume this chunk marks the beginning
                                        // of a new Assistant response.
                                        //
                                        // Importantly: We do *not* want to emit a `StreamedAssistantText` event here, as it
                                        // will result in duplicating the text of the chunk in the rendered Markdown.
                                        request_assistant_message_id =
                                            Some(thread.insert_assistant_message(
                                                vec![MessageSegment::Thinking {
                                                    text: chunk.to_string(),
                                                    signature,
                                                }],
                                                cx,
                                            ));
                                    };
                                }
                            }
                            LanguageModelCompletionEvent::ToolUse(tool_use) => {
                                let last_assistant_message_id = request_assistant_message_id
                                    .unwrap_or_else(|| {
                                        let new_assistant_message_id =
                                            thread.insert_assistant_message(vec![], cx);
                                        request_assistant_message_id =
                                            Some(new_assistant_message_id);
                                        new_assistant_message_id
                                    });

                                let tool_use_id = tool_use.id.clone();
                                let streamed_input = if tool_use.is_input_complete {
                                    None
                                } else {
                                    Some((&tool_use.input).clone())
                                };

                                let ui_text = thread.tool_use.request_tool_use(
                                    last_assistant_message_id,
                                    tool_use,
                                    tool_use_metadata.clone(),
                                    cx,
                                );

                                if let Some(input) = streamed_input {
                                    cx.emit(ThreadEvent::StreamedToolUse {
                                        tool_use_id,
                                        ui_text,
                                        input,
                                    });
                                }
                            }
                            LanguageModelCompletionEvent::StatusUpdate(status_update) => {
                                if let Some(completion) = thread
                                    .pending_completions
                                    .iter_mut()
                                    .find(|completion| completion.id == pending_completion_id)
                                {
                                    match status_update {
                                        CompletionRequestStatus::Queued {
                                            position,
                                        } => {
                                            completion.queue_state = QueueState::Queued { position };
                                        }
                                        CompletionRequestStatus::Started => {
                                            completion.queue_state =  QueueState::Started;
                                        }
                                        CompletionRequestStatus::Failed {
                                            code, message, request_id
                                        } => {
                                            anyhow::bail!("completion request failed. request_id: {request_id}, code: {code}, message: {message}");
                                        }
                                        CompletionRequestStatus::UsageUpdated {
                                            amount, limit
                                        } => {
                                            let usage = RequestUsage { limit, amount: amount as i32 };

                                            thread.last_usage = Some(usage);
                                        }
                                        CompletionRequestStatus::ToolUseLimitReached => {
                                            thread.tool_use_limit_reached = true;
                                        }
                                    }
                                }
                            }
                        }

                        thread.touch_updated_at();
                        cx.emit(ThreadEvent::StreamedCompletion);
                        cx.notify();

                        thread.auto_capture_telemetry(cx);
                        Ok(())
                    })??;

                    smol::future::yield_now().await;
                }

                thread.update(cx, |thread, cx| {
                    thread.last_received_chunk_at = None;
                    thread
                        .pending_completions
                        .retain(|completion| completion.id != pending_completion_id);

                    // If there is a response without tool use, summarize the message. Otherwise,
                    // allow two tool uses before summarizing.
                    if matches!(thread.summary, ThreadSummary::Pending)
                        && thread.messages.len() >= 2
                        && (!thread.has_pending_tool_uses() || thread.messages.len() >= 6)
                    {
                        thread.summarize(cx);
                    }
                })?;

                anyhow::Ok(stop_reason)
            };

            let result = stream_completion.await;

            thread
                .update(cx, |thread, cx| {
                    thread.finalize_pending_checkpoint(cx);
                    match result.as_ref() {
                        Ok(stop_reason) => match stop_reason {
                            StopReason::ToolUse => {
                                let tool_uses = thread.use_pending_tools(window, cx, model.clone());
                                cx.emit(ThreadEvent::UsePendingTools { tool_uses });
                            }
                            StopReason::EndTurn | StopReason::MaxTokens  => {
                                thread.project.update(cx, |project, cx| {
                                    project.set_agent_location(None, cx);
                                });
                            }
                            StopReason::Refusal => {
                                thread.project.update(cx, |project, cx| {
                                    project.set_agent_location(None, cx);
                                });

                                // Remove the turn that was refused.
                                //
                                // https://docs.anthropic.com/en/docs/test-and-evaluate/strengthen-guardrails/handle-streaming-refusals#reset-context-after-refusal
                                {
                                    let mut messages_to_remove = Vec::new();

                                    for (ix, message) in thread.messages.iter().enumerate().rev() {
                                        messages_to_remove.push(message.id);

                                        if message.role == Role::User {
                                            if ix == 0 {
                                                break;
                                            }

                                            if let Some(prev_message) = thread.messages.get(ix - 1) {
                                                if prev_message.role == Role::Assistant {
                                                    break;
                                                }
                                            }
                                        }
                                    }

                                    for message_id in messages_to_remove {
                                        thread.delete_message(message_id, cx);
                                    }
                                }

                                cx.emit(ThreadEvent::ShowError(ThreadError::Message {
                                    header: "Language model refusal".into(),
                                    message: "Model refused to generate content for safety reasons.".into(),
                                }));
                            }
                        },
                        Err(error) => {
                            thread.project.update(cx, |project, cx| {
                                project.set_agent_location(None, cx);
                            });

                            if error.is::<PaymentRequiredError>() {
                                cx.emit(ThreadEvent::ShowError(ThreadError::PaymentRequired));
                            } else if let Some(error) =
                                error.downcast_ref::<ModelRequestLimitReachedError>()
                            {
                                cx.emit(ThreadEvent::ShowError(
                                    ThreadError::ModelRequestLimitReached { plan: error.plan },
                                ));
                            } else if let Some(known_error) =
                                error.downcast_ref::<LanguageModelKnownError>()
                            {
                                match known_error {
                                    LanguageModelKnownError::ContextWindowLimitExceeded {
                                        tokens,
                                    } => {
                                        thread.exceeded_window_error = Some(ExceededWindowError {
                                            model_id: model.id(),
                                            token_count: *tokens,
                                        });
                                        cx.notify();
                                    }
                                }
                            } else {
                                let error_message = error
                                    .chain()
                                    .map(|err| err.to_string())
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                cx.emit(ThreadEvent::ShowError(ThreadError::Message {
                                    header: "Error interacting with language model".into(),
                                    message: SharedString::from(error_message.clone()),
                                }));
                            }

                            thread.cancel_last_completion(window, cx);
                        }
                    }

                    cx.emit(ThreadEvent::Stopped(result.map_err(Arc::new)));

                    if let Some((request_callback, (request, response_events))) = thread
                        .request_callback
                        .as_mut()
                        .zip(request_callback_parameters.as_ref())
                    {
                        request_callback(request, response_events);
                    }

                    thread.auto_capture_telemetry(cx);

                    if let Ok(initial_usage) = initial_token_usage {
                        let usage = thread.cumulative_token_usage - initial_usage;

                        telemetry::event!(
                            "Assistant Thread Completion",
                            thread_id = thread.id().to_string(),
                            prompt_id = prompt_id,
                            model = model.telemetry_id(),
                            model_provider = model.provider_id().to_string(),
                            input_tokens = usage.input_tokens,
                            output_tokens = usage.output_tokens,
                            cache_creation_input_tokens = usage.cache_creation_input_tokens,
                            cache_read_input_tokens = usage.cache_read_input_tokens,
                        );
                    }
                })
                .ok();
        });

        self.pending_completions.push(PendingCompletion {
            id: pending_completion_id,
            queue_state: QueueState::Sending,
            _task: task,
        });
    }

    pub fn summarize(&mut self, cx: &mut Context<Self>) {
        let Some(model) = LanguageModelRegistry::read_global(cx).thread_summary_model() else {
            println!("No thread summary model");
            return;
        };

        if !model.provider.is_authenticated(cx) {
            return;
        }

        let added_user_message = include_str!("./prompts/summarize_thread_prompt.txt");

        let request = self.to_summarize_request(
            &model.model,
            CompletionIntent::ThreadSummarization,
            added_user_message.into(),
            cx,
        );

        self.summary = ThreadSummary::Generating;

        self.pending_summary = cx.spawn(async move |this, cx| {
            let result = async {
                let mut messages = model.model.stream_completion(request, &cx).await?;

                let mut new_summary = String::new();
                while let Some(event) = messages.next().await {
                    let Ok(event) = event else {
                        continue;
                    };
                    let text = match event {
                        LanguageModelCompletionEvent::Text(text) => text,
                        LanguageModelCompletionEvent::StatusUpdate(
                            CompletionRequestStatus::UsageUpdated { amount, limit },
                        ) => {
                            this.update(cx, |thread, _cx| {
                                thread.last_usage = Some(RequestUsage {
                                    limit,
                                    amount: amount as i32,
                                });
                            })?;
                            continue;
                        }
                        _ => continue,
                    };

                    let mut lines = text.lines();
                    new_summary.extend(lines.next());

                    // Stop if the LLM generated multiple lines.
                    if lines.next().is_some() {
                        break;
                    }
                }

                anyhow::Ok(new_summary)
            }
            .await;

            this.update(cx, |this, cx| {
                match result {
                    Ok(new_summary) => {
                        if new_summary.is_empty() {
                            this.summary = ThreadSummary::Error;
                        } else {
                            this.summary = ThreadSummary::Ready(new_summary.into());
                        }
                    }
                    Err(err) => {
                        this.summary = ThreadSummary::Error;
                        log::error!("Failed to generate thread summary: {}", err);
                    }
                }
                cx.emit(ThreadEvent::SummaryGenerated);
            })
            .log_err()?;

            Some(())
        });
    }

    pub fn start_generating_detailed_summary_if_needed(
        &mut self,
        thread_store: WeakEntity<ThreadStore>,
        cx: &mut Context<Self>,
    ) {
        let Some(last_message_id) = self.messages.last().map(|message| message.id) else {
            return;
        };

        match &*self.detailed_summary_rx.borrow() {
            DetailedSummaryState::Generating { message_id, .. }
            | DetailedSummaryState::Generated { message_id, .. }
                if *message_id == last_message_id =>
            {
                // Already up-to-date
                return;
            }
            _ => {}
        }

        let Some(ConfiguredModel { model, provider }) =
            LanguageModelRegistry::read_global(cx).thread_summary_model()
        else {
            return;
        };

        if !provider.is_authenticated(cx) {
            return;
        }

        let added_user_message = include_str!("./prompts/summarize_thread_detailed_prompt.txt");

        let request = self.to_summarize_request(
            &model,
            CompletionIntent::ThreadContextSummarization,
            added_user_message.into(),
            cx,
        );

        *self.detailed_summary_tx.borrow_mut() = DetailedSummaryState::Generating {
            message_id: last_message_id,
        };

        // Replace the detailed summarization task if there is one, cancelling it. It would probably
        // be better to allow the old task to complete, but this would require logic for choosing
        // which result to prefer (the old task could complete after the new one, resulting in a
        // stale summary).
        self.detailed_summary_task = cx.spawn(async move |thread, cx| {
            let stream = model.stream_completion_text(request, &cx);
            let Some(mut messages) = stream.await.log_err() else {
                thread
                    .update(cx, |thread, _cx| {
                        *thread.detailed_summary_tx.borrow_mut() =
                            DetailedSummaryState::NotGenerated;
                    })
                    .ok()?;
                return None;
            };

            let mut new_detailed_summary = String::new();

            while let Some(chunk) = messages.stream.next().await {
                if let Some(chunk) = chunk.log_err() {
                    new_detailed_summary.push_str(&chunk);
                }
            }

            thread
                .update(cx, |thread, _cx| {
                    *thread.detailed_summary_tx.borrow_mut() = DetailedSummaryState::Generated {
                        text: new_detailed_summary.into(),
                        message_id: last_message_id,
                    };
                })
                .ok()?;

            // Save thread so its summary can be reused later
            if let Some(thread) = thread.upgrade() {
                if let Ok(Ok(save_task)) = cx.update(|cx| {
                    thread_store
                        .update(cx, |thread_store, cx| thread_store.save_thread(&thread, cx))
                }) {
                    save_task.await.log_err();
                }
            }

            Some(())
        });
    }

    pub async fn wait_for_detailed_summary_or_text(
        this: &Entity<Self>,
        cx: &mut AsyncApp,
    ) -> Option<SharedString> {
        let mut detailed_summary_rx = this
            .read_with(cx, |this, _cx| this.detailed_summary_rx.clone())
            .ok()?;
        loop {
            match detailed_summary_rx.recv().await? {
                DetailedSummaryState::Generating { .. } => {}
                DetailedSummaryState::NotGenerated => {
                    return this.read_with(cx, |this, _cx| this.text().into()).ok();
                }
                DetailedSummaryState::Generated { text, .. } => return Some(text),
            }
        }
    }

    pub fn latest_detailed_summary_or_text(&self) -> SharedString {
        self.detailed_summary_rx
            .borrow()
            .text()
            .unwrap_or_else(|| self.text().into())
    }

    pub fn is_generating_detailed_summary(&self) -> bool {
        matches!(
            &*self.detailed_summary_rx.borrow(),
            DetailedSummaryState::Generating { .. }
        )
    }

    pub fn use_pending_tools(
        &mut self,
        window: Option<AnyWindowHandle>,
        cx: &mut Context<Self>,
        model: Arc<dyn LanguageModel>,
    ) -> Vec<PendingToolUse> {
        self.auto_capture_telemetry(cx);
        let request =
            Arc::new(self.to_completion_request(model.clone(), CompletionIntent::ToolResults, cx));
        let pending_tool_uses = self
            .tool_use
            .pending_tool_uses()
            .into_iter()
            .filter(|tool_use| tool_use.status.is_idle())
            .cloned()
            .collect::<Vec<_>>();

        for tool_use in pending_tool_uses.iter() {
            if let Some(tool) = self.tools.read(cx).tool(&tool_use.name, cx) {
                if tool.needs_confirmation(&tool_use.input, cx)
                    && !AgentSettings::get_global(cx).always_allow_tool_actions
                {
                    self.tool_use.confirm_tool_use(
                        tool_use.id.clone(),
                        tool_use.ui_text.clone(),
                        tool_use.input.clone(),
                        request.clone(),
                        tool,
                    );
                    cx.emit(ThreadEvent::ToolConfirmationNeeded);
                } else {
                    self.run_tool(
                        tool_use.id.clone(),
                        tool_use.ui_text.clone(),
                        tool_use.input.clone(),
                        request.clone(),
                        tool,
                        model.clone(),
                        window,
                        cx,
                    );
                }
            } else {
                self.handle_hallucinated_tool_use(
                    tool_use.id.clone(),
                    tool_use.name.clone(),
                    window,
                    cx,
                );
            }
        }

        pending_tool_uses
    }

    pub fn handle_hallucinated_tool_use(
        &mut self,
        tool_use_id: LanguageModelToolUseId,
        hallucinated_tool_name: Arc<str>,
        window: Option<AnyWindowHandle>,
        cx: &mut Context<Thread>,
    ) {
        let available_tools = self.tools.read(cx).enabled_tools(cx);

        let tool_list = available_tools
            .iter()
            .map(|tool| format!("- {}: {}", tool.name(), tool.description()))
            .collect::<Vec<_>>()
            .join("\n");

        let error_message = format!(
            "The tool '{}' doesn't exist or is not enabled. Available tools:\n{}",
            hallucinated_tool_name, tool_list
        );

        let pending_tool_use = self.tool_use.insert_tool_output(
            tool_use_id.clone(),
            hallucinated_tool_name,
            Err(anyhow!("Missing tool call: {error_message}")),
            self.configured_model.as_ref(),
        );

        cx.emit(ThreadEvent::MissingToolUse {
            tool_use_id: tool_use_id.clone(),
            ui_text: error_message.into(),
        });

        self.tool_finished(tool_use_id, pending_tool_use, false, window, cx);
    }

    pub fn receive_invalid_tool_json(
        &mut self,
        tool_use_id: LanguageModelToolUseId,
        tool_name: Arc<str>,
        invalid_json: Arc<str>,
        error: String,
        window: Option<AnyWindowHandle>,
        cx: &mut Context<Thread>,
    ) {
        log::error!("The model returned invalid input JSON: {invalid_json}");

        let pending_tool_use = self.tool_use.insert_tool_output(
            tool_use_id.clone(),
            tool_name,
            Err(anyhow!("Error parsing input JSON: {error}")),
            self.configured_model.as_ref(),
        );
        let ui_text = if let Some(pending_tool_use) = &pending_tool_use {
            pending_tool_use.ui_text.clone()
        } else {
            log::error!(
                "There was no pending tool use for tool use {tool_use_id}, even though it finished (with invalid input JSON)."
            );
            format!("Unknown tool {}", tool_use_id).into()
        };

        cx.emit(ThreadEvent::InvalidToolInput {
            tool_use_id: tool_use_id.clone(),
            ui_text,
            invalid_input_json: invalid_json,
        });

        self.tool_finished(tool_use_id, pending_tool_use, false, window, cx);
    }

    pub fn run_tool(
        &mut self,
        tool_use_id: LanguageModelToolUseId,
        ui_text: impl Into<SharedString>,
        input: serde_json::Value,
        request: Arc<LanguageModelRequest>,
        tool: Arc<dyn Tool>,
        model: Arc<dyn LanguageModel>,
        window: Option<AnyWindowHandle>,
        cx: &mut Context<Thread>,
    ) {
        let task =
            self.spawn_tool_use(tool_use_id.clone(), request, input, tool, model, window, cx);
        self.tool_use
            .run_pending_tool(tool_use_id, ui_text.into(), task);
    }

    fn spawn_tool_use(
        &mut self,
        tool_use_id: LanguageModelToolUseId,
        request: Arc<LanguageModelRequest>,
        input: serde_json::Value,
        tool: Arc<dyn Tool>,
        model: Arc<dyn LanguageModel>,
        window: Option<AnyWindowHandle>,
        cx: &mut Context<Thread>,
    ) -> Task<()> {
        let tool_name: Arc<str> = tool.name().into();

        let tool_result = if self.tools.read(cx).is_disabled(&tool.source(), &tool_name) {
            Task::ready(Err(anyhow!("tool is disabled: {tool_name}"))).into()
        } else {
            tool.run(
                input,
                request,
                self.project.clone(),
                self.action_log.clone(),
                model,
                window,
                cx,
            )
        };

        // Store the card separately if it exists
        if let Some(card) = tool_result.card.clone() {
            self.tool_use
                .insert_tool_result_card(tool_use_id.clone(), card);
        }

        cx.spawn({
            async move |thread: WeakEntity<Thread>, cx| {
                let output = tool_result.output.await;

                thread
                    .update(cx, |thread, cx| {
                        let pending_tool_use = thread.tool_use.insert_tool_output(
                            tool_use_id.clone(),
                            tool_name,
                            output,
                            thread.configured_model.as_ref(),
                        );
                        thread.tool_finished(tool_use_id, pending_tool_use, false, window, cx);
                    })
                    .ok();
            }
        })
    }

    fn tool_finished(
        &mut self,
        tool_use_id: LanguageModelToolUseId,
        pending_tool_use: Option<PendingToolUse>,
        canceled: bool,
        window: Option<AnyWindowHandle>,
        cx: &mut Context<Self>,
    ) {
        if self.all_tools_finished() {
            if let Some(ConfiguredModel { model, .. }) = self.configured_model.as_ref() {
                if !canceled {
                    self.send_to_model(model.clone(), CompletionIntent::ToolResults, window, cx);
                }
                self.auto_capture_telemetry(cx);
            }
        }

        cx.emit(ThreadEvent::ToolFinished {
            tool_use_id,
            pending_tool_use,
        });
    }

    /// Cancels the last pending completion, if there are any pending.
    ///
    /// Returns whether a completion was canceled.
    pub fn cancel_last_completion(
        &mut self,
        window: Option<AnyWindowHandle>,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut canceled = self.pending_completions.pop().is_some();

        for pending_tool_use in self.tool_use.cancel_pending() {
            canceled = true;
            self.tool_finished(
                pending_tool_use.id.clone(),
                Some(pending_tool_use),
                true,
                window,
                cx,
            );
        }

        if canceled {
            cx.emit(ThreadEvent::CompletionCanceled);

            // When canceled, we always want to insert the checkpoint.
            // (We skip over finalize_pending_checkpoint, because it
            // would conclude we didn't have anything to insert here.)
            if let Some(checkpoint) = self.pending_checkpoint.take() {
                self.insert_checkpoint(checkpoint, cx);
            }
        } else {
            self.finalize_pending_checkpoint(cx);
        }

        canceled
    }

    /// Signals that any in-progress editing should be canceled.
    ///
    /// This method is used to notify listeners (like ActiveThread) that
    /// they should cancel any editing operations.
    pub fn cancel_editing(&mut self, cx: &mut Context<Self>) {
        cx.emit(ThreadEvent::CancelEditing);
    }

    pub fn feedback(&self) -> Option<ThreadFeedback> {
        self.feedback
    }

    pub fn message_feedback(&self, message_id: MessageId) -> Option<ThreadFeedback> {
        self.message_feedback.get(&message_id).copied()
    }

    pub fn report_message_feedback(
        &mut self,
        message_id: MessageId,
        feedback: ThreadFeedback,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        if self.message_feedback.get(&message_id) == Some(&feedback) {
            return Task::ready(Ok(()));
        }

        let final_project_snapshot = Self::project_snapshot(self.project.clone(), cx);
        let serialized_thread = self.serialize(cx);
        let thread_id = self.id().clone();
        let client = self.project.read(cx).client();

        let enabled_tool_names: Vec<String> = self
            .tools()
            .read(cx)
            .enabled_tools(cx)
            .iter()
            .map(|tool| tool.name())
            .collect();

        self.message_feedback.insert(message_id, feedback);

        cx.notify();

        let message_content = self
            .message(message_id)
            .map(|msg| msg.to_string())
            .unwrap_or_default();

        cx.background_spawn(async move {
            let final_project_snapshot = final_project_snapshot.await;
            let serialized_thread = serialized_thread.await?;
            let thread_data =
                serde_json::to_value(serialized_thread).unwrap_or_else(|_| serde_json::Value::Null);

            let rating = match feedback {
                ThreadFeedback::Positive => "positive",
                ThreadFeedback::Negative => "negative",
            };
            telemetry::event!(
                "Assistant Thread Rated",
                rating,
                thread_id,
                enabled_tool_names,
                message_id = message_id.0,
                message_content,
                thread_data,
                final_project_snapshot
            );
            client.telemetry().flush_events().await;

            Ok(())
        })
    }

    pub fn report_feedback(
        &mut self,
        feedback: ThreadFeedback,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        let last_assistant_message_id = self
            .messages
            .iter()
            .rev()
            .find(|msg| msg.role == Role::Assistant)
            .map(|msg| msg.id);

        if let Some(message_id) = last_assistant_message_id {
            self.report_message_feedback(message_id, feedback, cx)
        } else {
            let final_project_snapshot = Self::project_snapshot(self.project.clone(), cx);
            let serialized_thread = self.serialize(cx);
            let thread_id = self.id().clone();
            let client = self.project.read(cx).client();
            self.feedback = Some(feedback);
            cx.notify();

            cx.background_spawn(async move {
                let final_project_snapshot = final_project_snapshot.await;
                let serialized_thread = serialized_thread.await?;
                let thread_data = serde_json::to_value(serialized_thread)
                    .unwrap_or_else(|_| serde_json::Value::Null);

                let rating = match feedback {
                    ThreadFeedback::Positive => "positive",
                    ThreadFeedback::Negative => "negative",
                };
                telemetry::event!(
                    "Assistant Thread Rated",
                    rating,
                    thread_id,
                    thread_data,
                    final_project_snapshot
                );
                client.telemetry().flush_events().await;

                Ok(())
            })
        }
    }

    /// Create a snapshot of the current project state including git information and unsaved buffers.
    fn project_snapshot(
        project: Entity<Project>,
        cx: &mut Context<Self>,
    ) -> Task<Arc<ProjectSnapshot>> {
        let git_store = project.read(cx).git_store().clone();
        let worktree_snapshots: Vec<_> = project
            .read(cx)
            .visible_worktrees(cx)
            .map(|worktree| Self::worktree_snapshot(worktree, git_store.clone(), cx))
            .collect();

        cx.spawn(async move |_, cx| {
            let worktree_snapshots = futures::future::join_all(worktree_snapshots).await;

            let mut unsaved_buffers = Vec::new();
            cx.update(|app_cx| {
                let buffer_store = project.read(app_cx).buffer_store();
                for buffer_handle in buffer_store.read(app_cx).buffers() {
                    let buffer = buffer_handle.read(app_cx);
                    if buffer.is_dirty() {
                        if let Some(file) = buffer.file() {
                            let path = file.path().to_string_lossy().to_string();
                            unsaved_buffers.push(path);
                        }
                    }
                }
            })
            .ok();

            Arc::new(ProjectSnapshot {
                worktree_snapshots,
                unsaved_buffer_paths: unsaved_buffers,
                timestamp: Utc::now(),
            })
        })
    }

    fn worktree_snapshot(
        worktree: Entity<project::Worktree>,
        git_store: Entity<GitStore>,
        cx: &App,
    ) -> Task<WorktreeSnapshot> {
        cx.spawn(async move |cx| {
            // Get worktree path and snapshot
            let worktree_info = cx.update(|app_cx| {
                let worktree = worktree.read(app_cx);
                let path = worktree.abs_path().to_string_lossy().to_string();
                let snapshot = worktree.snapshot();
                (path, snapshot)
            });

            let Ok((worktree_path, _snapshot)) = worktree_info else {
                return WorktreeSnapshot {
                    worktree_path: String::new(),
                    git_state: None,
                };
            };

            let git_state = git_store
                .update(cx, |git_store, cx| {
                    git_store
                        .repositories()
                        .values()
                        .find(|repo| {
                            repo.read(cx)
                                .abs_path_to_repo_path(&worktree.read(cx).abs_path())
                                .is_some()
                        })
                        .cloned()
                })
                .ok()
                .flatten()
                .map(|repo| {
                    repo.update(cx, |repo, _| {
                        let current_branch =
                            repo.branch.as_ref().map(|branch| branch.name().to_owned());
                        repo.send_job(None, |state, _| async move {
                            let RepositoryState::Local { backend, .. } = state else {
                                return GitState {
                                    remote_url: None,
                                    head_sha: None,
                                    current_branch,
                                    diff: None,
                                };
                            };

                            let remote_url = backend.remote_url("origin");
                            let head_sha = backend.head_sha().await;
                            let diff = backend.diff(DiffType::HeadToWorktree).await.ok();

                            GitState {
                                remote_url,
                                head_sha,
                                current_branch,
                                diff,
                            }
                        })
                    })
                });

            let git_state = match git_state {
                Some(git_state) => match git_state.ok() {
                    Some(git_state) => git_state.await.ok(),
                    None => None,
                },
                None => None,
            };

            WorktreeSnapshot {
                worktree_path,
                git_state,
            }
        })
    }

    pub fn to_markdown(&self, cx: &App) -> Result<String> {
        let mut markdown = Vec::new();

        let summary = self.summary().or_default();
        writeln!(markdown, "# {summary}\n")?;

        for message in self.messages() {
            writeln!(
                markdown,
                "## {role}\n",
                role = match message.role {
                    Role::User => "User",
                    Role::Assistant => "Agent",
                    Role::System => "System",
                }
            )?;

            if !message.loaded_context.text.is_empty() {
                writeln!(markdown, "{}", message.loaded_context.text)?;
            }

            if !message.loaded_context.images.is_empty() {
                writeln!(
                    markdown,
                    "\n{} images attached as context.\n",
                    message.loaded_context.images.len()
                )?;
            }

            for segment in &message.segments {
                match segment {
                    MessageSegment::Text(text) => writeln!(markdown, "{}\n", text)?,
                    MessageSegment::Thinking { text, .. } => {
                        writeln!(markdown, "<think>\n{}\n</think>\n", text)?
                    }
                    MessageSegment::RedactedThinking(_) => {}
                }
            }

            for tool_use in self.tool_uses_for_message(message.id, cx) {
                writeln!(
                    markdown,
                    "**Use Tool: {} ({})**",
                    tool_use.name, tool_use.id
                )?;
                writeln!(markdown, "```json")?;
                writeln!(
                    markdown,
                    "{}",
                    serde_json::to_string_pretty(&tool_use.input)?
                )?;
                writeln!(markdown, "```")?;
            }

            for tool_result in self.tool_results_for_message(message.id) {
                write!(markdown, "\n**Tool Results: {}", tool_result.tool_use_id)?;
                if tool_result.is_error {
                    write!(markdown, " (Error)")?;
                }

                writeln!(markdown, "**\n")?;
                match &tool_result.content {
                    LanguageModelToolResultContent::Text(text) => {
                        writeln!(markdown, "{text}")?;
                    }
                    LanguageModelToolResultContent::Image(image) => {
                        writeln!(markdown, "![Image](data:base64,{})", image.source)?;
                    }
                }

                if let Some(output) = tool_result.output.as_ref() {
                    writeln!(
                        markdown,
                        "\n\nDebug Output:\n\n```json\n{}\n```\n",
                        serde_json::to_string_pretty(output)?
                    )?;
                }
            }
        }

        Ok(String::from_utf8_lossy(&markdown).to_string())
    }

    pub fn keep_edits_in_range(
        &mut self,
        buffer: Entity<language::Buffer>,
        buffer_range: Range<language::Anchor>,
        cx: &mut Context<Self>,
    ) {
        self.action_log.update(cx, |action_log, cx| {
            action_log.keep_edits_in_range(buffer, buffer_range, cx)
        });
    }

    pub fn keep_all_edits(&mut self, cx: &mut Context<Self>) {
        self.action_log
            .update(cx, |action_log, cx| action_log.keep_all_edits(cx));
    }

    pub fn reject_edits_in_ranges(
        &mut self,
        buffer: Entity<language::Buffer>,
        buffer_ranges: Vec<Range<language::Anchor>>,
        cx: &mut Context<Self>,
    ) -> Task<Result<()>> {
        self.action_log.update(cx, |action_log, cx| {
            action_log.reject_edits_in_ranges(buffer, buffer_ranges, cx)
        })
    }

    pub fn action_log(&self) -> &Entity<ActionLog> {
        &self.action_log
    }

    pub fn project(&self) -> &Entity<Project> {
        &self.project
    }

    pub fn auto_capture_telemetry(&mut self, cx: &mut Context<Self>) {
        if !cx.has_flag::<feature_flags::ThreadAutoCaptureFeatureFlag>() {
            return;
        }

        let now = Instant::now();
        if let Some(last) = self.last_auto_capture_at {
            if now.duration_since(last).as_secs() < 10 {
                return;
            }
        }

        self.last_auto_capture_at = Some(now);

        let thread_id = self.id().clone();
        let github_login = self
            .project
            .read(cx)
            .user_store()
            .read(cx)
            .current_user()
            .map(|user| user.github_login.clone());
        let client = self.project.read(cx).client();
        let serialize_task = self.serialize(cx);

        cx.background_executor()
            .spawn(async move {
                if let Ok(serialized_thread) = serialize_task.await {
                    if let Ok(thread_data) = serde_json::to_value(serialized_thread) {
                        telemetry::event!(
                            "Agent Thread Auto-Captured",
                            thread_id = thread_id.to_string(),
                            thread_data = thread_data,
                            auto_capture_reason = "tracked_user",
                            github_login = github_login
                        );

                        client.telemetry().flush_events().await;
                    }
                }
            })
            .detach();
    }

    pub fn cumulative_token_usage(&self) -> TokenUsage {
        self.cumulative_token_usage
    }

    pub fn token_usage_up_to_message(&self, message_id: MessageId) -> TotalTokenUsage {
        let Some(model) = self.configured_model.as_ref() else {
            return TotalTokenUsage::default();
        };

        let max = model.model.max_token_count();

        let index = self
            .messages
            .iter()
            .position(|msg| msg.id == message_id)
            .unwrap_or(0);

        if index == 0 {
            return TotalTokenUsage { total: 0, max };
        }

        // For partial token usage, we need to calculate proportionally from cumulative usage
        // or fall back to per-request usage if available
        let total = if self.cumulative_token_usage.total_tokens() > 0 {
            // If we have cumulative usage, calculate proportionally based on message position
            let total_messages = self.messages.len();
            if total_messages > 0 {
                let proportion = index as f32 / total_messages as f32;
                (self.cumulative_token_usage.total_tokens() as f32 * proportion) as usize
            } else {
                0
            }
        } else if let Some(usage) = self.request_token_usage.get(index - 1) {
            // Use actual token usage from specific request if available
            usage.total_tokens() as usize
        } else {
            // If no actual usage data, estimate tokens from messages up to this point
            // plus system prompt and tools overhead
            let mut total_tokens = 0;
            
            // System prompt and tools overhead
            total_tokens += 1000 + 500; // System prompt + tools estimate
            
            // Messages up to this point
            total_tokens += self.messages.iter()
                .take(index)
                .map(|msg| {
                    let mut msg_tokens = self.estimate_message_tokens(msg);
                    // Add rough estimate for tool uses/results
                    if msg.role == Role::Assistant {
                        msg_tokens += 200; // Rough estimate for potential tool use
                    }
                    msg_tokens
                })
                .sum::<usize>();
                
            total_tokens
        };

        TotalTokenUsage { total, max }
    }

    pub fn total_token_usage(&self) -> Option<TotalTokenUsage> {
        let model = self.configured_model.as_ref()?;

        let max = model.model.max_token_count();

        if let Some(exceeded_error) = &self.exceeded_window_error {
            if model.model.id() == exceeded_error.model_id {
                return Some(TotalTokenUsage {
                    total: exceeded_error.token_count,
                    max,
                });
            }
        }

        // Use cumulative token usage which tracks the total across all requests
        let total = if self.cumulative_token_usage.total_tokens() > 0 {
            // Use actual cumulative token usage from all completed requests
            self.cumulative_token_usage.total_tokens() as usize
        } else if !self.messages.is_empty() {
            // If no actual usage data but we have messages, use comprehensive estimation
            // This accounts for system prompts, tool uses, tool results, images, and stale files
            let mut estimated_tokens = 0;
            
            // Estimate system prompt (roughly 500-2000 tokens)
            estimated_tokens += 1000;
            
            // Estimate all messages with their content
            for message in &self.messages {
                estimated_tokens += self.estimate_message_tokens(message);
                
                // Add rough estimates for tool uses and results
                // This is a simplified approach without needing App context
                if message.role == Role::Assistant {
                    // Assistant messages often have tool uses
                    estimated_tokens += 200; // Rough estimate for potential tool use
                }
            }
            
            // Estimate available tools overhead (roughly 100-500 tokens per tool)
            estimated_tokens += 500; // Conservative estimate for tools
            
            estimated_tokens
        } else {
            // No messages yet, return 0
            0
        };

        Some(TotalTokenUsage { total, max })
    }

    fn token_usage_at_last_message(&self) -> Option<TokenUsage> {
        self.request_token_usage
            .get(self.messages.len().saturating_sub(1))
            .or_else(|| self.request_token_usage.last())
            .cloned()
    }

    fn update_token_usage_at_last_message(&mut self, token_usage: TokenUsage) {
        let placeholder = self.token_usage_at_last_message().unwrap_or_default();
        self.request_token_usage
            .resize(self.messages.len(), placeholder);

        if let Some(last) = self.request_token_usage.last_mut() {
            *last = token_usage;
        }
    }

    /// Get context optimization metrics for the current thread
    pub fn get_context_optimization_metrics(&self, model: &Arc<dyn LanguageModel>, cx: &App) -> OptimizedContext {
        self.optimize_context_for_request(model, cx)
    }

    /// Get detailed analytics about context optimization performance
    pub fn get_optimization_analytics(&self, model: &Arc<dyn LanguageModel>, cx: &App) -> ContextOptimizationAnalytics {
        let metrics = self.optimize_context_for_request(model, cx);
        
        ContextOptimizationAnalytics {
            current_strategy: metrics.strategy_used.clone(),
            efficiency_score: self.calculate_efficiency_score(&metrics),
            memory_pressure: self.calculate_memory_pressure(),
            optimization_frequency: self.calculate_optimization_frequency(),
            performance_metrics: metrics.optimization_metrics.clone(),
            recommendations: self.generate_optimization_recommendations(&metrics),
        }
    }

    fn calculate_efficiency_score(&self, metrics: &OptimizedContext) -> f32 {
        // Score based on memory savings vs context preservation
        let memory_component = metrics.memory_savings * 0.4;
        let preservation_component = metrics.context_preservation * 0.6;
        memory_component + preservation_component
    }

    fn calculate_memory_pressure(&self) -> MemoryPressure {
        let usage = self.total_token_usage().unwrap_or_default();
        let ratio = usage.ratio();
        
        match ratio {
            TokenUsageRatio::Normal => MemoryPressure::Low,
            TokenUsageRatio::Warning => MemoryPressure::Medium,
            TokenUsageRatio::Exceeded => MemoryPressure::High,
        }
    }

    fn calculate_optimization_frequency(&self) -> OptimizationFrequency {
        // Based on message count - more messages = more frequent optimization
        match self.messages.len() {
            0..=5 => OptimizationFrequency::Rare,
            6..=15 => OptimizationFrequency::Occasional,
            16..=30 => OptimizationFrequency::Frequent,
            _ => OptimizationFrequency::Constant,
        }
    }

    fn generate_optimization_recommendations(&self, metrics: &OptimizedContext) -> Vec<OptimizationRecommendation> {
        let mut recommendations = Vec::new();
        
        // Check if we're using too much memory
        if metrics.memory_savings < 0.1 && self.messages.len() > 10 {
            recommendations.push(OptimizationRecommendation::IncreaseCompression);
        }
        
        // Check if context preservation is too low
        if metrics.context_preservation < 0.7 {
            recommendations.push(OptimizationRecommendation::PreserveMoreContext);
        }
        
        // Check if we should switch strategies
        if matches!(metrics.strategy_used, ContextStrategy::Full) && self.messages.len() > 20 {
            recommendations.push(OptimizationRecommendation::EnableSmartCompression);
        }
        
        // Performance recommendations
        if metrics.optimization_metrics.optimization_time_ms > 50.0 {
            recommendations.push(OptimizationRecommendation::OptimizePerformance);
        }
        
        recommendations
    }

    pub fn deny_tool_use(
        &mut self,
        tool_use_id: LanguageModelToolUseId,
        tool_name: Arc<str>,
        window: Option<AnyWindowHandle>,
        cx: &mut Context<Self>,
    ) {
        let err = Err(anyhow::anyhow!(
            "Permission to run tool action denied by user"
        ));

        self.tool_use.insert_tool_output(
            tool_use_id.clone(),
            tool_name,
            err,
            self.configured_model.as_ref(),
        );
        self.tool_finished(tool_use_id.clone(), None, true, window, cx);
    }

    fn calculate_context_zones(&self, messages: &[Message]) -> ContextZoneBreakdown {
        let mut recent_zone_messages = 0;
        let mut compressed_zone_messages = 0;
        let mut dropped_zone_messages = 0;
        let mut recent_zone_tokens = 0;
        let mut compressed_zone_tokens = 0;
        let mut dropped_zone_tokens = 0;

        for message in messages {
            if message.role == Role::Assistant {
                recent_zone_messages += 1;
                recent_zone_tokens += self.estimate_message_tokens(message);
            } else if message.segments.iter().any(|segment| !segment.should_display()) {
                compressed_zone_messages += 1;
                compressed_zone_tokens += self.estimate_message_tokens(message);
            } else {
                dropped_zone_messages += 1;
                dropped_zone_tokens += self.estimate_message_tokens(message);
            }
        }

        ContextZoneBreakdown {
            recent_zone_messages,
            compressed_zone_messages,
            dropped_zone_messages,
            recent_zone_tokens,
            compressed_zone_tokens,
            dropped_zone_tokens,
        }
    }
}

#[derive(Debug, Clone, Error)]
pub enum ThreadError {
    #[error("Payment required")]
    PaymentRequired,
    #[error("Model request limit reached")]
    ModelRequestLimitReached { plan: Plan },
    #[error("Message {header}: {message}")]
    Message {
        header: SharedString,
        message: SharedString,
    },
}

#[derive(Debug, Clone)]
pub enum ThreadEvent {
    ShowError(ThreadError),
    StreamedCompletion,
    ReceivedTextChunk,
    NewRequest,
    StreamedAssistantText(MessageId, String),
    StreamedAssistantThinking(MessageId, String),
    StreamedToolUse {
        tool_use_id: LanguageModelToolUseId,
        ui_text: Arc<str>,
        input: serde_json::Value,
    },
    MissingToolUse {
        tool_use_id: LanguageModelToolUseId,
        ui_text: Arc<str>,
    },
    InvalidToolInput {
        tool_use_id: LanguageModelToolUseId,
        ui_text: Arc<str>,
        invalid_input_json: Arc<str>,
    },
    Stopped(Result<StopReason, Arc<anyhow::Error>>),
    MessageAdded(MessageId),
    MessageEdited(MessageId),
    MessageDeleted(MessageId),
    SummaryGenerated,
    SummaryChanged,
    UsePendingTools {
        tool_uses: Vec<PendingToolUse>,
    },
    ToolFinished {
        #[allow(unused)]
        tool_use_id: LanguageModelToolUseId,
        /// The pending tool use that corresponds to this tool.
        pending_tool_use: Option<PendingToolUse>,
    },
    CheckpointChanged,
    ToolConfirmationNeeded,
    CancelEditing,
    CompletionCanceled,
}

impl EventEmitter<ThreadEvent> for Thread {}

struct PendingCompletion {
    id: usize,
    queue_state: QueueState,
    _task: Task<()>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ThreadStore, context::load_context, context_store::ContextStore, thread_store};
    use agent_settings::{AgentSettings, LanguageModelParameters};
    use assistant_tool::ToolRegistry;
    use editor::EditorSettings;
    use gpui::TestAppContext;
    use language_model::fake_provider::{FakeLanguageModel, FakeLanguageModelProvider};
    use project::{FakeFs, Project};
    use prompt_store::PromptBuilder;
    use serde_json::json;
    use settings::{Settings, SettingsStore};
    use std::sync::Arc;
    use theme::ThemeSettings;
    use util::path;
    use workspace::Workspace;

    #[gpui::test]
    async fn test_message_with_context(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(
            cx,
            json!({"code.rs": "fn main() {\n    println!(\"Hello, world!\");\n}"}),
        )
        .await;

        let (_workspace, _thread_store, thread, context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        add_file_to_context(&project, &context_store, "test/code.rs", cx)
            .await
            .unwrap();

        let context =
            context_store.read_with(cx, |store, _| store.context().next().cloned().unwrap());
        let loaded_context = cx
            .update(|cx| load_context(vec![context], &project, &None, cx))
            .await;

        // Insert user message with context
        let message_id = thread.update(cx, |thread, cx| {
            thread.insert_user_message(
                "Please explain this code",
                loaded_context,
                None,
                Vec::new(),
                cx,
            )
        });

        // Check content and context in message object
        let message = thread.read_with(cx, |thread, _| thread.message(message_id).unwrap().clone());

        // Use different path format strings based on platform for the test
        #[cfg(windows)]
        let path_part = r"test\code.rs";
        #[cfg(not(windows))]
        let path_part = "test/code.rs";

        let expected_context = format!(
            r#"
<context>
The following items were attached by the user. They are up-to-date and don't need to be re-read.

<files>
```rs {path_part}
fn main() {{
    println!("Hello, world!");
}}
```
</files>
</context>
"#
        );

        assert_eq!(message.role, Role::User);
        assert_eq!(message.segments.len(), 1);
        assert_eq!(
            message.segments[0],
            MessageSegment::Text("Please explain this code".to_string())
        );
        assert_eq!(message.loaded_context.text, expected_context);

        // Check message in request
        let request = thread.update(cx, |thread, cx| {
            thread.to_completion_request(model.clone(), CompletionIntent::UserPrompt, cx)
        });

        assert_eq!(request.messages.len(), 2);
        let expected_full_message = format!("{}Please explain this code", expected_context);
        assert_eq!(request.messages[1].string_contents(), expected_full_message);
    }

    #[gpui::test]
    async fn test_only_include_new_contexts(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(
            cx,
            json!({
                "file1.rs": "fn function1() {}\n",
                "file2.rs": "fn function2() {}\n",
                "file3.rs": "fn function3() {}\n",
                "file4.rs": "fn function4() {}\n",
            }),
        )
        .await;

        let (_, _thread_store, thread, context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // First message with context 1
        add_file_to_context(&project, &context_store, "test/file1.rs", cx)
            .await
            .unwrap();
        let new_contexts = context_store.update(cx, |store, cx| {
            store.new_context_for_thread(thread.read(cx), None)
        });
        assert_eq!(new_contexts.len(), 1);
        let loaded_context = cx
            .update(|cx| load_context(new_contexts, &project, &None, cx))
            .await;
        let message1_id = thread.update(cx, |thread, cx| {
            thread.insert_user_message("Message 1", loaded_context, None, Vec::new(), cx)
        });

        // Second message with contexts 1 and 2 (context 1 should be skipped as it's already included)
        add_file_to_context(&project, &context_store, "test/file2.rs", cx)
            .await
            .unwrap();
        let new_contexts = context_store.update(cx, |store, cx| {
            store.new_context_for_thread(thread.read(cx), None)
        });
        assert_eq!(new_contexts.len(), 1);
        let loaded_context = cx
            .update(|cx| load_context(new_contexts, &project, &None, cx))
            .await;
        let message2_id = thread.update(cx, |thread, cx| {
            thread.insert_user_message("Message 2", loaded_context, None, Vec::new(), cx)
        });

        // Third message with all three contexts (contexts 1 and 2 should be skipped)
        //
        add_file_to_context(&project, &context_store, "test/file3.rs", cx)
            .await
            .unwrap();
        let new_contexts = context_store.update(cx, |store, cx| {
            store.new_context_for_thread(thread.read(cx), None)
        });
        assert_eq!(new_contexts.len(), 1);
        let loaded_context = cx
            .update(|cx| load_context(new_contexts, &project, &None, cx))
            .await;
        let message3_id = thread.update(cx, |thread, cx| {
            thread.insert_user_message("Message 3", loaded_context, None, Vec::new(), cx)
        });

        // Check what contexts are included in each message
        let (message1, message2, message3) = thread.read_with(cx, |thread, _| {
            (
                thread.message(message1_id).unwrap().clone(),
                thread.message(message2_id).unwrap().clone(),
                thread.message(message3_id).unwrap().clone(),
            )
        });

        // First message should include context 1
        assert!(message1.loaded_context.text.contains("file1.rs"));

        // Second message should include only context 2 (not 1)
        assert!(!message2.loaded_context.text.contains("file1.rs"));
        assert!(message2.loaded_context.text.contains("file2.rs"));

        // Third message should include only context 3 (not 1 or 2)
        assert!(!message3.loaded_context.text.contains("file1.rs"));
        assert!(!message3.loaded_context.text.contains("file2.rs"));
        assert!(message3.loaded_context.text.contains("file3.rs"));

        // Check entire request to make sure all contexts are properly included
        let request = thread.update(cx, |thread, cx| {
            thread.to_completion_request(model.clone(), CompletionIntent::UserPrompt, cx)
        });

        // The request should contain all 3 messages
        assert_eq!(request.messages.len(), 4);

        // Check that the contexts are properly formatted in each message
        assert!(request.messages[1].string_contents().contains("file1.rs"));
        assert!(!request.messages[1].string_contents().contains("file2.rs"));
        assert!(!request.messages[1].string_contents().contains("file3.rs"));

        assert!(!request.messages[2].string_contents().contains("file1.rs"));
        assert!(request.messages[2].string_contents().contains("file2.rs"));
        assert!(!request.messages[2].string_contents().contains("file3.rs"));

        assert!(!request.messages[3].string_contents().contains("file1.rs"));
        assert!(!request.messages[3].string_contents().contains("file2.rs"));
        assert!(request.messages[3].string_contents().contains("file3.rs"));

        add_file_to_context(&project, &context_store, "test/file4.rs", cx)
            .await
            .unwrap();
        let new_contexts = context_store.update(cx, |store, cx| {
            store.new_context_for_thread(thread.read(cx), Some(message2_id))
        });
        assert_eq!(new_contexts.len(), 3);
        let loaded_context = cx
            .update(|cx| load_context(new_contexts, &project, &None, cx))
            .await
            .loaded_context;

        assert!(!loaded_context.text.contains("file1.rs"));
        assert!(loaded_context.text.contains("file2.rs"));
        assert!(loaded_context.text.contains("file3.rs"));
        assert!(loaded_context.text.contains("file4.rs"));

        let new_contexts = context_store.update(cx, |store, cx| {
            // Remove file4.rs
            store.remove_context(&loaded_context.contexts[2].handle(), cx);
            store.new_context_for_thread(thread.read(cx), Some(message2_id))
        });
        assert_eq!(new_contexts.len(), 2);
        let loaded_context = cx
            .update(|cx| load_context(new_contexts, &project, &None, cx))
            .await
            .loaded_context;

        assert!(!loaded_context.text.contains("file1.rs"));
        assert!(loaded_context.text.contains("file2.rs"));
        assert!(loaded_context.text.contains("file3.rs"));
        assert!(!loaded_context.text.contains("file4.rs"));

        let new_contexts = context_store.update(cx, |store, cx| {
            // Remove file3.rs
            store.remove_context(&loaded_context.contexts[1].handle(), cx);
            store.new_context_for_thread(thread.read(cx), Some(message2_id))
        });
        assert_eq!(new_contexts.len(), 1);
        let loaded_context = cx
            .update(|cx| load_context(new_contexts, &project, &None, cx))
            .await
            .loaded_context;

        assert!(!loaded_context.text.contains("file1.rs"));
        assert!(loaded_context.text.contains("file2.rs"));
        assert!(!loaded_context.text.contains("file3.rs"));
        assert!(!loaded_context.text.contains("file4.rs"));
    }

    #[gpui::test]
    async fn test_message_without_files(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(
            cx,
            json!({"code.rs": "fn main() {\n    println!(\"Hello, world!\");\n}"}),
        )
        .await;

        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Insert user message without any context (empty context vector)
        let message_id = thread.update(cx, |thread, cx| {
            thread.insert_user_message(
                "What is the best way to learn Rust?",
                ContextLoadResult::default(),
                None,
                Vec::new(),
                cx,
            )
        });

        // Check content and context in message object
        let message = thread.read_with(cx, |thread, _| thread.message(message_id).unwrap().clone());

        // Context should be empty when no files are included
        assert_eq!(message.role, Role::User);
        assert_eq!(message.segments.len(), 1);
        assert_eq!(
            message.segments[0],
            MessageSegment::Text("What is the best way to learn Rust?".to_string())
        );
        assert_eq!(message.loaded_context.text, "");

        // Check message in request
        let request = thread.update(cx, |thread, cx| {
            thread.to_completion_request(model.clone(), CompletionIntent::UserPrompt, cx)
        });

        assert_eq!(request.messages.len(), 2);
        assert_eq!(
            request.messages[1].string_contents(),
            "What is the best way to learn Rust?"
        );

        // Add second message, also without context
        let message2_id = thread.update(cx, |thread, cx| {
            thread.insert_user_message(
                "Are there any good books?",
                ContextLoadResult::default(),
                None,
                Vec::new(),
                cx,
            )
        });

        let message2 =
            thread.read_with(cx, |thread, _| thread.message(message2_id).unwrap().clone());
        assert_eq!(message2.loaded_context.text, "");

        // Check that both messages appear in the request
        let request = thread.update(cx, |thread, cx| {
            thread.to_completion_request(model.clone(), CompletionIntent::UserPrompt, cx)
        });

        assert_eq!(request.messages.len(), 3);
        assert_eq!(
            request.messages[1].string_contents(),
            "What is the best way to learn Rust?"
        );
        assert_eq!(
            request.messages[2].string_contents(),
            "Are there any good books?"
        );
    }

    #[gpui::test]
    async fn test_stale_buffer_notification(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(
            cx,
            json!({"code.rs": "fn main() {\n    println!(\"Hello, world!\");\n}"}),
        )
        .await;

        let (_workspace, _thread_store, thread, context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Open buffer and add it to context
        let buffer = add_file_to_context(&project, &context_store, "test/code.rs", cx)
            .await
            .unwrap();

        let context =
            context_store.read_with(cx, |store, _| store.context().next().cloned().unwrap());
        let loaded_context = cx
            .update(|cx| load_context(vec![context], &project, &None, cx))
            .await;

        // Insert user message with the buffer as context
        thread.update(cx, |thread, cx| {
            thread.insert_user_message("Explain this code", loaded_context, None, Vec::new(), cx)
        });

        // Create a request and check that it doesn't have a stale buffer warning yet
        let initial_request = thread.update(cx, |thread, cx| {
            thread.to_completion_request(model.clone(), CompletionIntent::UserPrompt, cx)
        });

        // Make sure we don't have a stale file warning yet
        let has_stale_warning = initial_request.messages.iter().any(|msg| {
            msg.string_contents()
                .contains("These files changed since last read:")
        });
        assert!(
            !has_stale_warning,
            "Should not have stale buffer warning before buffer is modified"
        );

        // Modify the buffer
        buffer.update(cx, |buffer, cx| {
            // Find a position at the end of line 1
            buffer.edit(
                [(1..1, "\n    println!(\"Added a new line\");\n")],
                None,
                cx,
            );
        });

        // Insert another user message without context
        thread.update(cx, |thread, cx| {
            thread.insert_user_message(
                "What does the code do now?",
                ContextLoadResult::default(),
                None,
                Vec::new(),
                cx,
            )
        });

        // Create a new request and check for the stale buffer warning
        let new_request = thread.update(cx, |thread, cx| {
            thread.to_completion_request(model.clone(), CompletionIntent::UserPrompt, cx)
        });

        // We should have a stale file warning as the last message
        let last_message = new_request
            .messages
            .last()
            .expect("Request should have messages");

        // The last message should be the stale buffer notification
        assert_eq!(last_message.role, Role::User);

        // Check the exact content of the message
        let expected_content = "These files changed since last read:\n- code.rs\n";
        assert_eq!(
            last_message.string_contents(),
            expected_content,
            "Last message should be exactly the stale buffer notification"
        );
    }

    #[gpui::test]
    async fn test_temperature_setting(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(
            cx,
            json!({"code.rs": "fn main() {\n    println!(\"Hello, world!\");\n}"}),
        )
        .await;

        let (_workspace, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Both model and provider
        cx.update(|cx| {
            AgentSettings::override_global(
                AgentSettings {
                    model_parameters: vec![LanguageModelParameters {
                        provider: Some(model.provider_id().0.to_string().into()),
                        model: Some(model.id().0.clone()),
                        temperature: Some(0.66),
                    }],
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let request = thread.update(cx, |thread, cx| {
            thread.to_completion_request(model.clone(), CompletionIntent::UserPrompt, cx)
        });
        assert_eq!(request.temperature, Some(0.66));

        // Only model
        cx.update(|cx| {
            AgentSettings::override_global(
                AgentSettings {
                    model_parameters: vec![LanguageModelParameters {
                        provider: None,
                        model: Some(model.id().0.clone()),
                        temperature: Some(0.66),
                    }],
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let request = thread.update(cx, |thread, cx| {
            thread.to_completion_request(model.clone(), CompletionIntent::UserPrompt, cx)
        });
        assert_eq!(request.temperature, Some(0.66));

        // Only provider
        cx.update(|cx| {
            AgentSettings::override_global(
                AgentSettings {
                    model_parameters: vec![LanguageModelParameters {
                        provider: Some(model.provider_id().0.to_string().into()),
                        model: None,
                        temperature: Some(0.66),
                    }],
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let request = thread.update(cx, |thread, cx| {
            thread.to_completion_request(model.clone(), CompletionIntent::UserPrompt, cx)
        });
        assert_eq!(request.temperature, Some(0.66));

        // Same model name, different provider
        cx.update(|cx| {
            AgentSettings::override_global(
                AgentSettings {
                    model_parameters: vec![LanguageModelParameters {
                        provider: Some("anthropic".into()),
                        model: Some(model.id().0.clone()),
                        temperature: Some(0.66),
                    }],
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let request = thread.update(cx, |thread, cx| {
            thread.to_completion_request(model.clone(), CompletionIntent::UserPrompt, cx)
        });
        assert_eq!(request.temperature, None);
    }

    #[gpui::test]
    async fn test_thread_summary(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;

        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Initial state should be pending
        thread.read_with(cx, |thread, _| {
            assert!(matches!(thread.summary(), ThreadSummary::Pending));
            assert_eq!(thread.summary().or_default(), ThreadSummary::DEFAULT);
        });

        // Manually setting the summary should not be allowed in this state
        thread.update(cx, |thread, cx| {
            thread.set_summary("This should not work", cx);
        });

        thread.read_with(cx, |thread, _| {
            assert!(matches!(thread.summary(), ThreadSummary::Pending));
        });

        // Send a message
        thread.update(cx, |thread, cx| {
            thread.insert_user_message("Hi!", ContextLoadResult::default(), None, vec![], cx);
            thread.send_to_model(
                model.clone(),
                CompletionIntent::ThreadSummarization,
                None,
                cx,
            );
        });

        let fake_model = model.as_fake();
        simulate_successful_response(&fake_model, cx);

        // Should start generating summary when there are >= 2 messages
        thread.read_with(cx, |thread, _| {
            assert_eq!(*thread.summary(), ThreadSummary::Generating);
        });

        // Should not be able to set the summary while generating
        thread.update(cx, |thread, cx| {
            thread.set_summary("This should not work either", cx);
        });

        thread.read_with(cx, |thread, _| {
            assert!(matches!(thread.summary(), ThreadSummary::Generating));
            assert_eq!(thread.summary().or_default(), ThreadSummary::DEFAULT);
        });

        cx.run_until_parked();
        fake_model.stream_last_completion_response("Brief");
        fake_model.stream_last_completion_response(" Introduction");
        fake_model.end_last_completion_stream();
        cx.run_until_parked();

        // Summary should be set
        thread.read_with(cx, |thread, _| {
            assert!(matches!(thread.summary(), ThreadSummary::Ready(_)));
            assert_eq!(thread.summary().or_default(), "Brief Introduction");
        });

        // Now we should be able to set a summary
        thread.update(cx, |thread, cx| {
            thread.set_summary("Brief Intro", cx);
        });

        thread.read_with(cx, |thread, _| {
            assert_eq!(thread.summary().or_default(), "Brief Intro");
        });

        // Test setting an empty summary (should default to DEFAULT)
        thread.update(cx, |thread, cx| {
            thread.set_summary("", cx);
        });

        thread.read_with(cx, |thread, _| {
            assert!(matches!(thread.summary(), ThreadSummary::Ready(_)));
            assert_eq!(thread.summary().or_default(), ThreadSummary::DEFAULT);
        });
    }

    #[gpui::test]
    async fn test_thread_summary_error_set_manually(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;

        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        test_summarize_error(&model, &thread, cx);

        // Now we should be able to set a summary
        thread.update(cx, |thread, cx| {
            thread.set_summary("Brief Intro", cx);
        });

        thread.read_with(cx, |thread, _| {
            assert!(matches!(thread.summary(), ThreadSummary::Ready(_)));
            assert_eq!(thread.summary().or_default(), "Brief Intro");
        });
    }

    #[gpui::test]
    async fn test_token_count_estimation_for_new_thread(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Ensure the thread has the configured model
        thread.update(cx, |thread, cx| {
            if thread.configured_model().is_none() {
                thread.set_configured_model(Some(ConfiguredModel {
                    provider: Arc::new(FakeLanguageModelProvider),
                    model: model.clone(),
                }), cx);
            }
        });

        // Test empty thread returns 0 tokens
        thread.read_with(cx, |thread, _| {
            let token_usage = thread.total_token_usage().unwrap();
            assert_eq!(token_usage.total, 0);
            assert!(token_usage.max > 0); // Should have a max from the configured model
        });

        // Add a user message and verify token estimation
        thread.update(cx, |thread, cx| {
            thread.insert_user_message(
                "Hello, this is a test message that should have some estimated tokens.",
                ContextLoadResult::default(),
                None,
                vec![],
                cx,
            );
        });

        // Test that token count is now estimated (should be > 0)
        thread.read_with(cx, |thread, _| {
            let token_usage = thread.total_token_usage().unwrap();
            assert!(token_usage.total > 0, "Token count should be estimated for messages");
            assert!(token_usage.max > 0);
        });

        // Add another message and verify count increases
        let first_count = thread.read_with(cx, |thread, _| {
            thread.total_token_usage().unwrap().total
        });

        thread.update(cx, |thread, cx| {
            thread.insert_user_message(
                "This is another message that should increase the token count.",
                ContextLoadResult::default(),
                None,
                vec![],
                cx,
            );
        });

        thread.read_with(cx, |thread, _| {
            let token_usage = thread.total_token_usage().unwrap();
            assert!(token_usage.total > first_count, "Token count should increase with more messages");
        });
    }

    #[gpui::test]
    async fn test_thread_summary_error_retry(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;

        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        test_summarize_error(&model, &thread, cx);

        // Sending another message should not trigger another summarize request
        thread.update(cx, |thread, cx| {
            thread.insert_user_message(
                "How are you?",
                ContextLoadResult::default(),
                None,
                vec![],
                cx,
            );
            thread.send_to_model(model.clone(), CompletionIntent::UserPrompt, None, cx);
        });

        let fake_model = model.as_fake();
        simulate_successful_response(&fake_model, cx);

        thread.read_with(cx, |thread, _| {
            // State is still Error, not Generating
            assert!(matches!(thread.summary(), ThreadSummary::Error));
        });

        // But the summarize request can be invoked manually
        thread.update(cx, |thread, cx| {
            thread.summarize(cx);
        });

        thread.read_with(cx, |thread, _| {
            assert!(matches!(thread.summary(), ThreadSummary::Generating));
        });

        cx.run_until_parked();
        fake_model.stream_last_completion_response("A successful summary");
        fake_model.end_last_completion_stream();
        cx.run_until_parked();

        thread.read_with(cx, |thread, _| {
            assert!(matches!(thread.summary(), ThreadSummary::Ready(_)));
            assert_eq!(thread.summary().or_default(), "A successful summary");
        });
    }

    fn test_summarize_error(
        model: &Arc<dyn LanguageModel>,
        thread: &Entity<Thread>,
        cx: &mut TestAppContext,
    ) {
        thread.update(cx, |thread, cx| {
            thread.insert_user_message("Hi!", ContextLoadResult::default(), None, vec![], cx);
            thread.send_to_model(
                model.clone(),
                CompletionIntent::ThreadSummarization,
                None,
                cx,
            );
        });

        let fake_model = model.as_fake();
        simulate_successful_response(&fake_model, cx);

        thread.read_with(cx, |thread, _| {
            assert!(matches!(thread.summary(), ThreadSummary::Generating));
            assert_eq!(thread.summary().or_default(), ThreadSummary::DEFAULT);
        });

        // Simulate summary request ending
        cx.run_until_parked();
        fake_model.end_last_completion_stream();
        cx.run_until_parked();

        // State is set to Error and default message
        thread.read_with(cx, |thread, _| {
            assert!(matches!(thread.summary(), ThreadSummary::Error));
            assert_eq!(thread.summary().or_default(), ThreadSummary::DEFAULT);
        });
    }

    fn simulate_successful_response(fake_model: &FakeLanguageModel, cx: &mut TestAppContext) {
        cx.run_until_parked();
        fake_model.stream_last_completion_response("Assistant response");
        fake_model.end_last_completion_stream();
        cx.run_until_parked();
    }

    fn init_test_settings(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            language::init(cx);
            Project::init_settings(cx);
            AgentSettings::register(cx);
            prompt_store::init(cx);
            thread_store::init(cx);
            workspace::init_settings(cx);
            language_model::init_settings(cx);
            ThemeSettings::register(cx);
            EditorSettings::register(cx);
            ToolRegistry::default_global(cx);
        });
    }

    // Helper to create a test project with test files
    async fn create_test_project(
        cx: &mut TestAppContext,
        files: serde_json::Value,
    ) -> Entity<Project> {
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(path!("/test"), files).await;
        Project::test(fs, [path!("/test").as_ref()], cx).await
    }

    async fn setup_test_environment(
        cx: &mut TestAppContext,
        project: Entity<Project>,
    ) -> (
        Entity<Workspace>,
        Entity<ThreadStore>,
        Entity<Thread>,
        Entity<ContextStore>,
        Arc<dyn LanguageModel>,
    ) {
        let (workspace, cx) =
            cx.add_window_view(|window, cx| Workspace::test_new(project.clone(), window, cx));

        let thread_store = cx
            .update(|_, cx| {
                ThreadStore::load(
                    project.clone(),
                    cx.new(|_| ToolWorkingSet::default()),
                    None,
                    Arc::new(PromptBuilder::new(None).unwrap()),
                    cx,
                )
            })
            .await
            .unwrap();

        let thread = thread_store.update(cx, |store, cx| store.create_thread(cx));
        let context_store = cx.new(|_cx| ContextStore::new(project.downgrade(), None));

        let provider = Arc::new(FakeLanguageModelProvider);
        let model = provider.test_model();
        let model: Arc<dyn LanguageModel> = Arc::new(model);

        cx.update(|_, cx| {
            LanguageModelRegistry::global(cx).update(cx, |registry, cx| {
                registry.set_default_model(
                    Some(ConfiguredModel {
                        provider: provider.clone(),
                        model: model.clone(),
                    }),
                    cx,
                );
                registry.set_thread_summary_model(
                    Some(ConfiguredModel {
                        provider,
                        model: model.clone(),
                    }),
                    cx,
                );
            })
        });

        (workspace, thread_store, thread, context_store, model)
    }

    async fn add_file_to_context(
        project: &Entity<Project>,
        context_store: &Entity<ContextStore>,
        path: &str,
        cx: &mut TestAppContext,
    ) -> Result<Entity<language::Buffer>> {
        let buffer_path = project
            .read_with(cx, |project, cx| project.find_project_path(path, cx))
            .unwrap();

        let buffer = project
            .update(cx, |project, cx| {
                project.open_buffer(buffer_path.clone(), cx)
            })
            .await
            .unwrap();

        context_store.update(cx, |context_store, cx| {
            context_store.add_file_from_buffer(&buffer_path, buffer.clone(), false, cx);
        });

        Ok(buffer)
    }

    #[gpui::test]
    async fn test_context_compression_strategies(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Create a thread with many messages to trigger compression
        thread.update(cx, |thread, cx| {
            // Add 30 messages to trigger compression
            for i in 0..30 {
                let message_text = format!("This is message number {} with some content that should be compressed when we hit token limits. This message contains enough text to make token counting meaningful for our compression tests.", i);
                thread.insert_user_message(
                    message_text,
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
                
                // Add assistant responses
                thread.insert_assistant_message(
                    vec![MessageSegment::Text(format!("Assistant response to message {}", i))],
                    cx,
                );
            }
        });

        // Test context optimization
        let optimization_result = thread.read_with(cx, |thread, cx| {
            thread.get_context_optimization_metrics(&model, cx)
        });

        // Verify optimization was applied
        assert!(optimization_result.messages.len() <= thread.read_with(cx, |t, _| t.messages.len()));
        assert!(optimization_result.optimization_metrics.compression_ratio >= 0.0);
        assert!(optimization_result.memory_savings >= 0.0);
        assert!(optimization_result.context_preservation > 0.0);
        
        // Test different strategies based on message count
        let analytics = thread.read_with(cx, |thread, cx| {
            thread.get_optimization_analytics(&model, cx)
        });
        
        assert_eq!(analytics.optimization_frequency, OptimizationFrequency::Constant);
        assert!(analytics.recommendations.len() > 0);
    }

    #[gpui::test]
    async fn test_context_compression_with_large_context(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Create messages with large context to test compression
        thread.update(cx, |thread, cx| {
            // Create a large context string (simulating a large file)
            let large_context = "x".repeat(5000); // 5KB of content
            let context_result = ContextLoadResult {
                loaded_context: LoadedContext {
                    text: large_context,
                    contexts: vec![],
                    images: vec![],
                },
                ..Default::default()
            };

            for i in 0..10 {
                thread.insert_user_message(
                    format!("Message {} with large context", i),
                    context_result.clone(),
                    None,
                    vec![],
                    cx,
                );
            }
        });

        // Test that compression reduces context size
        let optimization_result = thread.read_with(cx, |thread, cx| {
            thread.get_context_optimization_metrics(&model, cx)
        });

        // Should have applied compression due to large context
        assert!(optimization_result.memory_savings > 0.0);
        assert_eq!(optimization_result.strategy_used, ContextStrategy::SmartCompression);
        
        // Verify context zones are calculated
        let zones = &optimization_result.optimization_metrics.context_zones;
        assert!(zones.recent_zone_messages > 0);
        assert!(zones.recent_zone_tokens > 0);
    }

    #[gpui::test]
    async fn test_context_compression_token_estimation(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Test token estimation accuracy
        thread.update(cx, |thread, cx| {
            thread.insert_user_message(
                "Short message",
                ContextLoadResult::default(),
                None,
                vec![],
                cx,
            );
        });

        let short_tokens = thread.read_with(cx, |thread, _| {
            thread.estimate_message_tokens(&thread.messages[0])
        });

        thread.update(cx, |thread, cx| {
            thread.insert_user_message(
                "This is a much longer message that should have significantly more tokens than the short message. It contains multiple sentences and should demonstrate that our token estimation is working correctly for different message lengths.",
                ContextLoadResult::default(),
                None,
                vec![],
                cx,
            );
        });

        let long_tokens = thread.read_with(cx, |thread, _| {
            thread.estimate_message_tokens(&thread.messages[1])
        });

        // Longer message should have more tokens
        assert!(long_tokens > short_tokens);
        assert!(short_tokens > 0);
        assert!(long_tokens > 50); // Reasonable estimate for long message
    }

    #[gpui::test]
    async fn test_context_compression_memory_pressure(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Test different memory pressure scenarios
        
        // Low pressure: few messages
        thread.update(cx, |thread, cx| {
            for i in 0..3 {
                thread.insert_user_message(
                    format!("Message {}", i),
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
            }
        });

        let analytics_low = thread.read_with(cx, |thread, cx| {
            thread.get_optimization_analytics(&model, cx)
        });
        assert_eq!(analytics_low.memory_pressure, MemoryPressure::Low);
        assert_eq!(analytics_low.optimization_frequency, OptimizationFrequency::Rare);

        // Medium pressure: more messages
        thread.update(cx, |thread, cx| {
            for i in 3..15 {
                thread.insert_user_message(
                    format!("Message {}", i),
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
            }
        });

        let analytics_medium = thread.read_with(cx, |thread, cx| {
            thread.get_optimization_analytics(&model, cx)
        });
        assert_eq!(analytics_medium.optimization_frequency, OptimizationFrequency::Occasional);

        // High pressure: many messages
        thread.update(cx, |thread, cx| {
            for i in 15..35 {
                thread.insert_user_message(
                    format!("Message {}", i),
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
            }
        });

        let analytics_high = thread.read_with(cx, |thread, cx| {
            thread.get_optimization_analytics(&model, cx)
        });
        assert_eq!(analytics_high.optimization_frequency, OptimizationFrequency::Constant);
    }

    #[gpui::test]
    async fn test_context_compression_recommendations(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Create scenario that should trigger recommendations
        thread.update(cx, |thread, cx| {
            // Add many messages to trigger compression recommendations
            for i in 0..25 {
                let large_message = format!("This is a very long message number {} that contains a lot of text and should trigger compression recommendations when we analyze the thread. {}", i, "x".repeat(200));
                thread.insert_user_message(
                    large_message,
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
            }
        });

        let analytics = thread.read_with(cx, |thread, cx| {
            thread.get_optimization_analytics(&model, cx)
        });

        // Should have recommendations for this scenario
        assert!(!analytics.recommendations.is_empty());
        
        // Should recommend smart compression for many messages
        assert!(analytics.recommendations.contains(&OptimizationRecommendation::EnableSmartCompression));
    }

    #[gpui::test]
    async fn test_context_compression_performance(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Create a large thread to test performance
        thread.update(cx, |thread, cx| {
            for i in 0..50 {
                thread.insert_user_message(
                    format!("Performance test message {} with substantial content to ensure we're testing realistic compression scenarios", i),
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
            }
        });

        // Measure optimization performance
        let start = std::time::Instant::now();
        let optimization_result = thread.read_with(cx, |thread, cx| {
            thread.get_context_optimization_metrics(&model, cx)
        });
        let duration = start.elapsed();

        // Optimization should complete quickly (under 100ms for 50 messages)
        assert!(duration.as_millis() < 100);
        
        // Should have meaningful optimization time recorded
        assert!(optimization_result.optimization_metrics.optimization_time_ms > 0.0);
        assert!(optimization_result.optimization_metrics.optimization_time_ms < 100.0);
    }

    #[gpui::test]
    async fn test_context_compression_zone_breakdown(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Create mixed message types to test zone breakdown
        thread.update(cx, |thread, cx| {
            // Add user messages
            for i in 0..10 {
                thread.insert_user_message(
                    format!("User message {}", i),
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
                
                // Add assistant responses
                thread.insert_assistant_message(
                    vec![MessageSegment::Text(format!("Assistant response {}", i))],
                    cx,
                );
            }
        });

        let optimization_result = thread.read_with(cx, |thread, cx| {
            thread.get_context_optimization_metrics(&model, cx)
        });

        let zones = &optimization_result.optimization_metrics.context_zones;
        
        // Should have messages in different zones
        let total_messages = zones.recent_zone_messages + zones.compressed_zone_messages + zones.dropped_zone_messages;
        assert!(total_messages > 0);
        
        // Should have token counts for zones
        let total_tokens = zones.recent_zone_tokens + zones.compressed_zone_tokens + zones.dropped_zone_tokens;
        assert!(total_tokens > 0);
        
        // Recent zone should have some messages (most recent ones)
        assert!(zones.recent_zone_messages > 0);
    }

    #[gpui::test]
    async fn test_context_compression_efficiency_score(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Test efficiency scoring with different scenarios
        
        // Scenario 1: Small thread (should have high efficiency)
        thread.update(cx, |thread, cx| {
            for i in 0..3 {
                thread.insert_user_message(
                    format!("Small thread message {}", i),
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
            }
        });

        let analytics_small = thread.read_with(cx, |thread, cx| {
            thread.get_optimization_analytics(&model, cx)
        });
        
        // Small thread should have high efficiency (no compression needed)
        assert!(analytics_small.efficiency_score >= 0.6);

        // Scenario 2: Large thread (should trigger compression)
        thread.update(cx, |thread, cx| {
            for i in 3..30 {
                let large_content = format!("Large thread message {} with substantial content that will require compression", i);
                thread.insert_user_message(
                    large_content,
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
            }
        });

        let analytics_large = thread.read_with(cx, |thread, cx| {
            thread.get_optimization_analytics(&model, cx)
        });
        
        // Efficiency score should be between 0.0 and 1.0
        assert!(analytics_large.efficiency_score >= 0.0);
        assert!(analytics_large.efficiency_score <= 1.0);
    }

    #[gpui::test]
    async fn test_context_compression_stress_test(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Stress test with 100 messages
        thread.update(cx, |thread, cx| {
            for i in 0..100 {
                let message_text = format!("Stress test message {} with substantial content to test compression performance under load. This message simulates a real conversation with meaningful content that would need to be compressed efficiently.", i);
                thread.insert_user_message(
                    message_text,
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
            }
        });

        // Test that compression still works efficiently with many messages
        let start = std::time::Instant::now();
        let optimization_result = thread.read_with(cx, |thread, cx| {
            thread.get_context_optimization_metrics(&model, cx)
        });
        let duration = start.elapsed();

        // Should complete in reasonable time even with 100 messages
        assert!(duration.as_millis() < 500);
        
        // Should have significant compression
        assert!(optimization_result.memory_savings > 0.0);
        assert!(optimization_result.optimization_metrics.messages_compressed > 0);
        
        // Should maintain some context preservation
        assert!(optimization_result.context_preservation > 0.5);
    }

    #[gpui::test]
    async fn test_context_compression_edge_cases(cx: &mut TestAppContext) {
        init_test_settings(cx);

        let project = create_test_project(cx, json!({})).await;
        let (_, _thread_store, thread, _context_store, model) =
            setup_test_environment(cx, project.clone()).await;

        // Test edge case: empty thread
        let empty_optimization = thread.read_with(cx, |thread, cx| {
            thread.get_context_optimization_metrics(&model, cx)
        });
        
        assert_eq!(empty_optimization.messages.len(), 0);
        assert_eq!(empty_optimization.memory_savings, 0.0);
        assert_eq!(empty_optimization.context_preservation, 1.0);

        // Test edge case: single message
        thread.update(cx, |thread, cx| {
            thread.insert_user_message(
                "Single message",
                ContextLoadResult::default(),
                None,
                vec![],
                cx,
            );
        });

        let single_optimization = thread.read_with(cx, |thread, cx| {
            thread.get_context_optimization_metrics(&model, cx)
        });
        
        assert_eq!(single_optimization.messages.len(), 1);
        assert_eq!(single_optimization.strategy_used, ContextStrategy::Full);

        // Test edge case: very large single message
        thread.update(cx, |thread, cx| {
            let huge_message = "x".repeat(10000); // 10KB message
            thread.insert_user_message(
                huge_message,
                ContextLoadResult::default(),
                None,
                vec![],
                cx,
            );
        });

        let large_single_optimization = thread.read_with(cx, |thread, cx| {
            thread.get_context_optimization_metrics(&model, cx)
        });
        
        // Should handle large single message gracefully
        assert!(large_single_optimization.optimization_metrics.original_token_count > 1000);
    }

    #[gpui::test]
    async fn test_context_compression_with_mixed_content(cx: &mut TestAppContext) {
        init_test_settings(cx);
        let project = create_test_project(cx, serde_json::json!({})).await;
        let (_, _, thread, _, model) = setup_test_environment(cx, project).await;

        // Create messages with mixed content types
        thread.update(cx, |thread, cx| {
            // Add code message
            thread.insert_user_message(
                "Here's some code:\n```rust\nfn main() {\n    println!(\"Hello\");\n}\n```",
                ContextLoadResult::default(),
                None,
                vec![],
                cx,
            );

            // Add text message
            thread.insert_user_message(
                "This is a regular text message with some explanation about the code above.",
                ContextLoadResult::default(),
                None,
                vec![],
                cx,
            );

            // Add structured data message
            thread.insert_user_message(
                "Here's some JSON:\n```json\n{\n  \"key\": \"value\",\n  \"array\": [1, 2, 3]\n}\n```",
                ContextLoadResult::default(),
                None,
                vec![],
                cx,
            );
        });

        let optimized = thread.read_with(cx, |thread, cx| {
            thread.optimize_context_for_request(&model, cx)
        });

        // Should handle mixed content appropriately
        assert!(optimized.messages.len() <= 3);
        assert!(optimized.strategy_used == ContextStrategy::Full || optimized.strategy_used == ContextStrategy::SmartCompression);
    }

    #[gpui::test]
    async fn test_context_compression_with_real_thread_file(cx: &mut TestAppContext) {
        init_test_settings(cx);
        let project = create_test_project(cx, serde_json::json!({})).await;
        let (_, _, thread, _, model) = setup_test_environment(cx, project).await;

        // Check if thread.md exists in the workspace root
        let thread_file_path = std::path::Path::new("../../thread.md");
        if !thread_file_path.exists() {
            eprintln!("Skipping real thread test - thread.md not found");
            return;
        }

        // Read file stats without loading entire content
        let metadata = std::fs::metadata(thread_file_path).expect("Failed to read file metadata");
        let file_size = metadata.len();
        
        println!("Testing compression on real thread file:");
        println!("  - File size: {} bytes", file_size);
        
        // Estimate tokens (rough approximation: 1 token ≈ 4 characters)
        let estimated_tokens = (file_size / 4) as usize;
        println!("  - Estimated tokens: {}", estimated_tokens);

        // Only proceed if file is actually large enough to test compression
        if estimated_tokens < 10000 {
            println!("  - File too small for compression testing, skipping");
            return;
        }

        // Read file in chunks to simulate large thread content
        let content = std::fs::read_to_string(thread_file_path)
            .expect("Failed to read thread file");
        
        // Split content into message-like chunks (simulate conversation)
        let chunks: Vec<&str> = content
            .split("\n\n")
            .filter(|chunk| !chunk.trim().is_empty())
            .collect();
        
        println!("  - Parsed {} message chunks", chunks.len());

        // Add chunks as messages (limit to reasonable number for testing)
        let max_messages = std::cmp::min(chunks.len(), 50);
        
        thread.update(cx, |thread, cx| {
            for (i, chunk) in chunks.iter().take(max_messages).enumerate() {
                let role = if i % 2 == 0 { "User" } else { "Assistant" };
                let message_text = format!("{}: {}", role, chunk.chars().take(1000).collect::<String>());
                
                thread.insert_user_message(
                    message_text,
                    ContextLoadResult::default(),
                    None,
                    vec![],
                    cx,
                );
            }
        });

        // Test compression
        let start_time = std::time::Instant::now();
        let optimized = thread.read_with(cx, |thread, cx| {
            thread.optimize_context_for_request(&model, cx)
        });
        let compression_time = start_time.elapsed();

        println!("Compression Results:");
        println!("  - Original messages: {}", optimized.optimization_metrics.original_message_count);
        println!("  - Optimized messages: {}", optimized.optimization_metrics.optimized_message_count);
        println!("  - Original tokens: {}", optimized.optimization_metrics.original_token_count);
        println!("  - Optimized tokens: {}", optimized.optimization_metrics.optimized_token_count);
        println!("  - Compression ratio: {:.2}%", optimized.optimization_metrics.compression_ratio * 100.0);
        println!("  - Strategy used: {:?}", optimized.strategy_used);
        println!("  - Compression time: {:?}", compression_time);

        // Verify compression worked
        assert!(optimized.optimization_metrics.optimized_token_count <= optimized.optimization_metrics.original_token_count);
        
        // For large files, should use smart compression
        if estimated_tokens > 32000 {
            assert_eq!(optimized.strategy_used, ContextStrategy::SmartCompression);
            assert!(optimized.optimization_metrics.compression_ratio < 1.0);
        }

        // Test analytics
        let analytics = thread.read_with(cx, |thread, cx| {
            thread.get_optimization_analytics(&model, cx)
        });

        println!("Analytics:");
        println!("  - Efficiency score: {:.2}", analytics.efficiency_score);
        println!("  - Memory pressure: {:?}", analytics.memory_pressure);
        println!("  - Optimization frequency: {:?}", analytics.optimization_frequency);
        println!("  - Recommendations: {:?}", analytics.recommendations);

        // Verify analytics make sense
        assert!(analytics.efficiency_score >= 0.0 && analytics.efficiency_score <= 1.0);
        
        // For large contexts, should have recommendations
        if estimated_tokens > 50000 {
            assert!(!analytics.recommendations.is_empty());
        }
    }
}
