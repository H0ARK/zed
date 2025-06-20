//! Protocol message types and structures
//! 
//! This module implements the message format specified in the protocol documentation

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use chrono::{DateTime, Utc};
use hub_core::types::*;

/// Protocol version
pub const PROTOCOL_VERSION: &str = "1.0";

/// Protocol magic bytes for message identification
pub const PROTOCOL_MAGIC: &[u8] = b"HUB1";

/// Top-level message envelope
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEnvelope {
    pub version: String,
    #[serde(rename = "type")]
    pub message_type: MessageType,
    pub session_id: String,
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub payload: MessagePayload,
}

/// Message types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    UiMessage,
    Control,
    Response,
    Event,
    AiMessage,
    Collaboration,
    Error,
    Batch,
}

/// Message payload variants
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessagePayload {
    Control(ControlPayload),
    UiMessage(UiMessagePayload),
    Response(ResponsePayload),
    Event(EventPayload),
    AiMessage(AiMessagePayload),
    Collaboration(CollaborationPayload),
    Error(ErrorPayload),
    Batch(BatchPayload),
}

/// Control message payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum ControlPayload {
    #[serde(rename = "session_start")]
    SessionStart {
        command: String,
        args: Vec<String>,
        cwd: String,
        capabilities: SessionCapabilities,
    },
    #[serde(rename = "session_end")]
    SessionEnd {
        exit_code: i32,
        duration_ms: u64,
        summary: Option<String>,
    },
}

/// Session capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCapabilities {
    pub ui_components: Vec<String>,
    pub interactions: Vec<String>,
    pub ai_integration: bool,
}

/// UI message payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "component")]
pub enum UiMessagePayload {
    #[serde(rename = "progress")]
    Progress(ProgressComponent),
    #[serde(rename = "table")]
    Table(TableComponent),
    #[serde(rename = "file_tree")]
    FileTree(FileTreeComponent),
    #[serde(rename = "form")]
    Form(FormComponent),
    #[serde(rename = "status_grid")]
    StatusGrid(StatusGridComponent),
    #[serde(rename = "update")]
    Update(ComponentUpdate),
    #[serde(rename = "stream")]
    Stream(StreamData),
}

/// Progress component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressComponent {
    pub props: ProgressProps,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressProps {
    pub current: u64,
    pub total: u64,
    pub message: String,
    pub show_percentage: bool,
    pub show_eta: bool,
    pub style: ProgressStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStyle {
    Bar,
    Spinner,
    Dots,
}

/// Table component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableComponent {
    pub props: TableProps,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableProps {
    pub headers: Vec<TableHeader>,
    pub rows: Vec<TableRow>,
    pub sortable: bool,
    pub filterable: bool,
    pub selectable: SelectionMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableHeader {
    pub text: String,
    pub width: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TableRow {
    pub id: String,
    pub cells: Vec<String>,
    pub actions: Vec<String>,
    pub status: Option<RowStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RowStatus {
    Normal,
    Warning,
    Error,
    Success,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    None,
    Single,
    Multiple,
}

/// File tree component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTreeComponent {
    pub props: FileTreeProps,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTreeProps {
    pub root: String,
    pub entries: Vec<FileEntry>,
    pub show_hidden: bool,
    pub icons: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub entry_type: FileEntryType,
    pub size: Option<u64>,
    pub modified: Option<DateTime<Utc>>,
    pub status: Option<FileStatus>,
    pub actions: Vec<String>,
    pub children: Option<Vec<FileEntry>>,
    pub expanded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileEntryType {
    File,
    Directory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Normal,
    Modified,
    Added,
    Deleted,
    Conflict,
}

/// Form component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormComponent {
    pub props: FormProps,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormProps {
    pub title: String,
    pub fields: Vec<FormField>,
    pub actions: Vec<FormAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FormFieldType,
    pub label: String,
    pub required: bool,
    pub placeholder: Option<String>,
    pub default_value: Option<String>,
    pub options: Option<Vec<FormOption>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldType {
    Text,
    Textarea,
    Number,
    Boolean,
    Select,
    CheckboxGroup,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormOption {
    pub value: String,
    pub label: String,
    pub checked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormAction {
    pub label: String,
    pub action: String,
    pub style: ActionStyle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStyle {
    Primary,
    Secondary,
    Danger,
    Success,
}

/// Status grid component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusGridComponent {
    pub props: StatusGridProps,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusGridProps {
    pub cards: Vec<StatusCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusCard {
    pub title: String,
    pub status: CardStatus,
    pub primary_metric: String,
    pub secondary_metrics: Vec<String>,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardStatus {
    Success,
    Warning,
    Error,
    Info,
    Loading,
}

/// Component update
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentUpdate {
    pub target: String,
    pub props: serde_json::Value,
}

/// Stream data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamData {
    pub stream_id: String,
    pub data: StreamDataType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StreamDataType {
    #[serde(rename = "log_line")]
    LogLine {
        content: String,
        level: LogLevel,
        timestamp: DateTime<Utc>,
    },
    #[serde(rename = "progress_update")]
    ProgressUpdate {
        current: u64,
        total: Option<u64>,
        message: String,
    },
    #[serde(rename = "json_data")]
    JsonData {
        data: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

/// Response payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponsePayload {
    pub interaction_id: String,
    pub action: String,
    pub data: serde_json::Value,
}

/// Event payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event")]
pub enum EventPayload {
    #[serde(rename = "selection_changed")]
    SelectionChanged {
        component_id: String,
        selected_ids: Vec<String>,
        selection_type: SelectionMode,
    },
    #[serde(rename = "component_action")]
    ComponentAction {
        component_id: String,
        action: String,
        data: serde_json::Value,
    },
}

/// AI message payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "suggestion_type")]
pub enum AiMessagePayload {
    #[serde(rename = "command_optimization")]
    CommandOptimization {
        context: String,
        suggestion: String,
        confidence: f64,
        actions: Vec<String>,
    },
    #[serde(rename = "context_sharing")]
    ContextSharing {
        context_type: String,
        data: serde_json::Value,
    },
}

/// Collaboration payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum CollaborationPayload {
    #[serde(rename = "share_session")]
    ShareSession {
        share_code: String,
        permissions: Vec<String>,
        expires_at: Option<DateTime<Utc>>,
    },
    #[serde(rename = "cursor_update")]
    CursorUpdate {
        user_id: String,
        position: CursorPosition,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPosition {
    pub component_id: String,
    pub row: u32,
    pub column: u32,
}

/// Error payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub error_code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

/// Batch payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchPayload {
    pub messages: Vec<MessageEnvelope>,
}

impl MessageEnvelope {
    pub fn new(
        message_type: MessageType,
        session_id: String,
        sequence: u64,
        payload: MessagePayload,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION.to_string(),
            message_type,
            session_id,
            sequence,
            timestamp: Utc::now(),
            payload,
        }
    }
    
    pub fn session_start(
        session_id: String,
        sequence: u64,
        command: String,
        args: Vec<String>,
        cwd: String,
        capabilities: SessionCapabilities,
    ) -> Self {
        Self::new(
            MessageType::Control,
            session_id,
            sequence,
            MessagePayload::Control(ControlPayload::SessionStart {
                command,
                args,
                cwd,
                capabilities,
            }),
        )
    }
    
    pub fn session_end(
        session_id: String,
        sequence: u64,
        exit_code: i32,
        duration_ms: u64,
        summary: Option<String>,
    ) -> Self {
        Self::new(
            MessageType::Control,
            session_id,
            sequence,
            MessagePayload::Control(ControlPayload::SessionEnd {
                exit_code,
                duration_ms,
                summary,
            }),
        )
    }
    
    pub fn progress(
        session_id: String,
        sequence: u64,
        current: u64,
        total: u64,
        message: String,
    ) -> Self {
        Self::new(
            MessageType::UiMessage,
            session_id,
            sequence,
            MessagePayload::UiMessage(UiMessagePayload::Progress(ProgressComponent {
                props: ProgressProps {
                    current,
                    total,
                    message,
                    show_percentage: true,
                    show_eta: true,
                    style: ProgressStyle::Bar,
                },
            })),
        )
    }
    
    pub fn error(
        session_id: String,
        sequence: u64,
        error_code: String,
        message: String,
        details: Option<serde_json::Value>,
    ) -> Self {
        Self::new(
            MessageType::Error,
            session_id,
            sequence,
            MessagePayload::Error(ErrorPayload {
                error_code,
                message,
                details,
            }),
        )
    }
}