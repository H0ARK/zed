//! Enhanced terminal with Hub protocol support

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::Result;
use gpui::{App, Context, Entity, EventEmitter, WeakEntity};
use terminal::Terminal;
use hub_protocol::{MessageEnvelope, MessageType, HubClient, HubClientBuilder};

/// Enhanced terminal that can detect and handle Hub protocol communications
pub struct HubTerminal {
    /// Base terminal instance
    terminal: Entity<Terminal>,
    
    /// Hub protocol client for communication
    hub_client: Option<Arc<Mutex<HubClient>>>,
    
    /// Current session ID for Hub communication
    session_id: Option<String>,
    
    /// Active Hub blocks in this terminal
    active_blocks: HashMap<String, HubBlock>,
    
    /// Protocol detection state
    protocol_state: ProtocolState,
}

/// State for detecting Hub protocol communications
#[derive(Debug, Clone)]
pub struct ProtocolState {
    /// Whether Hub protocol is active
    pub active: bool,
    
    /// Buffer for collecting protocol messages
    pub message_buffer: String,
    
    /// Last protocol activity timestamp
    pub last_activity: Option<SystemTime>,
}

/// Represents an active Hub block in the terminal
#[derive(Debug, Clone)]
pub struct HubBlock {
    /// Block identifier
    pub id: String,
    
    /// Block type (progress, table, etc.)
    pub block_type: String,
    
    /// Block content/state
    pub content: serde_json::Value,
    
    /// Position in terminal
    pub position: Option<(u16, u16)>,
}

impl HubTerminal {
    /// Create a new Hub-enhanced terminal
    pub fn new(terminal: Entity<Terminal>) -> Self {
        Self {
            terminal,
            hub_client: None,
            session_id: None,
            active_blocks: HashMap::new(),
            protocol_state: ProtocolState {
                active: false,
                message_buffer: String::new(),
                last_activity: None,
            },
        }
    }
    
    /// Initialize Hub protocol connection
    pub async fn initialize_hub_connection(&mut self) -> Result<()> {
        // Try to build and connect a Hub client using auto-discovery
        if let Ok(client) = HubClientBuilder::default().connect().await {
            self.hub_client = Some(Arc::new(Mutex::new(client)));
            self.protocol_state.active = true;
            self.session_id = Some(format!("terminal-{}", uuid::Uuid::new_v4()));
            log::info!("Hub protocol initialized via auto-discovery");
        } else {
            log::debug!("Hub protocol not available - falling back to traditional terminal mode");
        }
        
        Ok(())
    }
    
    /// Process terminal input and detect Hub protocol commands
    pub fn process_input(&mut self, input: &[u8]) -> Result<Vec<u8>> {
        let input_str = String::from_utf8_lossy(input);
        
        // Check for Hub protocol markers
        if self.detect_hub_protocol(&input_str) {
            self.handle_hub_command(&input_str)?;
        }
        
        // Pass through to underlying terminal
        Ok(input.to_vec())
    }
    
    /// Process terminal output and extract Hub protocol messages
    pub fn process_output(&mut self, output: &[u8]) -> Result<Vec<u8>> {
        let output_str = String::from_utf8_lossy(output);
        
        // Look for Hub protocol messages in output
        if let Some(messages) = self.extract_hub_messages(&output_str) {
            for message in messages {
                self.handle_hub_message(message)?;
            }
        }
        
        // Return filtered output (with protocol messages removed)
        Ok(self.filter_protocol_output(output))
    }
    
    /// Detect if input contains Hub protocol commands
    fn detect_hub_protocol(&self, input: &str) -> bool {
        // Look for Hub protocol markers or special commands
        input.contains("--hub") || 
        input.contains("HUB_PROTOCOL=1") ||
        self.session_id.is_some()
    }
    
    /// Handle Hub protocol command
    fn handle_hub_command(&mut self, command: &str) -> Result<()> {
        if let Some(client) = &self.hub_client {
            if let Some(session_id) = &self.session_id {
                // Parse command and send to Hub
                let parts: Vec<&str> = command.trim().split_whitespace().collect();
                if !parts.is_empty() {
                    let message = MessageEnvelope::session_start(
                        session_id.clone(),
                        1,
                        parts[0].to_string(),
                        parts[1..].iter().map(|s| s.to_string()).collect(),
                        std::env::current_dir()?.to_string_lossy().to_string(),
                        hub_protocol::SessionCapabilities {
                            ui_components: vec!["progress".to_string(), "table".to_string()],
                            interactions: vec!["click".to_string()],
                            ai_integration: false,
                        },
                    );
                    
                    // Send message asynchronously
                    if let Ok(mut client) = client.lock() {
                        // TODO: Send message asynchronously
                        log::info!("Would send Hub message: {:?}", message);
                    }
                }
            }
        }
        Ok(())
    }
    
    /// Extract Hub protocol messages from terminal output
    fn extract_hub_messages(&mut self, output: &str) -> Option<Vec<MessageEnvelope>> {
        let mut messages = Vec::new();
        
        // Add output to buffer
        self.protocol_state.message_buffer.push_str(output);
        
        // Look for complete JSON messages
        let mut start = 0;
        while let Some(open_brace) = self.protocol_state.message_buffer[start..].find('{') {
            let absolute_start = start + open_brace;
            
            // Try to find matching closing brace
            let mut brace_count = 0;
            let mut end_pos = None;
            
            for (i, ch) in self.protocol_state.message_buffer[absolute_start..].char_indices() {
                match ch {
                    '{' => brace_count += 1,
                    '}' => {
                        brace_count -= 1;
                        if brace_count == 0 {
                            end_pos = Some(absolute_start + i + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            
            if let Some(end) = end_pos {
                let json_str = &self.protocol_state.message_buffer[absolute_start..end];
                if let Ok(message) = serde_json::from_str::<MessageEnvelope>(json_str) {
                    messages.push(message);
                }
                start = end;
            } else {
                break;
            }
        }
        
        // Clean processed messages from buffer
        if start > 0 {
            self.protocol_state.message_buffer = self.protocol_state.message_buffer[start..].to_string();
        }
        
        if !messages.is_empty() {
            self.protocol_state.last_activity = Some(SystemTime::now());
            Some(messages)
        } else {
            None
        }
    }
    
    /// Handle Hub protocol message
    fn handle_hub_message(&mut self, message: MessageEnvelope) -> Result<()> {
        match message.message_type {
            MessageType::UiMessage => {
                if let hub_protocol::MessagePayload::UiMessage(payload) = message.payload {
                    // Extract component info based on payload type
                    let (component_type, props) = match payload {
                        hub_protocol::UiMessagePayload::Progress(component) => {
                            ("progress".to_string(), serde_json::to_value(&component).unwrap_or_default())
                        }
                        hub_protocol::UiMessagePayload::Table(component) => {
                            ("table".to_string(), serde_json::to_value(&component).unwrap_or_default())
                        }
                        hub_protocol::UiMessagePayload::FileTree(component) => {
                            ("file_tree".to_string(), serde_json::to_value(&component).unwrap_or_default())
                        }
                        hub_protocol::UiMessagePayload::Form(component) => {
                            ("form".to_string(), serde_json::to_value(&component).unwrap_or_default())
                        }
                        hub_protocol::UiMessagePayload::StatusGrid(component) => {
                            ("status_grid".to_string(), serde_json::to_value(&component).unwrap_or_default())
                        }
                        hub_protocol::UiMessagePayload::Stream(component) => {
                            ("stream".to_string(), serde_json::to_value(&component).unwrap_or_default())
                        }
                        hub_protocol::UiMessagePayload::Update(component) => {
                            ("update".to_string(), serde_json::to_value(&component).unwrap_or_default())
                        }
                    };
                    
                    // Create or update Hub block
                    let block = HubBlock {
                        id: format!("{}_{}", message.session_id, component_type),
                        block_type: component_type.clone(),
                        content: props,
                        position: None, // TODO: Determine position
                    };
                    
                    self.active_blocks.insert(block.id.clone(), block);
                    log::info!("Created Hub block: {} ({})", component_type, message.session_id);
                }
            }
            MessageType::Control => {
                // Handle control messages (session start/end, etc.)
                log::info!("Received Hub control message for session: {}", message.session_id);
            }
            _ => {
                log::debug!("Received Hub message: {:?}", message.message_type);
            }
        }
        Ok(())
    }
    
    /// Filter Hub protocol messages from terminal output
    fn filter_protocol_output(&self, output: &[u8]) -> Vec<u8> {
        let output_str = String::from_utf8_lossy(output);
        
        // Remove JSON protocol messages but keep regular output
        let mut filtered = String::new();
        let mut in_json = false;
        let mut brace_count = 0;
        
        for ch in output_str.chars() {
            match ch {
                '{' if !in_json => {
                    in_json = true;
                    brace_count = 1;
                }
                '{' if in_json => {
                    brace_count += 1;
                }
                '}' if in_json => {
                    brace_count -= 1;
                    if brace_count == 0 {
                        in_json = false;
                        // Skip this character (end of JSON)
                        continue;
                    }
                }
                _ if !in_json => {
                    filtered.push(ch);
                }
                _ => {
                    // Inside JSON, skip
                }
            }
        }
        
        filtered.into_bytes()
    }
    
    /// Get the underlying terminal entity
    pub fn terminal(&self) -> &Entity<Terminal> {
        &self.terminal
    }
    
    /// Get active Hub blocks
    pub fn active_blocks(&self) -> &HashMap<String, HubBlock> {
        &self.active_blocks
    }
    
    /// Check if Hub protocol is active
    pub fn is_hub_active(&self) -> bool {
        self.protocol_state.active
    }
}

impl EventEmitter<()> for HubTerminal {}