pub mod agent_profile;
pub mod context;
pub mod context_server_tool;
pub mod context_store;
pub mod history_store;
pub mod pointer_context_manager;
pub mod thread;
pub mod thread_store;
pub mod tool_use;

pub use context::{AgentContext, ContextId, ContextLoadResult};
pub use context_store::ContextStore;
pub use pointer_context_manager::{PointerContextManager, ContextManagerConfig, UsageTracker, DiffManager, TerminalCompressor};
pub use thread::{
    LastRestoreCheckpoint, Message, MessageCrease, MessageId, MessageSegment, Thread, ThreadError,
    ThreadEvent, ThreadFeedback, ThreadId, ThreadSummary, TokenUsageRatio, TotalTokenUsage,
};
pub use thread_store::{SerializedThread, TextThreadStore, ThreadStore};

pub fn init(cx: &mut gpui::App) {
    thread_store::init(cx);
}
