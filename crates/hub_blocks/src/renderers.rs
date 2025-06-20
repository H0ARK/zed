//! Block renderers for The Hub UI
//!
//! This module contains renderers that convert blocks and their content
//! into visual elements using the GPUI framework.

use crate::block::{Block, BlockContent, UiComponent};
use anyhow::Result;
use gpui::*;
use hub_protocol::messages::*;

/// Renderer for individual blocks
pub struct BlockRenderer {
    block: Block,
}

impl BlockRenderer {
    pub fn new(block: Block) -> Self {
        Self { block }
    }
    
    pub fn update_block(&mut self, block: Block) {
        self.block = block;
    }
}

impl Render for BlockRenderer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .bg(rgb(0x1e1e1e))
            .border_1()
            .border_color(rgb(0x3e3e3e))
            .rounded_md()
            .p_4()
            .child(self.render_header())
            .child(self.render_content())
            .child(self.render_status())
    }
}

impl BlockRenderer {
    fn render_header(&self) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .mb_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .text_color(rgb(0x9ca3af))
                            .text_xs()
                            .child(format!("$ {} {}", self.block.command, self.block.args.join(" ")))
                    )
            )
            .child(
                div()
                    .text_color(rgb(0x6b7280))
                    .text_xs()
                    .child(format!("Block {}", &self.block.id[..8]))
            )
    }
    
    fn render_content(&self) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(self.render_text_output())
            .child(self.render_ui_components())
    }
    
    fn render_text_output(&self) -> impl IntoElement {
        if self.block.content.text_output.is_empty() {
            return div();
        }
        
        div()
            .bg(rgb(0x111111))
            .border_1()
            .border_color(rgb(0x2a2a2a))
            .rounded_md()
            .p_3()
            .child(
                div()
                    .font_family("monospace")
                    .text_sm()
                    .text_color(rgb(0xd1d5db))
                    .child(self.block.content.text_output.join("\n"))
            )
    }
    
    fn render_ui_components(&self) -> impl IntoElement {
        if self.block.content.ui_components.is_empty() {
            return div();
        }
        
        div()
            .flex()
            .flex_col()
            .gap_2()
            .children(
                self.block.content.ui_components
                    .iter()
                    .map(|component| self.render_component(component))
            )
    }
    
    fn render_component(&self, component: &UiComponent) -> impl IntoElement {
        div()
            .child(match component.component_type.as_str() {
                "progress" => self.render_progress_component(component).into_any_element(),
                "table" => self.render_table_component(component).into_any_element(),
                "form" => self.render_form_component(component).into_any_element(),
                _ => self.render_generic_component(component).into_any_element(),
            })
    }
    
    fn render_progress_component(&self, _component: &UiComponent) -> impl IntoElement {
        div()
            .bg(rgb(0x1f2937))
            .border_1()
            .border_color(rgb(0x374151))
            .rounded_md()
            .p_3()
            .child(
                div()
                    .text_color(rgb(0xd1d5db))
                    .text_sm()
                    .child("Progress Component")
            )
    }
    
    fn render_table_component(&self, _component: &UiComponent) -> impl IntoElement {
        div()
            .bg(rgb(0x1f2937))
            .border_1()
            .border_color(rgb(0x374151))
            .rounded_md()
            .p_3()
            .child(
                div()
                    .text_color(rgb(0xd1d5db))
                    .text_sm()
                    .child("Table Component")
            )
    }
    
    fn render_form_component(&self, _component: &UiComponent) -> impl IntoElement {
        div()
            .bg(rgb(0x1f2937))
            .border_1()
            .border_color(rgb(0x374151))
            .rounded_md()
            .p_3()
            .child(
                div()
                    .text_color(rgb(0xd1d5db))
                    .text_sm()
                    .child("Form Component")
            )
    }
    
    fn render_generic_component(&self, component: &UiComponent) -> impl IntoElement {
        div()
            .bg(rgb(0x1f2937))
            .border_1()
            .border_color(rgb(0x374151))
            .rounded_md()
            .p_3()
            .child(
                div()
                    .text_color(rgb(0xd1d5db))
                    .text_sm()
                    .child(format!("Component: {}", component.component_type))
            )
    }
    
    fn render_status(&self) -> impl IntoElement {
        let (status_text, status_color) = match &self.block.status {
            crate::block::BlockStatus::Running => ("Running", rgb(0x10b981)),
            crate::block::BlockStatus::Completed { exit_code } => {
                if *exit_code == 0 {
                    ("Completed", rgb(0x10b981))
                } else {
                    ("Failed", rgb(0xef4444))
                }
            }
            crate::block::BlockStatus::Failed { .. } => ("Failed", rgb(0xef4444)),
            crate::block::BlockStatus::Paused => ("Paused", rgb(0xf59e0b)),
            crate::block::BlockStatus::Cancelled => ("Cancelled", rgb(0x6b7280)),
        };
        
        div()
            .flex()
            .items_center()
            .justify_between()
            .mt_2()
            .pt_2()
            .border_t_1()
            .border_color(rgb(0x374151))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .w_2()
                            .h_2()
                            .bg(status_color)
                            .rounded_full()
                    )
                    .child(
                        div()
                            .text_color(status_color)
                            .text_xs()
                            .child(status_text)
                    )
            )
            .child(
                div()
                    .text_color(rgb(0x6b7280))
                    .text_xs()
                    .child(format!("Updated: {}", self.block.metadata.updated_at.format("%H:%M:%S")))
            )
    }
}

/// Container for rendering multiple blocks
pub struct BlockContainer {
    blocks: Vec<Block>,
}

impl BlockContainer {
    pub fn new(blocks: Vec<Block>) -> Self {
        Self { blocks }
    }
    
    pub fn update_blocks(&mut self, blocks: Vec<Block>) {
        self.blocks = blocks;
    }
}

impl Render for BlockContainer {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .children(
                self.blocks
                    .iter()
                    .map(|block| {
                        div()
                            .bg(rgb(0x1e1e1e))
                            .border_1()
                            .border_color(rgb(0x3e3e3e))
                            .rounded_md()
                            .p_4()
                            .child(format!("Block: {}", block.command))
                    })
            )
    }
}