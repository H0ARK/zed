use super::*;
use crate::infinite_context::{InfiniteContext, SummaryJob};
use parking_lot::Mutex;

type MemoryStore = Arc<Mutex<Option<InfiniteContext>>>;
type MemoryTask = Shared<Task<std::result::Result<(), Arc<anyhow::Error>>>>;

pub(super) struct MemoryRuntime {
    pub enabled: bool,
    pub archived: bool,
    pub boundary: Option<(usize, u64)>,
    pub turn_start: usize,
    pub prior_view: Option<String>,
    progress_tx: watch::Sender<()>,
    _progress_rx: watch::Receiver<()>,
    store: MemoryStore,
    task: Option<MemoryTask>,
    running: bool,
    error: Option<Arc<anyhow::Error>>,
    cancellation_tx: Option<watch::Sender<bool>>,
}

impl MemoryRuntime {
    pub fn new(enabled: bool, archived: bool, boundary: Option<(usize, u64)>) -> Self {
        let (progress_tx, progress_rx) = watch::channel(());
        Self {
            prior_view: None,
            progress_tx,
            _progress_rx: progress_rx,
            enabled,
            archived,
            boundary,
            turn_start: boundary.map_or(0, |(index, _)| index),
            store: Arc::new(Mutex::new(None)),
            task: None,
            running: false,
            error: None,
            cancellation_tx: None,
        }
    }
}

impl Thread {
    pub fn infinite_context_enabled(&self) -> bool {
        self.memory.enabled
    }

    pub fn memory_history_is_append_only(&self) -> bool {
        self.memory.enabled || self.memory.archived
    }

    pub fn memory_summaries_running(&self) -> bool {
        self.memory.running
    }

    pub fn set_infinite_context_enabled(
        &mut self,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.running_turn.is_none() && !self.memory.running,
            "Wait for the current turn and memory summaries before changing infinite context."
        );
        self.memory.enabled = enabled;
        self.memory.archived |= enabled;
        self.memory.boundary = None;
        self.memory.prior_view = None;
        self.memory.turn_start = self.messages.len();
        self.memory.task = None;
        self.updated_at = Utc::now();
        cx.notify();
        Ok(())
    }

    pub(super) fn cancel_memory_work(&mut self) {
        if let Some(sender) = self.memory.cancellation_tx.as_mut() {
            sender.send(true).log_err();
        }
    }

    pub(super) fn clear_completed_memory_work(&mut self) {
        if !self.memory.running {
            self.memory.task = None;
            self.memory.error = None;
        }
    }

    pub(super) fn latest_memory_work(&self) -> Option<MemoryTask> {
        self.memory.task.clone()
    }

    pub(super) fn pending_memory_work(&self) -> Option<MemoryTask> {
        self.memory
            .task
            .as_ref()
            .filter(|_| self.memory.running)
            .cloned()
    }

    pub(super) fn start_memory_work(
        &mut self,
        summarize: bool,
        cx: &mut Context<Self>,
    ) -> MemoryTask {
        if let Some(task) = self.memory.task.as_ref().filter(|_| self.memory.running) {
            return task.clone();
        }
        self.memory.archived = true;
        self.memory.running = true;
        let (cancellation_tx, mut cancellation_rx) = watch::channel(false);
        self.memory.cancellation_tx = Some(cancellation_tx);
        let store = self.memory.store.clone();
        let path = paths::data_dir()
            .join("agent/infinite_context")
            .join(self.id.to_string());
        let messages = self.messages.clone();
        let turn_start = self.memory.turn_start;
        let boundary = self.memory.boundary;
        let model = self
            .summarization_model
            .clone()
            .or_else(|| self.model().cloned());
        let task = cx
            .spawn(async move |this, cx| {
                let archive_result = async {
                    let archived_boundary = cx
                        .background_spawn({
                            let store = store.clone();
                            async move {
                                let mut guard = store.lock();
                                if guard.is_none() {
                                    *guard = Some(InfiniteContext::open(path)?);
                                }
                                let memory = guard
                                    .as_mut()
                                    .ok_or_else(|| anyhow!("Memory journal is not open"))?;
                                memory.release_in_flight_jobs()?;
                                archive_messages(memory, &messages, turn_start, boundary)
                            }
                        })
                        .await?;
                    this.update(cx, |this, cx| {
                        // A new user turn must not inherit a boundary from an older worker.
                        if this.memory.turn_start == turn_start {
                            this.memory.boundary = Some((turn_start, archived_boundary));
                        }
                        this.memory.progress_tx.send(()).log_err();
                        cx.notify();
                    })?;
                    anyhow::Ok(())
                }
                .await;
                // Once archiving starts, publish its boundary before honoring cancellation:
                // the durable source checkpoint cannot be rolled back with the future.
                let (mut result, canceled): (Result<()>, bool) = match archive_result {
                    Err(error) => (Err(error), false),
                    Ok(()) if *cancellation_rx.borrow() => (Ok(()), true),
                    Ok(()) => {
                        let work = async {
                            if !summarize {
                                return Ok(());
                            }
                            let mut running = FuturesUnordered::new();
                            let mut failed = std::collections::BTreeSet::new();
                            let mut first_error = None;
                            loop {
                                while running.len() < 8 {
                                    let next = cx
                                        .background_spawn({
                                            let store = store.clone();
                                            let failed = failed.clone();
                                            async move {
                                                let mut guard = store.lock();
                                                let memory = guard.as_mut().ok_or_else(|| {
                                                    anyhow!("Memory journal is not open")
                                                })?;
                                                let Some(job) = memory.next_job()? else {
                                                    return Ok(None);
                                                };
                                                if failed.contains(&job.key) {
                                                    memory.retry_job(job.key)?;
                                                    return Ok(None);
                                                }
                                                let end = if job.key.l == 0 {
                                                    job.key.i
                                                } else {
                                                    job.end
                                                };
                                                let view = memory.render_view_before(end)?;
                                                anyhow::Ok(Some((job, view)))
                                            }
                                        })
                                        .await?;
                                    let Some((job, view)) = next else {
                                        break;
                                    };
                                    let model = model.clone().ok_or_else(|| {
                                        anyhow!("Configure a summary model to use infinite memory.")
                                    })?;
                                    let request = this.update(cx, |this, cx| {
                                        this.build_completion_request_with_memory(
                                            CompletionIntent::ThreadContextSummarization,
                                            Some(view),
                                            cx,
                                        )
                                    })??;
                                    let mut summary_cx = cx.clone();
                                    let summary_thread = this.clone();
                                    running.push(async move {
                                        let key = job.key;
                                        (
                                            key,
                                            summarize_node(
                                                model,
                                                request,
                                                job,
                                                summary_thread,
                                                &mut summary_cx,
                                            )
                                            .await,
                                        )
                                    });
                                }
                                let Some((key, result)) = running.next().await else {
                                    break;
                                };
                                if result.is_err() {
                                    failed.insert(key);
                                }
                                let error = cx
                                    .background_spawn({
                                        let store = store.clone();
                                        async move {
                                            let mut guard = store.lock();
                                            let memory = guard.as_mut().ok_or_else(|| {
                                                anyhow!("Memory journal is not open")
                                            })?;
                                            match result {
                                                Ok(text) => {
                                                    memory.complete_job(key, text)?;
                                                    anyhow::Ok(None)
                                                }
                                                Err(error) => {
                                                    memory.retry_job(key)?;
                                                    Ok(Some(error))
                                                }
                                            }
                                        }
                                    })
                                    .await?;
                                if first_error.is_none() {
                                    first_error = error;
                                }
                                this.update(cx, |this, cx| {
                                    this.memory.progress_tx.send(()).log_err();
                                    cx.notify();
                                })?;
                            }
                            if let Some(error) = first_error {
                                return Err(error);
                            }
                            Ok(())
                        };
                        futures::select! {
                            result = work.fuse() => (result, false),
                            _ = cancellation_rx.changed().fuse() => (Ok(()), true),
                        }
                    }
                };
                if canceled || result.is_err() {
                    let cleanup = cx
                        .background_spawn({
                            let store = store.clone();
                            async move {
                                if let Some(memory) = store.lock().as_mut() {
                                    memory.release_in_flight_jobs()?;
                                }
                                anyhow::Ok(())
                            }
                        })
                        .await;
                    if let Err(error) = cleanup {
                        result = Err(match result {
                            Ok(()) => error,
                            Err(previous) => {
                                error.context(format!("Memory cleanup failed after: {previous:#}"))
                            }
                        });
                    }
                }
                let result = result.map_err(Arc::new);
                this.update(cx, |this, cx| {
                    this.memory.error = result.as_ref().err().cloned();
                    this.memory.running = false;
                    this.memory.cancellation_tx = None;
                    this.memory.progress_tx.send(()).log_err();
                    cx.notify();
                })
                .log_err();
                result
            })
            .shared();
        self.memory.task = Some(task.clone());
        task
    }

    pub(super) async fn prepare_memory_request(
        this: &WeakEntity<Self>,
        mut cancellation_rx: watch::Receiver<bool>,
        cx: &mut AsyncApp,
    ) -> Result<ControlFlow<()>> {
        loop {
            if *cancellation_rx.borrow() {
                return Ok(ControlFlow::Break(()));
            }
            let preparation = this.update(cx, |this, cx| -> Result<_> {
                if !this.memory_history_is_append_only() {
                    return Ok(None);
                }
                if let Some(error) = &this.memory.error {
                    return Err(anyhow!("Infinite memory: {error:#}"));
                }
                let task = this.start_memory_work(this.memory.enabled, cx);
                if let Some(Err(error)) = task.peek() {
                    return Err(anyhow!("Infinite memory: {error:#}"));
                }
                if this.memory.enabled && this.memory.prior_view.is_none() {
                    if let Some((_, end)) = this.memory.boundary {
                        if let Some(guard) = this.memory.store.try_lock() {
                            if let Some(memory) = guard.as_ref() {
                                if memory.all_summarized_before(end) {
                                    this.memory.prior_view = Some(memory.render_turn_before(end)?);
                                }
                            }
                        }
                    }
                }
                if this.memory.enabled && this.memory.prior_view.is_some() {
                    return Ok(None);
                }
                Ok(Some((task, this.memory.progress_tx.receiver())))
            })??;
            let Some((task, mut progress)) = preparation else {
                return Ok(ControlFlow::Continue(()));
            };
            futures::select! {
                result = task.fuse() => {
                    result.map_err(|error| anyhow!("Infinite memory: {error:#}"))?;
                    if !this.read_with(cx, |this, _| this.memory.enabled)? { return Ok(ControlFlow::Continue(())); }
                },
                result = progress.changed().fuse() => { result?; },
                _ = cancellation_rx.changed().fuse() => {
                    if *cancellation_rx.borrow() { return Ok(ControlFlow::Break(())); }
                }
            }
        }
    }

    pub(super) fn memory_view(&self) -> Result<String> {
        self.memory
            .prior_view
            .clone()
            .ok_or_else(|| anyhow!("Memory turn is not ready"))
    }

    pub(super) fn memory_tools(
        &self,
    ) -> impl Iterator<Item = (SharedString, Arc<dyn AnyAgentTool>)> {
        ["zoom", "date"].into_iter().map(|name| {
            (
                name.into(),
                Arc::new(MemoryTool {
                    name,
                    store: self.memory.store.clone(),
                }) as Arc<dyn AnyAgentTool>,
            )
        })
    }
}

fn archive_messages(
    memory: &mut InfiniteContext,
    messages: &[Arc<Message>],
    turn_start: usize,
    saved_boundary: Option<(usize, u64)>,
) -> Result<u64> {
    let checkpoint = usize::try_from(memory.source_checkpoint().unwrap_or(0))?;
    anyhow::ensure!(
        checkpoint <= messages.len(),
        "Archived history cannot be shortened"
    );
    let mut boundary = saved_boundary
        .filter(|(index, _)| *index == turn_start)
        .map(|(_, end)| end);
    anyhow::ensure!(
        checkpoint <= turn_start || boundary.is_some(),
        "Missing saved memory turn boundary"
    );
    for (index, message) in messages.iter().enumerate().skip(checkpoint) {
        if index == turn_start {
            boundary = Some(memory.message_count());
        }
        for record in archive_records(message)? {
            memory.append_with_images(record.kind, &record.text, record.images, Utc::now())?;
        }
        memory.checkpoint_source(u32::try_from(
            index
                .checked_add(1)
                .ok_or_else(|| anyhow!("Message index overflow"))?,
        )?)?;
    }
    if turn_start == messages.len() {
        boundary = Some(memory.message_count());
    }
    boundary.ok_or_else(|| anyhow!("Memory turn boundary is missing"))
}

struct ArchiveRecord {
    kind: &'static str,
    text: String,
    images: Vec<serde_json::Value>,
}

fn archive_records(message: &Message) -> Result<Vec<ArchiveRecord>> {
    let mut records = Vec::new();
    let mut push = |kind, text: String, images: Vec<serde_json::Value>| {
        if !text.is_empty() || !images.is_empty() {
            records.push(ArchiveRecord { kind, text, images });
        }
    };
    match message {
        Message::User(message) => {
            for content in message.content.iter() {
                match content {
                    UserMessageContent::Text(text) => push("user", text.clone(), vec![]),
                    UserMessageContent::Mention { uri, content } => {
                        if content.is_empty() {
                            push("note", uri.as_link().to_string(), vec![]);
                        } else {
                            push("note", content.to_string(), vec![]);
                        }
                    }
                    UserMessageContent::Image(image) => {
                        push("user", String::new(), vec![serde_json::to_value(image)?])
                    }
                }
            }
        }
        Message::Agent(message) => {
            for content in &message.content {
                match content {
                    AgentMessageContent::Text(text) => push("unii", text.clone(), vec![]),
                    AgentMessageContent::ToolUse(tool) => {
                        push("tool", format!("{} {}", tool.name, tool.raw_input), vec![]);
                        if let Some(result) = message.tool_results.get(&tool.id) {
                            let mut content = result.content.clone();
                            clip_tool_output(&mut content);
                            for content in &content {
                                match content {
                                    LanguageModelToolResultContent::Text(text) => {
                                        push("echo", clip_memory_output(text), vec![])
                                    }
                                    LanguageModelToolResultContent::Image(image) => push(
                                        "echo",
                                        String::new(),
                                        vec![serde_json::to_value(image)?],
                                    ),
                                }
                            }
                        }
                    }
                    AgentMessageContent::Thinking { .. }
                    | AgentMessageContent::RedactedThinking(_) => {}
                }
            }
        }
        Message::Resume => push("user", "Continue where you left off".into(), vec![]),
        Message::Compaction(CompactionInfo::Summary(text)) => {
            push("note", text.to_string(), vec![])
        }
        Message::Compaction(CompactionInfo::ProviderNative { .. }) => {}
    }
    Ok(records)
}

pub(super) fn clip_tool_output(content: &mut Vec<LanguageModelToolResultContent>) {
    let text: String = content
        .iter()
        .filter_map(|part| match part {
            LanguageModelToolResultContent::Text(text) => Some(text.as_ref()),
            LanguageModelToolResultContent::Image(_) => None,
        })
        .collect();
    if text.chars().count() <= 30_000 {
        return;
    }
    let clipped = clip_memory_output(&text);
    let mut inserted = false;
    content.retain_mut(|part| match part {
        LanguageModelToolResultContent::Text(text) => {
            if inserted {
                return false;
            }
            *text = clipped.clone().into();
            inserted = true;
            true
        }
        LanguageModelToolResultContent::Image(_) => true,
    });
}

pub fn clip_memory_output(text: &str) -> String {
    if text.chars().count() <= 30_000 {
        return text.into();
    }
    let marker = "[middle omitted]";
    let remaining = 30_000 - marker.chars().count();
    let prefix: String = text.chars().take(remaining / 2).collect();
    let suffix: String = text
        .chars()
        .rev()
        .take(remaining - remaining / 2)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{prefix}{marker}{suffix}")
}

pub fn append_memory_view(request: &mut LanguageModelRequest, view: String) {
    let lines: Vec<_> = view
        .lines()
        .filter(|line| *line != "<chat>" && *line != "</chat>")
        .collect();
    let mut last_whole_block = None;
    for (index, block) in lines.chunks(4).enumerate() {
        let mut text = if index == 0 {
            "<chat>\n".to_owned()
        } else {
            String::new()
        };
        for line in block {
            text.push_str(line);
            text.push('\n');
        }
        if block.len() == 4 {
            last_whole_block = Some(request.messages.len());
        }
        request.messages.push(request_message(Role::User, text));
    }
    if let Some(index) = last_whole_block.and_then(|index| request.messages.get_mut(index)) {
        index.cache = true;
    }
    request.messages.push(request_message(
        Role::User,
        if lines.is_empty() {
            "<chat>\n</chat>"
        } else {
            "</chat>"
        }
        .into(),
    ));
}

fn request_message(role: Role, text: String) -> LanguageModelRequestMessage {
    LanguageModelRequestMessage {
        role,
        content: vec![text.into()],
        cache: false,
        reasoning_details: None,
    }
}

pub fn memory_summary_task(job: &SummaryJob) -> String {
    format!(
        "{}\nCompress <input> into one faithful line of at most 512 UTF-8 bytes (not characters), the length of this ruler:\n{}\n<chat> is context only: resolve references without adding facts absent from <input>.\n<input>\n{}\n</input>",
        job.task,
        "-".repeat(512),
        job.input
    )
}

async fn summarize_node(
    model: LanguageModel,
    mut request: LanguageModelRequest,
    job: SummaryJob,
    thread: WeakEntity<Thread>,
    cx: &mut AsyncApp,
) -> Result<String> {
    request
        .messages
        .push(request_message(Role::User, memory_summary_task(&job)));
    request.intent = Some(CompletionIntent::ThreadContextSummarization);
    request.max_output_tokens = Some(model.max_output_tokens().unwrap_or(4096).min(4096));
    let provider =
        cx.update(|cx| LanguageModelRegistry::read_global(cx).provider_for_model(&model))?;
    for correction_attempt in 0..5 {
        let count = provider
            .count_input_tokens(&model, request.clone(), cx)
            .await?;
        let tokens = match count {
            Some(count) => count,
            // One token per serialized UTF-8 byte deliberately overestimates
            // ordinary text when the provider has no token-count endpoint.
            None => u64::try_from(serde_json::to_vec(&request)?.len())?,
        };
        anyhow::ensure!(
            tokens
                <= compaction_input_capacity(
                    model.max_input_tokens(),
                    model.max_total_tokens(),
                    request.max_output_tokens
                ),
            "Memory summary request exceeds the model's input capacity; select a larger-context summary model."
        );
        let mut attempt = 0;
        let text = loop {
            let mut terminal_error = None;
            let result: Result<String, LanguageModelCompletionError> = async {
                let mut events = provider
                    .stream_completion(&model, request.clone(), cx)
                    .await?;
                let mut text = String::new();
                let mut accounted = TokenUsage::default();
                let mut cache_usage = None;
                while let Some(event) = events.next().await {
                    match event? {
                        LanguageModelCompletionEvent::Text(chunk) if terminal_error.is_none() => {
                            text.push_str(&chunk)
                        }
                        LanguageModelCompletionEvent::UsageUpdate(usage) => {
                            thread
                                .update(cx, |thread, cx| {
                                    let current = TokenUsage {
                                        input_tokens: accounted
                                            .input_tokens
                                            .max(usage.input_tokens),
                                        output_tokens: accounted
                                            .output_tokens
                                            .max(usage.output_tokens),
                                        cache_creation_input_tokens: accounted
                                            .cache_creation_input_tokens
                                            .max(usage.cache_creation_input_tokens),
                                        cache_read_input_tokens: accounted
                                            .cache_read_input_tokens
                                            .max(usage.cache_read_input_tokens),
                                    };
                                    thread.cumulative_token_usage =
                                        thread.cumulative_token_usage + current - accounted;
                                    accounted = current;
                                    cx.notify();
                                })
                                .map_err(LanguageModelCompletionError::Other)?;
                        }
                        LanguageModelCompletionEvent::CacheUsageUpdate(usage) => {
                            thread
                                .update(cx, |thread, cx| -> Result<()> {
                                    thread
                                        .measured_cache_usage
                                        .summary
                                        .update(&mut cache_usage, usage)?;
                                    thread.updated_at = Utc::now();
                                    cx.notify();
                                    Ok(())
                                })
                                .map_err(LanguageModelCompletionError::Other)?
                                .map_err(LanguageModelCompletionError::Other)?;
                        }
                        LanguageModelCompletionEvent::ToolUse(_) => {
                            terminal_error = Some(anyhow!(
                                "Summary model called a tool instead of summarizing"
                            ))
                        }
                        LanguageModelCompletionEvent::Stop(StopReason::Refusal) => {
                            terminal_error = Some(anyhow!("Summary model refused the memory task"))
                        }
                        LanguageModelCompletionEvent::Stop(StopReason::MaxTokens) => {
                            terminal_error =
                                Some(anyhow!("Summary model exceeded its output token limit"))
                        }
                        _ => {}
                    }
                }
                Ok(text)
            }
            .await;
            // A refusal or invalid summary response is not a transient provider failure.
            if let Some(error) = terminal_error {
                return Err(error);
            }
            match result {
                Ok(text) => break text,
                Err(error) => {
                    attempt += 1;
                    let delay = Thread::retry_strategy_for(&error)
                        .and_then(|strategy| strategy.delay_after(&error, attempt));
                    let Some(delay) = delay else {
                        return Err(error.into());
                    };
                    cx.background_executor()
                        .timer(crate::jitter_retry_delay(delay))
                        .await;
                }
            }
        };
        let text = text.trim().replace(['\n', '\r'], " ");
        anyhow::ensure!(!text.is_empty(), "Summary model returned an empty line");
        if text.len() <= 512 {
            return Ok(text);
        }
        if correction_attempt < 4 {
            let mut end = 512.min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let correction = format!(
                "Too long: your line is {} bytes, over the 512-byte limit. Rewrite the whole line for the same <input>, cutting the least valuable items to fit before this cut:\n{}| ← LIMIT",
                text.len(),
                &text[..end]
            );
            request
                .messages
                .push(request_message(Role::Assistant, text));
            request
                .messages
                .push(request_message(Role::User, correction));
        }
    }
    anyhow::bail!("Memory summary still exceeds 512 UTF-8 bytes after five attempts")
}

pub fn memory_tools() -> Vec<LanguageModelRequestTool> {
    ["zoom", "date"]
        .into_iter()
        .map(|name| {
            LanguageModelRequestTool::function(
                name.into(),
                memory_tool_description(name).to_string(),
                memory_tool_schema(name),
                false,
            )
        })
        .collect()
}

fn memory_tool_description(name: &str) -> SharedString {
    if name == "zoom" {
        "Open memory id+n into its two children. n=1 retrieves the original message; page is zero-based for long messages. Set image to its zero-based attachment index (n=1).".into()
    } else {
        "Retrieve the original date and time of a permanent memory message id.".into()
    }
}

fn memory_tool_schema(name: &str) -> serde_json::Value {
    if name == "zoom" {
        serde_json::json!({"type":"object","properties":{"id":{"type":"integer","minimum":0},"n":{"type":"integer","minimum":1},"page":{"type":"integer","minimum":0},"image":{"type":"integer","minimum":0}},"required":["id","n"],"additionalProperties":false})
    } else {
        serde_json::json!({"type":"object","properties":{"id":{"type":"integer","minimum":0}},"required":["id"],"additionalProperties":false})
    }
}

struct MemoryTool {
    name: &'static str,
    store: MemoryStore,
}

impl AnyAgentTool for MemoryTool {
    fn name(&self) -> SharedString {
        self.name.into()
    }
    fn description(&self) -> SharedString {
        memory_tool_description(self.name)
    }
    fn kind(&self) -> acp::ToolKind {
        acp::ToolKind::Read
    }
    fn initial_title(&self, _: serde_json::Value, _: &mut App) -> SharedString {
        self.name()
    }
    fn input_schema(&self) -> serde_json::Value {
        memory_tool_schema(self.name)
    }
    fn run(
        self: Arc<Self>,
        input: ToolInput<serde_json::Value>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<AgentToolOutput, AgentToolOutput>> {
        cx.spawn(async move |cx| {
            let result: Result<AgentToolOutput> = async {
                let input = input.recv().await?;
                let output = cx
                    .background_spawn(async move {
                        let guard = self.store.lock();
                        let memory = guard
                            .as_ref()
                            .ok_or_else(|| anyhow!("Memory journal is not open"))?;
                        let id = input
                            .get("id")
                            .and_then(serde_json::Value::as_u64)
                            .ok_or_else(|| anyhow!("id must be a nonnegative integer"))?;
                        let mut image = None;
                        let text = if self.name == "date" {
                            memory.date(id)?
                        } else {
                            let span = input
                                .get("n")
                                .and_then(serde_json::Value::as_u64)
                                .ok_or_else(|| anyhow!("n must be a positive power of two"))?;
                            let page = input
                                .get("page")
                                .map(|value| {
                                    value.as_u64().ok_or_else(|| {
                                        anyhow!("page must be a nonnegative integer")
                                    })
                                })
                                .transpose()?
                                .unwrap_or(0);
                            if let Some(index) = input.get("image") {
                                anyhow::ensure!(span == 1, "Image retrieval requires n=1");
                                let index = index.as_u64().ok_or_else(|| {
                                    anyhow!("image must be a nonnegative integer")
                                })?;
                                let images = memory.zoom_images(id)?;
                                image = Some(serde_json::from_value::<LanguageModelImage>(
                                    images
                                        .get(usize::try_from(index)?)
                                        .ok_or_else(|| anyhow!("Image index out of range"))?
                                        .clone(),
                                )?);
                            }
                            memory.zoom(id, span, usize::try_from(page)?)?
                        };
                        let mut content =
                            vec![LanguageModelToolResultContent::Text(text.clone().into())];
                        if let Some(image) = image {
                            content.push(LanguageModelToolResultContent::Image(image));
                        }
                        anyhow::Ok(AgentToolOutput {
                            llm_output: content,
                            raw_output: serde_json::json!({"text":text}),
                        })
                    })
                    .await?;
                event_stream.update_fields(
                    acp::ToolCallUpdateFields::new()
                        .status(acp::ToolCallStatus::Completed)
                        .raw_output(output.raw_output.clone()),
                );
                Ok(output)
            }
            .await;
            result.map_err(|error| {
                let output = AgentToolOutput::from(error);
                event_stream.update_fields(
                    acp::ToolCallUpdateFields::new()
                        .status(acp::ToolCallStatus::Failed)
                        .raw_output(output.raw_output.clone()),
                );
                output
            })
        })
    }
    fn replay(
        &self,
        _: serde_json::Value,
        output: serde_json::Value,
        event_stream: ToolCallEventStream,
        _: &mut App,
    ) -> Result<()> {
        event_stream.update_fields(
            acp::ToolCallUpdateFields::new()
                .status(acp::ToolCallStatus::Completed)
                .raw_output(output),
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipping_counts_unicode_characters_and_preserves_both_ends() {
        let text = format!("start{}end", "🦀".repeat(31_000));
        let clipped = clip_memory_output(&text);
        assert_eq!(clipped.chars().count(), 30_000);
        assert!(clipped.starts_with("start"));
        assert!(clipped.ends_with("end"));
        assert!(clipped.contains("[middle omitted]"));
        assert_eq!(clip_memory_output("unchanged\n🦀"), "unchanged\n🦀");
    }

    #[test]
    fn archive_omits_reasoning_without_rewriting_visible_text() {
        let message = Message::Agent(AgentMessage {
            content: vec![
                AgentMessageContent::Text(" exact\n🦀  ".into()),
                AgentMessageContent::Thinking {
                    text: "secret".into(),
                    signature: None,
                },
                AgentMessageContent::RedactedThinking("encrypted".into()),
            ],
            reasoning_details: Some(Arc::new(serde_json::json!(["secret"]))),
            ..Default::default()
        });
        let records = archive_records(&message).expect("archive");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].text, " exact\n🦀  ");
    }

    #[test]
    fn view_cache_boundary_uses_only_complete_four_line_blocks() {
        let mut request = LanguageModelRequest::default();
        append_memory_view(&mut request, "<chat>\na\nb\nc\nd\ne\n</chat>".into());
        assert_eq!(request.messages.len(), 3);
        assert!(request.messages[0].cache);
        assert!(!request.messages[1].cache);
        assert!(!request.messages[2].cache);
    }

    #[test]
    fn source_checkpoint_and_saved_boundary_survive_reopening() {
        let directory = tempfile::tempdir().expect("temporary memory directory");
        let messages = vec![
            Arc::new(Message::User(UserMessage {
                id: ClientUserMessageId::new(),
                content: vec![UserMessageContent::Text(" exact\n🦀  ".into())].into(),
            })),
            Arc::new(Message::Agent(AgentMessage {
                content: vec![AgentMessageContent::Text("response".into())],
                ..Default::default()
            })),
        ];
        let mut memory = InfiniteContext::open(directory.path().to_path_buf()).expect("open");
        let boundary = archive_messages(&mut memory, &messages, 1, None).expect("archive");
        assert_eq!(boundary, 1);
        assert_eq!(memory.message_count(), 2);
        assert!(memory.next_job().expect("verbatim summaries").is_none());
        let view = memory.render_turn_before(boundary).expect("view");
        assert!(
            memory
                .zoom(0, 1, 0)
                .expect("raw message")
                .contains(" exact\n🦀  ")
        );
        drop(memory);
        let mut memory = InfiniteContext::open(directory.path().to_path_buf()).expect("reopen");
        assert_eq!(
            memory.render_turn_before(boundary).expect("saved view"),
            view
        );
        assert_eq!(
            archive_messages(&mut memory, &messages, 1, Some((1, boundary)))
                .expect("idempotent sync"),
            boundary
        );
        assert_eq!(memory.message_count(), 2);
        assert!(archive_messages(&mut memory, &[], 0, None).is_err());
    }

    #[gpui::test(iterations = 20)]
    async fn canceled_archive_publishes_boundary_before_archive_only_flush(
        cx: &mut gpui::TestAppContext,
    ) {
        let (thread, _, fake) = super::super::tests::setup_thread_for_test(cx).await;
        let directory = tempfile::tempdir().expect("temporary memory directory");
        let memory = InfiniteContext::open(directory.path().to_path_buf()).expect("open");
        let task = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.messages = ["prior user", "current user"]
                    .into_iter()
                    .map(|text| {
                        Arc::new(Message::User(UserMessage {
                            id: ClientUserMessageId::new(),
                            content: vec![UserMessageContent::Text(text.into())].into(),
                        }))
                    })
                    .collect();
                thread.memory.turn_start = 1;
                *thread.memory.store.lock() = Some(memory);
                let task = thread.start_memory_work(true, cx);
                thread.cancel_memory_work();
                task
            })
        });
        task.await.expect("canceled archive");
        assert!(fake.pending_completions().is_empty());
        let task = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                assert_eq!(thread.memory.boundary, Some((1, 1)));
                assert_eq!(
                    thread
                        .memory
                        .store
                        .lock()
                        .as_ref()
                        .expect("store")
                        .source_checkpoint(),
                    Some(2)
                );
                thread.pending_message = Some(AgentMessage {
                    content: vec![AgentMessageContent::Text("partial response".into())],
                    ..Default::default()
                });
                thread.flush_pending_message(cx);
                thread.start_memory_work(false, cx)
            })
        });
        task.await.expect("archive-only flush after cancellation");
        thread.read_with(cx, |thread, _| {
            assert_eq!(thread.memory.boundary, Some((1, 1)));
            let guard = thread.memory.store.lock();
            let memory = guard.as_ref().expect("store");
            assert_eq!(memory.source_checkpoint(), Some(3));
            assert_eq!(memory.message_count(), 3);
            assert!(
                memory
                    .zoom(2, 1, 0)
                    .expect("partial reply")
                    .contains("partial response")
            );
        });
    }

    #[gpui::test]
    async fn resume_flushes_pending_reply_into_prior_memory(cx: &mut gpui::TestAppContext) {
        let (thread, _, fake) = super::super::tests::setup_thread_for_test(cx).await;
        let directory = tempfile::tempdir().expect("temporary memory directory");
        let memory = InfiniteContext::open(directory.path().to_path_buf()).expect("open");
        let model = fake.model("memory-resume");
        let _events = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model.clone(), cx);
                thread
                    .set_infinite_context_enabled(true, cx)
                    .expect("enable");
                *thread.memory.store.lock() = Some(memory);
                thread.messages.push(Arc::new(Message::User(UserMessage {
                    id: ClientUserMessageId::new(),
                    content: vec![UserMessageContent::Text("old user".into())].into(),
                })));
                thread.pending_message = Some(AgentMessage {
                    content: vec![AgentMessageContent::Text("old partial reply".into())],
                    ..Default::default()
                });
                let events = thread.resume(cx).expect("resume");
                assert!(matches!(thread.messages.as_slice(), [user, reply, resume]
                    if matches!(&**user, Message::User(_))
                        && matches!(&**reply, Message::Agent(_))
                        && matches!(&**resume, Message::Resume)));
                assert_eq!(thread.memory.turn_start, 2);
                events
            })
        });
        cx.run_until_parked();
        let request = fake
            .pending_completions_for(&model)
            .pop()
            .expect("resumed request");
        let texts: Vec<_> = request
            .messages
            .iter()
            .map(LanguageModelRequestMessage::string_contents)
            .collect();
        assert!(texts.iter().any(|text| text.contains("old partial reply")));
        assert!(!texts.iter().any(|text| text == "old partial reply"));
        thread.read_with(cx, |thread, _| {
            assert_eq!(thread.memory.boundary, Some((2, 2)));
            assert!(
                thread
                    .memory_view()
                    .expect("prior view")
                    .contains("old partial reply")
            );
        });
        let cancellation = cx.update(|cx| thread.update(cx, |thread, cx| thread.cancel(cx)));
        cancellation.await;
    }

    #[gpui::test]
    async fn disabling_memory_does_not_allow_destructive_edits(cx: &mut gpui::TestAppContext) {
        let (thread, _, _) = super::super::tests::setup_thread_for_test(cx).await;
        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                let id = ClientUserMessageId::new();
                thread.messages.push(Arc::new(Message::User(UserMessage {
                    id: id.clone(),
                    content: vec![UserMessageContent::Text("immutable".into())].into(),
                })));
                thread
                    .set_infinite_context_enabled(true, cx)
                    .expect("enable");
                thread
                    .set_infinite_context_enabled(false, cx)
                    .expect("disable");
                assert!(!thread.infinite_context_enabled());
                assert!(thread.memory_history_is_append_only());
                assert!(thread.truncate(id.clone(), cx).is_err());
                assert!(thread.compact(id, cx).is_err());
                assert!(!thread.auto_compaction_enabled(cx));
                assert_eq!(thread.messages.len(), 1);
                thread.measured_cache_usage = MeasuredCacheUsage {
                    agent: CacheUsageTotals {
                        requests: 2,
                        input_tokens: 300,
                        cached_tokens: 150,
                    },
                    summary: CacheUsageTotals {
                        requests: 1,
                        input_tokens: 100,
                        cached_tokens: 0,
                    },
                };
                let snapshot = thread.snapshot_for_cache_replay(cx);
                assert!(!snapshot.infinite_context);
                assert!(snapshot.memory_archived);
                let value = serde_json::to_value(snapshot).expect("serialize");
                let restored: DbThread =
                    serde_json::from_value(value.clone()).expect("deserialize");
                assert!(restored.memory_archived);
                assert_eq!(restored.measured_cache_usage, thread.measured_cache_usage());
                let mut legacy = value;
                let object = legacy.as_object_mut().expect("thread object");
                object.remove("infinite_context");
                object.remove("memory_archived");
                object.remove("memory_turn_start");
                object.remove("measured_cache_usage");
                let legacy: DbThread = serde_json::from_value(legacy).expect("legacy thread");
                assert_eq!(legacy.measured_cache_usage, MeasuredCacheUsage::default());
                assert!(!legacy.infinite_context);
                assert!(!legacy.memory_archived);
                assert!(legacy.memory_turn_start.is_none());
            })
        });
    }

    #[gpui::test]
    async fn fresh_request_uses_prior_view_full_user_and_current_raw_exchange(
        cx: &mut gpui::TestAppContext,
    ) {
        let (thread, event_stream, fake) = super::super::tests::setup_thread_for_test(cx).await;
        let directory = tempfile::tempdir().expect("temporary memory directory");
        let mut memory = InfiniteContext::open(directory.path().to_path_buf()).expect("open");
        memory
            .append("user", &"old raw ".repeat(200), Utc::now())
            .expect("old message");
        while let Some(job) = memory.next_job().expect("summary job") {
            memory
                .complete_job(job.key, "saved prior summary".into())
                .expect("summary");
        }
        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(fake.model("memory"), cx);
                thread
                    .set_infinite_context_enabled(true, cx)
                    .expect("enable");
                thread.messages = vec![
                    Arc::new(Message::User(UserMessage {
                        id: ClientUserMessageId::new(),
                        content: vec![UserMessageContent::Text("old raw".into())].into(),
                    })),
                    Arc::new(Message::User(UserMessage {
                        id: ClientUserMessageId::new(),
                        content: vec![UserMessageContent::Text("latest full\n🦀 user".into())]
                            .into(),
                    })),
                    Arc::new(Message::Agent(AgentMessage {
                        content: vec![AgentMessageContent::Text("current raw response".into())],
                        ..Default::default()
                    })),
                ];
                thread.memory.turn_start = 1;
                thread.memory.boundary = Some((1, 1));
                thread.memory.prior_view = Some(memory.render_turn_before(1).expect("prior view"));
                *thread.memory.store.lock() = Some(memory);
                let tools = thread.enabled_tools(cx);
                let (sender, _) = watch::channel(false);
                thread.running_turn = Some(RunningTurn::new(
                    event_stream,
                    tools,
                    sender,
                    Task::ready(()),
                ));
                let request = thread
                    .build_completion_request(CompletionIntent::UserPrompt, cx)
                    .expect("request");
                let texts: Vec<_> = request
                    .messages
                    .iter()
                    .map(LanguageModelRequestMessage::string_contents)
                    .collect();
                assert!(
                    texts
                        .iter()
                        .any(|text| text.contains("saved prior summary"))
                );
                assert!(texts.iter().any(|text| text == "latest full\n🦀 user"));
                assert!(texts.iter().any(|text| text == "current raw response"));
                assert!(!texts.iter().any(|text| text == "old raw"));
                assert!(request.tools.iter().any(|tool| tool.name == "zoom"));
                assert!(request.tools.iter().any(|tool| tool.name == "date"));
                assert_eq!(request.compact_at_tokens, None);
                thread.running_turn.take();
            })
        });
    }

    #[gpui::test]
    async fn summary_correction_enforces_real_utf8_byte_limit(cx: &mut gpui::TestAppContext) {
        let (thread, _, fake) = super::super::tests::setup_thread_for_test(cx).await;
        let mut model = fake.model("memory-summary");
        model.max_token_count = 200_000;
        model.max_input_tokens = 200_000;
        let summary_model = model.clone();
        let weak_thread = thread.downgrade();
        let task = cx.update(|cx| {
            cx.spawn(async move |cx| {
                summarize_node(
                    summary_model,
                    LanguageModelRequest::default(),
                    SummaryJob {
                        key: crate::infinite_context::NodeKey { l: 0, i: 0 },
                        input: "source input".into(),
                        task: "summarize".into(),
                        end: 1,
                    },
                    weak_thread,
                    cx,
                )
                .await
            })
        });
        cx.run_until_parked();
        fake.send_last_text(&model, "🦀".repeat(129));
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 100,
                cached_tokens: 25,
            }),
        );
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::Stop(StopReason::EndTurn),
        );
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::UsageUpdate(TokenUsage {
                input_tokens: 200,
                ..Default::default()
            }),
        );
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 200,
                cached_tokens: 50,
            }),
        );
        fake.end_last(&model);
        cx.run_until_parked();
        let correction = fake
            .pending_completions_for(&model)
            .pop()
            .expect("correction request");
        assert!(
            correction
                .messages
                .last()
                .expect("correction")
                .string_contents()
                .contains("516 bytes")
        );
        fake.send_last_text(&model, "🦀".repeat(128));
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::Stop(StopReason::EndTurn),
        );
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::UsageUpdate(TokenUsage {
                input_tokens: 300,
                ..Default::default()
            }),
        );
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 300,
                cached_tokens: 200,
            }),
        );
        fake.end_last(&model);
        cx.run_until_parked();
        assert_eq!(task.await.expect("bounded summary").len(), 512);
        assert_eq!(fake.input_token_count_requests().len(), 2);
        thread.read_with(cx, |thread, _| {
            assert_eq!(
                thread.measured_cache_usage().summary,
                CacheUsageTotals {
                    requests: 2,
                    input_tokens: 500,
                    cached_tokens: 250
                }
            );
            assert_eq!(
                thread
                    .measured_cache_usage()
                    .summary
                    .cache_read_percentage(),
                Some(50.0)
            );
            assert_eq!(thread.cumulative_token_usage().input_tokens, 500);
        });
    }

    #[gpui::test]
    async fn memory_summaries_are_bounded_and_cancellation_releases_jobs(
        cx: &mut gpui::TestAppContext,
    ) {
        let (thread, event_stream, fake) = super::super::tests::setup_thread_for_test(cx).await;
        let directory = tempfile::tempdir().expect("temporary memory directory");
        let messages: Vec<_> = (0..20)
            .map(|index| {
                Arc::new(Message::User(UserMessage {
                    id: ClientUserMessageId::new(),
                    content: vec![UserMessageContent::Text(format!(
                        "message {index} {}",
                        "source ".repeat(150)
                    ))]
                    .into(),
                }))
            })
            .collect();
        let mut memory = InfiniteContext::open(directory.path().to_path_buf()).expect("open");
        let boundary =
            archive_messages(&mut memory, &messages, messages.len(), None).expect("archive");
        let mut model = fake.model("memory-summary");
        model.max_token_count = 200_000;
        model.max_input_tokens = 200_000;
        let task = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model.clone(), cx);
                thread
                    .set_infinite_context_enabled(true, cx)
                    .expect("enable");
                thread.messages = messages;
                thread.memory.turn_start = thread.messages.len();
                thread.memory.boundary = Some((thread.messages.len(), boundary));
                *thread.memory.store.lock() = Some(memory);
                let tools = thread.enabled_tools(cx);
                let (sender, _) = watch::channel(false);
                thread.running_turn = Some(RunningTurn::new(
                    event_stream,
                    tools,
                    sender,
                    Task::ready(()),
                ));
                thread.start_memory_work(true, cx)
            })
        });
        cx.run_until_parked();
        assert_eq!(fake.pending_completions_for(&model).len(), 8);
        for request in fake.pending_completions_for(&model) {
            fake.send_event(
                &model,
                &request,
                LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                    input_tokens: 10,
                    cached_tokens: 0,
                }),
            );
        }
        cx.run_until_parked();
        cx.update(|cx| thread.update(cx, |thread, _| thread.cancel_memory_work()));
        cx.run_until_parked();
        task.await.expect("canceled worker");
        cx.update(|cx| {
            thread.update(cx, |thread, _| {
                assert!(!thread.memory_summaries_running());
                assert_eq!(
                    thread.measured_cache_usage().summary,
                    CacheUsageTotals {
                        requests: 8,
                        input_tokens: 80,
                        cached_tokens: 0
                    }
                );
                let mut guard = thread.memory.store.lock();
                let memory = guard.as_mut().expect("store");
                assert!(memory.next_job().expect("released job").is_some());
                memory.release_in_flight_jobs().expect("release test claim");
                thread.running_turn.take();
            })
        });
    }

    #[gpui::test(iterations = 20)]
    async fn terminal_stop_drains_delayed_usage_without_waiting_forever_for_eof(
        cx: &mut gpui::TestAppContext,
    ) {
        for (stop_reason, expected_stop) in [
            (StopReason::Refusal, acp::StopReason::Refusal),
            (StopReason::MaxTokens, acp::StopReason::MaxTokens),
        ] {
            let (thread, _, fake) = super::super::tests::setup_thread_for_test(cx).await;
            let model = fake.model("terminal-usage");
            let events = cx.update(|cx| {
                thread.update(cx, |thread, cx| {
                    thread.set_model(model.clone(), cx);
                    thread.title = Some("terminal usage test".into());
                    thread
                        .send(ClientUserMessageId::new(), ["user"], cx)
                        .expect("send")
                })
            });
            cx.run_until_parked();
            let request = fake.pending_completions_for(&model).pop().expect("request");
            fake.send_text(&model, &request, "partial reply");
            fake.send_event(
                &model,
                &request,
                LanguageModelCompletionEvent::Stop(stop_reason),
            );
            cx.run_until_parked();
            assert!(!fake.is_stream_closed(&model, &request));

            fake.send_text(&model, &request, "ignored after terminal stop");
            fake.send_event(
                &model,
                &request,
                LanguageModelCompletionEvent::UsageUpdate(TokenUsage {
                    input_tokens: 100,
                    ..Default::default()
                }),
            );
            fake.send_event(
                &model,
                &request,
                LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                    input_tokens: 100,
                    cached_tokens: 25,
                }),
            );
            cx.run_until_parked();
            thread.read_with(cx, |thread, _| {
                assert_eq!(
                    thread.measured_cache_usage().agent,
                    CacheUsageTotals {
                        requests: 1,
                        input_tokens: 100,
                        cached_tokens: 25,
                    }
                );
                assert_eq!(thread.cumulative_token_usage().input_tokens, 100);
                assert!(!thread.to_markdown().contains("ignored after terminal stop"));
            });

            cx.background_executor.advance_clock(TERMINAL_USAGE_TIMEOUT);
            cx.run_until_parked();
            assert!(fake.is_stream_closed(&model, &request));
            thread.read_with(cx, |thread, _| assert!(thread.running_turn.is_none()));
            let stops: Vec<_> = events
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .filter_map(|event| match event.expect("thread event") {
                    ThreadEvent::Stop(reason) => Some(reason),
                    _ => None,
                })
                .collect();
            assert_eq!(stops, vec![expected_stop]);
        }
    }

    #[gpui::test]
    async fn refusal_keeps_archived_user_and_partial_agent_response(cx: &mut gpui::TestAppContext) {
        let (thread, _, fake) = super::super::tests::setup_thread_for_test(cx).await;
        let directory = tempfile::tempdir().expect("temporary memory directory");
        let memory = InfiniteContext::open(directory.path().to_path_buf()).expect("open");
        let model = fake.model("memory-main");
        let mut events = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model.clone(), cx);
                thread
                    .set_infinite_context_enabled(true, cx)
                    .expect("enable");
                *thread.memory.store.lock() = Some(memory);
                thread
                    .send(ClientUserMessageId::new(), ["latest user"], cx)
                    .expect("send")
            })
        });
        cx.run_until_parked();
        assert_eq!(fake.pending_completions_for(&model).len(), 1);
        fake.send_last_text(&model, "partial response");
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::Stop(StopReason::Refusal),
        );
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::UsageUpdate(TokenUsage {
                input_tokens: 150,
                ..Default::default()
            }),
        );
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 100,
                cached_tokens: 25,
            }),
        );
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 150,
                cached_tokens: 50,
            }),
        );
        fake.end_last(&model);
        cx.run_until_parked();
        let mut refused = false;
        while let Some(event) = events.next().await {
            if matches!(
                event.expect("ACP event"),
                ThreadEvent::Stop(acp::StopReason::Refusal)
            ) {
                refused = true;
                break;
            }
        }
        assert!(refused);
        thread.read_with(cx, |thread, _| {
            assert_eq!(thread.messages.len(), 2);
            assert!(matches!(&*thread.messages[0], Message::User(_)));
            let records = archive_records(&thread.messages[1]).expect("agent archive");
            assert_eq!(records[0].text, "partial response");
            assert_eq!(
                thread.measured_cache_usage().agent,
                CacheUsageTotals {
                    requests: 1,
                    input_tokens: 150,
                    cached_tokens: 50
                }
            );
            assert_eq!(
                thread.measured_cache_usage().summary,
                CacheUsageTotals::default()
            );
            assert_eq!(thread.cumulative_token_usage().input_tokens, 150);
            assert!(thread.memory_history_is_append_only());
        });
    }

    #[test]
    fn multipart_tool_output_has_one_unicode_budget_and_retains_images() {
        let image = LanguageModelImage {
            source: "image".into(),
        };
        let mut content = vec![
            LanguageModelToolResultContent::Text(format!("start{}", "🦀".repeat(20_000)).into()),
            LanguageModelToolResultContent::Image(image.clone()),
            LanguageModelToolResultContent::Text(format!("{}end", "🦀".repeat(20_000)).into()),
        ];
        clip_tool_output(&mut content);
        let text: String = content
            .iter()
            .filter_map(|part| match part {
                LanguageModelToolResultContent::Text(text) => Some(text.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(text.chars().count(), 30_000);
        assert!(text.starts_with("start"));
        assert!(text.ends_with("end"));
        assert!(content.contains(&LanguageModelToolResultContent::Image(image)));
    }

    #[test]
    fn measured_cache_snapshots_replace_one_request_and_weight_by_input() {
        let mut totals = CacheUsageTotals::default();
        let mut previous = None;
        assert_eq!(totals.cache_read_percentage(), None);
        totals
            .update(
                &mut previous,
                LanguageModelCacheUsage {
                    input_tokens: 100,
                    cached_tokens: 0,
                },
            )
            .expect("known miss");
        totals
            .update(
                &mut previous,
                LanguageModelCacheUsage {
                    input_tokens: 100,
                    cached_tokens: 0,
                },
            )
            .expect("repeated snapshot");
        assert_eq!(
            totals,
            CacheUsageTotals {
                requests: 1,
                input_tokens: 100,
                cached_tokens: 0
            }
        );
        assert_eq!(totals.cache_read_percentage(), Some(0.0));
        let mut other_request = None;
        totals
            .update(
                &mut other_request,
                LanguageModelCacheUsage {
                    input_tokens: 900,
                    cached_tokens: 900,
                },
            )
            .expect("other request");
        assert_eq!(totals.cache_read_percentage(), Some(90.0));
        totals
            .update(
                &mut previous,
                LanguageModelCacheUsage {
                    input_tokens: 80,
                    cached_tokens: 20,
                },
            )
            .expect("corrected smaller snapshot");
        assert_eq!(
            totals,
            CacheUsageTotals {
                requests: 2,
                input_tokens: 980,
                cached_tokens: 920
            }
        );
    }

    #[test]
    fn measured_cache_errors_do_not_partially_update_totals_or_snapshot() {
        let mut totals = CacheUsageTotals::default();
        let mut previous = None;
        assert!(
            totals
                .update(
                    &mut previous,
                    LanguageModelCacheUsage {
                        input_tokens: 1,
                        cached_tokens: 2
                    }
                )
                .is_err()
        );
        assert_eq!(totals, CacheUsageTotals::default());
        assert_eq!(previous, None);
        totals.requests = u64::MAX;
        let original = totals;
        assert!(
            totals
                .update(
                    &mut previous,
                    LanguageModelCacheUsage {
                        input_tokens: 10,
                        cached_tokens: 5
                    }
                )
                .is_err()
        );
        assert_eq!(totals, original);
        assert_eq!(previous, None);
        totals = CacheUsageTotals {
            requests: 1,
            input_tokens: u64::MAX,
            cached_tokens: 0,
        };
        let original = totals;
        assert!(
            totals
                .update(
                    &mut previous,
                    LanguageModelCacheUsage {
                        input_tokens: 1,
                        cached_tokens: 0
                    }
                )
                .is_err()
        );
        assert_eq!(totals, original);
        assert_eq!(previous, None);
    }

    #[test]
    fn measured_cache_combined_is_weighted_and_checks_overflow() {
        let usage = MeasuredCacheUsage {
            agent: CacheUsageTotals {
                requests: 1,
                input_tokens: 100,
                cached_tokens: 0,
            },
            summary: CacheUsageTotals {
                requests: 1,
                input_tokens: 900,
                cached_tokens: 900,
            },
        };
        let combined = usage.combined().expect("combined");
        assert_eq!(combined.requests, 2);
        assert_eq!(combined.cache_read_percentage(), Some(90.0));
        let overflowing = MeasuredCacheUsage {
            agent: CacheUsageTotals {
                requests: u64::MAX,
                ..Default::default()
            },
            summary: CacheUsageTotals {
                requests: 1,
                ..Default::default()
            },
        };
        assert_eq!(overflowing.combined(), None);
    }

    #[gpui::test]
    async fn token_usage_without_explicit_cache_measurement_remains_unknown(
        cx: &mut gpui::TestAppContext,
    ) {
        let (thread, event_stream, _) = super::super::tests::setup_thread_for_test(cx).await;
        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                let (cancellation_tx, cancellation_rx) = watch::channel(false);
                thread.running_turn = Some(RunningTurn::new(
                    event_stream.clone(),
                    BTreeMap::default(),
                    cancellation_tx,
                    Task::ready(()),
                ));
                let mut cache_usage = None;
                thread
                    .handle_completion_event(
                        LanguageModelCompletionEvent::UsageUpdate(TokenUsage {
                            input_tokens: 100,
                            cache_read_input_tokens: 90,
                            ..Default::default()
                        }),
                        &event_stream,
                        cancellation_rx.clone(),
                        &mut cache_usage,
                        cx,
                    )
                    .expect("token usage");
                assert_eq!(thread.measured_cache_usage(), MeasuredCacheUsage::default());
                assert_eq!(
                    thread.measured_cache_usage().agent.cache_read_percentage(),
                    None
                );
                thread
                    .handle_completion_event(
                        LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                            input_tokens: 190,
                            cached_tokens: 0,
                        }),
                        &event_stream,
                        cancellation_rx,
                        &mut cache_usage,
                        cx,
                    )
                    .expect("known cache miss");
                assert_eq!(thread.measured_cache_usage().agent.requests, 1);
                assert_eq!(
                    thread.measured_cache_usage().agent.cache_read_percentage(),
                    Some(0.0)
                );
            })
        });
    }

    #[gpui::test(iterations = 20)]
    async fn cancellation_tool_output_stays_with_its_pending_message(
        cx: &mut gpui::TestAppContext,
    ) {
        struct CancellationOutputTool;

        impl AgentTool for CancellationOutputTool {
            type Input = serde_json::Value;
            type Output = String;
            const NAME: &'static str = "cancellation_output";

            fn kind() -> acp::ToolKind {
                acp::ToolKind::Other
            }
            fn initial_title(
                &self,
                _: Result<Self::Input, serde_json::Value>,
                _: &mut App,
            ) -> SharedString {
                "Cancellation output".into()
            }
            fn run(
                self: Arc<Self>,
                input: ToolInput<Self::Input>,
                event_stream: ToolCallEventStream,
                cx: &mut App,
            ) -> Task<Result<String, String>> {
                cx.spawn(async move |_| {
                    input.recv().await.map_err(|error| error.to_string())?;
                    event_stream.cancelled_by_user().await;
                    Err("partial output\nThe user stopped this command".into())
                })
            }
        }

        for supersede in [false, true] {
            let (thread, _, fake) = super::super::tests::setup_thread_for_test(cx).await;
            let model = fake.model("cancellation-output");
            let _old_events = cx.update(|cx| {
                let mut settings = AgentSettings::get_global(cx).clone();
                settings.tool_permissions.default = ToolPermissionMode::Allow;
                settings
                    .profiles
                    .get_mut(&thread.read(cx).profile_id)
                    .expect("test profile")
                    .tools
                    .insert(CancellationOutputTool::NAME.into(), true);
                AgentSettings::override_global(settings, cx);
                thread.update(cx, |thread, cx| {
                    thread.set_model(model.clone(), cx);
                    thread.title = Some("cancellation test".into());
                    thread.add_tool(CancellationOutputTool);
                    thread
                        .send(ClientUserMessageId::new(), ["old user"], cx)
                        .expect("old turn")
                })
            });
            cx.run_until_parked();
            let tool_id = LanguageModelToolUseId::from("canceled-tool");
            fake.send_last_event(
                &model,
                LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                    input_tokens: 100,
                    cached_tokens: 25,
                }),
            );
            fake.send_last_event(
                &model,
                LanguageModelCompletionEvent::ToolUse(LanguageModelToolUse {
                    id: tool_id.clone(),
                    name: CancellationOutputTool::NAME.into(),
                    raw_input: "{}".into(),
                    input: language_model::LanguageModelToolUseInput::Json(serde_json::json!({})),
                    is_input_complete: true,
                    thought_signature: None,
                }),
            );
            fake.end_last(&model);
            cx.run_until_parked();
            thread.read_with(cx, |thread, _| {
                assert!(
                    thread
                        .pending_message
                        .as_ref()
                        .expect("pending tool")
                        .content
                        .iter()
                        .any(|content| matches!(content,
                        AgentMessageContent::ToolUse(tool_use) if tool_use.id == tool_id))
                );
            });

            if supersede {
                let _new_events = cx.update(|cx| {
                    thread.update(cx, |thread, cx| {
                        thread
                            .send(ClientUserMessageId::new(), ["new user"], cx)
                            .expect("new turn")
                    })
                });
                cx.run_until_parked();
                fake.send_last_text(&model, "new reply");
                fake.send_last_event(
                    &model,
                    LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                        input_tokens: 200,
                        cached_tokens: 100,
                    }),
                );
                fake.end_last(&model);
                cx.run_until_parked();
                thread.read_with(cx, |thread, _| {
                    let message = thread
                        .messages
                        .last()
                        .and_then(|message| message.as_agent_message())
                        .expect("new reply");
                    assert_eq!(
                        message.content,
                        vec![AgentMessageContent::Text("new reply".into())]
                    );
                    assert!(message.tool_results.is_empty());
                    assert_eq!(
                        thread.measured_cache_usage().agent,
                        CacheUsageTotals {
                            requests: 2,
                            input_tokens: 300,
                            cached_tokens: 125,
                        }
                    );
                });
            } else {
                let cancellation =
                    cx.update(|cx| thread.update(cx, |thread, cx| thread.cancel(cx)));
                cancellation.await;
                thread.read_with(cx, |thread, _| {
                    let message = thread
                        .messages
                        .last()
                        .and_then(|message| message.as_agent_message())
                        .expect("canceled tool message");
                    let result = message
                        .tool_results
                        .get(&tool_id)
                        .expect("canceled tool output");
                    assert!(result.text_contents().contains("partial output"));
                    assert!(
                        result
                            .text_contents()
                            .contains("The user stopped this command")
                    );
                    assert_eq!(
                        thread.measured_cache_usage().agent,
                        CacheUsageTotals {
                            requests: 1,
                            input_tokens: 100,
                            cached_tokens: 25,
                        }
                    );
                });
            }
        }
    }

    #[gpui::test]
    async fn cancellation_drops_pending_provider_startup(cx: &mut gpui::TestAppContext) {
        use language_model::{
            LanguageModelClient, LanguageModelProvider, LanguageModelProviderState,
        };

        struct PendingStartupProvider(
            Arc<language_model::fake_provider::FakeLanguageModelProvider>,
        );

        impl LanguageModelClient for PendingStartupProvider {
            fn stream_completion(
                &self,
                model: &LanguageModel,
                request: LanguageModelRequest,
                cx: &AsyncApp,
            ) -> futures::future::BoxFuture<
                'static,
                Result<language_model::LanguageModelCompletionStream, LanguageModelCompletionError>,
            > {
                let events = self.0.stream_completion(model, request, cx);
                async move {
                    std::future::pending::<()>().await;
                    events.await
                }
                .boxed()
            }
        }

        impl LanguageModelProvider for PendingStartupProvider {
            fn id(&self) -> LanguageModelProviderId {
                self.0.id()
            }
            fn name(&self) -> language_model::LanguageModelProviderName {
                self.0.name()
            }
            fn default_model(&self, cx: &App) -> Option<LanguageModel> {
                self.0.default_model(cx)
            }
            fn default_fast_model(&self, cx: &App) -> Option<LanguageModel> {
                self.0.default_fast_model(cx)
            }
            fn provided_models(&self, cx: &App) -> Vec<LanguageModel> {
                self.0.provided_models(cx)
            }
            fn is_authenticated(&self, cx: &App) -> bool {
                self.0.is_authenticated(cx)
            }
            fn authenticate(
                &self,
                cx: &mut App,
            ) -> Task<Result<(), language_model::AuthenticateError>> {
                self.0.authenticate(cx)
            }
            fn settings_view(&self, cx: &mut App) -> Option<language_model::ProviderSettingsView> {
                self.0.settings_view(cx)
            }
        }

        impl LanguageModelProviderState for PendingStartupProvider {
            type ObservableEntity = ();
            fn observable_entity(&self) -> Option<Entity<()>> {
                None
            }
        }

        let (thread, _, fake) = super::super::tests::setup_thread_for_test(cx).await;
        let model = fake.model("pending-startup");
        let _events = cx.update(|cx| {
            LanguageModelRegistry::global(cx).update(cx, |registry, cx| {
                registry.register_provider(Arc::new(PendingStartupProvider(fake.clone())), cx);
            });
            thread.update(cx, |thread, cx| {
                thread.set_model(model.clone(), cx);
                thread
                    .send(ClientUserMessageId::new(), ["user"], cx)
                    .expect("turn")
            })
        });
        cx.run_until_parked();
        let request = fake
            .pending_completions_for(&model)
            .pop()
            .expect("pending startup");
        assert!(!fake.is_stream_closed(&model, &request));
        let cancellation = cx.update(|cx| thread.update(cx, |thread, cx| thread.cancel(cx)));
        cx.run_until_parked();
        assert!(fake.is_stream_closed(&model, &request));
        cancellation.await;
        thread.read_with(cx, |thread, _| {
            assert!(thread.running_turn.is_none());
            assert_eq!(thread.measured_cache_usage(), MeasuredCacheUsage::default());
        });
    }

    #[gpui::test]
    async fn agent_cache_snapshots_are_request_local_and_ignore_stale_events(
        cx: &mut gpui::TestAppContext,
    ) {
        let (thread, old_stream, _) = super::super::tests::setup_thread_for_test(cx).await;
        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                let (old_sender, old_receiver) = watch::channel(false);
                thread.running_turn = Some(RunningTurn::new(
                    old_stream.clone(),
                    BTreeMap::default(),
                    old_sender,
                    Task::ready(()),
                ));
                let mut old_usage = None;
                thread
                    .handle_completion_event(
                        LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                            input_tokens: 100,
                            cached_tokens: 25,
                        }),
                        &old_stream,
                        old_receiver.clone(),
                        &mut old_usage,
                        cx,
                    )
                    .expect("old snapshot");

                let (events, _receiver) = mpsc::unbounded();
                let new_stream = ThreadEventStream::new(events);
                let (new_sender, new_receiver) = watch::channel(false);
                thread.running_turn = Some(RunningTurn::new(
                    new_stream.clone(),
                    BTreeMap::default(),
                    new_sender,
                    Task::ready(()),
                ));
                let mut new_usage = None;
                thread
                    .handle_completion_event(
                        LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                            input_tokens: 200,
                            cached_tokens: 100,
                        }),
                        &new_stream,
                        new_receiver.clone(),
                        &mut new_usage,
                        cx,
                    )
                    .expect("new snapshot");
                for event in [
                    LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                        input_tokens: 999,
                        cached_tokens: 999,
                    }),
                    LanguageModelCompletionEvent::Text("stale reply".into()),
                    LanguageModelCompletionEvent::UsageUpdate(TokenUsage {
                        input_tokens: 999,
                        ..Default::default()
                    }),
                ] {
                    thread
                        .handle_completion_event(
                            event,
                            &old_stream,
                            old_receiver.clone(),
                            &mut old_usage,
                            cx,
                        )
                        .expect("ignore stale turn");
                }
                assert_eq!(
                    old_usage,
                    Some(LanguageModelCacheUsage {
                        input_tokens: 100,
                        cached_tokens: 25
                    })
                );
                assert!(thread.pending_message.is_none());
                assert_eq!(thread.cumulative_token_usage().input_tokens, 0);
                for event in [
                    LanguageModelCompletionEvent::Stop(StopReason::EndTurn),
                    LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                        input_tokens: 250,
                        cached_tokens: 150,
                    }),
                ] {
                    thread
                        .handle_completion_event(
                            event,
                            &new_stream,
                            new_receiver.clone(),
                            &mut new_usage,
                            cx,
                        )
                        .expect("trailing snapshot from active request");
                }
                thread
                    .running_turn
                    .as_mut()
                    .expect("active turn")
                    .cancellation_tx
                    .send(true)
                    .expect("cancel active request");
                thread
                    .handle_completion_event(
                        LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                            input_tokens: 999,
                            cached_tokens: 999,
                        }),
                        &new_stream,
                        new_receiver,
                        &mut new_usage,
                        cx,
                    )
                    .expect("ignore canceled request");
                assert_eq!(
                    new_usage,
                    Some(LanguageModelCacheUsage {
                        input_tokens: 250,
                        cached_tokens: 150
                    })
                );
                assert_eq!(
                    thread.measured_cache_usage().agent,
                    CacheUsageTotals {
                        requests: 2,
                        input_tokens: 350,
                        cached_tokens: 175,
                    }
                );
            })
        });
    }

    #[gpui::test(iterations = 20)]
    async fn canceled_event_batch_does_not_change_new_request_or_its_trailing_cache(
        cx: &mut gpui::TestAppContext,
    ) {
        let (thread, _, fake) = super::super::tests::setup_thread_for_test(cx).await;
        let model = fake.model("cache-batches");
        let _old_events = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread.set_model(model.clone(), cx);
                thread.title = Some("cache test".into());
                thread
                    .send(ClientUserMessageId::new(), ["old user"], cx)
                    .expect("old turn")
            })
        });
        cx.run_until_parked();
        let old_request = fake
            .pending_completions_for(&model)
            .pop()
            .expect("old request");
        fake.send_event(
            &model,
            &old_request,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 100,
                cached_tokens: 25,
            }),
        );
        cx.run_until_parked();
        for event in [
            LanguageModelCompletionEvent::Text("late canceled reply".into()),
            LanguageModelCompletionEvent::UsageUpdate(TokenUsage {
                input_tokens: 999,
                ..Default::default()
            }),
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 999,
                cached_tokens: 999,
            }),
        ] {
            fake.send_event(&model, &old_request, event);
        }
        let _new_events = cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread
                    .send(ClientUserMessageId::new(), ["new user"], cx)
                    .expect("new turn")
            })
        });
        cx.run_until_parked();
        assert!(fake.is_stream_closed(&model, &old_request));
        let new_request = fake
            .pending_completions_for(&model)
            .pop()
            .expect("new request");
        fake.send_event(
            &model,
            &new_request,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 200,
                cached_tokens: 100,
            }),
        );
        cx.run_until_parked();
        fake.send_text(&model, &new_request, "new reply");
        fake.send_event(
            &model,
            &new_request,
            LanguageModelCompletionEvent::Stop(StopReason::EndTurn),
        );
        cx.run_until_parked();
        fake.send_event(
            &model,
            &new_request,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 250,
                cached_tokens: 150,
            }),
        );
        fake.send_event(
            &model,
            &new_request,
            LanguageModelCompletionEvent::UsageUpdate(TokenUsage {
                input_tokens: 250,
                ..Default::default()
            }),
        );
        fake.end_stream(&model, &new_request);
        fake.end_stream(&model, &old_request);
        cx.run_until_parked();
        thread.read_with(cx, |thread, _| {
            assert_eq!(
                thread.measured_cache_usage().agent,
                CacheUsageTotals {
                    requests: 2,
                    input_tokens: 350,
                    cached_tokens: 175,
                }
            );
            assert_eq!(thread.cumulative_token_usage().input_tokens, 250);
            assert!(thread.messages.iter().all(|message| {
                !archive_records(message)
                    .expect("archive records")
                    .iter()
                    .any(|record| record.text.contains("late canceled reply"))
            }));
            assert!(thread.running_turn.is_none());
        });
    }

    #[gpui::test(iterations = 20)]
    async fn failed_summary_retry_keeps_each_requests_latest_cache_snapshot(
        cx: &mut gpui::TestAppContext,
    ) {
        let (thread, _, fake) = super::super::tests::setup_thread_for_test(cx).await;
        let model = fake.model("memory-retry");
        let summary_model = model.clone();
        let weak_thread = thread.downgrade();
        let task = cx.update(|cx| {
            cx.spawn(async move |cx| {
                summarize_node(
                    summary_model,
                    LanguageModelRequest::default(),
                    SummaryJob {
                        key: crate::infinite_context::NodeKey { l: 0, i: 0 },
                        input: "source input".into(),
                        task: "summarize".into(),
                        end: 1,
                    },
                    weak_thread,
                    cx,
                )
                .await
            })
        });
        cx.run_until_parked();
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 100,
                cached_tokens: 20,
            }),
        );
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 90,
                cached_tokens: 10,
            }),
        );
        fake.send_last_error(
            &model,
            LanguageModelCompletionError::ApiReadResponseError {
                provider: model.provider_name.clone(),
                error: std::io::Error::new(
                    std::io::ErrorKind::ConnectionReset,
                    "interrupted response",
                ),
            },
        );
        fake.end_last(&model);
        cx.run_until_parked();
        cx.background_executor
            .advance_clock(crate::maximum_retry_delay_with_jitter(BASE_RETRY_DELAY));
        cx.run_until_parked();
        assert_eq!(fake.pending_completions_for(&model).len(), 1);
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::Stop(StopReason::Refusal),
        );
        fake.send_last_event(
            &model,
            LanguageModelCompletionEvent::CacheUsageUpdate(LanguageModelCacheUsage {
                input_tokens: 200,
                cached_tokens: 100,
            }),
        );
        fake.end_last(&model);
        cx.run_until_parked();
        assert!(
            fake.pending_completions_for(&model).is_empty(),
            "A summary refusal must not start another request"
        );
        assert!(
            task.is_ready(),
            "A summary refusal must finish without another retry timer"
        );
        let error = task.await.expect_err("summary refusal");
        assert!(error.to_string().contains("refused the memory task"));
        thread.read_with(cx, |thread, _| {
            assert_eq!(
                thread.measured_cache_usage().summary,
                CacheUsageTotals {
                    requests: 2,
                    input_tokens: 290,
                    cached_tokens: 110
                }
            );
            assert_eq!(
                thread.measured_cache_usage().agent,
                CacheUsageTotals::default()
            );
        });
    }
}
