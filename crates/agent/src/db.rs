use crate::infinite_context::LockedMemoryArchive;
use crate::{AgentMessage, AgentMessageContent, UserMessage, UserMessageContent};
use acp_thread::ClientUserMessageId;
use agent_client_protocol::schema::v1 as acp;
use agent_client_protocol::schema::v2 as acp_v2;
use agent_settings::AgentProfileId;
use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use collections::{HashMap, IndexMap};
use futures::{FutureExt, future::Shared};
use gpui::{BackgroundExecutor, Global, Task};
use indoc::indoc;
use language_model::Speed;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sqlez::{
    bindable::{Bind, Column},
    connection::Connection,
    statement::Statement,
};
use std::{
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::Arc,
};
use ui::{App, SharedString};
use util::path_list::PathList;
use zed_env_vars::ZED_STATELESS;

pub type DbMessage = crate::Message;
pub type DbSummary = crate::legacy_thread::DetailedSummaryState;
pub type DbLanguageModel = crate::legacy_thread::SerializedLanguageModel;

#[derive(Debug, Clone)]
pub struct DbThreadMetadata {
    pub id: acp::SessionId,
    pub parent_session_id: Option<acp::SessionId>,
    pub title: SharedString,
    pub updated_at: DateTime<Utc>,
    pub created_at: Option<DateTime<Utc>>,
    /// The workspace folder paths this thread was created against, sorted
    /// lexicographically. Used for grouping threads by project in the sidebar.
    pub folder_paths: PathList,
}

impl From<&DbThreadMetadata> for acp_thread::AgentSessionInfo {
    fn from(meta: &DbThreadMetadata) -> Self {
        Self {
            session_id: meta.id.clone(),
            work_dirs: Some(meta.folder_paths.clone()),
            title: Some(meta.title.clone()),
            updated_at: Some(meta.updated_at),
            created_at: meta.created_at,
            meta: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DbThread {
    #[serde(default)]
    pub measured_cache_usage: crate::thread::MeasuredCacheUsage,
    #[serde(default)]
    pub infinite_context: bool,
    #[serde(default)]
    pub memory_archived: bool,
    #[serde(default)]
    pub memory_turn_start: Option<(usize, u64)>,
    pub title: SharedString,
    pub messages: Vec<Arc<DbMessage>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub detailed_summary: Option<SharedString>,
    #[serde(default)]
    pub initial_project_snapshot: Option<Arc<crate::ProjectSnapshot>>,
    #[serde(default)]
    pub cumulative_token_usage: language_model::TokenUsage,
    #[serde(default)]
    pub request_token_usage: HashMap<acp_thread::ClientUserMessageId, language_model::TokenUsage>,
    #[serde(default)]
    pub model: Option<DbLanguageModel>,
    #[serde(default)]
    pub profile: Option<AgentProfileId>,
    #[serde(default)]
    pub subagent_context: Option<crate::SubagentContext>,
    #[serde(default)]
    pub speed: Option<Speed>,
    #[serde(default)]
    pub thinking_enabled: bool,
    #[serde(default)]
    pub thinking_effort: Option<String>,
    #[serde(default)]
    pub draft_prompt: Option<Vec<acp_v2::ContentBlock>>,
    #[serde(default)]
    pub ui_scroll_position: Option<SerializedScrollPosition>,
    #[serde(default)]
    pub sandboxed_terminal_temp_dir: Option<PathBuf>,
    /// Sandbox escalations the user approved "for the rest of this thread".
    /// Persisted so reopening a thread keeps its grants. See
    /// [`crate::sandboxing::ThreadSandboxGrants`].
    #[serde(default)]
    pub sandbox_grants: DbSandboxGrants,
}

/// Serialized form of the sandbox permissions the user granted "for the rest of
/// this thread" (the "Allow for this thread" prompt option). Stored inside the
/// thread blob; round-trips with [`crate::sandboxing::ThreadSandboxGrants`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DbSandboxGrants {
    /// Paths granted write access, each paired with the canonical
    /// (symlink-resolved) target established when the grant was approved; each
    /// covers its whole subtree. Legacy rows stored a bare path string per
    /// entry, which still deserializes (as a grant with no resolved canonical)
    /// via [`settings::GrantedWritePath`]'s string-or-object format.
    #[serde(default)]
    pub write_paths: Vec<settings::GrantedWritePath>,
    /// Host patterns granted network access, in canonical string form (e.g.
    /// `github.com`, `*.npmjs.org`). Parsed back into patterns on load.
    #[serde(default)]
    pub network_hosts: Vec<String>,
    /// Whether arbitrary-host network access was granted.
    #[serde(default)]
    pub network_any_host: bool,
    /// Whether unrestricted filesystem writes (the broad escape hatch) were
    /// granted.
    #[serde(default)]
    pub allow_fs_write_all: bool,

    /// Whether the model-requested fully-unsandboxed escape was granted.
    #[serde(default)]
    pub unsandboxed: bool,
    /// Whether running commands unsandboxed was allowed because the OS sandbox
    /// could not be created (the fallback prompt's "for this thread" option).
    #[serde(default)]
    pub sandbox_fallback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SerializedScrollPosition {
    pub item_ix: usize,
    pub offset_in_item: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedThread {
    pub title: SharedString,
    pub messages: Vec<Arc<DbMessage>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub model: Option<DbLanguageModel>,
    pub version: String,
}

impl SharedThread {
    pub const VERSION: &'static str = "1.0.0";

    pub fn from_db_thread(thread: &DbThread) -> Self {
        Self {
            title: thread.title.clone(),
            messages: thread.messages.clone(),
            updated_at: thread.updated_at,
            model: thread.model.clone(),
            version: Self::VERSION.to_string(),
        }
    }

    pub fn to_db_thread(self) -> DbThread {
        DbThread {
            measured_cache_usage: Default::default(),
            infinite_context: false,
            memory_archived: false,
            memory_turn_start: None,
            title: format!("🔗 {}", self.title).into(),
            messages: self.messages,
            updated_at: self.updated_at,
            detailed_summary: None,
            initial_project_snapshot: None,
            cumulative_token_usage: Default::default(),
            request_token_usage: Default::default(),
            model: self.model,
            profile: None,
            subagent_context: None,
            speed: None,
            thinking_enabled: false,
            thinking_effort: None,
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: DbSandboxGrants::default(),
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        const COMPRESSION_LEVEL: i32 = 3;
        let json = serde_json::to_vec(self)?;
        let compressed = zstd::encode_all(json.as_slice(), COMPRESSION_LEVEL)?;
        Ok(compressed)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let decompressed = zstd::decode_all(data)?;
        Ok(serde_json::from_slice(&decompressed)?)
    }
}

impl DbThread {
    pub const VERSION: &'static str = "0.3.0";

    pub fn to_markdown(&self) -> String {
        crate::messages_to_markdown(&self.messages)
    }

    pub fn from_json(json: &[u8]) -> Result<Self> {
        let saved_thread_json = serde_json::from_slice::<serde_json::Value>(json)?;
        match saved_thread_json.get("version") {
            Some(serde_json::Value::String(version)) => match version.as_str() {
                Self::VERSION => Ok(serde_json::from_value(saved_thread_json)?),
                _ => Self::upgrade_from_agent_1(crate::legacy_thread::SerializedThread::from_json(
                    json,
                )?),
            },
            _ => {
                Self::upgrade_from_agent_1(crate::legacy_thread::SerializedThread::from_json(json)?)
            }
        }
    }

    fn upgrade_from_agent_1(thread: crate::legacy_thread::SerializedThread) -> Result<Self> {
        let mut messages = Vec::new();
        let mut request_token_usage = HashMap::default();

        let mut last_user_message_id = None;
        for (ix, msg) in thread.messages.into_iter().enumerate() {
            let message = match msg.role {
                language_model::Role::User => {
                    let mut content = Vec::new();

                    // Convert segments to content
                    for segment in msg.segments {
                        match segment {
                            crate::legacy_thread::SerializedMessageSegment::Text { text } => {
                                content.push(UserMessageContent::Text(text));
                            }
                            crate::legacy_thread::SerializedMessageSegment::Thinking {
                                text,
                                ..
                            } => {
                                // User messages don't have thinking segments, but handle gracefully
                                content.push(UserMessageContent::Text(text));
                            }
                            crate::legacy_thread::SerializedMessageSegment::RedactedThinking {
                                ..
                            } => {
                                // User messages don't have redacted thinking, skip.
                            }
                        }
                    }

                    // If no content was added, add context as text if available
                    if content.is_empty() && !msg.context.is_empty() {
                        content.push(UserMessageContent::Text(msg.context));
                    }

                    let id = ClientUserMessageId::new();
                    last_user_message_id = Some(id.clone());

                    crate::Message::User(UserMessage {
                        // MessageId from old format can't be meaningfully converted, so generate a new one
                        id,
                        content: Arc::from(content),
                    })
                }
                language_model::Role::Assistant => {
                    let mut content = Vec::new();

                    // Convert segments to content
                    for segment in msg.segments {
                        match segment {
                            crate::legacy_thread::SerializedMessageSegment::Text { text } => {
                                content.push(AgentMessageContent::Text(text));
                            }
                            crate::legacy_thread::SerializedMessageSegment::Thinking {
                                text,
                                signature,
                            } => {
                                content.push(AgentMessageContent::Thinking { text, signature });
                            }
                            crate::legacy_thread::SerializedMessageSegment::RedactedThinking {
                                data,
                            } => {
                                content.push(AgentMessageContent::RedactedThinking(data));
                            }
                        }
                    }

                    // Convert tool uses
                    let mut tool_names_by_id = HashMap::default();
                    for tool_use in msg.tool_uses {
                        tool_names_by_id.insert(tool_use.id.clone(), tool_use.name.clone());
                        content.push(AgentMessageContent::ToolUse(
                            language_model::LanguageModelToolUse {
                                id: tool_use.id,
                                name: tool_use.name.into(),
                                raw_input: serde_json::to_string(&tool_use.input)
                                    .unwrap_or_default(),
                                input: language_model::LanguageModelToolUseInput::Json(
                                    tool_use.input,
                                ),
                                is_input_complete: true,
                                thought_signature: None,
                            },
                        ));
                    }

                    // Convert tool results
                    let mut tool_results = IndexMap::default();
                    for tool_result in msg.tool_results {
                        let name = tool_names_by_id
                            .remove(&tool_result.tool_use_id)
                            .unwrap_or_else(|| SharedString::from("unknown"));
                        tool_results.insert(
                            tool_result.tool_use_id.clone(),
                            language_model::LanguageModelToolResult {
                                tool_use_id: tool_result.tool_use_id,
                                tool_name: name.into(),
                                is_error: tool_result.is_error,
                                content: vec![tool_result.content],
                                output: tool_result.output,
                            },
                        );
                    }

                    if let Some(last_user_message_id) = &last_user_message_id
                        && let Some(token_usage) = thread.request_token_usage.get(ix).copied()
                    {
                        request_token_usage.insert(last_user_message_id.clone(), token_usage);
                    }

                    crate::Message::Agent(AgentMessage {
                        content,
                        tool_results,
                        reasoning_details: None,
                    })
                }
                language_model::Role::System => {
                    // Skip system messages as they're not supported in the new format
                    continue;
                }
            };

            messages.push(Arc::new(message));
        }

        Ok(Self {
            infinite_context: false,
            memory_archived: false,
            memory_turn_start: None,
            measured_cache_usage: Default::default(),
            title: thread.summary,
            messages,
            updated_at: thread.updated_at,
            detailed_summary: match thread.detailed_summary_state {
                crate::legacy_thread::DetailedSummaryState::NotGenerated
                | crate::legacy_thread::DetailedSummaryState::Generating => None,
                crate::legacy_thread::DetailedSummaryState::Generated { text, .. } => Some(text),
            },
            initial_project_snapshot: thread.initial_project_snapshot,
            cumulative_token_usage: thread.cumulative_token_usage,
            request_token_usage,
            model: thread.model,
            profile: thread.profile,
            subagent_context: None,
            speed: None,
            thinking_enabled: false,
            thinking_effort: None,
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: DbSandboxGrants::default(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "zstd")]
    Zstd,
}

impl Bind for DataType {
    fn bind(&self, statement: &Statement, start_index: i32) -> Result<i32> {
        let value = match self {
            DataType::Json => "json",
            DataType::Zstd => "zstd",
        };
        value.bind(statement, start_index)
    }
}

impl Column for DataType {
    fn column(statement: &mut Statement, start_index: i32) -> Result<(Self, i32)> {
        let (value, next_index) = String::column(statement, start_index)?;
        let data_type = match value.as_str() {
            "json" => DataType::Json,
            "zstd" => DataType::Zstd,
            _ => anyhow::bail!("Unknown data type: {}", value),
        };
        Ok((data_type, next_index))
    }
}

pub(crate) struct ThreadsDatabase {
    executor: BackgroundExecutor,
    connection: Arc<Mutex<Connection>>,
    memory_archive_root: PathBuf,
    #[cfg(any(feature = "test-support", test))]
    _memory_archive_directory: tempfile::TempDir,
    /// In production, saves take real time (serialization, zstd, disk I/O) while
    /// the user keeps typing, so new save requests routinely arrive mid-write.
    /// The test executor completes writes instantly, so tests use this gate to
    /// hold a write in flight and interleave more save requests with it.
    #[cfg(test)]
    write_gate: Mutex<Option<Shared<futures::channel::oneshot::Receiver<()>>>>,
    #[cfg(test)]
    save_count: std::sync::atomic::AtomicUsize,
}

struct GlobalThreadsDatabase(Shared<Task<Result<Arc<ThreadsDatabase>, Arc<anyhow::Error>>>>);

impl Global for GlobalThreadsDatabase {}

impl ThreadsDatabase {
    pub fn connect(cx: &mut App) -> Shared<Task<Result<Arc<ThreadsDatabase>, Arc<anyhow::Error>>>> {
        if cx.has_global::<GlobalThreadsDatabase>() {
            return cx.global::<GlobalThreadsDatabase>().0.clone();
        }
        let executor = cx.background_executor().clone();
        let task = executor
            .spawn({
                let executor = executor.clone();
                async move {
                    match ThreadsDatabase::new(executor) {
                        Ok(db) => Ok(Arc::new(db)),
                        Err(err) => Err(Arc::new(err)),
                    }
                }
            })
            .shared();

        cx.set_global(GlobalThreadsDatabase(task.clone()));
        task
    }

    pub fn new(executor: BackgroundExecutor) -> Result<Self> {
        let connection = if *ZED_STATELESS {
            Connection::open_memory(Some("THREAD_FALLBACK_DB"))
        } else if cfg!(any(feature = "test-support", test)) {
            // rust stores the name of the test on the current thread.
            // We use this to automatically create a database that will
            // be shared within the test (for the test_retrieve_old_thread)
            // but not with concurrent tests.
            let thread = std::thread::current();
            let test_name = thread.name();
            Connection::open_memory(Some(&format!(
                "THREAD_FALLBACK_{}",
                test_name.unwrap_or_default()
            )))
        } else {
            let threads_dir = paths::data_dir().join("threads");
            std::fs::create_dir_all(&threads_dir)?;
            let sqlite_path = threads_dir.join("threads.db");
            Connection::open_file(&sqlite_path.to_string_lossy())
        };

        connection.exec(indoc! {"
            CREATE TABLE IF NOT EXISTS threads (
                id TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                data_type TEXT NOT NULL,
                data BLOB NOT NULL
            )
        "})?()
        .map_err(|e| e.context("Failed to create threads table"))?;

        if let Ok(mut s) = connection.exec(indoc! {"
            ALTER TABLE threads ADD COLUMN parent_id TEXT
        "})
        {
            s().ok();
        }

        if let Ok(mut s) = connection.exec(indoc! {"
            ALTER TABLE threads ADD COLUMN folder_paths TEXT;
            ALTER TABLE threads ADD COLUMN folder_paths_order TEXT;
        "})
        {
            s().ok();
        }

        if let Ok(mut s) = connection.exec(indoc! {"
            ALTER TABLE threads ADD COLUMN created_at TEXT;
        "})
        {
            if s().is_ok() {
                connection.exec(indoc! {"
                    UPDATE threads SET created_at = updated_at WHERE created_at IS NULL
                "})?()?;
            }
        }

        #[cfg(any(feature = "test-support", test))]
        let memory_archive_directory = tempfile::tempdir()?;
        #[cfg(any(feature = "test-support", test))]
        let memory_archive_root = memory_archive_directory.path().to_path_buf();
        #[cfg(not(any(feature = "test-support", test)))]
        let memory_archive_root = paths::data_dir().join("agent/infinite_context");

        let db = Self {
            executor,
            connection: Arc::new(Mutex::new(connection)),
            memory_archive_root,
            #[cfg(any(feature = "test-support", test))]
            _memory_archive_directory: memory_archive_directory,
            #[cfg(test)]
            write_gate: Mutex::new(None),
            #[cfg(test)]
            save_count: Default::default(),
        };

        Ok(db)
    }

    fn save_thread_sync(
        connection: &Arc<Mutex<Connection>>,
        id: acp::SessionId,
        thread: DbThread,
        folder_paths: &PathList,
    ) -> Result<()> {
        const COMPRESSION_LEVEL: i32 = 3;

        #[derive(Serialize)]
        struct SerializedThread {
            #[serde(flatten)]
            thread: DbThread,
            version: &'static str,
        }

        let title = thread.title.to_string();
        let updated_at = thread.updated_at.to_rfc3339();
        let parent_id = thread
            .subagent_context
            .as_ref()
            .map(|ctx| ctx.parent_thread_id.0.clone());
        let serialized_folder_paths = folder_paths.serialize();
        let (folder_paths_str, folder_paths_order_str): (Option<String>, Option<String>) =
            if folder_paths.is_empty() {
                (None, None)
            } else {
                (
                    Some(serialized_folder_paths.paths),
                    Some(serialized_folder_paths.order),
                )
            };
        let json_data = serde_json::to_string(&SerializedThread {
            thread,
            version: DbThread::VERSION,
        })?;

        let connection = connection.lock();

        let compressed = zstd::encode_all(json_data.as_bytes(), COMPRESSION_LEVEL)?;
        let data_type = DataType::Zstd;
        let data = compressed;

        // Use the thread's updated_at as created_at for new threads.
        // This ensures the creation time reflects when the thread was conceptually
        // created, not when it was saved to the database.
        let created_at = updated_at.clone();

        let mut insert = connection.exec_bound::<(Arc<str>, Option<Arc<str>>, Option<String>, Option<String>, String, String, DataType, Vec<u8>, String)>(indoc! {"
            INSERT INTO threads (id, parent_id, folder_paths, folder_paths_order, summary, updated_at, data_type, data, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
                parent_id = excluded.parent_id,
                folder_paths = excluded.folder_paths,
                folder_paths_order = excluded.folder_paths_order,
                summary = excluded.summary,
                updated_at = excluded.updated_at,
                data_type = excluded.data_type,
                data = excluded.data
        "})?;

        insert((
            id.0,
            parent_id,
            folder_paths_str,
            folder_paths_order_str,
            title,
            updated_at,
            data_type,
            data,
            created_at,
        ))?;

        Ok(())
    }

    pub fn list_threads(&self) -> Task<Result<Vec<DbThreadMetadata>>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let connection = connection.lock();

            let mut select = connection
                .select_bound::<(), (Arc<str>, Option<Arc<str>>, Option<String>, Option<String>, String, String, Option<String>)>(indoc! {"
                SELECT id, parent_id, folder_paths, folder_paths_order, summary, updated_at, created_at FROM threads ORDER BY updated_at DESC, created_at DESC
            "})?;

            let rows = select(())?;
            let mut threads = Vec::new();

            for (id, parent_id, folder_paths, folder_paths_order, summary, updated_at, created_at) in rows {
                let folder_paths = folder_paths
                    .map(|paths| {
                        PathList::deserialize(&util::path_list::SerializedPathList {
                            paths,
                            order: folder_paths_order.unwrap_or_default(),
                        })
                    })
                    .unwrap_or_default();
                let created_at = created_at
                    .as_deref()
                    .map(DateTime::parse_from_rfc3339)
                    .transpose()?
                    .map(|dt| dt.with_timezone(&Utc));

                threads.push(DbThreadMetadata {
                    id: acp::SessionId::new(id),
                    parent_session_id: parent_id.map(acp::SessionId::new),
                    title: summary.into(),
                    updated_at: DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&Utc),
                    created_at,
                    folder_paths,
                });
            }

            Ok(threads)
        })
    }

    pub fn load_thread(&self, id: acp::SessionId) -> Task<Result<Option<DbThread>>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let connection = connection.lock();
            let mut select = connection.select_bound::<Arc<str>, (DataType, Vec<u8>)>(indoc! {"
                SELECT data_type, data FROM threads WHERE id = ? LIMIT 1
            "})?;

            let rows = select(id.0)?;
            if let Some((data_type, data)) = rows.into_iter().next() {
                Ok(Some(Self::deserialize_thread(data_type, data)?))
            } else {
                Ok(None)
            }
        })
    }

    pub fn load_thread_json(&self, id: acp::SessionId) -> Task<Result<Vec<u8>>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let (data_type, data) = {
                let connection = connection.lock();
                let mut select =
                    connection.select_bound::<Arc<str>, (DataType, Vec<u8>)>(indoc! {"
                    SELECT data_type, data FROM threads WHERE id = ? LIMIT 1
                "})?;

                select(id.0.clone())?
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("Thread {} not found", id.0))?
            };

            match data_type {
                DataType::Json => Ok(data),
                DataType::Zstd => Ok(zstd::decode_all(data.as_slice())?),
            }
        })
    }

    pub fn save_thread(
        &self,
        id: acp::SessionId,
        thread: DbThread,
        folder_paths: PathList,
    ) -> Task<Result<()>> {
        let connection = self.connection.clone();
        #[cfg(test)]
        let write_gate = self.write_gate.lock().clone();
        #[cfg(test)]
        self.save_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        self.executor.spawn(async move {
            #[cfg(test)]
            if let Some(write_gate) = write_gate {
                write_gate.await.ok();
            }
            Self::save_thread_sync(&connection, id, thread, &folder_paths)
        })
    }

    #[cfg(test)]
    pub fn save_count(&self) -> usize {
        self.save_count.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[cfg(test)]
    pub fn set_write_gate(&self, gate: futures::channel::oneshot::Receiver<()>) {
        *self.write_gate.lock() = Some(gate.shared());
    }

    fn deserialize_thread(data_type: DataType, data: Vec<u8>) -> Result<DbThread> {
        let json_data = match data_type {
            DataType::Zstd => {
                let decompressed = zstd::decode_all(&data[..])?;
                String::from_utf8(decompressed)?
            }
            DataType::Json => String::from_utf8(data)?,
        };
        DbThread::from_json(json_data.as_bytes())
    }

    fn sandboxed_terminal_temp_dir(data_type: DataType, data: Vec<u8>) -> Option<PathBuf> {
        match Self::deserialize_thread(data_type, data) {
            Ok(thread) => thread.sandboxed_terminal_temp_dir,
            Err(error) => {
                log::warn!("failed to deserialize thread before deleting it: {error:#}");
                None
            }
        }
    }

    fn remove_sandboxed_terminal_temp_dir(temp_dir: PathBuf) {
        match std::fs::remove_dir_all(&temp_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                log::warn!(
                    "failed to remove sandboxed terminal temp directory {}: {error}",
                    temp_dir.display()
                );
            }
        }
    }

    fn lock_memory_archives(root: &Path, ids: &[Arc<str>]) -> Result<Vec<LockedMemoryArchive>> {
        for id in ids {
            let mut components = Path::new(id.as_ref()).components();
            anyhow::ensure!(
                matches!(components.next(), Some(Component::Normal(_)))
                    && components.next().is_none(),
                "Invalid session ID for memory archive deletion: {id}"
            );
        }
        std::fs::create_dir_all(root)
            .with_context(|| format!("Create memory archive root {}", root.display()))?;
        let mut archives = Vec::new();
        for id in ids {
            let path = root.join(id.as_ref());
            // Even an uninitialized archive needs a tombstone to fence a worker
            // that has not opened its journal yet.
            match std::fs::create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("Create memory archive {}", path.display()));
                }
            }
            let metadata = std::fs::symlink_metadata(&path)
                .with_context(|| format!("Inspect memory archive {}", path.display()))?;
            anyhow::ensure!(
                metadata.is_dir(),
                "Memory archive is not a directory: {}",
                path.display()
            );
            let archive = LockedMemoryArchive::lock(path.clone()).with_context(|| {
                format!(
                    "Cannot delete memory archive {}; close the thread and wait for memory work to finish before retrying",
                    path.display()
                )
            })?;
            archives.push(archive);
        }
        Ok(archives)
    }

    fn remove_memory_archives(archives: &[LockedMemoryArchive]) -> Result<()> {
        for archive in archives {
            archive.delete_payload()?;
        }
        Ok(())
    }

    pub fn delete_thread(&self, id: acp::SessionId) -> Task<Result<()>> {
        let connection = self.connection.clone();
        let memory_archive_root = self.memory_archive_root.clone();

        self.executor.spawn(async move {
            let sandboxed_terminal_temp_dirs = {
                let connection = connection.lock();

                let mut select_children =
                    connection.select_bound::<Arc<str>, Arc<str>>(indoc! {"
                    SELECT id FROM threads WHERE parent_id = ?
                "})?;

                let mut ids_to_delete = vec![id.0.clone()];
                let mut frontier = vec![id.0.clone()];
                while let Some(parent) = frontier.pop() {
                    for child in select_children(parent)? {
                        if !ids_to_delete.contains(&child) {
                            ids_to_delete.push(child.clone());
                            frontier.push(child);
                        }
                    }
                }

                let mut select =
                    connection.select_bound::<Arc<str>, (DataType, Vec<u8>)>(indoc! {"
                    SELECT data_type, data FROM threads WHERE id = ? LIMIT 1
                "})?;

                let mut delete = connection.exec_bound::<Arc<str>>(indoc! {"
                    DELETE FROM threads WHERE id = ?
                "})?;

                // A locked descendant must prevent deletion of the whole tree.
                let archives = Self::lock_memory_archives(&memory_archive_root, &ids_to_delete)?;
                Self::remove_memory_archives(&archives)?;

                let mut sandboxed_terminal_temp_dirs = Vec::new();
                for thread_id in ids_to_delete {
                    if let Some(temp_dir) = select(thread_id.clone())?.into_iter().next().and_then(
                        |(data_type, data)| Self::sandboxed_terminal_temp_dir(data_type, data),
                    ) {
                        sandboxed_terminal_temp_dirs.push(temp_dir);
                    }
                    delete(thread_id)?;
                }

                sandboxed_terminal_temp_dirs
            };

            for temp_dir in sandboxed_terminal_temp_dirs {
                Self::remove_sandboxed_terminal_temp_dir(temp_dir);
            }

            Ok(())
        })
    }

    pub fn delete_threads(&self) -> Task<Result<()>> {
        let connection = self.connection.clone();
        let memory_archive_root = self.memory_archive_root.clone();

        self.executor.spawn(async move {
            let sandboxed_terminal_temp_dirs = {
                let connection = connection.lock();

                let mut select =
                    connection.select_bound::<(), (Arc<str>, DataType, Vec<u8>)>(indoc! {"
                    SELECT id, data_type, data FROM threads
                "})?;
                let rows = select(())?;
                let mut ids_to_delete =
                    rows.iter().map(|(id, _, _)| id.clone()).collect::<Vec<_>>();
                // Earlier deletions may have left archives without database rows.
                match std::fs::read_dir(&memory_archive_root) {
                    Ok(entries) => {
                        for entry in entries {
                            let id: Arc<str> = entry?
                                .file_name()
                                .into_string()
                                .map_err(|_| anyhow::anyhow!("Invalid memory archive session ID"))?
                                .into();
                            if !ids_to_delete.contains(&id) {
                                ids_to_delete.push(id);
                            }
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error).with_context(|| {
                            format!("Read memory archive root {}", memory_archive_root.display())
                        });
                    }
                }

                let mut delete = connection.exec_bound::<()>(indoc! {"
                    DELETE FROM threads
                "})?;

                let archives = Self::lock_memory_archives(&memory_archive_root, &ids_to_delete)?;
                Self::remove_memory_archives(&archives)?;
                let sandboxed_terminal_temp_dirs = rows
                    .into_iter()
                    .filter_map(|(_, data_type, data)| {
                        Self::sandboxed_terminal_temp_dir(data_type, data)
                    })
                    .collect::<Vec<_>>();
                delete(())?;

                sandboxed_terminal_temp_dirs
            };

            for temp_dir in sandboxed_terminal_temp_dirs {
                Self::remove_sandboxed_terminal_temp_dir(temp_dir);
            }

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use collections::HashMap;
    use gpui::TestAppContext;
    use std::sync::Arc;

    #[test]
    fn test_shared_thread_roundtrip() {
        let original = SharedThread {
            title: "Test Thread".into(),
            messages: vec![],
            updated_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            model: None,
            version: SharedThread::VERSION.to_string(),
        };

        let bytes = original.to_bytes().expect("Failed to serialize");
        let restored = SharedThread::from_bytes(&bytes).expect("Failed to deserialize");

        assert_eq!(restored.title, original.title);
        assert_eq!(restored.version, original.version);
        assert_eq!(restored.updated_at, original.updated_at);
    }

    fn session_id(value: &str) -> acp::SessionId {
        acp::SessionId::new(Arc::<str>::from(value))
    }

    fn make_thread(title: &str, updated_at: DateTime<Utc>) -> DbThread {
        DbThread {
            measured_cache_usage: Default::default(),
            infinite_context: false,
            memory_archived: false,
            memory_turn_start: None,
            title: title.to_string().into(),
            messages: Vec::new(),
            updated_at,
            detailed_summary: None,
            initial_project_snapshot: None,
            cumulative_token_usage: Default::default(),
            request_token_usage: HashMap::default(),
            model: None,
            profile: None,
            subagent_context: None,
            speed: None,
            thinking_enabled: false,
            thinking_effort: None,
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: DbSandboxGrants::default(),
        }
    }

    #[gpui::test]
    async fn test_load_thread_json_preserves_stored_json(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let json = br#"{
            "version": "0.1.0",
            "summary": "Legacy thread",
            "unknown_field": {"preserved": true}
        }
"#;

        for data_type in [DataType::Json, DataType::Zstd] {
            let thread_id = session_id(match data_type {
                DataType::Json => "raw-json-thread",
                DataType::Zstd => "raw-zstd-thread",
            });
            let data = match data_type {
                DataType::Json => json.to_vec(),
                DataType::Zstd => zstd::encode_all(json.as_slice(), 3).expect("compress JSON"),
            };
            {
                let connection = database.connection.lock();
                let mut insert = connection
                    .exec_bound::<(Arc<str>, DataType, Vec<u8>)>(indoc! {"
                        INSERT INTO threads (id, summary, updated_at, data_type, data)
                        VALUES (?1, 'Legacy thread', '2024-01-01T00:00:00Z', ?2, ?3)
                    "})
                    .expect("prepare raw thread insert");
                insert((thread_id.0.clone(), data_type, data)).expect("insert raw thread");
            }

            let restored = database
                .load_thread_json(thread_id)
                .await
                .expect("load raw thread JSON without migration");
            assert_eq!(restored.as_slice(), json);
        }
    }

    #[gpui::test]
    async fn test_load_thread_json_rejects_invalid_zstd(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let thread_id = session_id("invalid-zstd-thread");
        {
            let connection = database.connection.lock();
            let mut insert = connection
                .exec_bound::<(Arc<str>, Vec<u8>)>(indoc! {"
                    INSERT INTO threads (id, summary, updated_at, data_type, data)
                    VALUES (?1, 'Invalid thread', '2024-01-01T00:00:00Z', 'zstd', ?2)
                "})
                .expect("prepare invalid thread insert");
            insert((thread_id.0.clone(), b"not zstd".to_vec())).expect("insert invalid thread");
        }

        assert!(database.load_thread_json(thread_id).await.is_err());
    }

    #[gpui::test]
    async fn test_load_thread_json_errors_for_missing_thread(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let error = database
            .load_thread_json(session_id("missing-thread"))
            .await
            .expect_err("missing thread must be an error");
        assert!(error.to_string().contains("missing-thread"));
    }

    #[gpui::test]
    async fn test_list_threads_orders_by_created_at(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let older_id = session_id("thread-a");
        let newer_id = session_id("thread-b");

        let older_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let newer_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        database
            .save_thread(older_id.clone(), older_thread, PathList::default())
            .await
            .unwrap();
        database
            .save_thread(newer_id.clone(), newer_thread, PathList::default())
            .await
            .unwrap();

        let entries = database.list_threads().await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, newer_id);
        assert_eq!(entries[1].id, older_id);
    }

    #[gpui::test]
    async fn test_save_thread_replaces_metadata(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("thread-a");
        let original_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let updated_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        database
            .save_thread(thread_id.clone(), original_thread, PathList::default())
            .await
            .unwrap();
        database
            .save_thread(thread_id.clone(), updated_thread, PathList::default())
            .await
            .unwrap();

        let entries = database.list_threads().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, thread_id);
        assert_eq!(entries[0].title.as_ref(), "Thread B");
        assert_eq!(
            entries[0].updated_at,
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()
        );
        assert!(
            entries[0].created_at.is_some(),
            "created_at should be populated"
        );
    }

    #[test]
    fn test_subagent_context_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.subagent_context.is_none(),
            "Legacy threads without subagent_context should default to None"
        );
    }

    #[test]
    fn test_draft_prompt_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.draft_prompt.is_none(),
            "Legacy threads without draft_prompt field should default to None"
        );
    }

    #[gpui::test]
    async fn test_draft_prompt_preserves_legacy_and_v2_content(cx: &mut TestAppContext) {
        let legacy_draft = serde_json::to_value(vec![
            acp::ContentBlock::Text(acp::TextContent::new("legacy draft")),
            acp::ContentBlock::ResourceLink(acp::ResourceLink::new("file", "file:///a.md")),
        ])
        .expect("serialize v1 draft");
        let mut thread: DbThread = serde_json::from_value(serde_json::json!({
            "title": "Draft Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z",
            "draft_prompt": legacy_draft,
        }))
        .expect("decode legacy thread with v2 draft blocks");
        assert_eq!(
            serde_json::to_value(&thread.draft_prompt).expect("serialize decoded draft"),
            legacy_draft
        );

        let extension = serde_json::json!({
            "type": "_draft_card",
            "payload": {"items": [1, {"enabled": true}], "optional": null},
            "_meta": {"source": "draft", "nested": {"version": 2}},
        });
        let extension_block: acp_v2::ContentBlock =
            serde_json::from_value(extension.clone()).expect("decode v2-only draft block");
        assert!(matches!(extension_block, acp_v2::ContentBlock::Other(_)));
        thread
            .draft_prompt
            .as_mut()
            .expect("legacy draft exists")
            .push(extension_block);
        let mut expected = legacy_draft.as_array().expect("draft is an array").clone();
        expected.push(extension);

        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let thread_id = session_id("draft-thread");
        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .expect("save mixed-version draft");
        let restored = database
            .load_thread(thread_id)
            .await
            .expect("load draft")
            .expect("saved thread exists");
        assert_eq!(
            serde_json::to_value(restored.draft_prompt).expect("serialize restored draft"),
            serde_json::Value::Array(expected)
        );
    }

    #[test]
    fn test_sandboxed_terminal_temp_dir_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.sandboxed_terminal_temp_dir.is_none(),
            "Legacy threads without sandboxed_terminal_temp_dir should default to None"
        );
    }

    #[test]
    fn test_sandbox_grants_default_when_absent() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert_eq!(
            db_thread.sandbox_grants,
            DbSandboxGrants::default(),
            "Legacy threads without sandbox_grants should default to empty grants"
        );
    }

    #[gpui::test]
    async fn test_sandbox_grants_roundtrip_through_save_load(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("sandbox-grants-thread");
        let mut thread = make_thread(
            "Sandbox Grants Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let grants = DbSandboxGrants {
            write_paths: vec![
                // A legacy bare-string grant (no resolved canonical) and a grant
                // carrying its resolved canonical, to exercise both forms of the
                // string-or-object round-trip.
                settings::GrantedWritePath::from_requested(PathBuf::from("/tmp/build")),
                settings::GrantedWritePath::resolved(
                    PathBuf::from("/tmp/link"),
                    PathBuf::from("/tmp/real"),
                ),
            ],
            network_hosts: vec!["github.com".to_string(), "*.npmjs.org".to_string()],
            network_any_host: false,
            allow_fs_write_all: false,
            unsandboxed: true,
            sandbox_fallback: true,
        };
        thread.sandbox_grants = grants.clone();

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");
        assert_eq!(loaded.sandbox_grants, grants);
    }

    #[gpui::test]
    async fn test_sandboxed_terminal_temp_dir_roundtrips_through_save_load(
        cx: &mut TestAppContext,
    ) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("sandbox-temp-dir-thread");
        let temp_dir = tempfile::Builder::new()
            .prefix("zed-agent-terminal-test-")
            .tempdir()
            .unwrap()
            .keep();
        let mut thread = make_thread(
            "Sandbox Temp Dir Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        thread.sandboxed_terminal_temp_dir = Some(temp_dir.clone());

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");
        assert_eq!(loaded.sandboxed_terminal_temp_dir, Some(temp_dir.clone()));
        std::fs::remove_dir_all(temp_dir).unwrap();
    }

    fn make_memory_archive(database: &ThreadsDatabase, id: &acp::SessionId) -> PathBuf {
        let path = database.memory_archive_root.join(id.0.as_ref());
        std::fs::create_dir_all(path.join("main")).expect("create memory archive");
        std::fs::write(path.join("main/plaintext.jsonl"), b"private conversation")
            .expect("write memory archive");
        path
    }

    fn assert_memory_archive_deleted(path: &Path) {
        let mut entries = std::fs::read_dir(path)
            .expect("read deleted archive")
            .map(|entry| {
                entry
                    .expect("read archive entry")
                    .file_name()
                    .into_string()
                    .expect("archive entry name")
            })
            .collect::<Vec<_>>();
        entries.sort();
        assert_eq!(entries, vec!["deleted".to_string(), "lock".to_string()]);
        assert_eq!(
            std::fs::metadata(path.join("deleted"))
                .expect("deletion marker")
                .len(),
            0
        );
        let error = crate::infinite_context::InfiniteContext::open(path.to_path_buf())
            .err()
            .expect("deleted archive must not reopen");
        assert!(error.to_string().contains("has been deleted"));
    }

    async fn save_memory_test_thread(
        database: &ThreadsDatabase,
        id: &acp::SessionId,
        parent: Option<&acp::SessionId>,
    ) {
        let mut thread = make_thread("Memory archive", Utc::now());
        thread.subagent_context = parent.map(|parent| crate::SubagentContext {
            parent_thread_id: parent.clone(),
            depth: 1,
        });
        database
            .save_thread(id.clone(), thread, PathList::default())
            .await
            .expect("save archive test thread");
    }

    #[gpui::test]
    async fn test_delete_thread_removes_memory_archives_recursively(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let parent_id = session_id("memory-parent");
        let child_id = session_id("memory-child");
        let grandchild_id = session_id("memory-grandchild");
        let unrelated_id = session_id("memory-unrelated");
        let mut deleted_paths = Vec::new();
        for (id, parent) in [
            (&parent_id, None),
            (&child_id, Some(&parent_id)),
            (&grandchild_id, Some(&child_id)),
        ] {
            save_memory_test_thread(&database, id, parent).await;
            deleted_paths.push(make_memory_archive(&database, id));
        }
        save_memory_test_thread(&database, &unrelated_id, None).await;
        let unrelated_path = make_memory_archive(&database, &unrelated_id);

        database
            .delete_thread(parent_id)
            .await
            .expect("delete tree");

        for path in &deleted_paths {
            assert_memory_archive_deleted(path);
        }
        assert!(unrelated_path.join("main/plaintext.jsonl").exists());
        let remaining = database
            .list_threads()
            .await
            .expect("list remaining threads");
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining.first().expect("unrelated thread").id,
            unrelated_id
        );
    }

    #[gpui::test]
    async fn test_delete_thread_removes_orphaned_memory_archive(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let id = session_id("memory-orphan");
        let path = make_memory_archive(&database, &id);

        database
            .delete_thread(id)
            .await
            .expect("delete orphan archive");

        assert_memory_archive_deleted(&path);
    }

    #[gpui::test]
    async fn test_delete_thread_tombstones_uninitialized_memory_archive(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let id = session_id("memory-uninitialized");
        save_memory_test_thread(&database, &id, None).await;
        let path = database.memory_archive_root.join(id.0.as_ref());
        assert!(!path.exists());

        database
            .delete_thread(id.clone())
            .await
            .expect("delete thread");
        assert_memory_archive_deleted(&path);
        database.delete_thread(id).await.expect("repeat deletion");
        assert_memory_archive_deleted(&path);
        assert!(
            database
                .list_threads()
                .await
                .expect("list threads")
                .is_empty()
        );
    }

    #[gpui::test]
    async fn test_delete_threads_removes_memory_archives_and_orphans(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let parent_id = session_id("memory-parent");
        let child_id = session_id("memory-child");
        let missing_id = session_id("memory-missing");
        save_memory_test_thread(&database, &parent_id, None).await;
        save_memory_test_thread(&database, &child_id, Some(&parent_id)).await;
        save_memory_test_thread(&database, &missing_id, None).await;
        let paths = [
            make_memory_archive(&database, &parent_id),
            make_memory_archive(&database, &child_id),
            make_memory_archive(&database, &session_id("memory-orphan")),
            database.memory_archive_root.join(missing_id.0.as_ref()),
        ];
        {
            let connection = database.connection.lock();
            let mut update = connection
                .exec_bound::<Arc<str>>("UPDATE threads SET data = X'00' WHERE id = ?")
                .expect("prepare corrupt thread update");
            update(child_id.0).expect("corrupt stored thread");
        }

        database.delete_threads().await.expect("delete all threads");

        for path in &paths {
            assert_memory_archive_deleted(path);
        }
        database
            .delete_threads()
            .await
            .expect("repeat bulk deletion");
        for path in &paths {
            assert_memory_archive_deleted(path);
        }
        assert!(
            database
                .list_threads()
                .await
                .expect("list threads")
                .is_empty()
        );
    }

    #[gpui::test]
    async fn test_delete_thread_refuses_locked_descendant_memory_archive(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let parent_id = session_id("memory-parent");
        let child_id = session_id("memory-child");
        save_memory_test_thread(&database, &parent_id, None).await;
        save_memory_test_thread(&database, &child_id, Some(&parent_id)).await;
        let parent_path = make_memory_archive(&database, &parent_id);
        let child_path = database.memory_archive_root.join(child_id.0.as_ref());
        let store = crate::infinite_context::InfiniteContext::open(child_path.clone())
            .expect("open locked memory store");
        std::fs::write(child_path.join("private.txt"), b"private conversation")
            .expect("write locked archive");

        let error = database
            .delete_thread(parent_id.clone())
            .await
            .expect_err("locked descendant must prevent deletion");

        assert!(error.to_string().contains("memory-child"));
        assert!(parent_path.join("main/plaintext.jsonl").exists());
        assert!(child_path.join("private.txt").exists());
        assert_eq!(
            database.list_threads().await.expect("list threads").len(),
            2
        );
        drop(store);

        database
            .delete_thread(parent_id)
            .await
            .expect("retry deletion");
        assert_memory_archive_deleted(&parent_path);
        assert_memory_archive_deleted(&child_path);
    }

    #[gpui::test]
    async fn test_delete_threads_refuses_locked_memory_archive(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let id = session_id("memory-locked");
        save_memory_test_thread(&database, &id, None).await;
        let path = database.memory_archive_root.join(id.0.as_ref());
        let store = crate::infinite_context::InfiniteContext::open(path.clone())
            .expect("open locked memory store");
        let orphan_path = make_memory_archive(&database, &session_id("memory-orphan"));

        database
            .delete_threads()
            .await
            .expect_err("locked archive must prevent bulk deletion");

        assert!(path.exists());
        assert!(orphan_path.join("main/plaintext.jsonl").exists());
        assert_eq!(
            database.list_threads().await.expect("list threads").len(),
            1
        );
        drop(store);

        database
            .delete_threads()
            .await
            .expect("retry bulk deletion");
        assert_memory_archive_deleted(&path);
        assert_memory_archive_deleted(&orphan_path);
    }

    #[gpui::test]
    async fn test_delete_thread_surfaces_memory_archive_cleanup_errors(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let id = session_id("memory-invalid-directory");
        save_memory_test_thread(&database, &id, None).await;
        let path = database.memory_archive_root.join(id.0.as_ref());
        std::fs::write(&path, b"private conversation").expect("create invalid archive path");

        let error = database
            .delete_thread(id)
            .await
            .expect_err("archive cleanup failure must reach the caller");

        assert!(error.to_string().contains("not a directory"));
        assert!(path.exists());
        assert_eq!(
            database.list_threads().await.expect("list threads").len(),
            1
        );
    }

    #[gpui::test]
    async fn test_delete_threads_surfaces_memory_archive_cleanup_errors(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).expect("create database");
        let id = session_id("memory-invalid-directory");
        save_memory_test_thread(&database, &id, None).await;
        let path = database.memory_archive_root.join(id.0.as_ref());
        std::fs::write(&path, b"private conversation").expect("create invalid archive path");
        let orphan_path = make_memory_archive(&database, &session_id("memory-orphan"));

        database
            .delete_threads()
            .await
            .expect_err("archive cleanup failure must reach the caller");

        assert!(path.exists());
        assert!(orphan_path.join("main/plaintext.jsonl").exists());
        assert_eq!(
            database.list_threads().await.expect("list threads").len(),
            1
        );
    }

    #[test]
    fn test_memory_archive_deletion_lock_blocks_engine_open() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let path = directory.path().join("memory-locked");
        std::fs::create_dir(&path).expect("create archive directory");
        let archives =
            ThreadsDatabase::lock_memory_archives(directory.path(), &[Arc::from("memory-locked")])
                .expect("acquire deletion lock");

        assert!(crate::infinite_context::InfiniteContext::open(path.clone()).is_err());
        drop(archives);
        crate::infinite_context::InfiniteContext::open(path)
            .expect("open after deletion lock drops");
    }

    #[cfg(unix)]
    #[test]
    fn test_memory_archive_deletion_rejects_directory_symlinks() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let root = directory.path().join("archives");
        std::fs::create_dir(&root).expect("create archive root");
        let outside = directory.path().join("outside");
        std::fs::create_dir(&outside).expect("create outside directory");
        let sentinel = outside.join("private.txt");
        std::fs::write(&sentinel, b"private conversation").expect("write sentinel");
        std::os::unix::fs::symlink(&outside, root.join("memory-symlink"))
            .expect("create archive symlink");

        assert!(
            ThreadsDatabase::lock_memory_archives(&root, &[Arc::from("memory-symlink")],).is_err()
        );
        assert!(sentinel.exists());
        assert!(!outside.join("deleted").exists());
    }

    #[test]
    fn test_memory_archive_deletion_rejects_path_traversal() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let sentinel = directory.path().join("private.txt");
        std::fs::write(&sentinel, b"private conversation").expect("write sentinel");

        assert!(
            ThreadsDatabase::lock_memory_archives(
                &directory.path().join("archives"),
                &[Arc::from("..")],
            )
            .is_err()
        );
        assert!(sentinel.exists());
    }

    #[gpui::test]
    async fn test_delete_thread_removes_sandboxed_terminal_temp_dir(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("sandbox-temp-dir-delete-thread");
        let temp_dir = tempfile::Builder::new()
            .prefix("zed-agent-terminal-test-")
            .tempdir()
            .unwrap()
            .keep();
        std::fs::write(temp_dir.join("sentinel"), b"content").unwrap();
        let mut thread = make_thread(
            "Sandbox Temp Dir Delete Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        thread.sandboxed_terminal_temp_dir = Some(temp_dir.clone());

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();
        database.delete_thread(thread_id).await.unwrap();

        assert!(!temp_dir.exists());
    }

    #[gpui::test]
    async fn test_delete_thread_deletes_subagent_threads(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let parent_id = session_id("parent-thread");
        let child_id = session_id("child-thread");
        let grandchild_id = session_id("grandchild-thread");
        let unrelated_id = session_id("unrelated-thread");

        let parent_thread = make_thread(
            "Parent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        let mut child_thread = make_thread(
            "Child Subagent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        child_thread.subagent_context = Some(crate::SubagentContext {
            parent_thread_id: parent_id.clone(),
            depth: 1,
        });

        let mut grandchild_thread = make_thread(
            "Grandchild Subagent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        grandchild_thread.subagent_context = Some(crate::SubagentContext {
            parent_thread_id: child_id.clone(),
            depth: 2,
        });

        let unrelated_thread = make_thread(
            "Unrelated Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        for (id, thread) in [
            (parent_id.clone(), parent_thread),
            (child_id.clone(), child_thread),
            (grandchild_id.clone(), grandchild_thread),
            (unrelated_id.clone(), unrelated_thread),
        ] {
            database
                .save_thread(id, thread, PathList::default())
                .await
                .unwrap();
        }

        database.delete_thread(parent_id.clone()).await.unwrap();

        let remaining = database.list_threads().await.unwrap();
        let remaining_ids: Vec<_> = remaining.iter().map(|thread| thread.id.clone()).collect();
        assert_eq!(remaining_ids, vec![unrelated_id]);
    }

    #[gpui::test]
    async fn test_subagent_context_roundtrips_through_save_load(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let parent_id = session_id("parent-thread");
        let child_id = session_id("child-thread");

        let mut child_thread = make_thread(
            "Subagent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        child_thread.subagent_context = Some(crate::SubagentContext {
            parent_thread_id: parent_id.clone(),
            depth: 2,
        });

        database
            .save_thread(child_id.clone(), child_thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(child_id)
            .await
            .unwrap()
            .expect("thread should exist");

        let context = loaded
            .subagent_context
            .expect("subagent_context should be restored");
        assert_eq!(context.parent_thread_id, parent_id);
        assert_eq!(context.depth, 2);
    }

    #[gpui::test]
    async fn test_non_subagent_thread_has_no_subagent_context(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("regular-thread");
        let thread = make_thread(
            "Regular Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");

        assert!(
            loaded.subagent_context.is_none(),
            "Regular threads should have no subagent_context"
        );
    }

    #[gpui::test]
    async fn test_folder_paths_roundtrip(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("folder-thread");
        let thread = make_thread(
            "Folder Thread",
            Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
        );

        let folder_paths = PathList::new(&[
            std::path::PathBuf::from("/home/user/project-a"),
            std::path::PathBuf::from("/home/user/project-b"),
        ]);

        database
            .save_thread(thread_id.clone(), thread, folder_paths.clone())
            .await
            .unwrap();

        let threads = database.list_threads().await.unwrap();
        assert_eq!(threads.len(), 1);
    }

    #[gpui::test]
    async fn test_folder_paths_empty_when_not_set(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("no-folder-thread");
        let thread = make_thread(
            "No Folder Thread",
            Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
        );

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let threads = database.list_threads().await.unwrap();
        assert_eq!(threads.len(), 1);
    }

    #[test]
    fn test_scroll_position_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.ui_scroll_position.is_none(),
            "Legacy threads without scroll_position field should default to None"
        );
    }

    #[gpui::test]
    async fn test_scroll_position_roundtrips_through_save_load(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("thread-with-scroll");

        let mut thread = make_thread(
            "Thread With Scroll",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        thread.ui_scroll_position = Some(SerializedScrollPosition {
            item_ix: 42,
            offset_in_item: 13.5,
        });

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");

        let scroll = loaded
            .ui_scroll_position
            .expect("scroll_position should be restored");
        assert_eq!(scroll.item_ix, 42);
        assert!((scroll.offset_in_item - 13.5).abs() < f32::EPSILON);
    }
}
