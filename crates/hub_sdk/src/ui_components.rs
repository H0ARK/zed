//! High-level UI component builders
//!
//! This module provides easy-to-use builders for creating rich UI components
//! that can be sent to The Hub for display.

use anyhow::Result;
use hub_protocol::messages::*;
use std::collections::HashMap;

/// Builder for progress indicators
pub struct ProgressBuilder {
    current: u64,
    total: u64,
    message: String,
    show_percentage: bool,
    show_eta: bool,
    style: ProgressStyle,
}

impl ProgressBuilder {
    pub fn new(current: u64, total: u64, message: impl Into<String>) -> Self {
        Self {
            current,
            total,
            message: message.into(),
            show_percentage: true,
            show_eta: true,
            style: ProgressStyle::Bar,
        }
    }
    
    pub fn style(mut self, style: ProgressStyle) -> Self {
        self.style = style;
        self
    }
    
    pub fn show_percentage(mut self, show: bool) -> Self {
        self.show_percentage = show;
        self
    }
    
    pub fn show_eta(mut self, show: bool) -> Self {
        self.show_eta = show;
        self
    }
    
    pub fn build(self) -> ProgressComponent {
        ProgressComponent {
            props: ProgressProps {
                current: self.current,
                total: self.total,
                message: self.message,
                show_percentage: self.show_percentage,
                show_eta: self.show_eta,
                style: self.style,
            },
        }
    }
}

/// Builder for tables
pub struct TableBuilder {
    headers: Vec<TableHeader>,
    rows: Vec<TableRow>,
    sortable: bool,
    filterable: bool,
    selectable: SelectionMode,
}

impl TableBuilder {
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
            rows: Vec::new(),
            sortable: false,
            filterable: false,
            selectable: SelectionMode::None,
        }
    }
    
    pub fn header(mut self, text: impl Into<String>, width: impl Into<String>) -> Self {
        self.headers.push(TableHeader {
            text: text.into(),
            width: width.into(),
        });
        self
    }
    
    pub fn row(mut self, id: impl Into<String>, cells: Vec<String>) -> Self {
        self.rows.push(TableRow {
            id: id.into(),
            cells,
            actions: Vec::new(),
            status: None,
        });
        self
    }
    
    pub fn sortable(mut self, sortable: bool) -> Self {
        self.sortable = sortable;
        self
    }
    
    pub fn filterable(mut self, filterable: bool) -> Self {
        self.filterable = filterable;
        self
    }
    
    pub fn selectable(mut self, mode: SelectionMode) -> Self {
        self.selectable = mode;
        self
    }
    
    pub fn build(self) -> TableComponent {
        TableComponent {
            props: TableProps {
                headers: self.headers,
                rows: self.rows,
                sortable: self.sortable,
                filterable: self.filterable,
                selectable: self.selectable,
            },
        }
    }
}

impl Default for TableBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper to create a simple progress bar
pub fn progress(current: u64, total: u64, message: impl Into<String>) -> ProgressComponent {
    ProgressBuilder::new(current, total, message).build()
}

/// Helper to create a table from data
pub fn table_from_data(headers: Vec<&str>, data: Vec<HashMap<String, String>>) -> TableComponent {
    let mut builder = TableBuilder::new();
    
    for header in headers.iter() {
        builder = builder.header(*header, "auto");
    }
    
    for (i, row_data) in data.into_iter().enumerate() {
        let cells: Vec<String> = headers.iter()
            .map(|header| row_data.get(*header).cloned().unwrap_or_default())
            .collect();
        builder = builder.row(format!("row_{}", i), cells);
    }
    
    builder.build()
}