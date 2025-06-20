//! Block system for interactive command blocks in The Hub
//!
//! This module implements the core block abstraction that represents
//! interactive command sessions as first-class objects.

use anyhow::Result;
use hub_protocol::messages::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use gpui::{Entity, Context, App};

/// A unique identifier for a block
pub type BlockId = String;

/// Core block structure representing an interactive command session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub id: BlockId,
    pub command: String,
    pub args: Vec<String>,
    pub working_directory: String,
    pub status: BlockStatus,
    pub content: BlockContent,
    pub metadata: BlockMetadata,
}

/// Status of a block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BlockStatus {
    Running,
    Completed { exit_code: i32 },
    Failed { error: String },
    Paused,
    Cancelled,
}

/// Content within a block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockContent {
    pub text_output: Vec<String>,
    pub ui_components: Vec<UiComponent>,
    pub interactions: Vec<BlockInteraction>,
}

/// UI component within a block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiComponent {
    pub id: String,
    pub component_type: String,
    pub props: serde_json::Value,
    pub position: ComponentPosition,
}

/// Position of a component within a block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentPosition {
    pub row: u32,
    pub column: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Interaction within a block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockInteraction {
    pub id: String,
    pub interaction_type: String,
    pub target: String,
    pub data: serde_json::Value,
}

/// Metadata about a block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockMetadata {
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub tags: Vec<String>,
    pub session_id: String,
}

/// GPUI entity for rendering blocks in the UI
pub struct BlockEntity {
    block: Block,
}

impl BlockEntity {
    pub fn new(block: Block) -> Self {
        Self { block }
    }
    
    pub fn block(&self) -> &Block {
        &self.block
    }
    
    pub fn update_block(&mut self, block: Block) {
        self.block = block;
    }
}

impl Block {
    /// Create a new block
    pub fn new(
        id: BlockId,
        command: String,
        args: Vec<String>,
        working_directory: String,
        session_id: String,
    ) -> Self {
        let now = chrono::Utc::now();
        
        Self {
            id,
            command,
            args,
            working_directory,
            status: BlockStatus::Running,
            content: BlockContent {
                text_output: Vec::new(),
                ui_components: Vec::new(),
                interactions: Vec::new(),
            },
            metadata: BlockMetadata {
                created_at: now,
                updated_at: now,
                tags: Vec::new(),
                session_id,
            },
        }
    }
    
    /// Add text output to the block
    pub fn add_output(&mut self, text: String) {
        self.content.text_output.push(text);
        self.metadata.updated_at = chrono::Utc::now();
    }
    
    /// Add a UI component to the block
    pub fn add_ui_component(&mut self, component: UiComponent) {
        self.content.ui_components.push(component);
        self.metadata.updated_at = chrono::Utc::now();
    }
    
    /// Add an interaction to the block
    pub fn add_interaction(&mut self, interaction: BlockInteraction) {
        self.content.interactions.push(interaction);
        self.metadata.updated_at = chrono::Utc::now();
    }
    
    /// Update block status
    pub fn set_status(&mut self, status: BlockStatus) {
        self.status = status;
        self.metadata.updated_at = chrono::Utc::now();
    }
    
    /// Check if block is still active
    pub fn is_active(&self) -> bool {
        matches!(self.status, BlockStatus::Running | BlockStatus::Paused)
    }
}