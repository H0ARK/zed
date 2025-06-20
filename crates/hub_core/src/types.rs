//! Core types for The Hub platform

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unique identifier for a command session
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
    
    pub fn from_string(s: String) -> Self {
        Self(s)
    }
    
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a block
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockId(pub String);

impl BlockId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
    
    pub fn from_string(s: String) -> Self {
        Self(s)
    }
    
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Command execution context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandContext {
    pub session_id: SessionId,
    pub command: String,
    pub args: Vec<String>,
    pub environment: HashMap<String, String>,
    pub working_directory: String,
}

/// Command execution state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CommandState {
    Running,
    Completed { exit_code: i32 },
    Failed { error: String },
}

/// Block content type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockContent {
    Text(String),
    Progress { current: u64, total: u64, message: String },
    Table { headers: Vec<String>, rows: Vec<Vec<String>> },
    Tree { nodes: Vec<TreeNode> },
    Form { fields: Vec<FormField> },
    Chart { data: ChartData },
    FileList { files: Vec<FileInfo> },
    LogStream { entries: Vec<LogEntry> },
    Status { cards: Vec<StatusCard> },
}

/// Tree node for hierarchical data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub label: String,
    pub children: Vec<TreeNode>,
    pub expanded: bool,
    pub icon: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Form field definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormField {
    pub name: String,
    pub label: String,
    pub field_type: FormFieldType,
    pub required: bool,
    pub default_value: Option<String>,
    pub validation: Option<FieldValidation>,
}

/// Form field types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FormFieldType {
    Text,
    Number,
    Boolean,
    Select { options: Vec<String> },
    File,
    Textarea,
    Password,
    Email,
    Url,
}

/// Field validation rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldValidation {
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
    pub pattern: Option<String>,
    pub custom_message: Option<String>,
}

/// Chart data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartData {
    pub chart_type: ChartType,
    pub title: String,
    pub series: Vec<ChartSeries>,
}

/// Chart types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChartType {
    Line,
    Bar,
    Pie,
    Scatter,
    Area,
    Histogram,
}

/// Chart data series
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartSeries {
    pub name: String,
    pub data: Vec<(String, f64)>,
    pub color: Option<String>,
}

/// File information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub name: String,
    pub size: Option<u64>,
    pub modified: Option<chrono::DateTime<chrono::Utc>>,
    pub file_type: FileType,
    pub permissions: Option<String>,
}

/// File type classification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Other,
}

/// Log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub level: LogLevel,
    pub message: String,
    pub source: Option<String>,
    pub metadata: Option<serde_json::Value>,
}

/// Log levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

/// Status card for dashboards
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusCard {
    pub title: String,
    pub status: StatusLevel,
    pub primary_metric: String,
    pub secondary_metrics: Vec<String>,
    pub description: Option<String>,
    pub actions: Vec<String>,
}

/// Status levels
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StatusLevel {
    Success,
    Warning,
    Error,
    Info,
    Unknown,
}

/// User interaction events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum UserInteraction {
    Click { target: String, position: Option<(f64, f64)> },
    Select { items: Vec<String> },
    Input { field: String, value: String },
    Submit { form: String, values: HashMap<String, String> },
    Scroll { direction: ScrollDirection, amount: f64 },
    Resize { component: String, size: (f64, f64) },
}

/// Scroll direction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScrollDirection {
    Up,
    Down,
    Left,
    Right,
}

/// Component configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentConfig {
    pub id: String,
    pub component_type: String,
    pub properties: serde_json::Value,
    pub styling: Option<ComponentStyling>,
    pub behavior: Option<ComponentBehavior>,
}

/// Component styling options
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentStyling {
    pub theme: Option<String>,
    pub colors: Option<HashMap<String, String>>,
    pub fonts: Option<HashMap<String, String>>,
    pub spacing: Option<HashMap<String, f64>>,
}

/// Component behavior configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentBehavior {
    pub interactive: bool,
    pub auto_refresh: Option<std::time::Duration>,
    pub lazy_load: bool,
    pub cache_results: bool,
}

/// Performance metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub response_time: std::time::Duration,
    pub memory_usage: u64,
    pub cpu_usage: f64,
    pub network_io: Option<NetworkIO>,
    pub disk_io: Option<DiskIO>,
}

/// Network I/O metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkIO {
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub requests_sent: u32,
    pub responses_received: u32,
}

/// Disk I/O metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskIO {
    pub bytes_read: u64,
    pub bytes_written: u64,
    pub operations_read: u32,
    pub operations_written: u32,
}