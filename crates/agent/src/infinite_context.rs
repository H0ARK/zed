use anyhow::{Context as _, Result, bail, ensure};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

const MESSAGE_BYTES: usize = 30_000;
const ECHO_CHARS: usize = 30_000;
const ECHO_OMISSION: &str = "[middle omitted]";
const SUMMARY_BYTES: usize = 512;
const MAX_IN_FLIGHT_JOBS: usize = 8;
const VIEW_HIGH: usize = 128_000;
const VIEW_LOW: usize = 64_000;
const CONTEXT_HIGH: usize = 32_000;
const CONTEXT_LOW: usize = 16_000;
// The 32,000-byte hard limit needs headroom for eight 512-byte completions plus
// their tags; 5,000 bytes covers that even at the maximum message index width.
const CONTEXT_TRIGGER: usize = 27_000;
const SUMMARY_TASK: &str = "Summarize this chat interval faithfully, preserving names, decisions, constraints, unresolved questions, and useful facts. Return only the summary, with no more than 512 UTF-8 bytes (bytes, not characters).";

/// `i` is a zero-based message index, aligned to the interval's `2^l` length.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct NodeKey {
    pub l: u32,
    pub i: u64,
}

impl NodeKey {
    fn span(self) -> Result<u64> {
        let span = 1u64.checked_shl(self.l).context("node level exceeds 63")?;
        ensure!(self.i % span == 0, "unaligned node {}+{}", self.i, span);
        Ok(span)
    }

    fn end(self) -> Result<u64> {
        self.i
            .checked_add(self.span()?)
            .context("node interval overflows")
    }

    fn parent(self) -> Option<Self> {
        let l = self.l.checked_add(1)?;
        let span = 1u64.checked_shl(l)?;
        let parent = Self {
            l,
            i: self.i / span * span,
        };
        parent.i.checked_add(span)?;
        Some(parent)
    }

    fn children(self) -> Result<[Self; 2]> {
        self.span()?;
        let l = self.l.checked_sub(1).context("a leaf has no children")?;
        let half = 1u64.checked_shl(l).context("invalid child level")?;
        Ok([
            Self { l, i: self.i },
            Self {
                l,
                i: self.i.checked_add(half).context("child index overflows")?,
            },
        ])
    }
}

#[derive(Clone, Debug)]
pub struct SummaryJob {
    pub key: NodeKey,
    pub input: String,
    pub task: String,
    /// Exclusive message boundary of the input interval.
    pub end: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Message {
    i: u64,
    kind: String,
    text: String,
    size: usize,
    date: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Node {
    l: u32,
    i: u64,
    text: String,
    size: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ImageRecord {
    i: u64,
    images: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    main_batch: bool,
    context_batch: bool,
    source_checkpoint: Option<u32>,
}

/// A synchronous, exclusively locked store. The parent owns async model calls.
///
/// A drain turn ends when `next_job` returns `None`; retries become eligible on
/// the following call. Dropping this object releases the filesystem lock.
pub struct InfiniteContext {
    path: PathBuf,
    _lock: File,
    messages: Vec<Message>,
    images: BTreeMap<u64, Vec<serde_json::Value>>,
    nodes: BTreeMap<NodeKey, Node>,
    view: Vec<NodeKey>,
    context_view: Vec<NodeKey>,
    summarized_end: u64,
    built_end: u64,
    ready: BTreeSet<(u64, u32)>,
    ready_leaves: BTreeSet<u64>,
    admitted_leaves: BTreeSet<u64>,
    unbuilt_leaves: BTreeSet<u64>,
    metadata: Metadata,
    resume_batches: bool,
    queued: BTreeSet<NodeKey>,
    in_flight: BTreeSet<NodeKey>,
    deferred: VecDeque<NodeKey>,
    deferred_keys: BTreeSet<NodeKey>,
    poison: Option<String>,
}

impl InfiniteContext {
    pub fn open(path: PathBuf) -> Result<Self> {
        fs::create_dir_all(&path)
            .with_context(|| format!("create memory store {}", path.display()))?;
        let lock = lock_store(&path.join("lock"))?;
        fs::create_dir_all(path.join("main"))?;
        fs::create_dir_all(path.join("tree"))?;
        fs::create_dir_all(path.join("images"))?;
        sync_directory(&path)?;

        let mut messages_by_id = BTreeMap::new();
        for file in journal_files(&path.join("main"))? {
            let day = file
                .file_stem()
                .and_then(|s| s.to_str())
                .context("invalid main journal name")?;
            let parsed_day = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
                .context("invalid daily journal name")?;
            ensure!(
                parsed_day.format("%Y-%m-%d").to_string() == day,
                "noncanonical daily journal name"
            );
            let records: Vec<Message> = read_journal(&file)?;
            let mut previous = None;
            for message in records {
                ensure!(
                    previous.is_none_or(|i| i < message.i),
                    "out-of-order daily journal"
                );
                previous = Some(message.i);
                ensure!(
                    message.date.format("%Y-%m-%d").to_string() == day,
                    "message is in the wrong daily journal: {}",
                    file.display()
                );
                ensure!(
                    message.size == message.text.len(),
                    "incorrect message size at {}",
                    message.i
                );
                ensure!(
                    !message.kind.is_empty(),
                    "empty message kind at {}",
                    message.i
                );
                ensure!(
                    message.size <= MESSAGE_BYTES,
                    "oversized non-tool message at {}",
                    message.i
                );
                ensure!(
                    messages_by_id.insert(message.i, message).is_none(),
                    "duplicate message index"
                );
            }
        }
        let mut messages = Vec::with_capacity(messages_by_id.len());
        for (i, message) in messages_by_id {
            ensure!(
                i == u64::try_from(messages.len())?,
                "gap in main journal at {}",
                messages.len()
            );
            messages.push(message);
        }
        let count = u64::try_from(messages.len())?;

        let mut images = BTreeMap::new();
        for file in journal_files(&path.join("images"))? {
            journal_day(&file)?;
            let records: Vec<ImageRecord> = read_journal(&file)?;
            for record in records {
                ensure!(
                    !record.images.is_empty(),
                    "empty image attachment record at {}",
                    record.i
                );
                if let Some(existing) = images.get(&record.i) {
                    ensure!(
                        existing == &record.images,
                        "conflicting image attachments at {}",
                        record.i
                    );
                } else {
                    images.insert(record.i, record.images);
                }
            }
        }
        // An attachment is flushed before its raw record, so a crash can leave
        // an orphan. Keep it immutable for an identical retry, without creating
        // a message, advancing a frontier, or exposing it through zoom.

        let mut nodes = BTreeMap::new();
        for file in journal_files(&path.join("tree"))? {
            journal_day(&file)?;
            let records: Vec<Node> = read_journal(&file)?;
            for node in records {
                let key = NodeKey {
                    l: node.l,
                    i: node.i,
                };
                ensure!(key.end()? <= count, "node extends past main journal");
                ensure!(!node.text.trim().is_empty(), "empty summary node {key:?}");
                ensure!(
                    node.size == node.text.len() && node.size <= SUMMARY_BYTES,
                    "invalid summary size for {key:?}"
                );
                ensure!(
                    nodes.insert(key, node).is_none(),
                    "duplicate immutable node {key:?}"
                );
            }
        }
        for key in nodes.keys() {
            if key.l > 0 {
                for child in key.children()? {
                    ensure!(
                        nodes.contains_key(&child),
                        "missing child {child:?} of {key:?}"
                    );
                }
            }
        }

        let view_path = path.join("view.json");
        let context_path = path.join("context.json");
        // A missing frontier is not permission to replay the summary tree.
        if !view_path.exists() || !context_path.exists() {
            ensure!(
                messages.is_empty() && nodes.is_empty(),
                "missing persisted memory frontier"
            );
            if !view_path.exists() {
                persist_view(&view_path, &[])?;
            }
            if !context_path.exists() {
                persist_view(&context_path, &[])?;
            }
        }
        let mut view = load_view(&view_path)?;
        let context_view = load_view(&context_path)?;
        let view_end = validate_view(&view, count, &nodes, false)?;
        let summarized_end = validate_view(&context_view, count, &nodes, true)?;
        ensure!(
            summarized_end <= view_end,
            "context frontier extends past persisted live frontier"
        );
        for key in &view {
            if key.l > 0 {
                ensure!(
                    key.end()? <= summarized_end,
                    "live summary extends past context frontier"
                );
            }
        }
        for key in nodes.keys() {
            ensure!(
                key.end()? <= view_end,
                "summary exists beyond persisted live frontier"
            );
        }
        // Only the raw append suffix may be recovered after an interrupted append.
        for i in view_end..count {
            view.push(NodeKey { l: 0, i });
        }
        if view_end < count {
            persist_view(&view_path, &view)?;
        }

        let metadata_path = path.join("metadata.json");
        let metadata = match File::open(&metadata_path) {
            Ok(file) => serde_json::from_reader(file).context("invalid memory metadata")?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let metadata = Metadata::default();
                persist_json(&metadata_path, &metadata)?;
                metadata
            }
            Err(error) => return Err(error).context("read memory metadata"),
        };
        let unbuilt_leaves = (0..count)
            .filter(|i| !nodes.contains_key(&NodeKey { l: 0, i: *i }))
            .collect();
        let mut store = Self {
            path,
            _lock: lock,
            messages,
            images,
            nodes,
            view,
            context_view,
            summarized_end,
            built_end: summarized_end,
            ready: BTreeSet::new(),
            ready_leaves: BTreeSet::new(),
            admitted_leaves: BTreeSet::new(),
            unbuilt_leaves,
            resume_batches: metadata.main_batch || metadata.context_batch,
            metadata,
            queued: BTreeSet::new(),
            in_flight: BTreeSet::new(),
            deferred: VecDeque::new(),
            deferred_keys: BTreeSet::new(),
            poison: None,
        };
        store.advance_built_prefix()?;
        for i in 0..count {
            store.enqueue(NodeKey { l: 0, i });
        }
        // Startup indexes existing records once; completion uses only sibling lookups.
        let parents: BTreeSet<_> = store.nodes.keys().filter_map(|key| key.parent()).collect();
        for parent in parents {
            if store.children_ready(parent)? {
                store.enqueue(parent);
            }
        }
        Ok(store)
    }

    /// Only `echo` output is clipped, once before journaling. All kinds, including
    /// tool JSON, are split losslessly into at most 30,000-byte UTF-8 records.
    pub fn append(&mut self, kind: &str, text: &str, date: DateTime<Utc>) -> Result<()> {
        self.healthy()?;
        ensure!(
            !self.images.contains_key(&self.message_count()),
            "orphan attachments reserve the next message; retry append_with_images with identical images"
        );
        self.append_text(kind, text, date, None)
    }

    /// Images remain opaque JSON values and are durable before their first raw
    /// message. Only that first message carries the attachment annotation.
    pub fn append_with_images(
        &mut self,
        kind: &str,
        text: &str,
        images: Vec<serde_json::Value>,
        date: DateTime<Utc>,
    ) -> Result<()> {
        self.healthy()?;
        ensure!(!kind.is_empty(), "message kind must not be empty");
        if images.is_empty() {
            return self.append(kind, text, date);
        }
        let i = self.message_count();
        i.checked_add(1).context("message count overflows")?;
        let annotation = format!(" [{} images attached: zoom to view]", images.len());
        if let Some(existing) = self.images.get(&i) {
            ensure!(existing == &images, "conflicting image attachments at {i}");
        } else {
            let record = ImageRecord { i, images };
            let journal = self
                .path
                .join("images")
                .join(format!("{}.jsonl", date.format("%Y-%m-%d")));
            let result = append_record(&journal, &record);
            self.durable(result)?;
            self.images.insert(i, record.images);
        }
        self.append_text(kind, text, date, Some(&annotation))
    }

    fn append_text(
        &mut self,
        kind: &str,
        text: &str,
        date: DateTime<Utc>,
        annotation: Option<&str>,
    ) -> Result<()> {
        self.healthy()?;
        ensure!(!kind.is_empty(), "message kind must not be empty");
        let clipped = (kind == "echo").then(|| echo(text));
        let text = match &clipped {
            Some(clipped) => clipped.as_str(),
            None => text,
        };
        let mut chunks = Vec::new();
        let remaining = if let Some(annotation) = annotation {
            let budget = MESSAGE_BYTES
                .checked_sub(annotation.len())
                .context("image annotation exceeds message budget")?;
            let end = utf8_prefix_len(text, budget);
            chunks.push(Cow::Owned(format!("{}{annotation}", &text[..end])));
            &text[end..]
        } else {
            text
        };
        if !remaining.is_empty() || chunks.is_empty() {
            chunks.extend(
                utf8_chunks(remaining, MESSAGE_BYTES)
                    .into_iter()
                    .map(Cow::Borrowed),
            );
        }
        let start = self.message_count();
        let end = start
            .checked_add(u64::try_from(chunks.len())?)
            .context("message count overflows")?;
        for (id, _) in self.images.range(start..end) {
            ensure!(
                annotation.is_some() && *id == start,
                "orphan attachments reserve message {id}; retry append_with_images with identical images"
            );
        }
        for chunk in chunks {
            let i = self.message_count();
            i.checked_add(1).context("message count overflows")?;
            let message = Message {
                i,
                kind: kind.to_owned(),
                size: chunk.len(),
                text: chunk.into_owned(),
                date,
            };
            let journal = self
                .path
                .join("main")
                .join(format!("{}.jsonl", date.format("%Y-%m-%d")));
            let result = append_record(&journal, &message);
            self.durable(result)?;
            self.messages.push(message);
            self.unbuilt_leaves.insert(i);
            self.view.push(NodeKey { l: 0, i });
            let result = persist_view(&self.path.join("view.json"), &self.view);
            self.durable(result)?;
            self.enqueue(NodeKey { l: 0, i });
        }
        Ok(())
    }

    pub fn message_count(&self) -> u64 {
        // Message indices are checked before insertion and when loading the store.
        self.messages.len() as u64
    }

    pub fn next_job(&mut self) -> Result<Option<SummaryJob>> {
        self.healthy()?;
        // Resume an explicitly persisted batch, not a frontier reconstructed on
        // open. Its last required parent may already be durable after a crash.
        if self.resume_batches {
            self.advance_views()?;
            self.resume_batches = false;
        }
        while self.in_flight.len() < MAX_IN_FLIGHT_JOBS {
            let Some(key) = self.peek_ready() else { break };
            if self.nodes.contains_key(&key) {
                self.remove_ready(key);
                continue;
            }
            let input = self.job_input(key)?;
            let end = key.end()?;
            if input.len() > SUMMARY_BYTES {
                let boundary = if key.l == 0 { key.i } else { end };
                let keys = self.context_keys_before(boundary)?;
                if self.frontier_size(&keys, boundary.min(self.built_end))? > CONTEXT_HIGH {
                    // The key remains queued. Existing claims may supply parents
                    // needed to shrink its context before it can be dispatched.
                    break;
                }
            }
            self.remove_ready(key);
            if key.l == 0 {
                self.admitted_leaves.insert(key.i);
            }
            self.in_flight.insert(key);
            if input.len() <= SUMMARY_BYTES {
                self.complete_job(key, input)?;
                continue;
            }
            return Ok(Some(SummaryJob {
                key,
                input,
                task: SUMMARY_TASK.to_owned(),
                end,
            }));
        }
        // End this turn before making deferred failures available to the next one.
        while let Some(key) = self.deferred.pop_front() {
            self.deferred_keys.remove(&key);
            self.enqueue(key);
        }
        Ok(None)
    }

    pub fn complete_job(&mut self, key: NodeKey, text: String) -> Result<()> {
        self.healthy()?;
        key.span()?;
        ensure!(!text.trim().is_empty(), "summary must not be empty");
        ensure!(
            self.in_flight.contains(&key),
            "summary job is not in flight: {key:?}"
        );
        ensure!(
            text.len() <= SUMMARY_BYTES,
            "summary exceeds the 512-byte ruler"
        );
        ensure!(
            !self.nodes.contains_key(&key),
            "summary node is immutable: {key:?}"
        );
        let node = Node {
            l: key.l,
            i: key.i,
            size: text.len(),
            text,
        };
        let result = append_record(
            &self
                .path
                .join("tree")
                .join(format!("{}.jsonl", Utc::now().format("%Y-%m-%d"))),
            &node,
        );
        self.durable(result)?;
        self.nodes.insert(key, node);
        if key.l == 0 {
            self.unbuilt_leaves.remove(&key.i);
            self.admitted_leaves.remove(&key.i);
        }
        self.advance_built_prefix()?;
        self.in_flight.remove(&key);
        if let Some(parent) = key.parent() {
            if self.children_ready(parent)? {
                self.enqueue(parent);
            }
        }
        self.advance_views()
    }

    /// Repeated requests are idempotent. A retry is never returned in this drain turn.
    pub fn retry_job(&mut self, key: NodeKey) -> Result<()> {
        self.healthy()?;
        if self.deferred_keys.contains(&key) {
            return Ok(());
        }
        ensure!(
            self.in_flight.remove(&key),
            "summary job is not in flight: {key:?}"
        );
        if self.deferred_keys.insert(key) {
            self.deferred.push_back(key);
        }
        Ok(())
    }

    /// Call after cancelling the parent's model futures. Claimed work is
    /// immediately ready again, unlike failed jobs deferred by `retry_job`.
    pub fn release_in_flight_jobs(&mut self) -> Result<()> {
        self.healthy()?;
        for key in std::mem::take(&mut self.in_flight) {
            self.enqueue(key);
        }
        Ok(())
    }

    /// Bounded summary-only context; intervals crossing `end` are opened into
    /// already-built children. Unbuilt messages never get synthetic placeholders.
    pub fn render_view_before(&self, end: u64) -> Result<String> {
        self.healthy()?;
        ensure!(
            end <= self.message_count(),
            "context boundary exceeds message count"
        );
        let keys = self.context_keys_before(end)?;
        self.render_keys(&keys, CONTEXT_HIGH)
    }

    fn context_keys_before(&self, end: u64) -> Result<Vec<NodeKey>> {
        let end = end.min(self.built_end);
        let mut keys = Vec::new();
        for key in &self.context_view {
            self.before_boundary(*key, end, &mut keys)?;
        }
        // A node append may survive a crash before context.json is replaced.
        // It is usable as model context without replaying or changing live view.
        for i in self.summarized_end..end {
            keys.push(NodeKey { l: 0, i });
        }
        let mut batching = false;
        self.compact_frontier(keys, (CONTEXT_HIGH, CONTEXT_HIGH), end, &mut batching)
    }

    pub fn render_view(&self) -> Result<String> {
        self.healthy()?;
        self.render_main_before(self.built_end)
    }

    /// Request context uses the main frontier and requires every prior leaf.
    pub fn render_turn_before(&self, end: u64) -> Result<String> {
        self.healthy()?;
        ensure!(
            end <= self.message_count(),
            "turn boundary exceeds message count"
        );
        ensure!(
            self.all_summarized_before(end),
            "previous messages are not all summarized"
        );
        self.render_main_before(end)
    }

    /// Call after all appends for these finished source messages succeed. A
    /// crash before checkpointing can duplicate records, but cannot lose them.
    pub fn checkpoint_source(&mut self, source: u32) -> Result<()> {
        self.healthy()?;
        let mut metadata = self.metadata.clone();
        metadata.source_checkpoint = Some(source);
        self.save_metadata(metadata)
    }

    pub fn source_checkpoint(&self) -> Option<u32> {
        self.metadata.source_checkpoint
    }

    fn render_main_before(&self, end: u64) -> Result<String> {
        let mut keys = Vec::new();
        for key in &self.view {
            self.before_boundary(*key, end, &mut keys)?;
        }
        self.render_keys(&keys, VIEW_HIGH)
    }

    pub fn all_summarized_before(&self, end: u64) -> bool {
        self.poison.is_none() && end <= self.built_end
    }

    /// For a summarized interval, opens its two children. For a raw interval,
    /// opens the original message. `page` is zero-based and pages are UTF-8 safe.
    pub fn zoom(&self, id: u64, n: u64, page: usize) -> Result<String> {
        self.healthy()?;
        ensure!(
            n.is_power_of_two(),
            "zoom span must be a nonzero power of two"
        );
        let key = NodeKey {
            l: n.trailing_zeros(),
            i: id,
        };
        ensure!(
            key.end()? <= self.message_count(),
            "zoom interval exceeds message count"
        );
        let mut text = String::new();
        if key.l == 0 {
            let message = self.message(id)?;
            text.push_str(&format!("{}+1|{}: {}", id, message.kind, message.text));
        } else {
            ensure!(
                self.nodes.contains_key(&key),
                "zoom node is not built: {key:?}"
            );
            for child in key.children()? {
                let node = self.nodes.get(&child).context("zoom child is not built")?;
                push_tagged(&mut text, child, &node.text)?;
            }
        }
        let pages = utf8_chunks(&text, MESSAGE_BYTES);
        Ok(pages
            .get(page)
            .context("zoom page is out of range")?
            .to_string())
    }

    /// The parent may deserialize these values into its own image type. An
    /// ordinary message (including later split chunks) has no attached images.
    pub fn zoom_images(&self, id: u64) -> Result<Vec<serde_json::Value>> {
        self.healthy()?;
        self.message(id)?;
        Ok(self.images.get(&id).map_or_else(Vec::new, Clone::clone))
    }

    pub fn date(&self, id: u64) -> Result<String> {
        self.healthy()?;
        Ok(self.message(id)?.date.to_rfc3339())
    }

    fn advance_built_prefix(&mut self) -> Result<()> {
        while self.built_end < self.message_count()
            && self.nodes.contains_key(&NodeKey {
                l: 0,
                i: self.built_end,
            })
        {
            self.built_end = self
                .built_end
                .checked_add(1)
                .context("built boundary overflows")?;
        }
        Ok(())
    }

    fn healthy(&self) -> Result<()> {
        if let Some(error) = &self.poison {
            bail!("memory persistence failed; drop and reopen the store: {error}");
        }
        Ok(())
    }

    fn durable(&mut self, result: Result<()>) -> Result<()> {
        if let Err(error) = result {
            self.poison = Some(format!("{error:#}"));
            return Err(error);
        }
        Ok(())
    }

    fn message(&self, i: u64) -> Result<&Message> {
        self.messages
            .get(usize::try_from(i)?)
            .context("message index is out of range")
    }

    fn enqueue(&mut self, key: NodeKey) {
        if !self.nodes.contains_key(&key)
            && !self.in_flight.contains(&key)
            && !self.deferred_keys.contains(&key)
            && self.queued.insert(key)
        {
            if key.l == 0 {
                self.ready_leaves.insert(key.i);
            } else {
                self.ready.insert((key.i, key.l));
            }
        }
    }

    fn peek_ready(&self) -> Option<NodeKey> {
        if let Some((i, l)) = self.ready.first().copied() {
            return Some(NodeKey { l, i });
        }
        // Cancellation or a failed call must not strand a previously admitted
        // leaf which can supply a missing sibling. New leaves wait for the batch.
        let i = if self.metadata.context_batch {
            self.admitted_leaves
                .iter()
                .find(|i| self.ready_leaves.contains(i))
                .copied()?
        } else {
            self.ready_leaves.first().copied()?
        };
        // In-flight and deferred leaves remain unbuilt. Only the first eight
        // missing leaves may start, even when later leaves complete out of order.
        let ninth_missing = self.unbuilt_leaves.iter().nth(8).copied();
        ninth_missing
            .is_none_or(|boundary| i < boundary)
            .then_some(NodeKey { l: 0, i })
    }

    fn remove_ready(&mut self, key: NodeKey) {
        self.queued.remove(&key);
        if key.l == 0 {
            self.ready_leaves.remove(&key.i);
        } else {
            self.ready.remove(&(key.i, key.l));
        }
    }

    fn children_ready(&self, key: NodeKey) -> Result<bool> {
        Ok(key
            .children()?
            .iter()
            .all(|child| self.nodes.contains_key(child)))
    }

    fn job_input(&self, key: NodeKey) -> Result<String> {
        if key.l == 0 {
            let message = self.message(key.i)?;
            return Ok(format!("{}: {}", message.kind, message.text));
        }
        let [left, right] = key.children()?;
        let left = &self
            .nodes
            .get(&left)
            .context("left summary child is missing")?
            .text;
        let right = &self
            .nodes
            .get(&right)
            .context("right summary child is missing")?
            .text;
        Ok(format!("{left}\n{right}"))
    }

    fn advance_views(&mut self) -> Result<()> {
        let mut context = self.context_view.clone();
        let mut end = self.summarized_end;
        while end < self.message_count() && self.nodes.contains_key(&NodeKey { l: 0, i: end }) {
            context.push(NodeKey { l: 0, i: end });
            end = end.checked_add(1).context("summary boundary overflows")?;
        }
        // Save active flags before changing either frontier. A crash between
        // snapshots must retain the low-watermark target, never restart a batch.
        let mut active = self.metadata.clone();
        active.context_batch |= self.frontier_size(&context, end)? > CONTEXT_TRIGGER;
        active.main_batch |= self.view_size(&self.view, end)? > VIEW_HIGH;
        self.save_metadata(active)?;
        let mut context_batch = self.metadata.context_batch;
        let mut main_batch = self.metadata.main_batch;
        let context = self.compact_frontier(
            context,
            (CONTEXT_TRIGGER, CONTEXT_LOW),
            end,
            &mut context_batch,
        )?;
        let view = self.compact(self.view.clone(), VIEW_HIGH, VIEW_LOW, end, &mut main_batch)?;
        // Persist context first: a crash may leave the live view less compact, but
        // cannot leave it referring to summaries outside the durable prefix.
        if context != self.context_view {
            let result = persist_view(&self.path.join("context.json"), &context);
            self.durable(result)?;
            self.context_view = context;
        }
        self.summarized_end = end;
        if view != self.view {
            let result = persist_view(&self.path.join("view.json"), &view);
            self.durable(result)?;
            self.view = view;
        }
        let mut finished = self.metadata.clone();
        finished.main_batch = main_batch;
        finished.context_batch = context_batch;
        self.save_metadata(finished)
    }

    fn save_metadata(&mut self, metadata: Metadata) -> Result<()> {
        if metadata != self.metadata {
            let result = persist_json(&self.path.join("metadata.json"), &metadata);
            self.durable(result)?;
            self.metadata = metadata;
        }
        Ok(())
    }

    fn compact(
        &self,
        original: Vec<NodeKey>,
        high: usize,
        low: usize,
        end: u64,
        batching: &mut bool,
    ) -> Result<Vec<NodeKey>> {
        self.compact_frontier(original, (high, low), end, batching)
    }

    fn compact_frontier(
        &self,
        original: Vec<NodeKey>,
        thresholds: (usize, usize),
        end: u64,
        batching: &mut bool,
    ) -> Result<Vec<NodeKey>> {
        let (high, low) = thresholds;
        let mut size = self.frontier_size(&original, end)?;
        *batching |= size > high;
        if !*batching {
            return Ok(original);
        }
        let mut view = original;
        while size > low {
            let mut best: Option<(usize, NodeKey)> = None;
            for (index, pair) in view.windows(2).enumerate() {
                let [left, right] = pair else { continue };
                if left.l != right.l || left.end()? != right.i {
                    continue;
                }
                let Some(parent) = left.parent() else {
                    continue;
                };
                if parent.i != left.i || parent.end()? > end || !self.nodes.contains_key(&parent) {
                    continue;
                }
                if best.is_none_or(|(_, current)| {
                    compare_due(parent, current, self.message_count()) == Ordering::Greater
                }) {
                    best = Some((index, parent));
                }
            }
            let Some((index, parent)) = best else {
                return Ok(view);
            };
            view.splice(index..index + 2, [parent]);
            size = self.frontier_size(&view, end)?;
        }
        *batching = false;
        Ok(view)
    }

    fn view_size(&self, view: &[NodeKey], end: u64) -> Result<usize> {
        self.frontier_size(view, end)
    }

    fn frontier_size(&self, view: &[NodeKey], end: u64) -> Result<usize> {
        view.iter()
            .take_while(|key| key.i < end)
            .try_fold("<chat>\n</chat>".len(), |size, key| {
                let text = &self.nodes.get(key).context("view summary is missing")?.text;
                let mut rendered = String::new();
                push_tagged(&mut rendered, *key, text)?;
                size.checked_add(rendered.len())
                    .context("view size overflows")
            })
    }

    fn before_boundary(&self, key: NodeKey, end: u64, keys: &mut Vec<NodeKey>) -> Result<()> {
        if key.i >= end {
            return Ok(());
        }
        if key.end()? <= end {
            ensure!(self.nodes.contains_key(&key), "context summary is missing");
            keys.push(key);
        } else if key.l > 0 {
            for child in key.children()? {
                self.before_boundary(child, end, keys)?;
            }
        }
        Ok(())
    }

    fn render_keys(&self, keys: &[NodeKey], limit: usize) -> Result<String> {
        let mut rendered = String::from("<chat>\n");
        for key in keys {
            let node = self.nodes.get(key).context("render summary is missing")?;
            push_tagged(&mut rendered, *key, &node.text)?;
            ensure!(
                rendered
                    .len()
                    .checked_add("</chat>".len())
                    .context("render size overflows")?
                    <= limit,
                "summary frontier exceeds {limit} bytes; more compaction is required (history was not discarded)"
            );
        }
        rendered.push_str("</chat>");
        Ok(rendered)
    }
}

fn utf8_prefix_len(text: &str, limit: usize) -> usize {
    let mut end = text.len().min(limit);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn utf8_chunks(mut text: &str, limit: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    while text.len() > limit {
        let end = utf8_prefix_len(text, limit);
        chunks.push(&text[..end]);
        text = &text[end..];
    }
    chunks.push(text);
    chunks
}

fn echo(text: &str) -> String {
    if text.chars().count() <= ECHO_CHARS {
        return text.to_owned();
    }
    let remaining = ECHO_CHARS - ECHO_OMISSION.chars().count();
    let head_chars = remaining / 2;
    let head: String = text.chars().take(head_chars).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(remaining - head_chars)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}{ECHO_OMISSION}{tail}")
}

fn push_tagged(output: &mut String, key: NodeKey, text: &str) -> Result<()> {
    output.push_str(&format!("{}+{}|", key.i, key.span()?));
    for character in text.chars() {
        output.push(match character {
            '\r' | '\n' => ' ',
            character => character,
        });
    }
    output.push('\n');
    Ok(())
}

/// Compare `(T - last) / 2^l` exactly. The older interval wins equal due values.
fn compare_due(left: NodeKey, right: NodeKey, t: u64) -> Ordering {
    // Callers have already validated the keys and checked their ends against T.
    let left_end = u128::from(left.i) + (1u128 << left.l);
    let right_end = u128::from(right.i) + (1u128 << right.l);
    let left_due = (u128::from(t) - left_end) * (1u128 << right.l);
    let right_due = (u128::from(t) - right_end) * (1u128 << left.l);
    left_due.cmp(&right_due).then_with(|| right.i.cmp(&left.i))
}

fn journal_day(path: &Path) -> Result<String> {
    let day = path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("invalid daily journal name")?;
    let parsed =
        chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").context("invalid daily journal name")?;
    ensure!(
        parsed.format("%Y-%m-%d").to_string() == day,
        "noncanonical daily journal name"
    );
    Ok(day.to_owned())
}

fn journal_files(path: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        ensure!(
            entry.file_type()?.is_file(),
            "unexpected journal directory entry: {}",
            entry.path().display()
        );
        let path = entry.path();
        ensure!(
            path.extension().is_some_and(|ext| ext == "jsonl"),
            "unexpected journal file: {}",
            path.display()
        );
        files.push(path);
    }
    files.sort();
    Ok(files)
}

fn read_journal<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut records = Vec::new();
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        if reader.read_until(b'\n', &mut bytes)? == 0 {
            break;
        }
        ensure!(
            bytes.last() == Some(&b'\n'),
            "incomplete journal record in {}",
            path.display()
        );
        let record = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "invalid journal record {} in {}",
                records.len() + 1,
                path.display()
            )
        })?;
        records.push(record);
    }
    Ok(records)
}

fn append_record(path: &Path, record: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec(record)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(&bytes)
        .with_context(|| format!("append {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("sync {}", path.display()))?;
    sync_directory(path.parent().context("journal has no parent directory")?)
}

fn load_view(path: &Path) -> Result<Vec<NodeKey>> {
    let pairs: Vec<(u32, u64)> = serde_json::from_reader(File::open(path)?)
        .with_context(|| format!("invalid frontier {}", path.display()))?;
    Ok(pairs.into_iter().map(|(l, i)| NodeKey { l, i }).collect())
}

fn validate_view(
    view: &[NodeKey],
    count: u64,
    nodes: &BTreeMap<NodeKey, Node>,
    summaries_only: bool,
) -> Result<u64> {
    let mut end = 0;
    for key in view {
        ensure!(key.i == end, "gap or overlap in persisted frontier");
        end = key.end()?;
        ensure!(end <= count, "persisted frontier extends past journal");
        if summaries_only || key.l > 0 {
            ensure!(
                nodes.contains_key(key),
                "persisted frontier references missing node {key:?}"
            );
        }
    }
    Ok(end)
}

fn persist_view(path: &Path, view: &[NodeKey]) -> Result<()> {
    let pairs: Vec<_> = view.iter().map(|key| (key.l, key.i)).collect();
    persist_json(path, &pairs)
}

fn persist_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    atomic_replace(&temporary, path)?;
    sync_directory(path.parent().context("frontier has no parent directory")?)
}

#[cfg(unix)]
fn lock_store(path: &Path) -> Result<File> {
    use std::os::fd::AsRawFd;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    // SAFETY: the descriptor belongs to `file`, which remains alive throughout
    // this call and is retained by the store. flock neither retains a pointer nor
    // transfers descriptor ownership; closing the File releases the lock.
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result != 0 {
        return Err(std::io::Error::last_os_error())
            .context("memory store is locked or cannot be locked");
    }
    Ok(file)
}

#[cfg(windows)]
fn lock_store(path: &Path) -> Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .share_mode(0)
        .open(path)
        .context("memory store is locked or cannot be locked")
}

#[cfg(not(any(unix, windows)))]
fn lock_store(_path: &Path) -> Result<File> {
    bail!("exclusive memory-store locking is unsupported on this platform")
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?
        .sync_all()
        .with_context(|| format!("sync directory {}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    // Windows has no portable directory fsync. Files are flushed explicitly and
    // frontier replacement uses MOVEFILE_WRITE_THROUGH below.
    Ok(())
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination).context("replace persisted memory frontier")
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let source: Vec<u16> = source.as_os_str().encode_wide().collect();
    let destination: Vec<u16> = destination.as_os_str().encode_wide().collect();
    ensure!(
        !source.contains(&0) && !destination.contains(&0),
        "path contains a NUL"
    );
    let source: Vec<u16> = source.into_iter().chain([0]).collect();
    let destination: Vec<u16> = destination.into_iter().chain([0]).collect();
    // SAFETY: both pointers refer to live, NUL-terminated UTF-16 buffers. The API
    // only reads these buffers during the call. REPLACE_EXISTING | WRITE_THROUGH
    // avoids a delete/rename gap and flushes the replacement before returning.
    let result = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 1 | 8) };
    if result == 0 {
        return Err(std::io::Error::last_os_error()).context("replace persisted memory frontier");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let serial = NEXT_DIRECTORY.fetch_add(1, AtomicOrdering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("uniichat-memory-{}-{serial}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn timestamp() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-08T12:34:56Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn drain(store: &mut InfiniteContext) {
        while let Some(job) = store.next_job().unwrap() {
            store
                .complete_job(
                    job.key,
                    format!("summary {}+{}", job.key.i, 1u64 << job.key.l),
                )
                .unwrap();
        }
    }

    #[test]
    fn utf8_split_echo_and_raw_zoom_pages() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        let text = format!("{}{}", "a".repeat(29_999), "🦀".repeat(8_000));
        store.append("user", &text, timestamp()).unwrap();
        assert_eq!(store.message_count(), 3);
        assert!(
            store
                .messages
                .iter()
                .all(|message| message.text.len() <= MESSAGE_BYTES)
        );
        assert_eq!(
            store
                .messages
                .iter()
                .map(|message| message.text.as_str())
                .collect::<String>(),
            text
        );
        let tool = format!(
            "{}{}{}",
            "🦀".repeat(15_001),
            "MIDDLE".repeat(10_000),
            "界".repeat(15_001)
        );
        store.append("tool_result", &tool, timestamp()).unwrap();
        assert_eq!(
            store.message_count(),
            3 + utf8_chunks(&tool, MESSAGE_BYTES).len() as u64
        );
        assert_eq!(
            store
                .messages
                .iter()
                .skip(3)
                .map(|m| m.text.as_str())
                .collect::<String>(),
            tool
        );
        for message in &store.messages {
            assert!(message.text.len() <= MESSAGE_BYTES);
            assert_eq!(
                store.job_input(NodeKey { l: 0, i: message.i }).unwrap(),
                format!("{}: {}", message.kind, message.text)
            );
        }
        let clipped = echo(&tool);
        assert_eq!(clipped.chars().count(), ECHO_CHARS);
        assert!(clipped.starts_with('🦀'));
        assert!(clipped.ends_with('界'));
        assert!(!clipped.contains("MIDDLE"));
        let remaining = ECHO_CHARS - ECHO_OMISSION.chars().count();
        assert_eq!(
            clipped,
            format!(
                "{}{}{}",
                "🦀".repeat(remaining / 2),
                ECHO_OMISSION,
                "界".repeat(remaining - remaining / 2)
            )
        );
        let expected = format!("3+1|tool_result: {}", store.message(3).unwrap().text);
        let mut reconstructed = String::new();
        let pages = utf8_chunks(&expected, MESSAGE_BYTES);
        for page in 0..pages.len() {
            let chunk = store.zoom(3, 1, page).unwrap();
            assert!(chunk.len() <= MESSAGE_BYTES);
            reconstructed.push_str(&chunk);
        }
        assert_eq!(reconstructed, expected);
        assert!(store.zoom(3, 1, pages.len()).is_err());
        let echo_start = store.message_count() as usize;
        store.append("echo", &tool, timestamp()).unwrap();
        assert_eq!(
            store
                .messages
                .iter()
                .skip(echo_start)
                .map(|m| m.text.as_str())
                .collect::<String>(),
            clipped
        );
        assert!(store.messages.iter().all(|m| m.text.len() <= MESSAGE_BYTES));
        assert_eq!(store.render_view().unwrap(), "<chat>\n</chat>");
        assert_eq!(store.date(0).unwrap(), timestamp().to_rfc3339());
    }

    #[test]
    fn short_inputs_are_verbatim_and_zoom_validates_alignment() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", "hello", timestamp()).unwrap();
        store.append("assistant", "world", timestamp()).unwrap();
        assert!(store.next_job().unwrap().is_none());
        assert_eq!(store.nodes[&NodeKey { l: 0, i: 0 }].text, "user: hello");
        assert_eq!(
            store.nodes[&NodeKey { l: 1, i: 0 }].text,
            "user: hello\nassistant: world"
        );
        assert!(store.all_summarized_before(2));
        assert!(!store.all_summarized_before(3));
        assert_eq!(
            store.zoom(0, 2, 0).unwrap(),
            "0+1|user: hello\n1+1|assistant: world\n"
        );
        for (id, span) in [(0, 0), (0, 3), (1, 2), (2, 2), (u64::MAX, 1)] {
            assert!(store.zoom(id, span, 0).is_err());
        }
        assert!(NodeKey { l: 64, i: 0 }.end().is_err());
        assert!(store.date(2).is_err());
    }

    #[test]
    fn concurrent_completions_advance_only_a_contiguous_prefix() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        for _ in 0..4 {
            store.append("user", &"x".repeat(600), timestamp()).unwrap();
        }
        let jobs: Vec<_> = (0..4).map(|_| store.next_job().unwrap().unwrap()).collect();
        assert!(store.next_job().unwrap().is_none());
        store
            .complete_job(jobs[1].key, "second".to_owned())
            .unwrap();
        assert!(!store.all_summarized_before(1));
        assert_eq!(store.render_view_before(4).unwrap(), "<chat>\n</chat>");
        assert_eq!(store.render_view().unwrap(), "<chat>\n</chat>");
        assert!(store.render_turn_before(4).is_err());
        store.complete_job(jobs[0].key, "first".to_owned()).unwrap();
        assert!(store.all_summarized_before(2));
        assert!(!store.all_summarized_before(3));
        assert!(store.render_view_before(1).unwrap().contains("first"));
        assert!(!store.render_view_before(1).unwrap().contains("second"));
        store
            .complete_job(jobs[3].key, "fourth".to_owned())
            .unwrap();
        store.complete_job(jobs[2].key, "third".to_owned()).unwrap();
        drain(&mut store);
        assert!(store.all_summarized_before(4));
        assert!(store.nodes.contains_key(&NodeKey { l: 2, i: 0 }));
    }

    #[test]
    fn failed_jobs_retry_once_on_the_next_drain_turn() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", &"x".repeat(600), timestamp()).unwrap();
        let first = store.next_job().unwrap().unwrap();
        assert_eq!(first.end, 1);
        assert!(first.task.contains("512 UTF-8 bytes"));
        assert!(store.complete_job(first.key, "🦀".repeat(129)).is_err());
        store.retry_job(first.key).unwrap();
        store.retry_job(first.key).unwrap();
        assert!(store.next_job().unwrap().is_none());
        let retry = store.next_job().unwrap().unwrap();
        assert_eq!(retry.key, first.key);
        assert!(store.next_job().unwrap().is_none());
        store.complete_job(retry.key, "done".to_owned()).unwrap();
        assert!(
            store
                .complete_job(retry.key, "overwrite".to_owned())
                .is_err()
        );
        assert!(store.retry_job(retry.key).is_err());
        assert!(store.next_job().unwrap().is_none());
    }

    #[test]
    fn append_reopen_preserves_frontiers_and_only_recovers_raw_suffix() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        for _ in 0..8 {
            store.append("user", "tiny", timestamp()).unwrap();
        }
        drain(&mut store);
        // Keep a deliberately less compact frontier even though its root exists.
        let prefix = vec![NodeKey { l: 2, i: 0 }, NodeKey { l: 2, i: 4 }];
        persist_view(&directory.0.join("view.json"), &prefix).unwrap();
        let context = fs::read(directory.0.join("context.json")).unwrap();
        let suffix = Message {
            i: 8,
            kind: "user".into(),
            text: "crash suffix".into(),
            size: 12,
            date: timestamp(),
        };
        append_record(&directory.0.join("main/2026-10-08.jsonl"), &suffix).unwrap();
        drop(store);
        let mut reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(&reopened.view[..2], &prefix);
        assert_eq!(reopened.view[2], NodeKey { l: 0, i: 8 });
        assert_eq!(fs::read(directory.0.join("context.json")).unwrap(), context);
        reopened.append("assistant", "next", timestamp()).unwrap();
        assert_eq!(&reopened.view[..2], &prefix);
        let view = fs::read(directory.0.join("view.json")).unwrap();
        let rendered = reopened.render_view().unwrap();
        drop(reopened);
        let reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(fs::read(directory.0.join("view.json")).unwrap(), view);
        assert_eq!(reopened.render_view().unwrap(), rendered);
    }

    #[test]
    fn a_crash_after_node_append_does_not_reconstruct_the_view() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", &"x".repeat(600), timestamp()).unwrap();
        let node = Node {
            l: 0,
            i: 0,
            text: "built but not committed".into(),
            size: 23,
        };
        append_record(&directory.0.join("tree/2026-10-08.jsonl"), &node).unwrap();
        let view = fs::read(directory.0.join("view.json")).unwrap();
        drop(store);
        let reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(fs::read(directory.0.join("view.json")).unwrap(), view);
        assert!(reopened.all_summarized_before(1));
        assert!(
            reopened
                .render_view()
                .unwrap()
                .contains("built but not committed")
        );
        assert!(!reopened.render_view().unwrap().contains("user: xxx"));
        assert!(
            reopened
                .render_view_before(1)
                .unwrap()
                .contains("built but not committed")
        );
        assert_eq!(fs::read(directory.0.join("context.json")).unwrap(), b"[]\n");
    }

    #[test]
    fn unfinished_jobs_are_ready_again_after_reopen() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", &"x".repeat(600), timestamp()).unwrap();
        let job = store.next_job().unwrap().unwrap();
        store.retry_job(job.key).unwrap();
        drop(store);
        let mut reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.next_job().unwrap().unwrap().key, job.key);
    }

    #[test]
    fn corrupt_journals_and_frontiers_are_errors_not_repaired() {
        for invalid in [
            b"{not-json}\n".as_slice(),
            b"{}".as_slice(),
            b"\n".as_slice(),
        ] {
            let directory = TestDirectory::new();
            let store = InfiniteContext::open(directory.0.clone()).unwrap();
            fs::write(directory.0.join("main/2026-10-08.jsonl"), invalid).unwrap();
            drop(store);
            assert!(InfiniteContext::open(directory.0.clone()).is_err());
        }
        for invalid in ["[[0,1]]", "[[64,0]]", "[[1,0]]", "null", "[] trailing"] {
            let directory = TestDirectory::new();
            let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
            store.append("user", "x", timestamp()).unwrap();
            fs::write(directory.0.join("view.json"), invalid).unwrap();
            drop(store);
            assert!(InfiniteContext::open(directory.0.clone()).is_err());
            assert_eq!(
                fs::read_to_string(directory.0.join("view.json")).unwrap(),
                invalid
            );
        }
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", "x", timestamp()).unwrap();
        fs::remove_file(directory.0.join("view.json")).unwrap();
        drop(store);
        assert!(InfiniteContext::open(directory.0.clone()).is_err());
    }

    #[test]
    fn persistence_errors_poison_the_store_and_propagate() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        fs::create_dir(directory.0.join("view.json.tmp")).unwrap();
        assert!(
            store
                .append("user", "durable raw append", timestamp())
                .is_err()
        );
        assert!(store.next_job().is_err());
        assert!(store.render_view().is_err());
        assert!(
            store
                .append("user", "must not append", timestamp())
                .is_err()
        );
        drop(store);
        fs::remove_dir(directory.0.join("view.json.tmp")).unwrap();
        let mut reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.message_count(), 1);
        assert_eq!(reopened.render_view().unwrap(), "<chat>\n</chat>");
        drain(&mut reopened);
        assert!(
            reopened
                .render_view()
                .unwrap()
                .contains("durable raw append")
        );
    }

    fn synthetic_tree(store: &mut InfiniteContext, count: u64, bytes: usize) {
        for i in 0..count {
            store.messages.push(Message {
                i,
                kind: "user".into(),
                text: "x".into(),
                size: 1,
                date: timestamp(),
            });
            let key = NodeKey { l: 0, i };
            store.view.push(key);
            store.context_view.push(key);
            store.nodes.insert(
                key,
                Node {
                    l: 0,
                    i,
                    text: "x".repeat(bytes),
                    size: bytes,
                },
            );
        }
        store.summarized_end = count;
        store.built_end = count;
        for l in 1..64 {
            let span = 1u64 << l;
            if span > count {
                break;
            }
            for i in (0..count).step_by(span as usize) {
                if i + span <= count {
                    store.nodes.insert(
                        NodeKey { l, i },
                        Node {
                            l,
                            i,
                            text: "x".repeat(bytes),
                            size: bytes,
                        },
                    );
                }
            }
        }
    }

    #[test]
    fn due_is_exact_at_ten_oldest_ties_and_recent_pair_remains_eligible() {
        assert_eq!(
            compare_due(NodeKey { l: 1, i: 4 }, NodeKey { l: 0, i: 7 }, 10),
            Ordering::Greater
        );
        assert_eq!(
            compare_due(NodeKey { l: 0, i: 7 }, NodeKey { l: 1, i: 4 }, 10),
            Ordering::Less
        );
        assert_eq!(
            compare_due(NodeKey { l: 1, i: 8 }, NodeKey { l: 3, i: 0 }, 10),
            Ordering::Less
        );
        let t = u64::MAX;
        assert_eq!(
            compare_due(NodeKey { l: 62, i: 0 }, NodeKey { l: 63, i: 0 }, t),
            Ordering::Greater
        );
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        synthetic_tree(&mut store, 10, 512);
        let view = vec![
            NodeKey { l: 3, i: 0 },
            NodeKey { l: 0, i: 8 },
            NodeKey { l: 0, i: 9 },
        ];
        let mut batching = false;
        let merged = store
            .compact(view, 1_500, 1_100, 10, &mut batching)
            .unwrap();
        assert!(!batching);
        assert_eq!(merged, vec![NodeKey { l: 3, i: 0 }, NodeKey { l: 1, i: 8 }]);
    }

    #[test]
    fn compaction_batches_watermarks_and_persists_context_separately() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        synthetic_tree(&mut store, 256, SUMMARY_BYTES);
        let before = store.view.clone();
        assert!(store.view_size(&before, 256).unwrap() >= VIEW_HIGH);
        store.advance_views().unwrap();
        assert!(store.view_size(&store.view, 256).unwrap() <= VIEW_LOW);
        assert!(store.frontier_size(&store.context_view, 256).unwrap() <= CONTEXT_LOW);
        assert_ne!(store.view, store.context_view);
        assert_eq!(
            load_view(&directory.0.join("view.json")).unwrap(),
            store.view
        );
        assert_eq!(
            load_view(&directory.0.join("context.json")).unwrap(),
            store.context_view
        );
        assert!(store.render_view_before(256).unwrap().len() <= CONTEXT_HIGH);
        assert_eq!(
            validate_view(&store.view, 256, &store.nodes, true).unwrap(),
            256
        );
        let below = vec![NodeKey { l: 0, i: 0 }, NodeKey { l: 0, i: 1 }];
        let mut batching = false;
        assert_eq!(
            store
                .compact(below.clone(), VIEW_HIGH, VIEW_LOW, 256, &mut batching)
                .unwrap(),
            below
        );
        store.context_view = vec![NodeKey { l: 8, i: 0 }];
        let render = store.render_view_before(255).unwrap();
        assert!(render.contains("254+1|"));
        assert!(!render.contains("255+1|"));
        assert!(!render.contains("0+256|"));
    }

    #[test]
    fn full_compaction_reopens_identical_frontiers_and_keeps_append_prefix() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        for _ in 0..256 {
            store.append("user", &"x".repeat(600), timestamp()).unwrap();
        }
        let mut calls = 0;
        while let Some(job) = store.next_job().unwrap() {
            store
                .complete_job(job.key, "s".repeat(SUMMARY_BYTES))
                .unwrap();
            calls += 1;
        }
        assert_eq!(calls, 511);
        assert!(store.all_summarized_before(256));
        assert!(store.view_size(&store.view, 256).unwrap() <= VIEW_HIGH);
        assert!(store.frontier_size(&store.context_view, 256).unwrap() <= CONTEXT_HIGH);
        assert!(!store.metadata.main_batch);
        assert!(!store.metadata.context_batch);
        let prefix = store.view.clone();
        let context = store.context_view.clone();
        let rendered = store.render_view().unwrap();
        drop(store);
        let mut reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.view, prefix);
        assert_eq!(reopened.context_view, context);
        assert_eq!(reopened.render_view().unwrap(), rendered);
        assert!(reopened.next_job().unwrap().is_none());
        reopened
            .append("user", "new raw message", timestamp())
            .unwrap();
        assert_eq!(&reopened.view[..prefix.len()], &prefix);
        assert_eq!(reopened.view.last(), Some(&NodeKey { l: 0, i: 256 }));
        assert_eq!(reopened.context_view, context);
    }

    #[test]
    fn corrupt_metadata_and_summary_suffix_cannot_be_recovered_as_appends() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", "tiny", timestamp()).unwrap();
        drain(&mut store);
        persist_view(&directory.0.join("view.json"), &[]).unwrap();
        persist_view(&directory.0.join("context.json"), &[]).unwrap();
        drop(store);
        assert!(InfiniteContext::open(directory.0.clone()).is_err());
        for (name, size, id) in [
            ("2026-10-08", 999, 0),
            ("2026-10-09", 1, 0),
            ("2026-10-08", 1, 1),
        ] {
            let directory = TestDirectory::new();
            let store = InfiniteContext::open(directory.0.clone()).unwrap();
            let record = Message {
                i: id,
                kind: "user".into(),
                text: "x".into(),
                size,
                date: timestamp(),
            };
            append_record(
                &directory.0.join("main").join(format!("{name}.jsonl")),
                &record,
            )
            .unwrap();
            drop(store);
            assert!(InfiniteContext::open(directory.0.clone()).is_err());
        }
        let directory = TestDirectory::new();
        let store = InfiniteContext::open(directory.0.clone()).unwrap();
        let path = directory.0.join("main/2026-10-08.jsonl");
        for i in [1, 0] {
            append_record(
                &path,
                &Message {
                    i,
                    kind: "user".into(),
                    text: "x".into(),
                    size: 1,
                    date: timestamp(),
                },
            )
            .unwrap();
        }
        drop(store);
        assert!(InfiniteContext::open(directory.0.clone()).is_err());
    }

    #[test]
    fn verbatim_ruler_counts_utf8_bytes_including_prefixes_and_newlines() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", &"x".repeat(506), timestamp()).unwrap();
        store
            .append("user", &format!("{}🦀", "x".repeat(503)), timestamp())
            .unwrap();
        let job = store.next_job().unwrap().unwrap();
        assert_eq!(job.key, NodeKey { l: 0, i: 1 });
        assert_eq!(job.input.len(), 513);
        assert_eq!(store.nodes[&NodeKey { l: 0, i: 0 }].size, 512);
        store.complete_job(job.key, "short".into()).unwrap();
        let parent = store.next_job().unwrap().unwrap();
        assert_eq!(parent.key, NodeKey { l: 1, i: 0 });
        assert_eq!(parent.input.len(), 518);

        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("u", &"x".repeat(253), timestamp()).unwrap();
        store.append("u", &"x".repeat(252), timestamp()).unwrap();
        assert!(store.next_job().unwrap().is_none());
        let node = &store.nodes[&NodeKey { l: 1, i: 0 }];
        assert_eq!(node.size, 512);
        assert_eq!(
            node.text,
            format!("u: {}\nu: {}", "x".repeat(253), "x".repeat(252))
        );
    }

    #[test]
    fn daily_tree_journal_uses_completion_day_and_loads_coordinates_independently() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", "short", timestamp()).unwrap();
        let before = Utc::now().format("%Y-%m-%d").to_string();
        drain(&mut store);
        let after = Utc::now().format("%Y-%m-%d").to_string();
        for path in journal_files(&directory.0.join("tree")).unwrap() {
            let day = journal_day(&path).unwrap();
            assert!(day == before || day == after);
        }
        drop(store);
        assert!(InfiniteContext::open(directory.0.clone()).is_ok());

        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        for day in ["2026-10-10", "2026-10-08", "2026-10-09", "2026-10-08"] {
            let date = DateTime::parse_from_rfc3339(&format!("{day}T00:00:00Z"))
                .unwrap()
                .with_timezone(&Utc);
            store.append("user", "raw", date).unwrap();
        }
        // Parent records can precede children in filename order, and coordinates
        // within a day's append stream need not be sorted by index or level.
        for (day, keys) in [
            (
                "2026-11-01",
                vec![NodeKey { l: 2, i: 0 }, NodeKey { l: 1, i: 2 }],
            ),
            (
                "2026-11-02",
                vec![
                    NodeKey { l: 0, i: 3 },
                    NodeKey { l: 0, i: 0 },
                    NodeKey { l: 1, i: 0 },
                    NodeKey { l: 0, i: 2 },
                    NodeKey { l: 0, i: 1 },
                ],
            ),
        ] {
            for key in keys {
                let text = format!("built {}+{}", key.i, 1u64 << key.l);
                append_record(
                    &directory.0.join("tree").join(format!("{day}.jsonl")),
                    &Node {
                        l: key.l,
                        i: key.i,
                        size: text.len(),
                        text,
                    },
                )
                .unwrap();
            }
        }
        let view = fs::read(directory.0.join("view.json")).unwrap();
        drop(store);
        let mut reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.message_count(), 4);
        assert_eq!(reopened.nodes.len(), 7);
        assert!(reopened.all_summarized_before(4));
        assert!(reopened.next_job().unwrap().is_none());
        assert_eq!(fs::read(directory.0.join("view.json")).unwrap(), view);
        assert!(
            reopened
                .render_turn_before(4)
                .unwrap()
                .contains("0+1|built 0+1")
        );
    }

    #[test]
    fn main_turn_renderer_requires_prior_summaries_and_opens_crossing_nodes() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        for i in 0..4 {
            store
                .append("user", &format!("message{i}"), timestamp())
                .unwrap();
        }
        assert_eq!(store.render_view().unwrap(), "<chat>\n</chat>");
        assert!(store.render_turn_before(4).is_err());
        drain(&mut store);
        store.context_view = vec![NodeKey { l: 2, i: 0 }];
        let main = store.render_turn_before(4).unwrap();
        let context = store.render_view_before(4).unwrap();
        assert!(main.contains("0+1|"));
        assert!(main.contains("3+1|"));
        assert!(context.contains("0+4|"));
        assert!(!main.contains("0+4|"));
        store.view = vec![NodeKey { l: 2, i: 0 }];
        let crossing = store.render_turn_before(3).unwrap();
        assert!(crossing.contains("0+2|"));
        assert!(crossing.contains("2+1|"));
        assert!(!crossing.contains("message3"));
        assert!(!crossing.contains("0+4|"));
        store.append("user", "UNBUILT", timestamp()).unwrap();
        assert!(store.render_turn_before(5).is_err());
        assert!(store.render_turn_before(4).is_ok());
        assert!(!store.render_view().unwrap().contains("UNBUILT"));
        assert!(!store.render_view_before(5).unwrap().contains("UNBUILT"));
        assert!(store.render_turn_before(6).is_err());
    }

    #[test]
    fn oversized_frontiers_error_without_dropping_history() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        synthetic_tree(&mut store, 256, SUMMARY_BYTES);
        let main = store.view.clone();
        let context = store.context_view.clone();
        assert!(store.render_view().is_err());
        assert!(store.render_turn_before(256).is_err());
        assert!(store.render_view_before(256).is_ok());
        let mut parents = store.nodes.split_off(&NodeKey { l: 1, i: 0 });
        assert!(store.render_view_before(256).is_err());
        store.nodes.append(&mut parents);
        assert_eq!(store.view, main);
        assert_eq!(store.context_view, context);
        store.advance_views().unwrap();
        for rendered in [
            store.render_turn_before(256).unwrap(),
            store.render_view_before(256).unwrap(),
        ] {
            let mut end = 0;
            for line in rendered.lines().filter(|line| line.contains('|')) {
                let (tag, _) = line.split_once('|').unwrap();
                let (id, n) = tag.split_once('+').unwrap();
                let id: u64 = id.parse().unwrap();
                let n: u64 = n.parse().unwrap();
                assert_eq!(id, end);
                end += n;
            }
            assert_eq!(end, 256);
        }
    }

    #[test]
    fn eight_missing_leaf_gate_counts_inflight_and_failed_jobs() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        for _ in 0..12 {
            store.append("user", &"x".repeat(600), timestamp()).unwrap();
        }
        let jobs: Vec<_> = (0..8).map(|_| store.next_job().unwrap().unwrap()).collect();
        assert!(store.next_job().unwrap().is_none());
        assert!(store.next_job().unwrap().is_none());
        store.retry_job(jobs[0].key).unwrap();
        assert!(store.next_job().unwrap().is_none());
        assert_eq!(store.next_job().unwrap().unwrap().key, jobs[0].key);
        assert!(store.complete_job(jobs[0].key, String::new()).is_err());
        assert!(store.complete_job(jobs[0].key, "  \n ".into()).is_err());
        store.complete_job(jobs[0].key, "a".repeat(512)).unwrap();
        store.complete_job(jobs[1].key, "b".repeat(512)).unwrap();
        let parent = store.next_job().unwrap().unwrap();
        assert_eq!(parent.key, NodeKey { l: 1, i: 0 });
        assert_eq!(
            store.next_job().unwrap().unwrap().key,
            NodeKey { l: 0, i: 8 }
        );
        assert!(store.next_job().unwrap().is_none());
        store.complete_job(parent.key, "merged".into()).unwrap();
        assert_eq!(
            store.next_job().unwrap().unwrap().key,
            NodeKey { l: 0, i: 9 }
        );
        assert!(store.next_job().unwrap().is_none());
        // A later completion reduces the missing count without advancing prefix.
        store.complete_job(jobs[7].key, "later".into()).unwrap();
        assert_eq!(store.built_end, 2);
        assert_eq!(
            store.next_job().unwrap().unwrap().key,
            NodeKey { l: 0, i: 10 }
        );
    }

    #[test]
    fn compaction_trigger_is_strict_and_partial_merges_are_kept() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        synthetic_tree(&mut store, 4, 512);
        let view = store.view.clone();
        let high = store.view_size(&view, 4).unwrap();
        let mut batching = false;
        assert_eq!(
            store
                .compact(view.clone(), high, 600, 4, &mut batching)
                .unwrap(),
            view
        );
        assert!(!batching);
        store.nodes.remove(&NodeKey { l: 1, i: 2 });
        store.nodes.remove(&NodeKey { l: 2, i: 0 });
        let partial = store
            .compact(view, high - 1, 600, 4, &mut batching)
            .unwrap();
        assert_eq!(
            partial,
            vec![
                NodeKey { l: 1, i: 0 },
                NodeKey { l: 0, i: 2 },
                NodeKey { l: 0, i: 3 }
            ]
        );
        assert!(batching);
        // A new parent completes below the high threshold, but the batch continues.
        for key in [NodeKey { l: 1, i: 2 }, NodeKey { l: 2, i: 0 }] {
            store.nodes.insert(
                key,
                Node {
                    l: key.l,
                    i: key.i,
                    text: "x".repeat(512),
                    size: 512,
                },
            );
        }
        let finished = store
            .compact(partial, high - 1, 600, 4, &mut batching)
            .unwrap();
        assert_eq!(finished, vec![NodeKey { l: 2, i: 0 }]);
        assert!(!batching);
    }

    fn write_fixture_journal<T: Serialize>(path: &Path, records: impl IntoIterator<Item = T>) {
        let mut bytes = Vec::new();
        for record in records {
            serde_json::to_writer(&mut bytes, &record).unwrap();
            bytes.push(b'\n');
        }
        fs::write(path, bytes).unwrap();
        File::open(path).unwrap().sync_all().unwrap();
    }

    #[test]
    fn interrupted_batches_resume_below_high_after_reopen() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        synthetic_tree(&mut store, 256, SUMMARY_BYTES);
        store
            .nodes
            .retain(|key, _| key.l == 0 || (key.l == 1 && key.i < 128));
        write_fixture_journal(&directory.0.join("main/2026-10-08.jsonl"), &store.messages);
        write_fixture_journal(
            &directory.0.join("tree/2026-10-08.jsonl"),
            store.nodes.values(),
        );
        persist_view(&directory.0.join("view.json"), &store.view).unwrap();
        persist_view(&directory.0.join("context.json"), &store.context_view).unwrap();
        store.checkpoint_source(57).unwrap();
        store.advance_views().unwrap();
        assert_eq!(store.view.len(), 192);
        assert_eq!(store.context_view.len(), 192);
        assert!(store.metadata.main_batch);
        assert!(store.metadata.context_batch);
        assert!(store.view_size(&store.view, 256).unwrap() < VIEW_HIGH);
        assert!(store.view_size(&store.view, 256).unwrap() > VIEW_LOW);
        let view = store.view.clone();
        let context = store.context_view.clone();
        drop(store);
        let mut reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.view, view);
        assert_eq!(reopened.context_view, context);
        assert_eq!(reopened.source_checkpoint(), Some(57));
        assert!(reopened.metadata.main_batch);
        assert!(reopened.metadata.context_batch);
        let job = reopened.next_job().unwrap().unwrap();
        assert_eq!(job.key, NodeKey { l: 2, i: 0 });
        reopened.complete_job(job.key, "s".repeat(512)).unwrap();
        assert_eq!(reopened.view.len(), 191);
        assert!(reopened.metadata.main_batch);
        drain(&mut reopened);
        assert!(reopened.view_size(&reopened.view, 256).unwrap() <= VIEW_LOW);
        assert!(reopened.frontier_size(&reopened.context_view, 256).unwrap() <= CONTEXT_LOW);
        assert!(!reopened.metadata.main_batch);
        assert!(!reopened.metadata.context_batch);
        assert_eq!(reopened.source_checkpoint(), Some(57));
        drop(reopened);
        let reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert!(!reopened.metadata.main_batch);
        assert!(!reopened.metadata.context_batch);
        assert_eq!(reopened.source_checkpoint(), Some(57));
    }

    #[test]
    fn batch_flag_precedes_frontier_changes_when_persistence_fails() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        synthetic_tree(&mut store, 256, SUMMARY_BYTES);
        fs::create_dir(directory.0.join("context.json.tmp")).unwrap();
        assert!(store.advance_views().is_err());
        let metadata: Metadata =
            serde_json::from_reader(File::open(directory.0.join("metadata.json")).unwrap())
                .unwrap();
        assert!(metadata.main_batch);
        assert!(metadata.context_batch);
        assert!(store.render_view().is_err());
        assert_eq!(
            load_view(&directory.0.join("context.json")).unwrap(),
            vec![]
        );
    }

    #[test]
    fn source_checkpoint_is_atomic_and_corrupt_metadata_is_an_error() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(store.source_checkpoint(), None);
        store.append("user", "durable", timestamp()).unwrap();
        store.checkpoint_source(17).unwrap();
        drop(store);
        let mut reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.source_checkpoint(), Some(17));
        fs::create_dir(directory.0.join("metadata.json.tmp")).unwrap();
        assert!(reopened.checkpoint_source(18).is_err());
        assert_eq!(reopened.source_checkpoint(), Some(17));
        assert!(
            reopened
                .append("user", "no append after failure", timestamp())
                .is_err()
        );
        drop(reopened);
        fs::remove_dir(directory.0.join("metadata.json.tmp")).unwrap();
        let reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.source_checkpoint(), Some(17));
        assert_eq!(reopened.message_count(), 1);
        drop(reopened);
        fs::write(directory.0.join("metadata.json"), "{broken").unwrap();
        assert!(InfiniteContext::open(directory.0.clone()).is_err());
    }

    #[test]
    fn empty_tree_nodes_are_corruption() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", "raw", timestamp()).unwrap();
        append_record(
            &directory.0.join("tree/2026-10-08.jsonl"),
            &Node {
                l: 0,
                i: 0,
                text: String::new(),
                size: 0,
            },
        )
        .unwrap();
        drop(store);
        assert!(InfiniteContext::open(directory.0.clone()).is_err());
    }

    #[test]
    fn saved_batch_with_all_parents_durable_resumes_on_drain_not_open() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        synthetic_tree(&mut store, 256, SUMMARY_BYTES);
        write_fixture_journal(&directory.0.join("main/2026-10-08.jsonl"), &store.messages);
        write_fixture_journal(
            &directory.0.join("tree/2026-10-08.jsonl"),
            store.nodes.values(),
        );
        persist_view(&directory.0.join("view.json"), &store.view).unwrap();
        persist_view(&directory.0.join("context.json"), &store.context_view).unwrap();
        let active = Metadata {
            main_batch: true,
            context_batch: true,
            source_checkpoint: Some(256),
        };
        store.save_metadata(active).unwrap();
        let view = store.view.clone();
        drop(store);
        let mut reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.view, view);
        assert_eq!(load_view(&directory.0.join("view.json")).unwrap(), view);
        assert!(reopened.render_turn_before(256).is_err());
        assert!(reopened.next_job().unwrap().is_none());
        assert!(!reopened.metadata.main_batch);
        assert!(!reopened.metadata.context_batch);
        assert_eq!(reopened.source_checkpoint(), Some(256));
        assert!(reopened.render_turn_before(256).is_ok());
        assert!(reopened.render_view_before(256).is_ok());
        assert_eq!(
            validate_view(&reopened.view, 256, &reopened.nodes, true).unwrap(),
            256
        );
    }

    #[test]
    fn image_only_message_is_durable_and_remains_summarizable() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        let images = vec![
            serde_json::json!({"data": "QUJD", "mime_type": "image/png"}),
            serde_json::json!({"url": "https://example.invalid/image.jpg"}),
        ];
        store
            .append_with_images("user", "", images.clone(), timestamp())
            .unwrap();
        let marker = " [2 images attached: zoom to view]";
        assert_eq!(store.message_count(), 1);
        assert_eq!(store.message(0).unwrap().text, marker);
        assert_eq!(store.zoom_images(0).unwrap(), images);
        assert!(store.zoom(0, 1, 0).unwrap().contains(marker));
        assert!(!store.all_summarized_before(1));
        assert!(store.render_turn_before(1).is_err());
        assert!(store.next_job().unwrap().is_none());
        assert!(store.all_summarized_before(1));
        assert!(store.render_turn_before(1).unwrap().contains(marker));
        assert!(store.render_view_before(1).unwrap().contains(marker));
        assert!(store.render_view().unwrap().contains(marker));
        let records: Vec<ImageRecord> =
            read_journal(&directory.0.join("images/2026-10-08.jsonl")).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].i, 0);
        assert_eq!(records[0].images, images);
        let view = fs::read(directory.0.join("view.json")).unwrap();
        let rendered = store.render_turn_before(1).unwrap();
        drop(store);
        let reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.zoom_images(0).unwrap(), images);
        assert_eq!(reopened.render_turn_before(1).unwrap(), rendered);
        assert!(reopened.all_summarized_before(1));
        assert_eq!(fs::read(directory.0.join("view.json")).unwrap(), view);
        assert!(reopened.zoom_images(1).is_err());
        assert!(reopened.zoom_images(u64::MAX).is_err());
    }

    #[test]
    fn split_text_owns_images_only_on_first_record_without_cutting_base64() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", "earlier", timestamp()).unwrap();
        let image = serde_json::json!({"data": format!("{}END", "QUJD".repeat(40_000)), "metadata": {"width": 20, "enabled": true, "extra": null}});
        let images = vec![image];
        let text = format!("{}🦀{}", "x".repeat(MESSAGE_BYTES - 1), "界".repeat(20_000));
        let marker = " [1 images attached: zoom to view]";
        store
            .append_with_images("user", &text, images.clone(), timestamp())
            .unwrap();
        assert!(store.message_count() > 2);
        assert!(store.zoom_images(0).unwrap().is_empty());
        assert_eq!(store.zoom_images(1).unwrap(), images);
        assert!(store.message(1).unwrap().text.ends_with(marker));
        let reconstructed: String = store
            .messages
            .iter()
            .skip(1)
            .map(|message| message.text.replace(marker, ""))
            .collect();
        assert_eq!(reconstructed, text);
        for message in store.messages.iter().skip(1) {
            assert!(message.text.len() <= MESSAGE_BYTES);
            assert_eq!(
                store.job_input(NodeKey { l: 0, i: message.i }).unwrap(),
                format!("user: {}", message.text)
            );
            if message.i > 1 {
                assert!(store.zoom_images(message.i).unwrap().is_empty());
                assert!(!message.text.contains(marker));
            }
        }
        let raw = format!("1+1|user: {}", store.message(1).unwrap().text);
        let pages = utf8_chunks(&raw, MESSAGE_BYTES);
        let zoomed: String = (0..pages.len())
            .map(|page| store.zoom(1, 1, page).unwrap())
            .collect();
        assert_eq!(zoomed, raw);
        assert!(zoomed.contains(marker));
        let records: Vec<ImageRecord> =
            read_journal(&directory.0.join("images/2026-10-08.jsonl")).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].i, 1);
        assert_eq!(records[0].images, images);
        drop(store);
        let reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.zoom_images(1).unwrap(), images);
        assert!(reopened.zoom_images(2).unwrap().is_empty());
    }

    #[test]
    fn attachment_flush_precedes_raw_append_and_orphans_retry_identically() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        let images = vec![serde_json::json!({"data": "intact image payload"})];
        // Fail the raw append after the attachment journal has been synced.
        fs::create_dir(directory.0.join("main/2026-10-08.jsonl")).unwrap();
        assert!(
            store
                .append_with_images("user", "retry me", images.clone(), timestamp())
                .is_err()
        );
        assert_eq!(store.message_count(), 0);
        let journal = directory.0.join("images/2026-10-08.jsonl");
        let attachment_bytes = fs::read(&journal).unwrap();
        let records: Vec<ImageRecord> = read_journal(&journal).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].i, 0);
        assert_eq!(records[0].images, images);
        assert!(store.next_job().is_err());
        assert!(store.zoom_images(0).is_err());
        drop(store);
        fs::remove_dir(directory.0.join("main/2026-10-08.jsonl")).unwrap();
        let mut reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.message_count(), 0);
        assert_eq!(reopened.render_view().unwrap(), "<chat>\n</chat>");
        assert!(reopened.next_job().unwrap().is_none());
        assert!(reopened.zoom_images(0).is_err());
        assert!(
            reopened
                .append("user", "unrelated message", timestamp())
                .is_err()
        );
        assert!(
            reopened
                .append_with_images(
                    "user",
                    "conflicting",
                    vec![serde_json::json!({"data": "different"})],
                    timestamp()
                )
                .is_err()
        );
        reopened
            .append_with_images("user", "retry me", images.clone(), timestamp())
            .unwrap();
        assert_eq!(reopened.message_count(), 1);
        assert_eq!(reopened.zoom_images(0).unwrap(), images);
        assert_eq!(fs::read(journal).unwrap(), attachment_bytes);
        drain(&mut reopened);
        assert!(reopened.all_summarized_before(1));
        assert!(
            reopened
                .render_turn_before(1)
                .unwrap()
                .contains("retry me [1 images attached: zoom to view]")
        );
    }

    #[test]
    fn image_journal_errors_propagate_before_any_text_append() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        fs::create_dir(directory.0.join("images/2026-10-08.jsonl")).unwrap();
        assert!(
            store
                .append_with_images(
                    "user",
                    "must not log",
                    vec![serde_json::json!({"data": "image"})],
                    timestamp()
                )
                .is_err()
        );
        assert_eq!(store.message_count(), 0);
        assert!(!directory.0.join("main/2026-10-08.jsonl").exists());
        assert!(store.append("user", "poisoned", timestamp()).is_err());
        assert!(store.zoom_images(0).is_err());
    }

    #[test]
    fn duplicate_image_records_allow_identical_values_but_reject_conflicts() {
        for conflicting in [false, true] {
            let directory = TestDirectory::new();
            let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
            let images = vec![serde_json::json!({"data": "unchanged"})];
            store
                .append_with_images("user", "text", images.clone(), timestamp())
                .unwrap();
            let duplicate = if conflicting {
                vec![serde_json::json!({"data": "changed"})]
            } else {
                images.clone()
            };
            append_record(
                &directory.0.join("images/2026-10-09.jsonl"),
                &ImageRecord {
                    i: 0,
                    images: duplicate,
                },
            )
            .unwrap();
            drop(store);
            let reopened = InfiniteContext::open(directory.0.clone());
            if conflicting {
                assert!(reopened.is_err());
            } else {
                assert_eq!(reopened.unwrap().zoom_images(0).unwrap(), images);
            }
        }
        let directory = TestDirectory::new();
        let store = InfiniteContext::open(directory.0.clone()).unwrap();
        for data in ["one", "two"] {
            append_record(
                &directory.0.join("images/2026-10-08.jsonl"),
                &ImageRecord {
                    i: 0,
                    images: vec![serde_json::json!({"data": data})],
                },
            )
            .unwrap();
        }
        drop(store);
        assert!(InfiniteContext::open(directory.0.clone()).is_err());
    }

    #[test]
    fn no_images_preserves_plain_append_and_invalid_input_writes_nothing() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        assert!(
            store
                .append_with_images(
                    "",
                    "invalid",
                    vec![serde_json::json!({"data": "image"})],
                    timestamp()
                )
                .is_err()
        );
        assert_eq!(store.message_count(), 0);
        assert!(
            journal_files(&directory.0.join("images"))
                .unwrap()
                .is_empty()
        );
        store
            .append_with_images("user", "plain", vec![], timestamp())
            .unwrap();
        assert_eq!(store.message(0).unwrap().text, "plain");
        assert!(store.zoom_images(0).unwrap().is_empty());
        assert!(
            journal_files(&directory.0.join("images"))
                .unwrap()
                .is_empty()
        );
        drain(&mut store);
        assert!(store.all_summarized_before(1));
        assert_eq!(
            store.render_turn_before(1).unwrap(),
            "<chat>\n0+1|user: plain\n</chat>"
        );
    }

    #[test]
    fn corrupt_image_journals_are_errors() {
        for invalid in [
            "{broken}\n",
            "{\"i\":0,\"images\":[]}\n",
            "{\"i\":0,\"images\":{}}\n",
            "{\"i\":0,\"images\":[null]}",
        ] {
            let directory = TestDirectory::new();
            let store = InfiniteContext::open(directory.0.clone()).unwrap();
            fs::write(directory.0.join("images/2026-10-08.jsonl"), invalid).unwrap();
            drop(store);
            assert!(InfiniteContext::open(directory.0.clone()).is_err());
        }
    }

    fn assert_context_coverage(rendered: &str, end: u64) {
        assert!(rendered.len() <= CONTEXT_HIGH);
        let mut covered = 0;
        for line in rendered.lines().filter(|line| line.contains('|')) {
            let (tag, _) = line.split_once('|').unwrap();
            let (id, span) = tag.split_once('+').unwrap();
            assert_eq!(id.parse::<u64>().unwrap(), covered);
            covered += span.parse::<u64>().unwrap();
        }
        assert_eq!(covered, end);
    }

    #[test]
    fn ready_parents_precede_leaves_and_cancellation_requeues_immediately() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        for _ in 0..4 {
            store.append("user", &"x".repeat(600), timestamp()).unwrap();
        }
        let left = store.next_job().unwrap().unwrap();
        let right = store.next_job().unwrap().unwrap();
        store.complete_job(left.key, "l".repeat(512)).unwrap();
        store.complete_job(right.key, "r".repeat(512)).unwrap();
        let parent = store.next_job().unwrap().unwrap();
        assert_eq!(parent.key, NodeKey { l: 1, i: 0 });
        let third = store.next_job().unwrap().unwrap();
        let fourth = store.next_job().unwrap().unwrap();
        let cancelled = BTreeSet::from([parent.key, third.key, fourth.key]);
        store.release_in_flight_jobs().unwrap();
        store.release_in_flight_jobs().unwrap();
        assert!(store.in_flight.is_empty());
        assert!(store.deferred.is_empty());
        assert_eq!(store.queued, cancelled);
        let resumed_parent = store.next_job().unwrap().unwrap();
        assert_eq!(resumed_parent.key, parent.key);
        let resumed = BTreeSet::from([
            resumed_parent.key,
            store.next_job().unwrap().unwrap().key,
            store.next_job().unwrap().unwrap().key,
        ]);
        assert_eq!(resumed, cancelled);
        assert!(store.next_job().unwrap().is_none());
    }

    #[test]
    fn oversized_job_context_does_not_claim_or_lose_its_key() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        synthetic_tree(&mut store, 96, SUMMARY_BYTES);
        store.nodes.retain(|key, _| key.l == 0);
        let key = NodeKey { l: 1, i: 94 };
        store.enqueue(key);
        assert!(store.render_view_before(96).is_err());
        assert!(store.next_job().unwrap().is_none());
        assert!(store.in_flight.is_empty());
        assert!(store.queued.contains(&key));
        assert!(store.ready.contains(&(key.i, key.l)));
        store.release_in_flight_jobs().unwrap();
        assert!(store.queued.contains(&key));
        for i in (0..94).step_by(2) {
            store.enqueue(NodeKey { l: 1, i });
        }
        let first = store.next_job().unwrap().unwrap();
        assert_eq!(first.key, NodeKey { l: 1, i: 0 });
        assert_context_coverage(&store.render_view_before(first.end).unwrap(), first.end);
        store.complete_job(first.key, "s".repeat(512)).unwrap();
        drain(&mut store);
        assert!(store.nodes.contains_key(&key));
        assert_context_coverage(&store.render_view_before(96).unwrap(), 96);
    }

    #[test]
    fn context_pressure_waits_for_parents_instead_of_claiming_leaves() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        synthetic_tree(&mut store, 64, SUMMARY_BYTES);
        store.nodes.retain(|key, _| key.l == 0);
        store
            .append("user", &"later".repeat(120), timestamp())
            .unwrap();
        // Model failures may temporarily leave only a subset of parents ready.
        for i in (0..16).step_by(2) {
            store.enqueue(NodeKey { l: 1, i });
        }
        let parent = store.next_job().unwrap().unwrap();
        store.complete_job(parent.key, "s".repeat(512)).unwrap();
        assert!(store.metadata.context_batch);
        let mut running = Vec::new();
        while let Some(job) = store.next_job().unwrap() {
            assert!(job.key.l > 0);
            assert_context_coverage(&store.render_view_before(job.end).unwrap(), job.end);
            running.push(job);
        }
        assert!(!running.is_empty());
        assert!(store.ready_leaves.contains(&64));
        assert!(!store.in_flight.contains(&NodeKey { l: 0, i: 64 }));
        for job in running {
            store.complete_job(job.key, "s".repeat(512)).unwrap();
        }
        for i in (16..64).step_by(2) {
            store.enqueue(NodeKey { l: 1, i });
        }
        drain(&mut store);
        assert!(store.all_summarized_before(65));
        assert!(store.nodes.contains_key(&NodeKey { l: 0, i: 64 }));
        assert_context_coverage(&store.render_view_before(65).unwrap(), 65);
    }

    #[test]
    fn summaries_flatten_line_breaks_without_escaping_or_changing_stored_text() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        store.append("user", &"x".repeat(600), timestamp()).unwrap();
        let job = store.next_job().unwrap().unwrap();
        let text = "literal\\n\ncarriage\rslash\\".to_owned();
        store.complete_job(job.key, text.clone()).unwrap();
        assert_eq!(store.nodes[&job.key].text, text);
        let context = store.render_view_before(1).unwrap();
        assert_eq!(context, "<chat>\n0+1|literal\\n carriage slash\\\n</chat>");
        assert_context_coverage(&context, 1);
        assert_eq!(store.render_view().unwrap(), context);
        assert_eq!(store.render_turn_before(1).unwrap(), context);
        assert!(store.zoom(0, 1, 0).unwrap().contains("user: xxx"));
    }

    #[test]
    fn eight_concurrent_jobs_over_520_messages_always_have_complete_bounded_context() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        for i in 0..520 {
            store
                .append(
                    "user",
                    &format!("message {i} {}", "x".repeat(600)),
                    timestamp(),
                )
                .unwrap();
        }
        let mut running = Vec::new();
        let mut claims = BTreeSet::new();
        let mut completed = 0;
        let mut max_running = 0;
        let mut reverse = false;
        loop {
            while running.len() < MAX_IN_FLIGHT_JOBS {
                let Some(job) = store.next_job().unwrap() else {
                    break;
                };
                assert!(claims.insert(job.key));
                let end = if job.key.l == 0 { job.key.i } else { job.end };
                let context = store.render_view_before(end).unwrap();
                assert_context_coverage(&context, end.min(store.built_end));
                running.push(job);
                max_running = max_running.max(running.len());
            }
            if running.is_empty() {
                assert!(
                    store.unbuilt_leaves.is_empty(),
                    "scheduler stranded leaf work"
                );
                assert!(store.ready.is_empty(), "scheduler stranded parent work");
                break;
            }
            assert_eq!(store.in_flight.len(), running.len());
            if reverse {
                running.reverse();
            }
            reverse = !reverse;
            // Complete whole batches out of order, so eight claimed leaves can
            // advance the prefix together and trigger compaction pressure.
            for job in std::mem::take(&mut running) {
                let text = if completed % 3 == 0 {
                    format!("x{}", "\n".repeat(511))
                } else {
                    "s".repeat(512)
                };
                assert_eq!(text.len(), SUMMARY_BYTES);
                store.complete_job(job.key, text).unwrap();
                completed += 1;
                assert_context_coverage(
                    &store.render_view_before(store.built_end).unwrap(),
                    store.built_end,
                );
            }
        }
        assert_eq!(max_running, 8);
        assert_eq!(completed, 1038);
        assert_eq!(store.nodes.len(), 1038);
        assert!(store.all_summarized_before(520));
        assert!(store.in_flight.is_empty());
        assert!(!store.metadata.context_batch);
        assert_context_coverage(&store.render_view_before(520).unwrap(), 520);
        drop(store);
        let mut reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert!(reopened.next_job().unwrap().is_none());
        assert!(reopened.all_summarized_before(520));
        assert_context_coverage(&reopened.render_view_before(520).unwrap(), 520);
    }

    #[test]
    fn cancellation_and_failed_leaf_retries_remain_ready_during_backpressure() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        for _ in 0..3 {
            store.append("user", &"x".repeat(600), timestamp()).unwrap();
        }
        let first = store.next_job().unwrap().unwrap();
        let second = store.next_job().unwrap().unwrap();
        let mut metadata = store.metadata.clone();
        metadata.context_batch = true;
        store.save_metadata(metadata).unwrap();
        store.release_in_flight_jobs().unwrap();
        assert_eq!(store.next_job().unwrap().unwrap().key, first.key);
        assert_eq!(store.next_job().unwrap().unwrap().key, second.key);
        assert!(store.next_job().unwrap().is_none());
        assert!(store.ready_leaves.contains(&2));
        store.retry_job(first.key).unwrap();
        assert!(store.next_job().unwrap().is_none());
        let retry = store.next_job().unwrap().unwrap();
        assert_eq!(retry.key, first.key);
        store.complete_job(retry.key, "s".repeat(512)).unwrap();
        store.complete_job(second.key, "s".repeat(512)).unwrap();
        drain(&mut store);
        assert!(store.all_summarized_before(3));
        assert!(store.admitted_leaves.is_empty());
    }

    #[test]
    fn one_tag_per_node_flattens_merged_views_and_children_but_not_raw_zoom() {
        let directory = TestDirectory::new();
        let mut store = InfiniteContext::open(directory.0.clone()).unwrap();
        let raw = "first\r\nsecond\nthird\rfourth";
        for text in [raw, "message1", "message2", "message3"] {
            store.append("user", text, timestamp()).unwrap();
        }
        drain(&mut store);
        let root = NodeKey { l: 2, i: 0 };
        let original = store.nodes[&root].text.clone();
        store.view = vec![root];
        store.context_view = vec![root];
        let flattened = original.replace(['\r', '\n'], " ");
        let expected = format!("<chat>\n0+4|{flattened}\n</chat>");
        assert_eq!(store.render_view().unwrap(), expected);
        assert_eq!(store.render_turn_before(4).unwrap(), expected);
        assert_eq!(store.render_view_before(4).unwrap(), expected);
        assert_eq!(
            expected.lines().filter(|line| line.contains('|')).count(),
            1
        );
        assert_context_coverage(&expected, 4);
        let children = store.zoom(0, 4, 0).unwrap();
        assert_eq!(children.lines().count(), 2);
        assert!(children.starts_with("0+2|user: first  second third fourth user: message1\n"));
        assert!(children.ends_with("2+2|user: message2 user: message3\n"));
        assert_eq!(store.zoom(0, 1, 0).unwrap(), format!("0+1|user: {raw}"));
        assert_eq!(store.message(0).unwrap().text, raw);
        assert_eq!(store.nodes[&root].text, original);
    }

    #[test]
    fn echo_omission_marker_is_inside_the_unicode_character_budget() {
        let exact = "🦀".repeat(ECHO_CHARS);
        assert_eq!(echo(&exact), exact);
        assert_eq!(echo("short\r\ntext"), "short\r\ntext");
        let longer = "界".repeat(ECHO_CHARS + 1);
        let clipped = echo(&longer);
        let remaining = ECHO_CHARS - ECHO_OMISSION.chars().count();
        assert_eq!(clipped.chars().count(), ECHO_CHARS);
        assert_eq!(
            clipped,
            format!(
                "{}{}{}",
                "界".repeat(remaining / 2),
                ECHO_OMISSION,
                "界".repeat(remaining - remaining / 2)
            )
        );
        assert_eq!(clipped.matches(ECHO_OMISSION).count(), 1);
        assert_eq!(clipped.chars().filter(|c| *c == '界').count(), remaining);
    }

    #[test]
    fn lock_lives_until_the_store_is_dropped() {
        let directory = TestDirectory::new();
        let store = InfiniteContext::open(directory.0.clone()).unwrap();
        assert!(InfiniteContext::open(directory.0.clone()).is_err());
        drop(store);
        let reopened = InfiniteContext::open(directory.0.clone()).unwrap();
        assert_eq!(reopened.message_count(), 0);
    }
}
