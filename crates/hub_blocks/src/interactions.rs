//! Block interactions for The Hub
//!
//! This module handles user interactions with blocks, including clicks,
//! selections, form submissions, and other interactive elements.

use crate::block::BlockId;
use anyhow::Result;
use gpui::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Types of interactions that can occur within blocks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InteractionType {
    Click,
    Select,
    Input,
    Submit,
    Toggle,
    Drag,
    Resize,
}

/// Interaction event data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionEvent {
    pub interaction_id: String,
    pub block_id: BlockId,
    pub component_id: String,
    pub interaction_type: InteractionType,
    pub data: serde_json::Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Handler for block interactions
pub trait InteractionHandler: Send + Sync {
    /// Handle an interaction event
    fn handle_interaction(&self, event: InteractionEvent) -> Result<InteractionResponse>;
}

/// Response to an interaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionResponse {
    pub success: bool,
    pub message: Option<String>,
    pub updates: Vec<ComponentUpdate>,
    pub new_components: Vec<serde_json::Value>,
}

/// Update to an existing component
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentUpdate {
    pub component_id: String,
    pub property: String,
    pub value: serde_json::Value,
}

/// Manager for handling block interactions
pub struct InteractionManager {
    handlers: HashMap<String, Box<dyn InteractionHandler>>,
}

impl InteractionManager {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }
    
    /// Register an interaction handler for a specific component type
    pub fn register_handler(&mut self, component_type: String, handler: Box<dyn InteractionHandler>) {
        self.handlers.insert(component_type, handler);
    }
    
    /// Process an interaction event
    pub async fn process_interaction(&self, event: InteractionEvent) -> Result<InteractionResponse> {
        // Find the appropriate handler based on the component type
        // For now, we'll use a simple default handler
        let response = InteractionResponse {
            success: true,
            message: Some(format!("Processed {:?} interaction", event.interaction_type)),
            updates: Vec::new(),
            new_components: Vec::new(),
        };
        
        Ok(response)
    }
}

impl Default for InteractionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Default handler for common interactions
pub struct DefaultInteractionHandler;

impl InteractionHandler for DefaultInteractionHandler {
    fn handle_interaction(&self, event: InteractionEvent) -> Result<InteractionResponse> {
        match event.interaction_type {
            InteractionType::Click => self.handle_click(event),
            InteractionType::Select => self.handle_select(event),
            InteractionType::Input => self.handle_input(event),
            InteractionType::Submit => self.handle_submit(event),
            InteractionType::Toggle => self.handle_toggle(event),
            _ => Ok(InteractionResponse {
                success: true,
                message: Some("Interaction processed".to_string()),
                updates: Vec::new(),
                new_components: Vec::new(),
            }),
        }
    }
}

impl DefaultInteractionHandler {
    fn handle_click(&self, _event: InteractionEvent) -> Result<InteractionResponse> {
        Ok(InteractionResponse {
            success: true,
            message: Some("Click handled".to_string()),
            updates: Vec::new(),
            new_components: Vec::new(),
        })
    }
    
    fn handle_select(&self, _event: InteractionEvent) -> Result<InteractionResponse> {
        Ok(InteractionResponse {
            success: true,
            message: Some("Selection updated".to_string()),
            updates: Vec::new(),
            new_components: Vec::new(),
        })
    }
    
    fn handle_input(&self, _event: InteractionEvent) -> Result<InteractionResponse> {
        Ok(InteractionResponse {
            success: true,
            message: Some("Input received".to_string()),
            updates: Vec::new(),
            new_components: Vec::new(),
        })
    }
    
    fn handle_submit(&self, _event: InteractionEvent) -> Result<InteractionResponse> {
        Ok(InteractionResponse {
            success: true,
            message: Some("Form submitted".to_string()),
            updates: Vec::new(),
            new_components: Vec::new(),
        })
    }
    
    fn handle_toggle(&self, _event: InteractionEvent) -> Result<InteractionResponse> {
        Ok(InteractionResponse {
            success: true,
            message: Some("Toggle state changed".to_string()),
            updates: Vec::new(),
            new_components: Vec::new(),
        })
    }
}

/// Interactive element that can be embedded in blocks
pub struct InteractiveElement {
    pub id: String,
    pub element_type: String,
    pub enabled: bool,
}

impl InteractiveElement {
    pub fn new(id: String, element_type: String) -> Self {
        Self {
            id,
            element_type,
            enabled: true,
        }
    }
    
    /// Create a clickable button
    pub fn button(id: String, label: String) -> (Self, impl IntoElement) {
        let element = Self::new(id.clone(), "button".to_string());
        
        let button = div()
            .bg(rgb(0x3b82f6))
            .text_color(rgb(0xffffff))
            .px_4()
            .py_2()
            .rounded_md()
            .child(label);
        
        (element, button)
    }
    
    /// Create a text input field
    pub fn text_input(id: String, placeholder: String) -> (Self, impl IntoElement) {
        let element = Self::new(id.clone(), "text_input".to_string());
        
        let input = div()
            .bg(rgb(0x1f2937))
            .border_1()
            .border_color(rgb(0x374151))
            .text_color(rgb(0xd1d5db))
            .px_3()
            .py_2()
            .rounded_md()
            .child(placeholder);
        
        (element, input)
    }
    
    /// Create a checkbox
    pub fn checkbox(id: String, label: String, checked: bool) -> (Self, impl IntoElement) {
        let element = Self::new(id.clone(), "checkbox".to_string());
        
        let checkbox = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .w_4()
                    .h_4()
                    .bg(if checked { rgb(0x3b82f6) } else { rgb(0x374151) })
                    .border_1()
                    .border_color(rgb(0x6b7280))
                    .rounded_sm()
            )
            .child(
                div()
                    .text_color(rgb(0xd1d5db))
                    .text_sm()
                    .child(label)
            );
        
        (element, checkbox)
    }
}