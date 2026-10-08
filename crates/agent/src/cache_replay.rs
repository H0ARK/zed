//! Deterministic offline cache estimation using the production memory engine.
//! Run `replay_json` on a background worker; it performs synchronous scratch I/O,
//! never calls a model, executes a tool, or makes a network request.

use crate::{
    infinite_context::InfiniteContext,
    legacy_thread::{SerializedMessageSegment, SerializedThread},
    thread::{append_memory_view, clip_memory_output, memory_summary_task, memory_tools},
};
use anyhow::{Context as _, Result, bail, ensure};
use chrono::{DateTime, Utc};
use language_model::{
    LanguageModelRequest, LanguageModelRequestMessage, LanguageModelRequestTool,
    LanguageModelRequestToolInput, MessageContent, Role,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use uuid::Uuid;

const MEMORY_PROMPT: &str = include_str!("prompts/infinite_context_prompt.txt");
const LOOKBACK: usize = 20;
/// Controls deterministic offline replay. Defaults match the example CLI.
#[derive(Clone)]
pub struct ReplayOptions {
    pub summary_bytes: usize,
    pub ttl_seconds: u64,
    pub request_gap_seconds: u64,
    /// One-based global agent request index, across repetitions.
    pub pause_after_request: Option<u64>,
    pub pause_seconds: u64,
    pub shared_model: bool,
    pub repeats: usize,
    /// Byte proxy only; this is not a provider token minimum.
    pub min_cache_bytes: usize,
    pub prefix: Option<ReplayPrefix>,
}

impl Default for ReplayOptions {
    fn default() -> Self {
        Self {
            summary_bytes: 512,
            ttl_seconds: 300,
            request_gap_seconds: 1,
            pause_after_request: None,
            pause_seconds: 600,
            shared_model: false,
            repeats: 1,
            min_cache_bytes: 0,
            prefix: None,
        }
    }
}

impl ReplayOptions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.summary_bytes, 256 | 512),
            "--summary-bytes must be 256 or 512"
        );
        ensure!(self.repeats > 0, "--repeats must be positive");
        ensure!(
            self.pause_after_request != Some(0),
            "--pause-after-request is one-based"
        );
        Ok(())
    }
}

/// Distinguishes cooperative cancellation from invalid input or I/O failures.
#[derive(Debug, thiserror::Error)]
#[error("offline replay cancelled")]
pub struct ReplayCancelled;

/// Cleanup takes precedence over cancellation or replay failure: callers should
/// route this warning independently of whether the replay report is still open.
/// Its display is sanitized; underlying errors are retained only in the source
/// chain, without making the outer error downcastable to `ReplayCancelled`.
#[derive(Debug, thiserror::Error)]
#[error(
    "Offline replay could not remove its private journal. Sensitive replay data may remain on disk."
)]
pub struct ReplayCleanupFailed {
    #[source]
    source: anyhow::Error,
}

fn check_cancelled(cancellation: &AtomicBool) -> Result<()> {
    if cancellation.load(Ordering::Relaxed) {
        return Err(ReplayCancelled.into());
    }
    Ok(())
}

#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplayPrefix {
    #[serde(default)]
    pub system: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_prefix_tools")]
    pub tools: Vec<LanguageModelRequestTool>,
}

fn deserialize_prefix_tools<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<LanguageModelRequestTool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ToolSchema {
        name: String,
        description: String,
        input: Option<Value>,
        input_schema: Option<Value>,
    }

    Vec::<ToolSchema>::deserialize(deserializer)?
        .into_iter()
        .map(|tool| {
            let input = match (tool.input, tool.input_schema) {
                (Some(input), None) => {
                    validate_prefix_tool_input(&input).map_err(serde::de::Error::custom)?;
                    serde_json::from_value(input).map_err(serde::de::Error::custom)?
                }
                (None, Some(input_schema)) => LanguageModelRequestToolInput::Function {
                    input_schema,
                    use_input_streaming: false,
                },
                _ => {
                    return Err(serde::de::Error::custom(
                        "expected exactly one of input or input_schema",
                    ));
                }
            };
            Ok(LanguageModelRequestTool {
                name: tool.name,
                description: tool.description,
                input,
            })
        })
        .collect()
}

fn validate_prefix_tool_input(input: &Value) -> Result<()> {
    let (kind, payload) = variant(input)?;
    let allowed: &[&str] = match kind {
        "Function" => &["input_schema", "use_input_streaming"],
        "Custom" => &["format"],
        _ => bail!("unsupported prefix tool input variant"),
    };
    ensure!(
        payload
            .as_object()
            .context("prefix tool input must be an object")?
            .keys()
            .all(|key| allowed.contains(&key.as_str())),
        "unsupported prefix tool input field"
    );
    if let Some(format) = payload.get("format") {
        if !format.is_null() && format.as_str() != Some("Text") {
            let (kind, grammar) = variant(format)?;
            ensure!(kind == "Grammar", "unsupported custom tool format");
            ensure!(
                grammar
                    .as_object()
                    .context("custom tool grammar must be an object")?
                    .keys()
                    .all(|key| matches!(key.as_str(), "syntax" | "definition")),
                "unsupported custom tool grammar field"
            );
        }
    }
    Ok(())
}

impl ReplayPrefix {
    fn request(&self) -> LanguageModelRequest {
        let mut content: Vec<_> = self
            .system
            .iter()
            .cloned()
            .map(MessageContent::Text)
            .collect();
        content.push(MessageContent::Text(MEMORY_PROMPT.into()));
        let mut tools = self.tools.clone();
        tools.extend(memory_tools());
        LanguageModelRequest {
            messages: vec![LanguageModelRequestMessage {
                role: Role::System,
                content,
                cache: true,
                reasoning_details: None,
            }],
            tools,
            ..Default::default()
        }
    }
}

#[derive(Default, Serialize)]
pub struct SourceCounts {
    pub users: u64,
    pub assistants: u64,
    pub resume_messages: u64,
    pub summary_compactions: u64,
    pub mentions: u64,
    pub hidden_messages: u64,
    pub saved_creases: u64,
    pub tool_calls: u64,
    pub tool_results: u64,
    pub tool_result_original_text_bytes: u64,
    pub tool_result_replayed_text_bytes: u64,
    pub tool_results_clipped: u64,
    pub error_results: u64,
    pub incomplete_tool_calls: u64,
    pub results_with_incomplete_input_flag: u64,
    pub pending_tool_calls: u64,
    pub images: u64,
    pub reasoning_blocks_omitted: u64,
    pub reasoning_metadata_entries_omitted: u64,
    pub reasoning_bytes_omitted: u64,
    pub ignored_metadata_entries: u64,
    pub ignored_top_level_metadata_fields: BTreeMap<String, u64>,
    pub ignored_tool_output_metadata_types: BTreeMap<String, u64>,
    pub ignored_tool_output_metadata_bytes: u64,
}

#[derive(Clone)]
struct RecordedTool {
    id: String,
    name: String,
    raw_input: String,
    input: Value,
    complete: bool,
    result: Option<RecordedResult>,
}

#[derive(Clone)]
struct RecordedResult {
    is_error: bool,
    content: Vec<Value>,
    text: String,
    replayed_text: String,
    images: Vec<Value>,
}

impl RecordedResult {
    fn new(
        is_error: bool,
        content: Vec<Value>,
        text: String,
        images: Vec<Value>,
        counts: &mut SourceCounts,
    ) -> Self {
        let replayed_text = clip_memory_output(&text);
        counts.tool_result_original_text_bytes += text.len() as u64;
        counts.tool_result_replayed_text_bytes += replayed_text.len() as u64;
        counts.tool_results_clipped += u64::from(replayed_text != text);
        Self {
            is_error,
            content,
            text,
            replayed_text,
            images,
        }
    }

    fn replayed_content(&self) -> Vec<Value> {
        if self.replayed_text == self.text {
            return self.content.clone();
        }
        let mut content = Vec::new();
        let mut emitted_text = false;
        for block in &self.content {
            if block.get("Text").is_some() {
                if !emitted_text {
                    content.push(json!({"Text": self.replayed_text}));
                    emitted_text = true;
                }
            } else {
                content.push(block.clone());
            }
        }
        content
    }

    fn live_content(&self) -> Vec<Value> {
        let mut content = self.replayed_content();
        if content.is_empty() {
            content.push(json!({"Text": "<Tool returned an empty string>"}));
        }
        content
    }
}

struct RecordedArchiveRecord {
    kind: &'static str,
    text: String,
    images: Vec<Value>,
}

struct RecordedMessage {
    role: Role,
    content: Vec<Value>,
    text: String,
    images: Vec<Value>,
    tools: Vec<RecordedTool>,
    context: String,
    archive_records: Option<Vec<RecordedArchiveRecord>>,
}

struct Conversation {
    version: String,
    date: DateTime<Utc>,
    model: Value,
    messages: Vec<RecordedMessage>,
    counts: SourceCounts,
    stored_usage: Value,
}

const TOP_LEVEL_METADATA: &[&str] = &[
    "title",
    "updated_at",
    "cumulative_token_usage",
    "request_token_usage",
    "baseline_usage",
    "initial_project_snapshot",
    "detailed_summary",
    "draft_prompt",
    "profile",
    "subagent_context",
    "speed",
    "thinking_effort",
    "thinking_enabled",
    "ui_scroll_position",
    "sandbox_grants",
    "sandboxed_terminal_temp_dir",
    "infinite_context",
    "memory_archived",
    "memory_turn_start",
    "measured_cache_usage",
    "_cache_replay_export",
];

fn stored_usage(value: &Value) -> Result<Value> {
    let mut raw = serde_json::Map::new();
    for key in ["baseline_usage", "cumulative_token_usage"] {
        if let Some(usage) = value.get(key) {
            if !usage.is_null() {
                let object = usage
                    .as_object()
                    .context("stored usage metadata must be an object or null")?;
                ensure!(
                    object.iter().all(|(field, count)| matches!(
                        field.as_str(),
                        "input_tokens"
                            | "output_tokens"
                            | "cache_read_input_tokens"
                            | "cache_creation_input_tokens"
                    ) && count.as_u64().is_some()),
                    "unsupported stored usage metadata structure"
                );
            }
            raw.insert(key.into(), usage.clone());
        }
    }
    Ok(Value::Object(raw))
}

fn ignore_tool_output(output: &Value, counts: &mut SourceCounts) -> Result<()> {
    let kind = match output {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    };
    counts.ignored_metadata_entries += 1;
    *counts
        .ignored_tool_output_metadata_types
        .entry(kind.into())
        .or_default() += 1;
    counts.ignored_tool_output_metadata_bytes += canonical(output)?.len() as u64;
    Ok(())
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value> {
    value.get(key).context("missing required input field")
}

fn string_field(value: &Value, key: &str) -> Result<String> {
    Ok(field(value, key)?
        .as_str()
        .context("input field must be a string")?
        .into())
}

fn bool_field(value: &Value, key: &str) -> Result<bool> {
    field(value, key)?
        .as_bool()
        .context("input field must be a boolean")
}

fn variant(value: &Value) -> Result<(&str, &Value)> {
    let object = value
        .as_object()
        .context("content must be externally tagged JSON")?;
    ensure!(
        object.len() == 1,
        "content must have exactly one variant tag"
    );
    let (tag, payload) = object.iter().next().context("empty content variant")?;
    Ok((tag, payload))
}

fn parse_content(
    value: &Value,
    counts: &mut SourceCounts,
    cancellation: &AtomicBool,
) -> Result<(Vec<Value>, String, Vec<Value>)> {
    let mut content = Vec::new();
    let mut text = String::new();
    let mut images = Vec::new();
    for block in value.as_array().context("content must be an array")? {
        check_cancelled(cancellation)?;
        let (tag, payload) = variant(block)?;
        match tag {
            "Text" => {
                let part = payload.as_str().context("Text payload must be a string")?;
                text.push_str(part);
                if !part.is_empty() {
                    content.push(block.clone());
                }
            }
            "Image" => {
                ensure!(
                    payload.is_object() || payload.is_string(),
                    "invalid opaque Image payload"
                );
                images.push(payload.clone());
                content.push(block.clone());
                counts.images += 1;
            }
            "Thinking" | "RedactedThinking" => {
                counts.reasoning_blocks_omitted += 1;
                counts.reasoning_bytes_omitted += canonical(payload)?.len() as u64;
            }
            _ => bail!(
                "unsupported content variant (only Text, Image, and explicitly omitted thinking are accepted here)"
            ),
        }
    }
    Ok((content, text, images))
}

fn metadata(value: &Value, allowed: &[&str], counts: &mut SourceCounts) -> Result<()> {
    for key in value
        .as_object()
        .context("message payload must be an object")?
        .keys()
    {
        ensure!(allowed.contains(&key.as_str()), "unsupported message field");
        if matches!(key.as_str(), "id" | "reasoning_details") {
            counts.ignored_metadata_entries += 1;
        }
    }
    if let Some(reasoning) = value.get("reasoning_details") {
        if !reasoning.is_null() {
            counts.reasoning_metadata_entries_omitted += 1;
            counts.reasoning_bytes_omitted += canonical(reasoning)?.len() as u64;
        }
    }
    Ok(())
}

fn validate_mention(payload: &Value) -> Result<()> {
    ensure!(
        payload
            .as_object()
            .context("Mention must be an object")?
            .keys()
            .all(|key| matches!(key.as_str(), "uri" | "content")),
        "unsupported Mention field"
    );
    string_field(payload, "content")?;
    let (kind, uri) = variant(field(payload, "uri")?)?;
    let allowed: &[&str] = match kind {
        "File" | "Directory" => &["abs_path"],
        "PastedImage" => bail!("pasted image mentions must be saved as Image content"),
        "Symbol" => &["abs_path", "name", "line_range"],
        "Thread" | "Rule" => &["id", "name"],
        "Diagnostics" => &["include_errors", "include_warnings"],
        "Selection" => &["abs_path", "line_range", "column"],
        "Fetch" => &["url"],
        "TerminalSelection" => &["line_count"],
        "GitDiff" => &["base_ref"],
        "MergeConflict" => &["file_path"],
        "Skill" => &["name", "source", "skill_file_path"],
        _ => bail!("unsupported Mention URI variant"),
    };
    ensure!(
        uri.as_object()
            .context("Mention URI must be an object")?
            .keys()
            .all(|key| allowed.contains(&key.as_str())),
        "unsupported Mention URI field"
    );
    if let Some(range) = uri.get("line_range") {
        ensure!(
            range
                .as_object()
                .context("Mention line range must be an object")?
                .keys()
                .all(|key| matches!(key.as_str(), "start" | "end")),
            "unsupported Mention line range field"
        );
    }
    Ok(())
}

fn modern_user_content(
    entry: &Value,
    counts: &mut SourceCounts,
    cancellation: &AtomicBool,
) -> Result<(Vec<Value>, String, Vec<Value>)> {
    // Use production formatting for mentions and synthetic user messages, without
    // resolving paths, fetching URLs, or loading any referenced thread or skill.
    let mut formatted_entry = entry.clone();
    let mut images = Vec::new();
    // The typed formatter must not discard fields from opaque saved images.
    if let Some(content) = formatted_entry
        .get_mut("User")
        .and_then(|user| user.get_mut("content"))
        .and_then(Value::as_array_mut)
    {
        for block in content {
            if let Some(image) = block.get_mut("Image") {
                images.push(image.clone());
                *image = json!({"source": ""});
            }
        }
    }
    let message: crate::Message = serde_json::from_value(formatted_entry)
        .map_err(|_| anyhow::anyhow!("invalid modern user message"))?;
    let content = message
        .to_request()
        .into_iter()
        .flat_map(|message| message.content)
        .collect::<Vec<_>>();
    let mut content = serde_json::to_value(content)?;
    let mut images = images.into_iter();
    for block in content
        .as_array_mut()
        .context("formatted content must be an array")?
    {
        if let Some(image) = block.get_mut("Image") {
            *image = images
                .next()
                .context("formatter added an unexpected image")?;
        }
    }
    ensure!(images.next().is_none(), "formatter omitted a saved image");
    parse_content(&content, counts, cancellation)
}

fn modern_archive_records(
    entry: &Value,
    tools: &[RecordedTool],
    cancellation: &AtomicBool,
) -> Result<Vec<RecordedArchiveRecord>> {
    // Request formatting groups mention context; the journal instead preserves
    // each source block as its own record, including tool results at the call site.
    let mut records = Vec::new();
    let mut push = |kind, text: String, images: Vec<Value>| {
        if !text.is_empty() || !images.is_empty() {
            records.push(RecordedArchiveRecord { kind, text, images });
        }
    };
    if entry.as_str() == Some("Resume") {
        push("user", "Continue where you left off".into(), Vec::new());
        return Ok(records);
    }
    let (tag, payload) = variant(entry)?;
    if tag == "Compaction" {
        push("note", string_field(payload, "Summary")?, Vec::new());
        return Ok(records);
    }
    let kind = if tag == "User" { "user" } else { "unii" };
    for block in field(payload, "content")?
        .as_array()
        .context("content must be an array")?
    {
        check_cancelled(cancellation)?;
        let (tag, payload) = variant(block)?;
        match tag {
            "Text" => push(
                kind,
                payload
                    .as_str()
                    .context("Text payload must be a string")?
                    .into(),
                Vec::new(),
            ),
            "Image" => push(kind, String::new(), vec![payload.clone()]),
            "Mention" => {
                let content = string_field(payload, "content")?;
                let text = if content.is_empty() {
                    let uri: acp_thread::MentionUri =
                        serde_json::from_value(field(payload, "uri")?.clone())
                            .map_err(|_| anyhow::anyhow!("invalid Mention URI"))?;
                    uri.as_link().to_string()
                } else {
                    content
                };
                push("note", text, Vec::new());
            }
            "ToolUse" => {
                let id = string_field(payload, "id")?;
                let tool = tools
                    .iter()
                    .find(|tool| tool.id == id)
                    .context("missing recorded tool")?;
                push(
                    "tool",
                    format!("{} {}", tool.name, tool.raw_input),
                    Vec::new(),
                );
                if let Some(result) = &tool.result {
                    for content in result.replayed_content() {
                        check_cancelled(cancellation)?;
                        let (tag, payload) = variant(&content)?;
                        match tag {
                            "Text" => push(
                                "echo",
                                clip_memory_output(
                                    payload.as_str().context("Text payload must be a string")?,
                                ),
                                Vec::new(),
                            ),
                            "Image" => push("echo", String::new(), vec![payload.clone()]),
                            _ => bail!("unsupported archived tool result content"),
                        }
                    }
                }
            }
            "Thinking" | "RedactedThinking" => {}
            _ => bail!("unsupported archived content variant"),
        }
    }
    Ok(records)
}

fn recorded_user(
    content: Vec<Value>,
    text: String,
    images: Vec<Value>,
    archive_records: Vec<RecordedArchiveRecord>,
) -> RecordedMessage {
    RecordedMessage {
        role: Role::User,
        content,
        text,
        images,
        tools: Vec::new(),
        context: String::new(),
        archive_records: Some(archive_records),
    }
}

fn parse_modern(value: &Value, cancellation: &AtomicBool) -> Result<Conversation> {
    let date = string_field(value, "updated_at")?
        .parse()
        .context("invalid updated_at timestamp")?;
    let object = value
        .as_object()
        .context("conversation must be an object")?;
    ensure!(
        object.keys().all(
            |key| matches!(key.as_str(), "version" | "messages" | "model")
                || TOP_LEVEL_METADATA.contains(&key.as_str())
        ),
        "unsupported conversation field"
    );
    if let Some(export) = value.get("_cache_replay_export") {
        ensure!(
            export.is_object(),
            "_cache_replay_export metadata must be an object"
        );
    }
    let model = value.get("model").cloned().unwrap_or(Value::Null);
    validate_model(&model)?;
    let mut counts = SourceCounts::default();
    for key in TOP_LEVEL_METADATA {
        if object.contains_key(*key) {
            counts.ignored_metadata_entries += 1;
            counts
                .ignored_top_level_metadata_fields
                .insert((*key).into(), 1);
        }
    }
    let mut messages = Vec::new();
    for entry in field(value, "messages")?
        .as_array()
        .context("messages must be an array")?
    {
        check_cancelled(cancellation)?;
        if entry.as_str() == Some("Resume") {
            counts.resume_messages += 1;
            let (content, text, images) = modern_user_content(entry, &mut counts, cancellation)?;
            messages.push(recorded_user(
                content,
                text,
                images,
                modern_archive_records(entry, &[], cancellation)?,
            ));
            continue;
        }
        let (tag, payload) = variant(entry)?;
        if tag == "Compaction" {
            let (kind, summary) = variant(payload)?;
            match kind {
                "Summary" => ensure!(summary.is_string(), "compaction summary must be a string"),
                "ProviderNative" => {
                    bail!("provider-native compaction cannot be reconstructed offline")
                }
                _ => bail!("unsupported compaction variant"),
            }
            counts.summary_compactions += 1;
            let (content, text, images) = modern_user_content(entry, &mut counts, cancellation)?;
            messages.push(recorded_user(
                content,
                text,
                images,
                modern_archive_records(entry, &[], cancellation)?,
            ));
            continue;
        }
        let role = match tag {
            "User" => {
                counts.users += 1;
                Role::User
            }
            "Agent" => {
                counts.assistants += 1;
                Role::Assistant
            }
            _ => bail!("unsupported message variant; expected User or Agent"),
        };
        metadata(
            payload,
            if role == Role::User {
                &["id", "content"]
            } else {
                &["id", "content", "tool_results", "reasoning_details"]
            },
            &mut counts,
        )?;
        let blocks = field(payload, "content")?
            .as_array()
            .context("content must be an array")?;
        let mut ordinary = Vec::new();
        let mut tools = Vec::new();
        let mut ids = BTreeSet::new();
        for block in blocks {
            check_cancelled(cancellation)?;
            let (kind, tool) = variant(block)?;
            if kind == "Mention" {
                ensure!(role == Role::User, "Agent cannot contain Mention");
                validate_mention(tool)?;
                counts.mentions += 1;
            }
            if kind != "ToolUse" {
                ordinary.push(block.clone());
                continue;
            }
            ensure!(role == Role::Assistant, "User cannot contain ToolUse");
            ensure!(
                tool.as_object()
                    .context("ToolUse must be an object")?
                    .keys()
                    .all(|key| matches!(
                        key.as_str(),
                        "id" | "name"
                            | "raw_input"
                            | "input"
                            | "is_input_complete"
                            | "thought_signature"
                    )),
                "unsupported ToolUse field"
            );
            if let Some(signature) = tool.get("thought_signature") {
                if !signature.is_null() {
                    counts.reasoning_metadata_entries_omitted += 1;
                    counts.reasoning_bytes_omitted += canonical(signature)?.len() as u64;
                }
            }
            let id = string_field(tool, "id")?;
            ensure!(ids.insert(id.clone()), "duplicate tool call id");
            let complete = bool_field(tool, "is_input_complete")?;
            counts.tool_calls += 1;
            if !complete {
                counts.incomplete_tool_calls += 1;
            }
            tools.push(RecordedTool {
                id,
                name: string_field(tool, "name")?,
                raw_input: string_field(tool, "raw_input")?,
                input: field(tool, "input")?.clone(),
                complete,
                result: None,
            });
        }
        let (content, text, images) = if ordinary.iter().any(|block| block.get("Mention").is_some())
        {
            modern_user_content(entry, &mut counts, cancellation)?
        } else {
            parse_content(&Value::Array(ordinary), &mut counts, cancellation)?
        };
        if let Some(results) = payload.get("tool_results") {
            for (id, result) in results
                .as_object()
                .context("tool_results must be an object")?
            {
                check_cancelled(cancellation)?;
                ensure!(
                    result
                        .as_object()
                        .context("tool result must be an object")?
                        .keys()
                        .all(|key| matches!(
                            key.as_str(),
                            "tool_use_id" | "tool_name" | "is_error" | "content" | "output"
                        )),
                    "unsupported tool result field"
                );
                let tool = tools
                    .iter_mut()
                    .find(|tool| &tool.id == id)
                    .context("tool result has no matching call")?;
                if !tool.complete {
                    counts.results_with_incomplete_input_flag += 1;
                }
                ensure!(
                    string_field(result, "tool_use_id")? == *id,
                    "tool result id mismatch"
                );
                ensure!(
                    string_field(result, "tool_name")? == tool.name,
                    "tool result name mismatch"
                );
                // Output is saved bookkeeping, not a second provider content block.
                if let Some(output) = result.get("output") {
                    ignore_tool_output(output, &mut counts)?;
                }
                let saved_content = field(result, "content")?;
                let legacy_content;
                let saved_content = if saved_content.is_array() {
                    saved_content
                } else {
                    legacy_content = json!([saved_content]);
                    &legacy_content
                };
                let (content, text, images) =
                    parse_content(saved_content, &mut counts, cancellation)?;
                let is_error = bool_field(result, "is_error")?;
                counts.tool_results += 1;
                if is_error {
                    counts.error_results += 1;
                }
                tool.result = Some(RecordedResult::new(
                    is_error,
                    content,
                    text,
                    images,
                    &mut counts,
                ));
            }
        }
        counts.pending_tool_calls +=
            tools.iter().filter(|tool| tool.result.is_none()).count() as u64;
        let archive_records = modern_archive_records(entry, &tools, cancellation)?;
        messages.push(RecordedMessage {
            role,
            content,
            text,
            images,
            tools,
            context: String::new(),
            archive_records: Some(archive_records),
        });
    }
    Ok(Conversation {
        version: "0.3.0".into(),
        date,
        model,
        messages,
        counts,
        stored_usage: stored_usage(value)?,
    })
}

fn validate_model(model: &Value) -> Result<()> {
    if model.is_null() {
        return Ok(());
    }
    let object = model
        .as_object()
        .context("model metadata must be an object or null")?;
    ensure!(
        object
            .keys()
            .all(|key| matches!(key.as_str(), "provider" | "model")),
        "unsupported model metadata field"
    );
    string_field(model, "provider")?;
    string_field(model, "model")?;
    Ok(())
}

fn parse_old(bytes: &[u8], cancellation: &AtomicBool) -> Result<Conversation> {
    // The serde error may quote transcript data; never forward it to stderr/stdout.
    let thread = SerializedThread::from_json(bytes)
        .map_err(|_| anyhow::anyhow!("invalid v0.2.0 SerializedThread"))?;
    let mut counts = SourceCounts::default();
    let mut messages = Vec::new();
    for message in thread.messages {
        check_cancelled(cancellation)?;
        match message.role {
            Role::User => counts.users += 1,
            Role::Assistant => counts.assistants += 1,
            Role::System => {
                bail!("saved system messages are not supported; supply a prefix JSON instead")
            }
        }
        counts.hidden_messages += u64::from(message.is_hidden);
        counts.saved_creases += message.creases.len() as u64;
        counts.ignored_metadata_entries +=
            u64::from(message.is_hidden) + message.creases.len() as u64;
        let mut blocks = Vec::new();
        for segment in message.segments {
            check_cancelled(cancellation)?;
            match segment {
                SerializedMessageSegment::Text { text } => blocks.push(json!({"Text": text})),
                SerializedMessageSegment::Thinking { text, signature } => {
                    counts.reasoning_blocks_omitted += 1;
                    counts.reasoning_bytes_omitted +=
                        text.len() as u64 + signature.as_ref().map_or(0, |s| s.len() as u64);
                }
                SerializedMessageSegment::RedactedThinking { data } => {
                    counts.reasoning_blocks_omitted += 1;
                    counts.reasoning_bytes_omitted += data.len() as u64;
                }
            }
        }
        let (content, text, images) =
            parse_content(&Value::Array(blocks), &mut counts, cancellation)?;
        let mut tools = Vec::new();
        let mut results = BTreeMap::new();
        for result in message.tool_results {
            check_cancelled(cancellation)?;
            ensure!(
                results
                    .insert(result.tool_use_id.to_string(), result)
                    .is_none(),
                "duplicate recorded tool result"
            );
        }
        let mut ids = BTreeSet::new();
        for tool in message.tool_uses {
            check_cancelled(cancellation)?;
            ensure!(
                message.role == Role::Assistant,
                "non-assistant tool calls are unsupported"
            );
            let id = tool.id.to_string();
            ensure!(ids.insert(id.clone()), "duplicate tool call id");
            counts.tool_calls += 1;
            let result = if let Some(result) = results.remove(&id) {
                let block = serde_json::to_value(result.content)
                    .context("cannot serialize recorded tool content")?;
                let (content, text, images) =
                    parse_content(&json!([block]), &mut counts, cancellation)?;
                counts.tool_results += 1;
                if result.is_error {
                    counts.error_results += 1;
                }
                if let Some(output) = &result.output {
                    ignore_tool_output(output, &mut counts)?;
                }
                Some(RecordedResult::new(
                    result.is_error,
                    content,
                    text,
                    images,
                    &mut counts,
                ))
            } else {
                counts.pending_tool_calls += 1;
                None
            };
            tools.push(RecordedTool {
                id,
                name: tool.name.to_string(),
                raw_input: String::from_utf8(canonical(&tool.input)?)
                    .context("canonical input was not UTF-8")?,
                input: tool.input,
                complete: true,
                result,
            });
        }
        ensure!(results.is_empty(), "tool result has no matching call");
        messages.push(RecordedMessage {
            role: message.role,
            content,
            text,
            images,
            tools,
            context: message.context,
            archive_records: None,
        });
    }
    Ok(Conversation {
        version: "0.2.0".into(),
        date: thread.updated_at,
        model: serde_json::to_value(thread.model)?,
        messages,
        counts,
        stored_usage: Value::Null,
    })
}

#[cfg(test)]
fn parse_conversation(bytes: &[u8]) -> Result<Conversation> {
    parse_conversation_cancellable(bytes, &AtomicBool::new(false))
}

fn parse_conversation_cancellable(bytes: &[u8], cancellation: &AtomicBool) -> Result<Conversation> {
    check_cancelled(cancellation)?;
    let value: Value = serde_json::from_slice(bytes).map_err(|_| {
        anyhow::anyhow!("input is not a single valid JSON document; JSONL is not supported")
    })?;
    match value.get("version").and_then(Value::as_str) {
        Some("0.3.0") => parse_modern(&value, cancellation),
        Some("0.2.0") => {
            let mut conversation = parse_old(bytes, cancellation)?;
            conversation.stored_usage = stored_usage(&value)?;
            Ok(conversation)
        }
        _ => bail!("unsupported conversation format; expected version 0.3.0 or 0.2.0"),
    }
}

fn canonical(value: &impl Serialize) -> Result<Vec<u8>> {
    fn sort(value: Value) -> Value {
        match value {
            Value::Object(object) => {
                let sorted: BTreeMap<_, _> = object
                    .into_iter()
                    .map(|(key, value)| (key, sort(value)))
                    .collect();
                Value::Object(sorted.into_iter().collect())
            }
            Value::Array(values) => Value::Array(values.into_iter().map(sort).collect()),
            value => value,
        }
    }
    Ok(serde_json::to_vec(&sort(serde_json::to_value(value)?))?)
}

#[derive(Clone)]
struct LiveMessage {
    role: Role,
    content: Vec<Value>,
    cache_eligible: bool,
}

impl RecordedMessage {
    fn live(&self) -> Vec<LiveMessage> {
        let mut content = Vec::new();
        if !self.context.is_empty() {
            content.push(json!({"Text": self.context}));
        }
        content.extend(self.content.clone());
        let mut results = Vec::new();
        let eligible = self.tools.iter().all(|tool| tool.result.is_some());
        for tool in &self.tools {
            if let Some(result) = &tool.result {
                content.push(json!({"ToolUse": {"id": tool.id, "name": tool.name, "raw_input": tool.raw_input, "input": tool.input, "is_input_complete": tool.complete}}));
                let result_content = result.live_content();
                results.push(json!({"ToolResult": {"tool_use_id": tool.id, "tool_name": tool.name, "is_error": result.is_error, "content": result_content, "output": null}}));
            }
        }
        let mut messages = vec![LiveMessage {
            role: self.role,
            content,
            cache_eligible: eligible,
        }];
        if !results.is_empty() {
            messages.push(LiveMessage {
                role: Role::User,
                content: results,
                cache_eligible: eligible,
            });
        }
        messages
    }

    #[cfg(test)]
    fn archive(
        &self,
        memory: &mut InfiniteContext,
        directory: &Scratch,
        date: DateTime<Utc>,
    ) -> Result<()> {
        self.archive_cancellable(memory, directory, date, &AtomicBool::new(false))
    }

    fn archive_cancellable(
        &self,
        memory: &mut InfiniteContext,
        directory: &Scratch,
        date: DateTime<Utc>,
        cancellation: &AtomicBool,
    ) -> Result<()> {
        check_cancelled(cancellation)?;
        if let Some(records) = &self.archive_records {
            for record in records {
                check_cancelled(cancellation)?;
                directory.prepare(date)?;
                memory.append_with_images(
                    record.kind,
                    &record.text,
                    record.images.clone(),
                    date,
                )?;
            }
            return Ok(());
        }
        directory.prepare(date)?;
        if !self.text.is_empty() || !self.images.is_empty() {
            memory.append_with_images(
                if self.role == Role::User {
                    "user"
                } else {
                    "unii"
                },
                &self.text,
                self.images.clone(),
                date,
            )?;
        }
        if !self.context.is_empty() {
            directory.prepare(date)?;
            memory.append("note", &self.context, date)?;
        }
        for tool in &self.tools {
            check_cancelled(cancellation)?;
            // Pending calls are not finished source messages in production sync_memory.
            if tool.result.is_none() {
                continue;
            }
            directory.prepare(date)?;
            memory.append("tool", &format!("{} {}", tool.name, tool.raw_input), date)?;
            if let Some(result) = &tool.result {
                directory.prepare(date)?;
                if !result.images.is_empty() {
                    memory.append_with_images(
                        "echo",
                        if result.text.is_empty() {
                            "Tool returned an image."
                        } else {
                            &result.text
                        },
                        result.images.clone(),
                        date,
                    )?;
                } else {
                    memory.append("echo", &result.text, date)?;
                }
            }
        }
        Ok(())
    }
}

struct Scratch {
    path: PathBuf,
    removed: bool,
}

impl Scratch {
    fn new() -> Result<Self> {
        ensure!(
            cfg!(unix),
            "private replay journals currently require Unix file permissions"
        );

        let path =
            std::env::temp_dir().join(format!("zed-infinite-context-replay-{}", Uuid::new_v4()));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&path)?;
        let scratch = Self {
            path,
            removed: false,
        };
        for name in ["main", "tree", "images"] {
            builder.create(scratch.path.join(name))?;
        }
        private_file(&scratch.path.join("lock"))?;
        scratch.prepare(Utc::now())?;
        Ok(scratch)
    }

    fn prepare(&self, date: DateTime<Utc>) -> Result<()> {
        // The 0700 containing directory protects even replacement files created
        // internally by the engine. Never change the process-wide umask: replay
        // can run alongside unrelated UI/background file writers.
        for name in ["view.json.tmp", "context.json.tmp", "metadata.json.tmp"] {
            private_file(&self.path.join(name))?;
        }
        for (name, day) in [("main", date), ("images", date), ("tree", Utc::now())] {
            private_file(
                &self
                    .path
                    .join(name)
                    .join(format!("{}.jsonl", day.format("%Y-%m-%d"))),
            )?;
        }
        Ok(())
    }

    fn count_nodes(&self, metrics: &mut EngineMetrics, cancellation: &AtomicBool) -> Result<()> {
        #[derive(Deserialize)]
        struct Coordinate {
            l: u32,
        }

        for entry in fs::read_dir(self.path.join("tree"))? {
            check_cancelled(cancellation)?;
            let path = entry?.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "jsonl")
            {
                continue;
            }
            for line in BufReader::new(fs::File::open(path)?).lines() {
                check_cancelled(cancellation)?;
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let node: Coordinate = serde_json::from_str(&line)
                    .map_err(|_| anyhow::anyhow!("invalid replay summary journal"))?;
                metrics.total_nodes += 1;
                if node.l == 0 {
                    metrics.leaf_nodes += 1;
                } else {
                    metrics.merge_nodes += 1;
                }
            }
        }
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        fs::remove_dir_all(&self.path).context("remove private replay journal")?;
        self.removed = true;
        Ok(())
    }
}

fn with_scratch<T>(scratch: Scratch, run: impl FnOnce(&Scratch) -> Result<T>) -> Result<T> {
    with_scratch_cleanup(scratch, run, Scratch::cleanup)
}

fn with_scratch_cleanup<T>(
    mut scratch: Scratch,
    run: impl FnOnce(&Scratch) -> Result<T>,
    cleanup: impl FnOnce(&mut Scratch) -> Result<()>,
) -> Result<T> {
    let result = run(&scratch);
    match (result, cleanup(&mut scratch)) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (result, Err(cleanup)) => {
            let source = match result {
                Ok(_) => cleanup,
                Err(replay) => replay.context(cleanup),
            };
            Err(ReplayCleanupFailed { source }.into())
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !self.removed {
            if let Err(error) = fs::remove_dir_all(&self.path) {
                log::error!(
                    "Offline replay private journal fallback cleanup failed at {}: {error}",
                    self.path.display()
                );
            }
        }
    }
}

fn private_file(path: &Path) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    drop(options.open(path)?);
    Ok(())
}

#[cfg(test)]
fn extractive_summary(input: &str, limit: usize) -> Result<String> {
    extractive_summary_cancellable(input, limit, &AtomicBool::new(false))
}

fn extractive_summary_cancellable(
    input: &str,
    limit: usize,
    cancellation: &AtomicBool,
) -> Result<String> {
    check_cancelled(cancellation)?;
    let mut flat = String::with_capacity(input.len());
    for (index, ch) in input.chars().enumerate() {
        if index % 4096 == 0 {
            check_cancelled(cancellation)?;
        }
        flat.push(if matches!(ch, '\r' | '\n') { ' ' } else { ch });
    }
    check_cancelled(cancellation)?;
    let flat = flat.trim();
    if flat.is_empty() {
        return Ok("[empty textual input]".into());
    }
    if flat.len() <= limit {
        return Ok(flat.into());
    }
    let marker = " [...] ";
    let budget = limit.saturating_sub(marker.len());
    let mut head = budget / 2;
    while !flat.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = flat.len().saturating_sub(budget - head);
    while !flat.is_char_boundary(tail) {
        tail += 1;
    }
    Ok(format!("{}{marker}{}", &flat[..head], &flat[tail..]))
}

struct Blocks {
    data: Vec<u8>,
    ends: Vec<usize>,
    marks: Vec<usize>,
}

impl Blocks {
    #[cfg(test)]
    fn new(request: &LanguageModelRequest, live: &[LiveMessage]) -> Result<Self> {
        Self::new_cancellable(request, live, &AtomicBool::new(false))
    }

    fn new_cancellable(
        request: &LanguageModelRequest,
        live: &[LiveMessage],
        cancellation: &AtomicBool,
    ) -> Result<Self> {
        check_cancelled(cancellation)?;
        let mut blocks = Self {
            data: Vec::new(),
            ends: Vec::new(),
            marks: Vec::new(),
        };
        blocks.push(json!({"tools": request.tools}))?;
        for message in &request.messages {
            check_cancelled(cancellation)?;
            for content in &message.content {
                check_cancelled(cancellation)?;
                blocks.push(json!({"role": message.role, "content": content}))?;
            }
            if message.cache && !message.content.is_empty() {
                blocks.mark();
            }
        }
        let mut last_eligible = None;
        for message in live {
            check_cancelled(cancellation)?;
            for content in &message.content {
                check_cancelled(cancellation)?;
                blocks.push(json!({"role": message.role, "content": content}))?;
            }
            if message.cache_eligible && !message.content.is_empty() {
                last_eligible = blocks.ends.len().checked_sub(1);
            }
        }
        if let Some(mark) = last_eligible {
            blocks.marks.push(mark);
        }
        Ok(blocks)
    }

    fn push(&mut self, value: Value) -> Result<()> {
        self.data.extend(canonical(&value)?);
        self.data.push(b'\n');
        self.ends.push(self.data.len());
        Ok(())
    }

    fn mark(&mut self) {
        if let Some(index) = self.ends.len().checked_sub(1) {
            self.marks.push(index);
        }
    }

    fn bytes(&self) -> u64 {
        self.data.len() as u64
    }

    fn prefix(&self, index: usize) -> Option<&[u8]> {
        self.ends.get(index).and_then(|end| self.data.get(..*end))
    }
}

#[derive(Default, Serialize)]
/// Byte-weighted cache traffic, not provider tokens or billable usage.
pub struct Traffic {
    pub requests: u64,
    pub requests_with_any_hit: u64,
    pub input_bytes: u64,
    pub cache_read_bytes: u64,
    pub cache_write_bytes: u64,
    pub uncached_bytes: u64,
    pub reply_bytes: u64,
    pub marked_prefixes: u64,
    pub ineligible_marks: u64,
    #[serde(rename = "cacheReadPercentage")]
    pub cache_read_percentage: Option<f64>,
}

fn percentage(numerator: u64, denominator: u64) -> Option<f64> {
    (denominator > 0).then(|| 100.0 * numerator as f64 / denominator as f64)
}

impl Traffic {
    fn record(&mut self, blocks: &Blocks, hit: u64, writes: u64, ineligible: u64, reply: u64) {
        self.requests += 1;
        self.requests_with_any_hit += u64::from(hit > 0);
        self.input_bytes += blocks.bytes();
        self.cache_read_bytes += hit;
        self.cache_write_bytes += writes;
        self.uncached_bytes += blocks.bytes().saturating_sub(hit);
        self.reply_bytes += reply;
        self.marked_prefixes += blocks.marks.len() as u64;
        self.ineligible_marks += ineligible;
        self.cache_read_percentage = percentage(self.cache_read_bytes, self.input_bytes);
    }
}

#[derive(Default, Serialize)]
pub struct Comparison {
    pub warm: Traffic,
    pub cold: Traffic,
    #[serde(rename = "inputReductionVsColdPercentage")]
    pub input_reduction_vs_cold_percentage: Option<f64>,
    #[serde(rename = "totalInputReductionVsColdPercentage")]
    pub total_input_reduction_vs_cold_percentage: Option<f64>,
}

impl Comparison {
    fn update_rates(&mut self) {
        self.input_reduction_vs_cold_percentage = percentage(
            self.cold
                .uncached_bytes
                .saturating_sub(self.warm.uncached_bytes),
            self.cold.uncached_bytes,
        );
        self.total_input_reduction_vs_cold_percentage = percentage(
            self.cold.input_bytes.saturating_sub(self.warm.input_bytes),
            self.cold.input_bytes,
        );
    }

    fn combined(turn: &Self, summary: &Self) -> Self {
        fn sum(a: &Traffic, b: &Traffic) -> Traffic {
            let input_bytes = a.input_bytes + b.input_bytes;
            let cache_read_bytes = a.cache_read_bytes + b.cache_read_bytes;
            Traffic {
                requests: a.requests + b.requests,
                requests_with_any_hit: a.requests_with_any_hit + b.requests_with_any_hit,
                input_bytes,
                cache_read_bytes,
                cache_write_bytes: a.cache_write_bytes + b.cache_write_bytes,
                uncached_bytes: a.uncached_bytes + b.uncached_bytes,
                reply_bytes: a.reply_bytes + b.reply_bytes,
                marked_prefixes: a.marked_prefixes + b.marked_prefixes,
                ineligible_marks: a.ineligible_marks + b.ineligible_marks,
                cache_read_percentage: percentage(cache_read_bytes, input_bytes),
            }
        }
        let mut combined = Self {
            warm: sum(&turn.warm, &summary.warm),
            cold: sum(&turn.cold, &summary.cold),
            ..Default::default()
        };
        combined.update_rates();
        combined
    }
}

struct Cache {
    entries: BTreeMap<(String, Vec<u8>), u64>,
    ttl: u64,
    min_bytes: usize,
}

impl Cache {
    fn new(ttl: u64, min_bytes: usize) -> Self {
        Self {
            entries: BTreeMap::new(),
            ttl,
            min_bytes,
        }
    }

    fn request(
        &mut self,
        scope: &str,
        blocks: &Blocks,
        now: u64,
        reply: u64,
        metrics: &mut Comparison,
        cancellation: &AtomicBool,
    ) -> Result<()> {
        check_cancelled(cancellation)?;
        self.entries.retain(|_, written| {
            cancellation.load(Ordering::Relaxed) || now.saturating_sub(*written) < self.ttl
        });
        check_cancelled(cancellation)?;
        let mut best: Option<(String, Vec<u8>)> = None;
        for mark in &blocks.marks {
            check_cancelled(cancellation)?;
            for index in mark.saturating_sub(LOOKBACK - 1)..=*mark {
                check_cancelled(cancellation)?;
                if let Some(prefix) = blocks.prefix(index) {
                    let key = (scope.to_owned(), prefix.to_vec());
                    if self.entries.contains_key(&key)
                        && best
                            .as_ref()
                            .is_none_or(|(_, bytes)| prefix.len() > bytes.len())
                    {
                        best = Some(key);
                    }
                }
            }
        }
        let hit = best.as_ref().map_or(0, |(_, bytes)| bytes.len() as u64);
        if let Some(key) = best {
            self.entries.insert(key, now);
        }
        let mut writes = 0;
        let mut cold_writes = 0;
        let mut ineligible = 0;
        // A cached read covers earlier marked prefixes. Only the new suffix up to
        // each eligible mark is counted as write traffic, never overlapping bytes.
        let mut stored_end = hit;
        for mark in &blocks.marks {
            check_cancelled(cancellation)?;
            if let Some(prefix) = blocks.prefix(*mark) {
                if prefix.len() < self.min_bytes {
                    ineligible += 1;
                    continue;
                }
                let end = prefix.len() as u64;
                writes += end.saturating_sub(stored_end);
                stored_end = stored_end.max(end);
                cold_writes = cold_writes.max(end);
                self.entries
                    .insert((scope.to_owned(), prefix.to_vec()), now);
            }
        }
        metrics.warm.record(blocks, hit, writes, ineligible, reply);
        metrics
            .cold
            .record(blocks, 0, cold_writes, ineligible, reply);
        metrics.update_rates();
        check_cancelled(cancellation)?;
        Ok(())
    }
}

#[derive(Default, Serialize)]
pub struct EngineMetrics {
    pub archived_records: u64,
    /// Persisted nodes across independent repetitions, including verbatim leaves.
    pub total_nodes: u64,
    pub leaf_nodes: u64,
    pub merge_nodes: u64,
    /// Sum of final frontier sizes across repetitions (not all historical nodes).
    pub main_frontier_nodes: u64,
    pub context_frontier_nodes: u64,
    pub leaf_summary_calls: u64,
    pub merge_summary_calls: u64,
    pub summary_job_input_bytes: u64,
    pub summary_reply_bytes: u64,
    pub largest_summary_job_input_bytes: usize,
    pub largest_summary_reply_bytes: usize,
    pub main_frontier_rewrites: u64,
    pub context_frontier_rewrites: u64,
    pub observed_main_batch_starts: u64,
    pub observed_context_batch_starts: u64,
    pub turn_memory_marked_prefix_changes: u64,
    pub turn_memory_marked_prefix_nonappend_changes: u64,
}

#[derive(Default)]
struct FrontierObserver {
    main: Vec<(u32, u64)>,
    context: Vec<(u32, u64)>,
    main_batch: bool,
    context_batch: bool,
}

impl FrontierObserver {
    fn observe(&mut self, path: &Path, metrics: &mut EngineMetrics) -> Result<()> {
        let main: Vec<(u32, u64)> = serde_json::from_slice(&fs::read(path.join("view.json"))?)?;
        let context: Vec<(u32, u64)> =
            serde_json::from_slice(&fs::read(path.join("context.json"))?)?;
        if !main.starts_with(&self.main) {
            metrics.main_frontier_rewrites += 1;
        }
        if !context.starts_with(&self.context) {
            metrics.context_frontier_rewrites += 1;
        }
        let metadata: Value = match fs::read(path.join("metadata.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
            Err(error) => return Err(error.into()),
        };
        let main_batch = metadata
            .get("main_batch")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let context_batch = metadata
            .get("context_batch")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        metrics.observed_main_batch_starts += u64::from(main_batch && !self.main_batch);
        metrics.observed_context_batch_starts += u64::from(context_batch && !self.context_batch);
        self.main = main;
        self.context = context;
        self.main_batch = main_batch;
        self.context_batch = context_batch;
        Ok(())
    }
}

/// Serializable report with typed, public UI metrics; metadata is not replay content.
#[derive(Serialize)]
pub struct ReplayReport {
    pub benchmark: &'static str,
    pub report_version: u32,
    pub source_format_version: String,
    pub source_model_metadata: Value,
    pub source_counts: SourceCounts,
    pub baseline_usage: Value,
    pub configuration: Value,
    pub assumptions: Value,
    pub metrics: ReplayMetrics,
}

impl ReplayReport {
    pub fn agent(&self) -> &Comparison {
        &self.metrics.turn
    }

    pub fn summary(&self) -> &Comparison {
        &self.metrics.summary
    }

    pub fn combined(&self) -> &Comparison {
        &self.metrics.combined
    }
}

#[derive(Default, Serialize)]
pub struct ReplayMetrics {
    pub turn: Comparison,
    pub summary: Comparison,
    pub combined: Comparison,
    pub engine: EngineMetrics,
    pub simulated_elapsed_seconds: u64,
    pub repetitions: Vec<Repetition>,
}

#[derive(Serialize)]
pub struct Repetition {
    pub index: usize,
    pub turn_requests: u64,
    pub summary_requests: u64,
    pub archived_records: u64,
}

struct Replay<'a> {
    cancellation: &'a AtomicBool,
    args: &'a ReplayOptions,
    prefix: &'a ReplayPrefix,
    cache: Cache,
    turn_scope: String,
    summary_scope: String,
    metrics: ReplayMetrics,
    clock: u64,
}

impl<'a> Replay<'a> {
    fn new(
        args: &'a ReplayOptions,
        prefix: &'a ReplayPrefix,
        model: &Value,
        cancellation: &'a AtomicBool,
    ) -> Result<Self> {
        let model_key = String::from_utf8(canonical(model)?).context("model key is not UTF-8")?;
        let turn_scope = format!("offline-account:{model_key}");
        let summary_scope = if args.shared_model {
            turn_scope.clone()
        } else {
            format!("{turn_scope}:synthetic-separate-summary-model")
        };
        Ok(Self {
            cancellation,
            args,
            prefix,
            cache: Cache::new(args.ttl_seconds, args.min_cache_bytes),
            turn_scope,
            summary_scope,
            metrics: ReplayMetrics::default(),
            clock: 0,
        })
    }

    fn drain(
        &mut self,
        memory: &mut InfiniteContext,
        scratch: &Scratch,
        observer: &mut FrontierObserver,
        date: DateTime<Utc>,
    ) -> Result<()> {
        loop {
            check_cancelled(self.cancellation)?;
            scratch.prepare(date)?;
            let job = memory.next_job()?;
            check_cancelled(self.cancellation)?;
            observer.observe(&scratch.path, &mut self.metrics.engine)?;
            let Some(job) = job else {
                break;
            };
            ensure!(
                job.input.len() > 512,
                "engine returned a verbatim summary as a model job"
            );
            let context_end = if job.key.l == 0 { job.key.i } else { job.end };
            let mut request = self.prefix.request();
            append_memory_view(&mut request, memory.render_view_before(context_end)?);
            request.messages.push(LanguageModelRequestMessage {
                role: Role::User,
                content: vec![MessageContent::Text(memory_summary_task(&job))],
                cache: false,
                reasoning_details: None,
            });
            let reply = extractive_summary_cancellable(
                &job.input,
                self.args.summary_bytes,
                self.cancellation,
            )?;
            ensure!(
                !reply.trim().is_empty() && reply.len() <= 512,
                "invalid extractive placeholder summary"
            );
            self.cache.request(
                &self.summary_scope,
                &Blocks::new_cancellable(&request, &[], self.cancellation)?,
                self.clock,
                reply.len() as u64,
                &mut self.metrics.summary,
                self.cancellation,
            )?;
            if job.key.l == 0 {
                self.metrics.engine.leaf_summary_calls += 1;
            } else {
                self.metrics.engine.merge_summary_calls += 1;
            }
            self.metrics.engine.summary_job_input_bytes += job.input.len() as u64;
            self.metrics.engine.summary_reply_bytes += reply.len() as u64;
            self.metrics.engine.largest_summary_job_input_bytes = self
                .metrics
                .engine
                .largest_summary_job_input_bytes
                .max(job.input.len());
            self.metrics.engine.largest_summary_reply_bytes = self
                .metrics
                .engine
                .largest_summary_reply_bytes
                .max(reply.len());
            check_cancelled(self.cancellation)?;
            scratch.prepare(date)?;
            memory.complete_job(job.key, reply)?;
            observer.observe(&scratch.path, &mut self.metrics.engine)?;
        }
        Ok(())
    }

    fn repeat(&mut self, conversation: &Conversation, index: usize) -> Result<()> {
        check_cancelled(self.cancellation)?;
        with_scratch(Scratch::new()?, |scratch| {
            self.repeat_in(conversation, index, scratch)
        })
    }

    fn repeat_in(
        &mut self,
        conversation: &Conversation,
        index: usize,
        scratch: &Scratch,
    ) -> Result<()> {
        check_cancelled(self.cancellation)?;
        let mut memory = InfiniteContext::open(scratch.path.clone())?;
        let mut observer = FrontierObserver::default();
        let mut live = Vec::new();
        let mut turn_start = 0;
        let mut last_was_user = false;
        let mut previous_memory_prefix: Option<Vec<u8>> = None;
        let turn_before = self.metrics.turn.warm.requests;
        let summary_before = self.metrics.summary.warm.requests;
        for message in &conversation.messages {
            check_cancelled(self.cancellation)?;
            if message.role == Role::User {
                if !last_was_user {
                    live.clear();
                    turn_start = memory.message_count();
                }
                live.extend(message.live());
            } else {
                let mut request = self.prefix.request();
                append_memory_view(&mut request, memory.render_turn_before(turn_start)?);
                let memory_blocks = Blocks::new_cancellable(&request, &[], self.cancellation)?;
                let marked = memory_blocks
                    .marks
                    .last()
                    .and_then(|mark| memory_blocks.prefix(*mark))
                    .map(<[u8]>::to_vec);
                if let (Some(previous), Some(current)) = (&previous_memory_prefix, &marked) {
                    if previous != current {
                        self.metrics.engine.turn_memory_marked_prefix_changes += 1;
                        if !current.starts_with(previous) {
                            self.metrics
                                .engine
                                .turn_memory_marked_prefix_nonappend_changes += 1;
                        }
                    }
                }
                previous_memory_prefix = marked;
                let reply_bytes = canonical(&message.content)?.len()
                    + message
                        .tools
                        .iter()
                        .map(|tool| tool.raw_input.len())
                        .sum::<usize>();
                self.cache.request(
                    &self.turn_scope,
                    &Blocks::new_cancellable(&request, &live, self.cancellation)?,
                    self.clock,
                    reply_bytes as u64,
                    &mut self.metrics.turn,
                    self.cancellation,
                )?;
                // The saved assistant is the reply to this request, never its input.
                live.extend(message.live());
            }
            message.archive_cancellable(
                &mut memory,
                scratch,
                conversation.date,
                self.cancellation,
            )?;
            observer.observe(&scratch.path, &mut self.metrics.engine)?;
            self.drain(&mut memory, scratch, &mut observer, conversation.date)?;
            if message.role == Role::Assistant {
                self.clock = self
                    .clock
                    .checked_add(self.args.request_gap_seconds)
                    .context("simulated clock overflow")?;
                if self.args.pause_after_request == Some(self.metrics.turn.warm.requests) {
                    self.clock = self
                        .clock
                        .checked_add(self.args.pause_seconds)
                        .context("simulated pause overflows clock")?;
                }
            }
            last_was_user = message.role == Role::User;
        }
        check_cancelled(self.cancellation)?;
        let records = memory.message_count();
        self.metrics.engine.archived_records += records;
        scratch.count_nodes(&mut self.metrics.engine, self.cancellation)?;
        self.metrics.engine.main_frontier_nodes += observer.main.len() as u64;
        self.metrics.engine.context_frontier_nodes += observer.context.len() as u64;
        self.metrics.repetitions.push(Repetition {
            index,
            turn_requests: self.metrics.turn.warm.requests - turn_before,
            summary_requests: self.metrics.summary.warm.requests - summary_before,
            archived_records: records,
        });
        Ok(())
    }
}

/// Replays uncompressed v0.3.0 conversation or v0.2.0 `SerializedThread` JSON.
///
/// This synchronous API is intended for a background worker. Cancellation is
/// cooperative between records, content blocks, archive writes, cache scans and
/// summary jobs, and during extractive summaries. Individual serde/engine/I/O
/// operations cannot be interrupted. Cancellation returns `ReplayCancelled`
/// (downcastable from the error), not a partial report. Scratch journals are
/// removed before returning success or failure. If cleanup fails,
/// `ReplayCleanupFailed` takes precedence over cancellation and other errors;
/// callers should check for it first and display its sanitized warning even if
/// the report was dismissed. The original errors remain in its source chain.
/// No model, tool or network calls are made, and source bytes are never logged.
pub fn replay_json(
    bytes: &[u8],
    args: &ReplayOptions,
    cancellation: &AtomicBool,
) -> Result<ReplayReport> {
    check_cancelled(cancellation)?;
    args.validate()?;
    let conversation = parse_conversation_cancellable(bytes, cancellation)?;
    check_cancelled(cancellation)?;
    let default_prefix = ReplayPrefix::default();
    let prefix = args.prefix.as_ref().unwrap_or(&default_prefix);
    let mut tool_names = BTreeSet::new();
    for tool in prefix.request().tools {
        check_cancelled(cancellation)?;
        ensure!(
            tool_names.insert(tool.name),
            "duplicate tool name in prefix/memory schemas"
        );
    }
    let mut replay = Replay::new(args, prefix, &conversation.model, cancellation)?;
    for index in 1..=args.repeats {
        check_cancelled(cancellation)?;
        replay.repeat(&conversation, index)?;
    }
    replay.metrics.simulated_elapsed_seconds = replay.clock;
    replay.metrics.combined = Comparison::combined(&replay.metrics.turn, &replay.metrics.summary);
    check_cancelled(cancellation)?;
    Ok(ReplayReport {
        benchmark: "infinite_context_offline_replay",
        report_version: 1,
        source_format_version: conversation.version,
        source_model_metadata: conversation.model,
        source_counts: conversation.counts,
        baseline_usage: conversation.stored_usage,
        configuration: json!({
            "summary_bytes": args.summary_bytes, "ttl_seconds": args.ttl_seconds,
            "production_tool_output_clipping": true,
            "request_gap_seconds": args.request_gap_seconds, "pause_after_request": args.pause_after_request,
            "pause_seconds": args.pause_seconds, "shared_model": args.shared_model,
            "repeats": args.repeats, "artificial_repetition": args.repeats > 1,
            "min_cache_bytes": args.min_cache_bytes,
            "additional_prefix_supplied": args.prefix.is_some(),
            "additional_system_blocks": prefix.system.len(), "additional_tool_schemas": prefix.tools.len()
        }),
        assumptions: json!({
            "offline_only": true, "live_model_calls": 0, "tool_executions": 0, "network_calls": 0,
            "transcript_logged": false, "saved_instructions_obeyed": false,
            "request_count": "Exactly one turn request before each recorded Agent/assistant entry, including empty entries; no hidden retries inferred.",
            "turn_boundaries": "A User after an Agent starts a fresh raw turn. Consecutive Users are combined. Tool-result User-role blocks do not start turns. All earlier turns use the real main memory frontier.",
            "prompt_coverage": "Fixed include_str infinite_context_prompt.txt plus exact production memory_tools only, unless explicit prefix JSON supplied. Full assistant system prompt, other real tool schemas, project rules, loaded context, and provider transformations are unavailable; supplied prefix is user-provided, not recovered.",
            "cache_units": "UTF-8 bytes of sorted-key compact JSON blocks followed by newline. All tool schemas form the first block, followed by system and role+content blocks. These are NOT tokens, provider wire bytes, billable usage, or latency measurements.",
            "cache_lookup": "Longest exact stored prefix found within 20 content-block boundaries ending at each cache mark (mark plus previous 19). Role, content, tools and system are part of the key. Only marked prefixes are stored, not every intermediate boundary.",
            "cache_ttl": "Valid iff simulated age < TTL. Longest cache read refreshes its TTL; marked prefixes stored/refreshed at an instantaneous response. Expiry is evaluated before reads. TTL zero disables subsequent reuse.",
            "cache_scope": "One synthetic offline account; source provider/model metadata identifies turn scope. Summary scope is a synthetic different model unless --shared-model; no actual summary model or account was captured.",
            "eligibility_limit_not_modeled": true,
            "cache_eligibility": "Provider token minimum, cache quotas, routing, eviction, mark limits, and provider-specific cache policy are NOT modeled. --min-cache-bytes is only a configurable byte proxy (default 0).",
            "cache_traffic": "Read is the longest hit once per request; write is the non-overlapping new suffix from that hit through the furthest eligible mark. Uncached includes new writes and unmarked suffix. Cold baseline has no reuse, same requests and eligible marks.",
            "cache_rates": "cacheReadPercentage = 100 * cache_read_bytes / input_bytes, byte-weighted, not fraction of requests with any hit. inputReductionVsColdPercentage = 100 * (cold.uncached_bytes - warm.uncached_bytes) / cold.uncached_bytes: reduction in uncached processing, NOT total context size. totalInputReductionVsColdPercentage compares total input bytes (zero here because cold reuses the same constructed requests). Empty denominators yield null. Combined metrics include both turn and summary requests.",
            "baseline_usage": "Unmodified stored baseline_usage/cumulative_token_usage metadata only, when present. The source input_tokens definition (whether cache reads are included) is unknown; no normalized cached-token ratio is claimed and these counters are never mixed with simulated byte traffic. Per-request usage IDs and bookkeeping are not printed.",
            "top_level_metadata": "Known non-content fields explicitly allowlisted and counted by field name as excluded from replay input. Includes initial_project_snapshot, detailed_summary, draft_prompt, profile, subagent_context, speed, thinking configuration, UI scroll position, sandbox state, infinite_context, memory_archived, memory_turn_start, measured_cache_usage and _cache_replay_export. Their values are not printed. Unknown top-level fields still error.",
            "summary": "Deterministic extractive head/tail placeholder, whitespace flattened, <= configured UTF-8 byte budget and nonempty. Not a model-generated or semantically validated summary; facts in the omitted middle can be lost. Engine jobs <=512 bytes complete verbatim with no simulated model request.",
            "summary_input_cap": "512 bytes is the completion cap and model-job threshold, NOT an input cap. Real job inputs can be up to a raw 30000-byte record or merged children plus tags; summary requests include real bounded compaction context and production task/ruler.",
            "scheduling": "Archive each source entry then fully drain real queued jobs serially at the same simulated timestamp. Production can run eight summaries concurrently and interleave streaming; no concurrency, retries or model duration modeled. Gap/pause occur after the Agent reply and its summary drain; subsequent summaries can warm their own scope after a pause.",
            "timestamps": "All source records use conversation updated_at because individual timestamps are unavailable. Engine summary journals use wall-clock day; no simulated clock sleeps.",
            "repetition": "Each repetition is a NEW private engine/thread with raw turn boundary reset and memory IDs restarting at zero; cache and simulated clock persist. Artificial repeated content is never concatenated into one thread and does not represent organic history growth.",
            "modern_control_messages": "Resume and summary Compaction entries use production user-request formatting, are explicitly counted and archived, and never infer an additional Agent request. The offline infinite-context simulation retains preceding history rather than applying destructive source compaction. Provider-native compaction is rejected because provider transformations cannot be reconstructed offline.",
            "mentions": "User Mention content uses the current production request formatter with only saved content; no files, URLs, threads, or skills are loaded. Mention variants and fields are validated and counted. PastedImage mentions are rejected; saved images must be Image content.",
            "images": "Opaque saved Image payloads remain canonical JSON in live estimator blocks and in real image archive values. Never decoded, encoded, fetched, or summarized as base64; only engine attachment annotations enter summaries. Unknown non-text variants error.",
            "reasoning": "Thinking, RedactedThinking, thought_signature and reasoning_details are explicitly omitted with counters. Saved title, IDs, token-usage and output bookkeeping are metadata, not request text. Unknown message/content variants error.",
            "tools": "Recorded calls with matching stored results enter live provider-shaped blocks, matching production result-presence gating. Modern archives preserve saved calls at their source positions, followed by per-content echo records when results exist. Missing-result calls are counted and omitted from live requests, never executed. is_input_complete flags are preserved, false flags are counted as incomplete_tool_calls, and stored results paired with false flags are separately counted; no inference about why a recorded flag is false. Stored results become available after their Agent entry, not before its request; finer timing unavailable. Saved output may be arbitrary JSON bookkeeping: explicitly omitted and counted by JSON type and canonical byte size, matching production requests that send content with output=None, never duplicated or substituted as result text. Error flags retained in live blocks; archive echo follows production text/image format, not extra failure labels.",
            "archive_limits": "Real engine splits core messages at 30000 UTF-8 bytes and clips echo head/tail to 30000 Unicode characters including omission marker. Live textual tool results use the exact production clip_memory_output helper with the same 30000-Unicode-character limit, not a byte limit. User/assistant text remains unbounded in live requests.",
            "live_tool_output_clipping": "Combine stored Text blocks into result.text to check the production 30000-character limit. Short results retain individual Text blocks; clipped results emit one Text block at the first original Text position. Opaque Image values remain unchanged and in original relative order. tool_result_original_text_bytes, tool_result_replayed_text_bytes and tool_results_clipped count source results once before JSON encoding and empty-result fallback, not repeated appearances in requests or artificial repeats. Modern archives use the same clipped result blocks as production, retaining per-content order; legacy archives delegate echo clipping to the real engine. No execution.",
            "old_format": "v0.2.0 uses existing SerializedThread serde parser; old raw_input absent, so canonical input JSON reconstructs it. Non-text tool images are retained. Hidden messages and saved creases are counted as UI metadata, never used to omit true request content. Saved system messages are rejected. Main JSONL and other versions are unsupported.",
            "frontier_counts": "Observe actual persisted main/context coordinate frontiers after append/drain/completion. Non-append changes count as rewrites. Batch-start counts are observed rising flags and may miss transitions completed inside one synchronous call. Merge calls and marked-memory-prefix changes are reported separately; not claims of valid semantic summaries.",
            "privacy": "Read-only source, no transcript or summaries printed. UUID temporary directory under std::env::temp_dir with Unix 0700 directories; precreated journal/atomic files use 0600. Engine-created replacements are protected by the enclosing private directories, without changing process-wide umask. Cleanup runs on success, errors and cancellation; failures propagate, Drop fallback logs failures. CLI --report uses create_new and cannot overwrite source or existing files. Non-Unix is rejected rather than claiming ACL privacy."
        }),
        metrics: replay.metrics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_typed_report_are_background_safe() -> Result<()> {
        let options = ReplayOptions::default();
        assert_eq!(options.summary_bytes, 512);
        assert_eq!(options.ttl_seconds, 300);
        assert_eq!(options.request_gap_seconds, 1);
        assert_eq!(options.pause_after_request, None);
        assert_eq!(options.pause_seconds, 600);
        assert!(!options.shared_model);
        assert_eq!(options.repeats, 1);
        assert_eq!(options.min_cache_bytes, 0);
        assert!(options.prefix.is_none());
        options.validate()?;
        let bytes = serde_json::to_vec(&fixture())?;
        let report =
            std::thread::spawn(move || replay_json(&bytes, &options, &AtomicBool::new(false)))
                .join()
                .map_err(|_| anyhow::anyhow!("background replay panicked"))??;
        assert_eq!(report.agent().warm.requests, 2);
        assert_eq!(
            report.combined().warm.requests,
            report.agent().warm.requests + report.summary().warm.requests
        );
        assert_eq!(
            report.combined().warm.cache_read_percentage,
            percentage(
                report.combined().warm.cache_read_bytes,
                report.combined().warm.input_bytes
            )
        );
        let engine = &report.metrics.engine;
        assert_eq!(engine.archived_records, 5);
        assert_eq!(engine.total_nodes, engine.leaf_nodes + engine.merge_nodes);
        assert!(engine.leaf_nodes >= engine.leaf_summary_calls);
        assert!(engine.merge_nodes >= engine.merge_summary_calls);
        let json = serde_json::to_value(&report)?;
        assert_eq!(json["metrics"]["turn"]["warm"]["requests"], 2);
        assert_eq!(json["configuration"]["ttl_seconds"], 300);
        assert_eq!(json["assumptions"]["network_calls"], 0);
        assert!(!serde_json::to_string(&report)?.contains("do not execute this"));
        Ok(())
    }

    #[test]
    fn public_options_are_validated_even_without_cli() -> Result<()> {
        for options in [
            ReplayOptions {
                summary_bytes: 1,
                ..Default::default()
            },
            ReplayOptions {
                repeats: 0,
                ..Default::default()
            },
            ReplayOptions {
                pause_after_request: Some(0),
                ..Default::default()
            },
            ReplayOptions {
                prefix: Some(ReplayPrefix {
                    system: Vec::new(),
                    tools: memory_tools(),
                }),
                ..Default::default()
            },
        ] {
            assert!(
                replay_json(
                    &serde_json::to_vec(&fixture())?,
                    &options,
                    &AtomicBool::new(false)
                )
                .is_err()
            );
        }
        Ok(())
    }

    #[test]
    fn cleanup_failure_takes_precedence_and_drop_removes_scratch() -> Result<()> {
        for result in [
            Ok(()),
            Err(ReplayCancelled.into()),
            Err(anyhow::anyhow!("PRIVATE_REPLAY_CONTENT")),
        ] {
            let cancelled = result
                .as_ref()
                .is_err_and(|error| error.is::<ReplayCancelled>());
            let failed = result.is_err();
            let scratch = Scratch::new()?;
            let path = scratch.path.clone();
            let error = with_scratch_cleanup(
                scratch,
                |_| result,
                |_| {
                    Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        "PRIVATE_CLEANUP_CONTENT",
                    )
                    .into())
                },
            )
            .err()
            .context("expected cleanup failure")?;

            assert!(!path.exists(), "Drop must remove the simulated residue");
            let cleanup = error
                .downcast_ref::<ReplayCleanupFailed>()
                .context("cleanup warning must take precedence")?;
            assert_eq!(cleanup.source.is::<ReplayCancelled>(), cancelled);
            assert!(!error.is::<ReplayCancelled>());
            assert_eq!(error.to_string(), cleanup.to_string());
            assert_eq!(
                cleanup.to_string(),
                "Offline replay could not remove its private journal. Sensitive replay data may remain on disk."
            );
            assert!(!cleanup.to_string().contains("PRIVATE"));
            assert!(
                error
                    .chain()
                    .any(|cause| cause.to_string().contains("PRIVATE_CLEANUP_CONTENT"))
            );
            if failed && !cancelled {
                assert!(
                    error
                        .chain()
                        .any(|cause| cause.to_string().contains("PRIVATE_REPLAY_CONTENT"))
                );
            }
            let error = error.context("background replay failed");
            assert!(error.downcast_ref::<ReplayCleanupFailed>().is_some());
            assert!(!error.is::<ReplayCancelled>());
        }
        Ok(())
    }

    #[test]
    fn successful_cleanup_preserves_original_cancellation_or_error() -> Result<()> {
        for original in [
            anyhow::Error::from(ReplayCancelled),
            anyhow::anyhow!("original replay error"),
        ] {
            let cancelled = original.is::<ReplayCancelled>();
            let message = original.to_string();
            let scratch = Scratch::new()?;
            let path = scratch.path.clone();
            let result: Result<()> = with_scratch(scratch, |_| Err(original));
            let error = result.err().context("expected original replay error")?;
            assert!(!path.exists());
            assert!(!error.is::<ReplayCleanupFailed>());
            assert_eq!(error.is::<ReplayCancelled>(), cancelled);
            assert_eq!(error.to_string(), message);
        }
        Ok(())
    }

    #[test]
    fn pre_cancelled_replay_and_summary_return_typed_cancellation() -> Result<()> {
        let cancellation = AtomicBool::new(true);
        let error = replay_json(
            b"invalid private input",
            &ReplayOptions::default(),
            &cancellation,
        )
        .err()
        .context("expected cancellation")?;
        assert!(error.is::<ReplayCancelled>());
        assert_eq!(error.to_string(), "offline replay cancelled");
        let error = extractive_summary_cancellable("summary", 512, &cancellation)
            .err()
            .context("expected summary cancellation")?;
        assert!(error.is::<ReplayCancelled>());
        Ok(())
    }

    #[test]
    fn cancellation_during_summary_drain_cleans_scratch() -> Result<()> {
        let mut saved = fixture();
        saved["messages"][0]["User"]["content"][0]["Text"] = json!("large input ".repeat(100));
        let conversation = parse_conversation(&serde_json::to_vec(&saved)?)?;
        let options = ReplayOptions::default();
        let prefix = ReplayPrefix::default();
        let cancellation = AtomicBool::new(false);
        let mut replay = Replay::new(&options, &prefix, &conversation.model, &cancellation)?;
        let scratch = Scratch::new()?;
        let path = scratch.path.clone();
        let result = with_scratch(scratch, |scratch| {
            let mut memory = InfiniteContext::open(scratch.path.clone())?;
            conversation
                .messages
                .first()
                .context("missing user")?
                .archive(&mut memory, scratch, conversation.date)?;
            cancellation.store(true, Ordering::Relaxed);
            replay.drain(
                &mut memory,
                scratch,
                &mut FrontierObserver::default(),
                conversation.date,
            )
        });
        assert!(
            result
                .err()
                .context("expected cancellation")?
                .is::<ReplayCancelled>()
        );
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn replay_errors_and_drop_fallback_clean_scratch() -> Result<()> {
        let conversation = parse_conversation(&serde_json::to_vec(&fixture())?)?;
        let options = ReplayOptions {
            request_gap_seconds: u64::MAX,
            ..Default::default()
        };
        let prefix = ReplayPrefix::default();
        let cancellation = AtomicBool::new(false);
        let mut replay = Replay::new(&options, &prefix, &conversation.model, &cancellation)?;
        let scratch = Scratch::new()?;
        let path = scratch.path.clone();
        let result = with_scratch(scratch, |scratch| {
            replay.repeat_in(&conversation, 1, scratch)
        });
        assert_eq!(
            result.err().context("expected clock overflow")?.to_string(),
            "simulated clock overflow"
        );
        assert!(!path.exists());
        let scratch = Scratch::new()?;
        let path = scratch.path.clone();
        drop(scratch);
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn serialized_ui_snapshot_preserves_hidden_text_context_and_tools() -> Result<()> {
        let saved = json!({
            "version": "0.2.0", "summary": "UI_ONLY_PRIVATE_LABEL", "updated_at": "2026-01-01T00:00:00Z",
            "infinite_context": true, "memory_archived": false,
            "messages": [
                {"id": 0, "role": "user", "is_hidden": true,
                 "segments": [{"type": "text", "text": "REAL_HIDDEN_REQUEST"}],
                 "context": "REAL_LOADED_CONTEXT",
                 "creases": [{"start": 0, "end": 1, "icon_path": "UI_ONLY_PRIVATE_ICON", "label": "UI_ONLY_PRIVATE_LABEL"}]},
                {"id": 1, "role": "assistant", "is_hidden": true,
                 "segments": [{"type": "text", "text": "REAL_ASSISTANT_REPLY"}],
                 "tool_uses": [{"id": "call", "name": "terminal", "input": {"command": "DO_NOT_EXECUTE"}}],
                 "tool_results": [{"tool_use_id": "call", "is_error": true,
                     "content": {"Text": "REAL_TOOL_RESULT"}, "output": null}]},
                {"id": 2, "role": "assistant", "segments": []}
            ]
        });
        let snapshot: SerializedThread = serde_json::from_value(saved)?;
        let bytes = serde_json::to_vec(&snapshot)?;
        let conversation = parse_conversation(&bytes)?;
        assert_eq!(conversation.counts.hidden_messages, 2);
        assert_eq!(conversation.counts.saved_creases, 1);
        let user = conversation
            .messages
            .first()
            .context("missing hidden user")?;
        let live =
            String::from_utf8(Blocks::new(&ReplayPrefix::default().request(), &user.live())?.data)?;
        assert!(live.contains("REAL_HIDDEN_REQUEST"));
        assert!(live.contains("REAL_LOADED_CONTEXT"));
        assert!(!live.contains("UI_ONLY_PRIVATE"));
        let assistant = conversation
            .messages
            .get(1)
            .context("missing hidden assistant")?;
        let live = String::from_utf8(
            Blocks::new(&ReplayPrefix::default().request(), &assistant.live())?.data,
        )?;
        assert!(live.contains("REAL_ASSISTANT_REPLY"));
        assert!(live.contains("REAL_TOOL_RESULT"));
        assert!(live.contains("DO_NOT_EXECUTE"));
        with_scratch(Scratch::new()?, |scratch| {
            let mut memory = InfiniteContext::open(scratch.path.clone())?;
            user.archive(&mut memory, scratch, conversation.date)?;
            assistant.archive(&mut memory, scratch, conversation.date)?;
            let mut archive = String::new();
            for id in 0..memory.message_count() {
                archive.push_str(&memory.zoom(id, 1, 0)?);
            }
            assert!(archive.contains("REAL_HIDDEN_REQUEST"));
            assert!(archive.contains("REAL_LOADED_CONTEXT"));
            assert!(archive.contains("REAL_TOOL_RESULT"));
            assert!(!archive.contains("UI_ONLY_PRIVATE"));
            Ok(())
        })?;
        let report = replay_json(&bytes, &ReplayOptions::default(), &AtomicBool::new(false))?;
        assert_eq!(report.agent().warm.requests, 2);
        assert_eq!(report.source_counts.hidden_messages, 2);
        assert_eq!(report.source_counts.saved_creases, 1);
        assert_eq!(report.source_counts.tool_results, 1);
        assert_eq!(report.metrics.engine.archived_records, 5);
        assert!(!serde_json::to_string(&report)?.contains("UI_ONLY_PRIVATE"));
        Ok(())
    }

    fn args() -> Result<ReplayOptions> {
        Ok(ReplayOptions::default())
    }

    fn fixture() -> Value {
        json!({
            "version": "0.3.0", "title": "not printed", "updated_at": "2026-01-01T00:00:00Z",
            "model": {"provider": "openai-subscribed", "model": "gpt-6.1-sol"},
            "cumulative_token_usage": {"input_tokens": 99, "output_tokens": 5, "cache_read_input_tokens": 40}, "request_token_usage": {},
            "messages": [
                {"User": {"id": "u", "content": [{"Text": "do not execute this"}]}},
                {"Agent": {"content": [{"Text": "recorded reply"}, {"Thinking": {"text": "omit me"}}, {"ToolUse": {"id": "call", "name": "terminal", "raw_input": "{\"command\":\"never execute\"}", "input": {"command": "never execute"}, "is_input_complete": true, "thought_signature": "omit signature"}}], "tool_results": {"call": {"tool_use_id": "call", "tool_name": "terminal", "is_error": true, "content": [{"Text": "recorded failure"}], "output": "bookkeeping"}}, "reasoning_details": {"opaque": "omit details"}}},
                {"Agent": {"content": [{"Text": "second reply"}], "tool_results": {}, "reasoning_details": null}}
            ]
        })
    }

    fn blocks(count: usize, marks: &[usize]) -> Result<Blocks> {
        let mut blocks = Blocks {
            data: Vec::new(),
            ends: Vec::new(),
            marks: marks.to_vec(),
        };
        for index in 0..count {
            blocks.push(json!({"role": "user", "content": {"Text": index.to_string()}}))?;
        }
        Ok(blocks)
    }

    #[test]
    fn realistic_top_level_metadata_is_counted_without_replaying_values() -> Result<()> {
        let mut saved = fixture();
        let metadata = json!({
            "initial_project_snapshot": {"worktrees": [{"path": "PRIVATE_METADATA_VALUE"}]},
            "detailed_summary": null,
            "draft_prompt": {"text": "PRIVATE_METADATA_VALUE"},
            "profile": "PRIVATE_METADATA_VALUE",
            "subagent_context": null,
            "speed": null,
            "thinking_effort": "high",
            "thinking_enabled": true,
            "ui_scroll_position": {"x": 0, "y": 1},
            "sandbox_grants": {"paths": ["PRIVATE_METADATA_VALUE"]},
            "sandboxed_terminal_temp_dir": "PRIVATE_METADATA_VALUE",
            "_cache_replay_export": {"thread_id": "PRIVATE_METADATA_VALUE", "saved_messages": 3, "decoded_bytes": 100},
            "infinite_context": true,
            "memory_archived": true,
            "memory_turn_start": [0, 0],
            "measured_cache_usage": {"agent": {"requests": 3, "input_tokens": 123, "cached_tokens": 45}, "summary": {"requests": 1, "input_tokens": 100, "cached_tokens": 0}},
            "baseline_usage": {"input_tokens": 123, "output_tokens": 12, "cache_read_input_tokens": 45, "cache_creation_input_tokens": 7}
        });
        let object = saved.as_object_mut().context("missing fixture object")?;
        for (key, value) in metadata.as_object().context("missing metadata object")? {
            object.insert(key.clone(), value.clone());
        }
        let conversation = parse_conversation(&serde_json::to_vec(&saved)?)?;
        for key in TOP_LEVEL_METADATA {
            assert_eq!(
                conversation
                    .counts
                    .ignored_top_level_metadata_fields
                    .get(*key),
                Some(&1)
            );
        }
        assert_eq!(
            conversation.stored_usage["cumulative_token_usage"],
            saved["cumulative_token_usage"]
        );
        assert_eq!(
            conversation.stored_usage["baseline_usage"],
            saved["baseline_usage"]
        );
        assert_eq!(conversation.counts.tool_calls, 1);
        for message in &conversation.messages {
            assert!(
                !String::from_utf8(canonical(
                    &message
                        .live()
                        .iter()
                        .flat_map(|message| &message.content)
                        .collect::<Vec<_>>()
                )?)?
                .contains("PRIVATE_METADATA_VALUE")
            );
        }
        assert!(!serde_json::to_string(&conversation.counts)?.contains("PRIVATE_METADATA_VALUE"));
        saved["unknown_metadata"] = json!("PRIVATE_METADATA_VALUE");
        let error = parse_conversation(&serde_json::to_vec(&saved)?)
            .err()
            .context("expected unknown top-level field error")?;
        assert_eq!(error.to_string(), "unsupported conversation field");
        Ok(())
    }

    #[test]
    fn current_db_thread_snapshot_preserves_memory_and_cache_creation_metadata() -> Result<()> {
        let mut saved = fixture();
        saved["infinite_context"] = json!(true);
        saved["memory_archived"] = json!(true);
        saved["memory_turn_start"] = json!([1, 2]);
        saved["cumulative_token_usage"]["cache_creation_input_tokens"] = json!(23);
        let thread = crate::DbThread::from_json(&serde_json::to_vec(&saved)?)?;
        let mut snapshot = serde_json::to_value(thread)?;
        snapshot["version"] = json!(crate::DbThread::VERSION);
        let bytes = serde_json::to_vec(&snapshot)?;
        let report = replay_json(&bytes, &ReplayOptions::default(), &AtomicBool::new(false))?;
        assert_eq!(report.source_format_version, "0.3.0");
        assert_eq!(report.agent().warm.requests, 2);
        assert_eq!(
            report.baseline_usage["cumulative_token_usage"]["cache_creation_input_tokens"],
            23
        );
        for field in [
            "infinite_context",
            "memory_archived",
            "memory_turn_start",
            "measured_cache_usage",
        ] {
            assert_eq!(
                report
                    .source_counts
                    .ignored_top_level_metadata_fields
                    .get(field),
                Some(&1)
            );
        }
        snapshot["future_memory_field"] = json!("PRIVATE_UNKNOWN_VALUE");
        let error = replay_json(
            &serde_json::to_vec(&snapshot)?,
            &ReplayOptions::default(),
            &AtomicBool::new(false),
        )
        .err()
        .context("expected unsupported field")?;
        assert_eq!(error.to_string(), "unsupported conversation field");
        Ok(())
    }

    #[test]
    fn modern_mentions_resume_and_summary_compaction_use_saved_content_only() -> Result<()> {
        let saved = json!({
            "version": "0.3.0", "title": "fixture", "updated_at": "2026-10-08T00:00:00Z",
            "messages": [
                {"User": {"id": "user", "content": [
                    {"Text": "Review this"},
                    {"Image": {"source": "OPAQUE_IMAGE", "future_image_metadata": {"keep": true}}},
                    {"Mention": {"uri": {"File": {"abs_path": "/not-read/private.rs"}}, "content": "SAVED_FILE_CONTENT"}},
                    {"Mention": {"uri": {"Fetch": {"url": "https://invalid.example/not-fetched"}}, "content": "SAVED_URL_CONTENT"}}
                ]}},
                {"Agent": {"content": [{"Text": "first reply"}], "tool_results": {}, "reasoning_details": null}},
                "Resume",
                {"Compaction": {"Summary": "SAVED_COMPACTION_SUMMARY"}},
                {"Agent": {"content": [{"Text": "final reply"}], "tool_results": {}, "reasoning_details": null}}
            ]
        });
        let conversation = parse_conversation(&serde_json::to_vec(&saved)?)?;
        let first = conversation.messages.first().context("missing user")?;
        assert!(first.text.contains("SAVED_FILE_CONTENT"));
        assert!(first.text.contains("SAVED_URL_CONTENT"));
        assert!(first.text.contains("<context>"));
        assert_eq!(
            first.images,
            vec![saved["messages"][0]["User"]["content"][1]["Image"].clone()]
        );
        assert_eq!(conversation.counts.images, 1);
        assert_eq!(conversation.counts.mentions, 2);
        assert_eq!(conversation.counts.resume_messages, 1);
        assert_eq!(conversation.counts.summary_compactions, 1);
        assert_eq!(
            conversation.messages.get(2).context("missing resume")?.text,
            "Continue where you left off"
        );
        let report = replay_json(
            &serde_json::to_vec(&saved)?,
            &ReplayOptions::default(),
            &AtomicBool::new(false),
        )?;
        assert_eq!(report.agent().warm.requests, 2);
        let report = serde_json::to_string(&report)?;
        assert!(!report.contains("SAVED_FILE_CONTENT"));
        assert!(!report.contains("SAVED_URL_CONTENT"));
        assert!(!report.contains("SAVED_COMPACTION_SUMMARY"));
        let mut unknown = saved.clone();
        unknown["messages"][0]["User"]["content"][2]["Mention"]["uri"]["File"]["future_field"] =
            json!("PRIVATE");
        let error = parse_conversation(&serde_json::to_vec(&unknown)?)
            .err()
            .context("expected unknown mention field")?;
        assert_eq!(error.to_string(), "unsupported Mention URI field");
        unknown["messages"] = json!([{ "Compaction": {"ProviderNative": {"provider": "openai", "items": ["PRIVATE"]}}}]);
        let error = parse_conversation(&serde_json::to_vec(&unknown)?)
            .err()
            .context("expected provider native error")?;
        assert_eq!(
            error.to_string(),
            "provider-native compaction cannot be reconstructed offline"
        );
        Ok(())
    }

    #[test]
    fn modern_archive_preserves_content_order_without_changing_request_estimates() -> Result<()> {
        let saved = json!({
            "version": "0.3.0", "updated_at": "2026-01-01T00:00:00Z",
            "messages": [
                {"User": {"id": "user", "content": [
                    {"Text": "first text"},
                    {"Image": {"source": "FIRST_IMAGE"}},
                    {"Text": "second text"},
                    {"Mention": {"uri": {"File": {"abs_path": "/not-read/private.rs"}}, "content": "SAVED_FILE_CONTENT"}},
                    {"Image": {"source": "SECOND_IMAGE"}},
                    {"Mention": {"uri": {"Fetch": {"url": "https://invalid.example/not-fetched"}}, "content": ""}},
                    {"Text": "last text"}
                ]}},
                {"Agent": {"content": [{"Text": "first reply"}, {"Text": ""}, {"Text": "second reply"}], "tool_results": {}}},
                "Resume",
                {"Compaction": {"Summary": "RAW_COMPACTION_SUMMARY"}}
            ]
        });
        let bytes = serde_json::to_vec(&saved)?;
        let conversation = parse_conversation(&bytes)?;
        let user = conversation.messages.first().context("missing user")?;
        let production_user: crate::Message = serde_json::from_value(saved["messages"][0].clone())?;
        let mut production_request = ReplayPrefix::default().request();
        append_memory_view(&mut production_request, "<chat>\n</chat>".into());
        let replay_request = production_request.clone();
        for mut message in production_user.to_request() {
            message.cache = true;
            production_request.messages.push(message);
        }
        let expected_blocks = Blocks::new(&production_request, &[])?;
        let replay_blocks = Blocks::new(&replay_request, &user.live())?;
        assert_eq!(replay_blocks.data, expected_blocks.data);
        assert_eq!(replay_blocks.marks, expected_blocks.marks);
        assert!(user.text.contains("<context>"));
        let empty_mention_uri: acp_thread::MentionUri = serde_json::from_value(
            saved["messages"][0]["User"]["content"][5]["Mention"]["uri"].clone(),
        )?;
        let expected = vec![
            ("user", "first text".to_string(), vec![]),
            (
                "user",
                " [1 images attached: zoom to view]".into(),
                vec![json!({"source": "FIRST_IMAGE"})],
            ),
            ("user", "second text".into(), vec![]),
            ("note", "SAVED_FILE_CONTENT".into(), vec![]),
            (
                "user",
                " [1 images attached: zoom to view]".into(),
                vec![json!({"source": "SECOND_IMAGE"})],
            ),
            ("note", empty_mention_uri.as_link().to_string(), vec![]),
            ("user", "last text".into(), vec![]),
            ("unii", "first reply".into(), vec![]),
            ("unii", "second reply".into(), vec![]),
            ("user", "Continue where you left off".into(), vec![]),
            ("note", "RAW_COMPACTION_SUMMARY".into(), vec![]),
        ];
        with_scratch(Scratch::new()?, |scratch| {
            let mut memory = InfiniteContext::open(scratch.path.clone())?;
            for message in &conversation.messages {
                message.archive(&mut memory, scratch, conversation.date)?;
            }
            let journal = fs::read_to_string(scratch.path.join("main/2026-01-01.jsonl"))?;
            let records = journal
                .lines()
                .map(serde_json::from_str::<Value>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            assert_eq!(records.len(), expected.len());
            for (index, (record, (kind, text, images))) in records.iter().zip(&expected).enumerate()
            {
                assert_eq!(record["i"], index as u64);
                assert_eq!(record["kind"], *kind);
                assert_eq!(record["text"], *text);
                assert_eq!(memory.zoom_images(index as u64)?, *images);
            }
            Ok(())
        })?;
        let report = replay_json(&bytes, &ReplayOptions::default(), &AtomicBool::new(false))?;
        assert_eq!(report.agent().warm.requests, 1);
        assert_eq!(report.agent().warm.input_bytes, expected_blocks.bytes());
        assert_eq!(
            report.metrics.engine.archived_records,
            expected.len() as u64
        );
        Ok(())
    }

    #[test]
    fn modern_archive_keeps_tool_calls_and_result_parts_at_the_call_site() -> Result<()> {
        let mut saved = fixture();
        let call = saved["messages"][1]["Agent"]["content"][2].clone();
        saved["messages"][1]["Agent"]["content"] = json!([
            {"Text": "before tool"}, call, {"Text": "after tool"}
        ]);
        let result_content = json!([
            {"Text": "first result"}, {"Image": {"source": "OPAQUE", "future_field": true}},
            {"Text": "second result"}
        ]);
        saved["messages"][1]["Agent"]["tool_results"]["call"]["content"] = result_content.clone();
        let conversation = parse_conversation(&serde_json::to_vec(&saved)?)?;
        let agent = conversation.messages.get(1).context("missing agent")?;
        let result = agent
            .tools
            .first()
            .and_then(|tool| tool.result.as_ref())
            .context("missing result")?;
        assert_eq!(serde_json::to_value(result.live_content())?, result_content);
        with_scratch(Scratch::new()?, |scratch| {
            let mut memory = InfiniteContext::open(scratch.path.clone())?;
            agent.archive(&mut memory, scratch, conversation.date)?;
            let journal = fs::read_to_string(scratch.path.join("main/2026-01-01.jsonl"))?;
            let records = journal
                .lines()
                .map(serde_json::from_str::<Value>)
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let expected = [
                ("unii", "before tool"),
                ("tool", "terminal {\"command\":\"never execute\"}"),
                ("echo", "first result"),
                ("echo", " [1 images attached: zoom to view]"),
                ("echo", "second result"),
                ("unii", "after tool"),
            ];
            assert_eq!(records.len(), expected.len());
            for (record, (kind, text)) in records.iter().zip(expected) {
                assert_eq!(record["kind"], kind);
                assert_eq!(record["text"], text);
            }
            assert_eq!(
                memory.zoom_images(3)?,
                vec![json!({"source": "OPAQUE", "future_field": true})]
            );
            Ok(())
        })
    }

    #[test]
    fn summary_task_is_uncached_but_still_counted_as_request_input() -> Result<()> {
        let options = ReplayOptions::default();
        let prefix = ReplayPrefix::default();
        let cancellation = AtomicBool::new(false);
        with_scratch(Scratch::new()?, |scratch| {
            let mut memory = InfiniteContext::open(scratch.path.clone())?;
            let date = "2026-01-01T00:00:00Z".parse()?;
            scratch.prepare(date)?;
            memory.append("user", &"summary input ".repeat(100), date)?;
            let job = memory.next_job()?.context("missing summary job")?;
            let mut expected_request = prefix.request();
            append_memory_view(&mut expected_request, memory.render_view_before(job.key.i)?);
            let prefix_blocks = Blocks::new(&expected_request, &[])?;
            expected_request.messages.push(LanguageModelRequestMessage {
                role: Role::User,
                content: vec![MessageContent::Text(memory_summary_task(&job))],
                cache: false,
                reasoning_details: None,
            });
            let expected_blocks = Blocks::new(&expected_request, &[])?;
            assert!(expected_blocks.bytes() > prefix_blocks.bytes());
            assert_eq!(expected_blocks.marks, prefix_blocks.marks);
            memory.release_in_flight_jobs()?;
            let mut replay = Replay::new(&options, &prefix, &Value::Null, &cancellation)?;
            replay.drain(&mut memory, scratch, &mut FrontierObserver::default(), date)?;
            assert_eq!(replay.metrics.summary.warm.requests, 1);
            assert_eq!(
                replay.metrics.summary.warm.input_bytes,
                expected_blocks.bytes()
            );
            assert_eq!(
                replay.metrics.summary.warm.marked_prefixes,
                prefix_blocks.marks.len() as u64
            );
            let marked_prefix_bytes = prefix_blocks
                .marks
                .last()
                .and_then(|mark| prefix_blocks.prefix(*mark))
                .context("missing prefix cache mark")?
                .len() as u64;
            assert_eq!(
                replay.metrics.summary.warm.cache_write_bytes,
                marked_prefix_bytes
            );
            assert!(replay.metrics.summary.warm.uncached_bytes > 0);
            Ok(())
        })
    }

    #[test]
    fn current_tool_inputs_and_legacy_single_result_content_are_preserved() -> Result<()> {
        for input in [
            json!({"type": "json", "value": {"command": "NEVER_EXECUTE"}}),
            json!({"type": "text", "value": "NEVER_EXECUTE"}),
        ] {
            let mut saved = fixture();
            saved["messages"][1]["Agent"]["content"][2]["ToolUse"]["input"] = input.clone();
            saved["messages"][1]["Agent"]["tool_results"]["call"]["content"] =
                json!({"Text": "SAVED_RESULT"});
            let conversation = parse_conversation(&serde_json::to_vec(&saved)?)?;
            let tool = conversation
                .messages
                .get(1)
                .and_then(|message| message.tools.first())
                .context("missing tool")?;
            assert_eq!(tool.input, input);
            assert_eq!(
                tool.result.as_ref().context("missing result")?.text,
                "SAVED_RESULT"
            );
            let live = conversation
                .messages
                .get(1)
                .context("missing agent")?
                .live();
            assert_eq!(
                live.first()
                    .and_then(|message| message.content.last())
                    .context("missing call block")?["ToolUse"]["input"],
                input
            );
        }
        Ok(())
    }

    #[test]
    fn prefix_accepts_old_function_schemas_and_current_function_and_custom_tools() -> Result<()> {
        let legacy: ReplayPrefix = serde_json::from_value(
            json!({"tools": [{"name": "legacy", "description": "saved", "input_schema": {"type": "object"}}]}),
        )?;
        assert_eq!(
            legacy.tools.first().context("missing legacy tool")?,
            &LanguageModelRequestTool::function(
                "legacy".into(),
                "saved".into(),
                json!({"type": "object"}),
                false
            )
        );
        let tools = vec![
            LanguageModelRequestTool::function(
                "current".into(),
                "saved".into(),
                json!({"type": "object"}),
                true,
            ),
            LanguageModelRequestTool {
                name: "custom".into(),
                description: "saved".into(),
                input: LanguageModelRequestToolInput::Custom { format: None },
            },
        ];
        let current: ReplayPrefix =
            serde_json::from_value(json!({"system": ["saved"], "tools": tools}))?;
        assert_eq!(current.tools, tools);
        assert!(serde_json::from_value::<ReplayPrefix>(json!({"tools": [{"name": "unknown", "description": "saved", "input": {"Function": {"input_schema": {}, "use_input_streaming": false, "future_field": true}}}]})).is_err());
        assert!(serde_json::from_value::<ReplayPrefix>(json!({"tools": [{"name": "ambiguous", "description": "saved", "input_schema": {}, "input": {"Custom": {"format": null}}}]})).is_err());
        assert!(serde_json::from_value::<ReplayPrefix>(json!({"tools": [{"name": "unknown", "description": "saved", "input_schema": {}, "future_field": true}]})).is_err());
        Ok(())
    }

    #[test]
    fn stored_result_with_incomplete_input_flag_is_preserved_not_reexecuted() -> Result<()> {
        let mut saved = fixture();
        saved["messages"][1]["Agent"]["content"][2]["ToolUse"]["is_input_complete"] = json!(false);
        let conversation = parse_conversation(&serde_json::to_vec(&saved)?)?;
        assert_eq!(conversation.counts.incomplete_tool_calls, 1);
        assert_eq!(conversation.counts.results_with_incomplete_input_flag, 1);
        assert_eq!(conversation.counts.pending_tool_calls, 0);
        let message = conversation.messages.get(1).context("missing agent")?;
        let live = message.live();
        let call = live
            .first()
            .context("missing live assistant")?
            .content
            .iter()
            .find_map(|block| block.get("ToolUse"))
            .context("missing recorded tool use")?;
        assert_eq!(call["is_input_complete"], false);
        assert_eq!(live.len(), 2);
        assert!(live.iter().all(|message| message.cache_eligible));
        let mut scratch = Scratch::new()?;
        let mut memory = InfiniteContext::open(scratch.path.clone())?;
        message.archive(&mut memory, &scratch, conversation.date)?;
        assert_eq!(memory.message_count(), 3);
        drop(memory);
        scratch.cleanup()?;
        Ok(())
    }

    #[test]
    fn structured_tool_output_is_counted_as_opaque_metadata_not_result_text() -> Result<()> {
        for output in [
            json!({"raw_output": "PRIVATE_OUTPUT_METADATA", "exit_code": 1, "nested": {"items": ["PRIVATE_OUTPUT_METADATA"]}}),
            json!(["PRIVATE_OUTPUT_METADATA"]),
            json!("PRIVATE_OUTPUT_METADATA"),
            json!(null),
        ] {
            let mut saved = fixture();
            saved["messages"][1]["Agent"]["tool_results"]["call"]["output"] = output.clone();
            let conversation = parse_conversation(&serde_json::to_vec(&saved)?)?;
            assert_eq!(
                conversation
                    .counts
                    .ignored_tool_output_metadata_types
                    .values()
                    .sum::<u64>(),
                1
            );
            assert_eq!(
                conversation.counts.ignored_tool_output_metadata_bytes,
                canonical(&output)?.len() as u64
            );
            let message = conversation.messages.get(1).context("missing agent")?;
            let result = message
                .tools
                .first()
                .and_then(|tool| tool.result.as_ref())
                .context("missing result")?;
            assert_eq!(result.text, "recorded failure");
            assert!(result.is_error);
            let live = String::from_utf8(canonical(
                &message
                    .live()
                    .iter()
                    .flat_map(|message| &message.content)
                    .collect::<Vec<_>>(),
            )?)?;
            assert!(!live.contains("PRIVATE_OUTPUT_METADATA"));
        }
        Ok(())
    }

    #[test]
    fn cache_rates_are_byte_weighted_and_do_not_claim_context_reduction() -> Result<()> {
        let blocks = blocks(2, &[1])?;
        let mut cache = Cache::new(300, 0);
        let mut metrics = Comparison::default();
        cache.request(
            "account:model",
            &blocks,
            0,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        cache.request(
            "account:model",
            &blocks,
            1,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert_eq!(metrics.warm.cache_read_percentage, Some(50.0));
        assert_eq!(metrics.cold.cache_read_percentage, Some(0.0));
        assert_eq!(metrics.input_reduction_vs_cold_percentage, Some(50.0));
        assert_eq!(metrics.total_input_reduction_vs_cold_percentage, Some(0.0));
        let combined = Comparison::combined(&metrics, &Comparison::default());
        let report = serde_json::to_value(&combined)?;
        assert_eq!(report["warm"]["cacheReadPercentage"], 50.0);
        assert_eq!(report["inputReductionVsColdPercentage"], 50.0);
        assert_eq!(report["totalInputReductionVsColdPercentage"], 0.0);
        assert_eq!(
            Comparison::combined(&Comparison::default(), &Comparison::default())
                .warm
                .cache_read_percentage,
            None
        );
        Ok(())
    }

    #[test]
    fn current_turn_live_blocks_keep_recorded_tool_call_and_result_bytes() -> Result<()> {
        let conversation = parse_conversation(&serde_json::to_vec(&fixture())?)?;
        let mut request = ReplayPrefix::default().request();
        append_memory_view(&mut request, "<chat>\n</chat>".into());
        let mut live = conversation
            .messages
            .first()
            .context("missing user")?
            .live();
        let before = Blocks::new(&request, &live)?.bytes();
        live.extend(
            conversation
                .messages
                .get(1)
                .context("missing first agent")?
                .live(),
        );
        let after = Blocks::new(&request, &live)?;
        assert!(after.bytes() > before);
        let canonical_live = String::from_utf8(after.data)?;
        assert!(canonical_live.contains("ToolUse"));
        assert!(canonical_live.contains("ToolResult"));
        assert!(canonical_live.contains("recorded failure"));
        assert!(!canonical_live.contains("second reply"));
        Ok(())
    }

    #[test]
    fn cache_marks_only_and_missing_lookback() -> Result<()> {
        let mut cache = Cache::new(300, 0);
        let mut metrics = Comparison::default();
        cache.request(
            "account:model",
            &blocks(2, &[1])?,
            0,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert_eq!(cache.entries.len(), 1);
        cache.request(
            "account:model",
            &blocks(1, &[0])?,
            1,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert_eq!(metrics.warm.requests_with_any_hit, 0);
        let mut cache = Cache::new(300, 0);
        let mut metrics = Comparison::default();
        cache.request(
            "account:model",
            &blocks(1, &[0])?,
            0,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        cache.request(
            "account:model",
            &blocks(20, &[19])?,
            1,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert_eq!(metrics.warm.requests_with_any_hit, 1);
        let mut cache = Cache::new(300, 0);
        let mut metrics = Comparison::default();
        cache.request(
            "account:model",
            &blocks(1, &[0])?,
            0,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        cache.request(
            "account:model",
            &blocks(21, &[20])?,
            1,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert_eq!(metrics.warm.requests_with_any_hit, 0);
        Ok(())
    }

    #[test]
    fn cache_expiry_read_refresh_and_scope_isolation() -> Result<()> {
        let original = blocks(1, &[0])?;
        let extended = blocks(2, &[1])?;
        let mut cache = Cache::new(300, 0);
        let mut metrics = Comparison::default();
        cache.request(
            "account:model",
            &original,
            0,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        cache.request(
            "account:summary",
            &original,
            1,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        cache.request(
            "other-account:model",
            &original,
            1,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert_eq!(metrics.warm.requests_with_any_hit, 0);
        cache.request(
            "account:model",
            &extended,
            299,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        cache.request(
            "account:model",
            &original,
            598,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert_eq!(metrics.warm.requests_with_any_hit, 2);
        cache.request(
            "account:model",
            &original,
            898,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert_eq!(metrics.warm.requests_with_any_hit, 2);
        assert_eq!(metrics.cold.requests_with_any_hit, 0);
        assert_eq!(
            metrics.warm.input_bytes,
            metrics.warm.cache_read_bytes + metrics.warm.uncached_bytes
        );
        Ok(())
    }

    #[test]
    fn unicode_summaries_preserve_head_tail_and_byte_caps() -> Result<()> {
        for limit in [256, 512] {
            let input = format!("HEAD {}\n{} TAIL", "🦀é中".repeat(200), "λ".repeat(200));
            let summary = extractive_summary(&input, limit)?;
            assert!(summary.len() <= limit);
            assert!(summary.starts_with("HEAD"));
            assert!(summary.ends_with("TAIL"));
            assert!(!summary.contains('\n'));
            assert!(
                !extractive_summary(&" \n".repeat(1000), limit)?
                    .trim()
                    .is_empty()
            );
        }
        Ok(())
    }

    #[test]
    fn modern_user_agent_tools_and_reasoning_parsing() -> Result<()> {
        let conversation = parse_conversation(&serde_json::to_vec(&fixture())?)?;
        assert_eq!(conversation.counts.users, 1);
        assert_eq!(conversation.counts.assistants, 2);
        assert_eq!(conversation.counts.tool_calls, 1);
        assert_eq!(conversation.counts.error_results, 1);
        assert_eq!(conversation.counts.reasoning_blocks_omitted, 1);
        assert_eq!(conversation.counts.reasoning_metadata_entries_omitted, 2);
        let agent = conversation.messages.get(1).context("missing agent")?;
        let live = agent.live();
        assert_eq!(live.len(), 2);
        assert!(
            live.get(1)
                .context("missing results")?
                .content
                .iter()
                .any(|block| block["ToolResult"]["is_error"] == true)
        );
        assert_eq!(agent.text, "recorded reply");
        Ok(())
    }

    #[test]
    fn multiline_unicode_tool_text_matches_production_live_and_archive_clipping() -> Result<()> {
        let head = format!("HEAD\n{}", "🦀é中\n".repeat(5000));
        let tail = format!("{}\nTAIL", "λ🦀é\n".repeat(5000));
        let original = format!("{head}{tail}");
        let expected = clip_memory_output(&original);
        assert_eq!(expected.chars().count(), 30_000);
        assert!(expected.len() > 30_000);
        assert!(expected.starts_with("HEAD\n"));
        assert!(expected.ends_with("\nTAIL"));
        assert!(expected.contains("[middle omitted]"));
        let mut saved = fixture();
        saved["messages"][0]["User"]["content"][0]["Text"] = json!(original);
        saved["messages"][1]["Agent"]["content"][0]["Text"] = json!(original);
        saved["messages"][1]["Agent"]["tool_results"]["call"]["content"] =
            json!([{"Text": head}, {"Text": tail}]);
        let conversation = parse_conversation(&serde_json::to_vec(&saved)?)?;
        assert_eq!(conversation.counts.tool_results_clipped, 1);
        assert_eq!(
            conversation.counts.tool_result_original_text_bytes,
            original.len() as u64
        );
        assert_eq!(
            conversation.counts.tool_result_replayed_text_bytes,
            expected.len() as u64
        );
        let user = conversation.messages.first().context("missing user")?;
        assert!(
            user.live()
                .first()
                .and_then(|message| message.content.first())
                .and_then(|block| block.get("Text"))
                .and_then(Value::as_str)
                .is_some_and(|text| text == original)
        );
        let agent = conversation.messages.get(1).context("missing agent")?;
        let live = agent.live();
        assert!(
            live.first()
                .and_then(|message| message.content.first())
                .and_then(|block| block.get("Text"))
                .and_then(Value::as_str)
                .is_some_and(|text| text == original)
        );
        let result = live
            .get(1)
            .and_then(|message| message.content.first())
            .and_then(|block| block.get("ToolResult"))
            .context("missing live result")?;
        let blocks = field(result, "content")?
            .as_array()
            .context("missing result blocks")?;
        assert_eq!(blocks.len(), 1);
        assert!(
            blocks
                .first()
                .and_then(|block| block.get("Text"))
                .and_then(Value::as_str)
                .is_some_and(|text| text == expected)
        );
        assert!(bool_field(result, "is_error")?);
        let mut scratch = Scratch::new()?;
        let mut memory = InfiniteContext::open(scratch.path.clone())?;
        agent.archive(&mut memory, &scratch, conversation.date)?;
        let journal = fs::read_to_string(
            scratch
                .path
                .join("main")
                .join(format!("{}.jsonl", conversation.date.format("%Y-%m-%d"))),
        )?;
        let mut archived_echo = String::new();
        for line in journal.lines() {
            let record: Value = serde_json::from_str(line)?;
            if record.get("kind").and_then(Value::as_str) == Some("echo") {
                archived_echo.push_str(&string_field(&record, "text")?);
            }
        }
        assert!(archived_echo == expected);
        drop(memory);
        scratch.cleanup()?;
        Ok(())
    }

    #[test]
    fn mixed_tool_images_keep_values_and_relative_order_with_combined_clipped_text() -> Result<()> {
        let images = [
            json!({"opaque": "first"}),
            json!({"opaque": "middle"}),
            json!({"opaque": "last"}),
        ];
        let mut saved = fixture();
        let head = "é\n".repeat(16000);
        let tail = "🦀\n".repeat(16000);
        let expected = clip_memory_output(&format!("{head}{tail}"));
        saved["messages"][1]["Agent"]["tool_results"]["call"]["content"] = json!([
            {"Image": images.first()}, {"Text": head}, {"Image": images.get(1)}, {"Text": tail}, {"Image": images.last()}
        ]);
        let conversation = parse_conversation(&serde_json::to_vec(&saved)?)?;
        let result = conversation
            .messages
            .get(1)
            .and_then(|message| message.tools.first())
            .and_then(|tool| tool.result.as_ref())
            .context("missing mixed result")?;
        let content = result.live_content();
        assert_eq!(content.len(), 4);
        assert_eq!(
            content.first().and_then(|block| block.get("Image")),
            images.first()
        );
        assert!(
            content
                .get(1)
                .and_then(|block| block.get("Text"))
                .and_then(Value::as_str)
                .is_some_and(|text| text == expected)
        );
        assert_eq!(
            content.get(2).and_then(|block| block.get("Image")),
            images.get(1)
        );
        assert_eq!(
            content.last().and_then(|block| block.get("Image")),
            images.last()
        );
        assert_eq!(conversation.counts.images, 3);
        assert_eq!(conversation.counts.tool_results_clipped, 1);
        with_scratch(Scratch::new()?, |scratch| {
            let mut memory = InfiniteContext::open(scratch.path.clone())?;
            conversation
                .messages
                .get(1)
                .context("missing agent")?
                .archive(&mut memory, scratch, conversation.date)?;
            let journal = fs::read_to_string(scratch.path.join("main/2026-01-01.jsonl"))?;
            let mut archived_text = String::new();
            let mut archived_images = Vec::new();
            let mut echo_parts = Vec::new();
            for line in journal.lines() {
                let record: Value = serde_json::from_str(line)?;
                if record["kind"] != "echo" {
                    continue;
                }
                let record_images = memory.zoom_images(
                    field(&record, "i")?
                        .as_u64()
                        .context("invalid record index")?,
                )?;
                if record_images.is_empty() {
                    archived_text.push_str(&string_field(&record, "text")?);
                    if echo_parts.last() != Some(&"text") {
                        echo_parts.push("text");
                    }
                } else {
                    assert_eq!(record["text"], " [1 images attached: zoom to view]");
                    archived_images.extend(record_images);
                    echo_parts.push("image");
                }
            }
            assert_eq!(archived_text, expected);
            assert_eq!(archived_images, images);
            assert_eq!(echo_parts, ["image", "text", "image", "image"]);
            Ok(())
        })
    }

    #[test]
    fn replay_never_reexecutes_and_uses_real_engine() -> Result<()> {
        let mut fixture = fixture();
        let mut args = args()?;
        args.repeats = 2;
        fixture["messages"][0]["User"]["content"][0]["Text"] = json!("head ".repeat(150));
        let conversation = parse_conversation(&serde_json::to_vec(&fixture)?)?;
        let prefix = ReplayPrefix::default();
        let cancellation = AtomicBool::new(false);
        let mut replay = Replay::new(&args, &prefix, &conversation.model, &cancellation)?;
        replay.repeat(&conversation, 1)?;
        replay.repeat(&conversation, 2)?;
        assert_eq!(replay.metrics.turn.warm.requests, 4);
        assert_eq!(replay.metrics.engine.leaf_summary_calls, 2);
        assert!(replay.metrics.summary.warm.requests >= 2);
        assert_eq!(replay.metrics.engine.archived_records, 10);
        assert!(replay.metrics.turn.warm.requests_with_any_hit > 0);
        assert_eq!(replay.metrics.repetitions.len(), 2);
        assert!(replay.metrics.engine.largest_summary_reply_bytes <= 512);
        Ok(())
    }

    #[test]
    fn opaque_images_not_summarized_and_private_files_cleaned() -> Result<()> {
        let mut fixture = fixture();
        fixture["messages"][0]["User"]["content"] =
            json!([{"Image": {"source": "BASE64_NOT_TEXT", "size": {"width": 1, "height": 1}}}]);
        let conversation = parse_conversation(&serde_json::to_vec(&fixture)?)?;
        let mut scratch = Scratch::new()?;
        let path = scratch.path.clone();
        let mut memory = InfiniteContext::open(path.clone())?;
        let user = conversation.messages.first().context("missing user")?;
        user.archive(&mut memory, &scratch, conversation.date)?;
        assert!(!memory.zoom(0, 1, 0)?.contains("BASE64_NOT_TEXT"));
        assert_eq!(
            memory.zoom_images(0)?,
            vec![json!({"source": "BASE64_NOT_TEXT", "size": {"width": 1, "height": 1}})]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path)?.permissions().mode() & 0o777, 0o700);
            assert_eq!(
                fs::metadata(path.join("main/2026-01-01.jsonl"))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.join("view.json"))?.permissions().mode() & 0o777,
                0o600
            );
        }
        drop(memory);
        scratch.cleanup()?;
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn unsupported_formats_and_variants_error_without_echo() -> Result<()> {
        assert!(parse_conversation(b"{\"version\":\"9.9.9\"}").is_err());
        assert!(parse_conversation(b"{}\n{}").is_err());
        let mut fixture = fixture();
        fixture["messages"][0]["User"]["content"] = json!([{"UnknownTranscriptSecret": "PRIVATE"}]);
        let error = parse_conversation(&serde_json::to_vec(&fixture)?)
            .err()
            .context("expected unsupported variant error")?;
        assert!(!error.to_string().contains("PRIVATE"));
        assert!(!error.to_string().contains("UnknownTranscriptSecret"));
        fixture["messages"] = json!([{"System": {"content": []}}]);
        assert!(parse_conversation(&serde_json::to_vec(&fixture)?).is_err());
        Ok(())
    }

    #[test]
    fn old_thread_parse_preserves_tool_images_and_rejects_unknown_segments() -> Result<()> {
        let mut old = json!({
            "version": "0.2.0", "summary": "old", "updated_at": "2026-01-01T00:00:00Z",
            "messages": [
                {"id": 0, "role": "user", "segments": [{"type": "text", "text": "hello"}]},
                {"id": 1, "role": "assistant", "segments": [{"type": "thinking", "text": "omit"}],
                 "tool_uses": [{"id": "call", "name": "camera", "input": {}}],
                 "tool_results": [{"tool_use_id": "call", "is_error": false, "content": {"Image": {"source": "OPAQUE_BASE64", "size": {"width": 1, "height": 1}}}, "output": null}]}
            ]
        });
        let conversation = parse_conversation(&serde_json::to_vec(&old)?)?;
        assert_eq!(conversation.counts.tool_results, 1);
        assert_eq!(conversation.counts.images, 1);
        assert_eq!(conversation.counts.reasoning_blocks_omitted, 1);
        let result = conversation
            .messages
            .get(1)
            .and_then(|message| message.tools.first())
            .and_then(|tool| tool.result.as_ref())
            .context("missing old image result")?;
        assert!(result.text.is_empty());
        assert_eq!(result.images.len(), 1);
        old["messages"][0]["segments"] =
            json!([{"type": "UnknownPrivateVariant", "text": "PRIVATE"}]);
        let error = parse_conversation(&serde_json::to_vec(&old)?)
            .err()
            .context("expected old segment error")?;
        assert!(!error.to_string().contains("PRIVATE"));
        Ok(())
    }

    #[test]
    fn eligibility_zero_ttl() -> Result<()> {
        let blocks = blocks(1, &[0])?;
        let mut cache = Cache::new(300, usize::MAX);
        let mut metrics = Comparison::default();
        cache.request(
            "account:model",
            &blocks,
            0,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert!(cache.entries.is_empty());
        assert_eq!(metrics.warm.ineligible_marks, 1);
        let mut cache = Cache::new(0, 0);
        cache.request(
            "account:model",
            &blocks,
            0,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        cache.request(
            "account:model",
            &blocks,
            0,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert_eq!(metrics.warm.requests_with_any_hit, 0);
        let mut scratch = Scratch::new()?;
        scratch.cleanup()?;
        Ok(())
    }

    #[test]
    fn production_memory_marks_and_role_tool_system_changes() -> Result<()> {
        let mut request = ReplayPrefix::default().request();
        append_memory_view(
            &mut request,
            "<chat>\n0+1|a\n1+1|b\n2+1|c\n3+1|d\n4+1|e\n</chat>".into(),
        );
        let blocks = Blocks::new(&request, &[])?;
        assert_eq!(blocks.marks.len(), 2);
        assert!(
            !request
                .messages
                .last()
                .context("missing closing block")?
                .cache
        );
        let mut cache = Cache::new(300, 0);
        let mut metrics = Comparison::default();
        cache.request(
            "account:model",
            &blocks,
            0,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        request.tools.clear();
        cache.request(
            "account:model",
            &Blocks::new(&request, &[])?,
            1,
            0,
            &mut metrics,
            &AtomicBool::new(false),
        )?;
        assert_eq!(metrics.warm.requests_with_any_hit, 0);
        Ok(())
    }
}
