//! Block types for Hub terminal enhanced UI
//! 
//! Extracted from hub_core for grid-based semantic parsing architecture.
//! These types represent rich UI components that can be displayed in terminal command blocks.

use serde::{Deserialize, Serialize};

/// Unique identifier for a block
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockId(pub String);

impl BlockId {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
    
    #[allow(dead_code)]
    pub fn from_string(s: String) -> Self {
        Self(s)
    }
    
    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Block content type - rich UI components for terminal output
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