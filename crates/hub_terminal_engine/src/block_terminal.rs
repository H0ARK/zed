//! Block-based terminal integration for The Hub

use std::collections::HashMap;

use anyhow::Result;
use gpui::{App, AppContext, Context, Entity, EventEmitter, IntoElement, ParentElement, Render, Styled};
use terminal::Terminal;

use crate::enhanced_terminal::{HubBlock, HubTerminal};
use hub_blocks::{Block, BlockContent, UiComponent, BlockEntity};

/// A terminal enhanced with Hub block capabilities
pub struct BlockTerminalView {
    /// Base terminal
    terminal: Entity<Terminal>,
    
    /// Hub enhancement
    hub_terminal: Option<Entity<HubTerminal>>,
    
    /// Active blocks displayed above/below terminal
    active_blocks: HashMap<String, Entity<BlockEntity>>,
    
    /// Block layout configuration
    layout_config: BlockLayoutConfig,
}

/// Configuration for how blocks are laid out around the terminal
#[derive(Debug, Clone)]
pub struct BlockLayoutConfig {
    /// Show blocks above terminal
    pub blocks_above: bool,
    
    /// Show blocks below terminal
    pub blocks_below: bool,
    
    /// Maximum number of blocks to show
    pub max_blocks: usize,
    
    /// Block spacing
    pub block_spacing: f32,
}

impl Default for BlockLayoutConfig {
    fn default() -> Self {
        Self {
            blocks_above: true,
            blocks_below: true,
            max_blocks: 10,
            block_spacing: 8.0,
        }
    }
}

impl BlockTerminalView {
    /// Create a new block terminal view
    pub fn new(terminal: Entity<Terminal>) -> Self {
        Self {
            terminal,
            hub_terminal: None,
            active_blocks: HashMap::new(),
            layout_config: BlockLayoutConfig::default(),
        }
    }
    
    /// Enable Hub enhancement for this terminal view
    pub fn enable_hub(&mut self, hub_terminal: Entity<HubTerminal>) {
        self.hub_terminal = Some(hub_terminal);
    }
    
    /// Update blocks from Hub terminal state
    pub fn update_blocks(&mut self, cx: &mut Context<Self>) {
        if let Some(hub_terminal) = &self.hub_terminal {
            // Read the hub terminal state to get current blocks
            let hub_blocks_snapshot = hub_terminal.read(cx).active_blocks().clone();
            
            // Remove blocks that no longer exist
            self.active_blocks.retain(|block_id, _| {
                hub_blocks_snapshot.contains_key(block_id)
            });
            
            // Add or update blocks
            for (block_id, hub_block) in &hub_blocks_snapshot {
                if !self.active_blocks.contains_key(block_id) {
                    // Create new block
                    if let Ok(block_entity) = self.create_block_from_hub_block(hub_block, cx) {
                        self.active_blocks.insert(block_id.clone(), block_entity);
                    }
                } else {
                    // Update existing block
                    if let Some(block_entity) = self.active_blocks.get(block_id) {
                        self.update_block_from_hub_block(block_entity, hub_block, cx);
                    }
                }
            }
        }
    }
    
    /// Create a Block entity from a HubBlock
    fn create_block_from_hub_block(
        &self, 
        hub_block: &HubBlock, 
        cx: &mut Context<Self>
    ) -> Result<Entity<BlockEntity>> {
        let _content = self.create_content_from_hub_block(hub_block)?;
        
        let block = Block::new(
            hub_block.id.clone(),
            "hub_command".to_string(), // Default command
            vec![], // Default args
            std::env::current_dir()?.to_string_lossy().to_string(),
            format!("session_{}", uuid::Uuid::new_v4()),
        );
        
        let block_entity = BlockEntity::new(block);
        
        Ok(cx.new(|_cx| block_entity))
    }
    
    /// Update a Block entity from a HubBlock
    fn update_block_from_hub_block(
        &self,
        _block_entity: &Entity<BlockEntity>,
        hub_block: &HubBlock,
        _cx: &mut Context<Self>
    ) {
        // For now, we just log the update since we need to understand the update mechanism
        log::info!("Updating block {} with type {}", hub_block.id, hub_block.block_type);
    }
    
    /// Create content from a HubBlock
    fn create_content_from_hub_block(&self, hub_block: &HubBlock) -> Result<BlockContent> {
        // Create a UI component based on the Hub block
        let ui_component = UiComponent {
            id: hub_block.id.clone(),
            component_type: hub_block.block_type.clone(),
            props: hub_block.content.clone(),
            position: hub_blocks::ComponentPosition {
                row: 0,
                column: 0,
                width: None,
                height: None,
            },
        };
        
        Ok(BlockContent {
            text_output: vec![format!("Hub {} block", hub_block.block_type)],
            ui_components: vec![ui_component],
            interactions: vec![],
        })
    }
    
    /// Get terminal view
    pub fn terminal(&self) -> &Entity<Terminal> {
        &self.terminal
    }
    
    /// Get active blocks
    pub fn active_blocks(&self) -> &HashMap<String, Entity<BlockEntity>> {
        &self.active_blocks
    }
}

impl EventEmitter<()> for BlockTerminalView {}

impl Render for BlockTerminalView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Update blocks from Hub state
        self.update_blocks(cx);
        
        // We need to get the terminal element differently since we can't pass our context
        // For now, just create a placeholder
        let terminal_element = gpui::div()
            .bg(gpui::rgb(0x1e1e1e))
            .border_1()
            .border_color(gpui::rgb(0x454545))
            .child("Terminal View - Hub Integration in Progress");
        
        // Create container with blocks
        let mut container = gpui::div()
            .flex()
            .flex_col()
            .size_full();
        
        // Add blocks above terminal if configured
        if self.layout_config.blocks_above {
            let blocks_above: Vec<_> = self.active_blocks
                .values()
                .take(self.layout_config.max_blocks / 2)
                .collect();
                
            for block_entity in blocks_above {
                // For now, just show a simple representation of the block
                let block_info = block_entity.read(cx);
                let block_element = gpui::div()
                    .p_2()
                    .bg(gpui::rgb(0x2d2d2d))
                    .border_1()
                    .border_color(gpui::rgb(0x454545))
                    .child(format!("Hub Block: {}", block_info.block().id));
                    
                container = container.child(
                    gpui::div()
                        .mb(gpui::px(self.layout_config.block_spacing))
                        .child(block_element)
                );
            }
        }
        
        // Add terminal
        container = container.child(terminal_element);
        
        // Add blocks below terminal if configured
        if self.layout_config.blocks_below {
            let blocks_below: Vec<_> = self.active_blocks
                .values()
                .skip(self.layout_config.max_blocks / 2)
                .take(self.layout_config.max_blocks / 2)
                .collect();
                
            for block_entity in blocks_below {
                // For now, just show a simple representation of the block
                let block_info = block_entity.read(cx);
                let block_element = gpui::div()
                    .p_2()
                    .bg(gpui::rgb(0x2d2d2d))
                    .border_1()
                    .border_color(gpui::rgb(0x454545))
                    .child(format!("Hub Block: {}", block_info.block().id));
                    
                container = container.child(
                    gpui::div()
                        .mt(gpui::px(self.layout_config.block_spacing))
                        .child(block_element)
                );
            }
        }
        
        container
    }
}

/// Create a Hub-enhanced terminal view
pub fn create_block_terminal_view(
    terminal: Entity<Terminal>,
    hub_terminal: Option<Entity<HubTerminal>>,
    cx: &mut App,
) -> Entity<BlockTerminalView> {
    cx.new(|_cx| {
        let mut view = BlockTerminalView::new(terminal);
        if let Some(hub_terminal) = hub_terminal {
            view.enable_hub(hub_terminal);
        }
        view
    })
}