//! Hub protocol server implementation

use crate::messages::*;
use crate::transport::*;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Hub protocol server
pub struct HubServer {
    connection_manager: ConnectionManager,
    sessions: Arc<RwLock<HashMap<String, Session>>>,
    message_router: MessageRouter,
}

/// Active session state
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub capabilities: SessionCapabilities,
    pub start_time: chrono::DateTime<chrono::Utc>,
    pub sequence_counter: u64,
    pub components: HashMap<String, ComponentState>,
}

/// Component state tracking
#[derive(Debug, Clone)]
pub struct ComponentState {
    pub component_type: String,
    pub properties: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Message routing and handling
pub struct MessageRouter {
    handlers: HashMap<MessageType, Box<dyn MessageHandler>>,
}

/// Session event notifications
#[derive(Debug, Clone)]
pub enum SessionEvent {
    SessionStarted(Session),
    SessionEnded { session_id: String, exit_code: i32 },
    ComponentCreated { session_id: String, component_id: String },
    ComponentUpdated { session_id: String, component_id: String },
    UserInteraction { session_id: String, interaction: UserInteraction },
}

/// User interaction types
#[derive(Debug, Clone)]
pub enum UserInteraction {
    ComponentAction {
        component_id: String,
        action: String,
        data: serde_json::Value,
    },
    FormSubmission {
        form_id: String,
        values: HashMap<String, serde_json::Value>,
    },
    SelectionChanged {
        component_id: String,
        selected_ids: Vec<String>,
    },
}

/// Session event handler trait
#[async_trait::async_trait]
pub trait SessionEventHandler: Send + Sync {
    async fn handle_event(&self, event: SessionEvent) -> Result<()>;
}

impl HubServer {
    pub fn new() -> Self {
        let mut connection_manager = ConnectionManager::new();
        let message_router = MessageRouter::new();
        
        // Add default message handlers
        connection_manager.add_message_handler(Box::new(DefaultMessageHandler::new()));
        
        Self {
            connection_manager,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            message_router,
        }
    }
    
    pub async fn add_listener(
        &mut self,
        name: String,
        config: TransportConfig,
    ) -> Result<()> {
        self.connection_manager.add_listener(name, config).await
    }
    
    pub fn add_session_event_handler(&mut self, handler: Box<dyn SessionEventHandler>) {
        // Implementation would add handler to a list of event handlers
    }
    
    pub async fn start(&mut self) -> Result<()> {
        self.connection_manager.start().await?;
        
        println!("Hub server started and listening for connections");
        
        // Keep the server running
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
        }
    }
    
    pub async fn get_session(&self, session_id: &str) -> Option<Session> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).cloned()
    }
    
    pub async fn list_sessions(&self) -> Vec<Session> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }
    
    pub async fn broadcast_to_session(
        &self,
        session_id: &str,
        message: MessageEnvelope,
    ) -> Result<()> {
        // For now, broadcast to all connections
        // In a real implementation, we'd track which connections belong to which sessions
        self.connection_manager.broadcast(message).await
    }
    
    async fn handle_session_start(&self, payload: ControlPayload) -> Result<()> {
        if let ControlPayload::SessionStart {
            command,
            args,
            cwd,
            capabilities,
        } = payload
        {
            let session = Session {
                id: uuid::Uuid::new_v4().to_string(),
                command,
                args,
                cwd,
                capabilities,
                start_time: chrono::Utc::now(),
                sequence_counter: 0,
                components: HashMap::new(),
            };
            
            let session_id = session.id.clone();
            
            {
                let mut sessions = self.sessions.write().await;
                sessions.insert(session_id.clone(), session.clone());
            }
            
            println!("Session started: {} - {}", session_id, session.command);
            
            // Notify event handlers
            // self.notify_event_handlers(SessionEvent::SessionStarted(session)).await?;
        }
        
        Ok(())
    }
    
    async fn handle_session_end(&self, session_id: &str, payload: ControlPayload) -> Result<()> {
        if let ControlPayload::SessionEnd { exit_code, .. } = payload {
            {
                let mut sessions = self.sessions.write().await;
                sessions.remove(session_id);
            }
            
            println!("Session ended: {} (exit code: {})", session_id, exit_code);
            
            // Notify event handlers
            // self.notify_event_handlers(SessionEvent::SessionEnded {
            //     session_id: session_id.to_string(),
            //     exit_code,
            // }).await?;
        }
        
        Ok(())
    }
    
    async fn handle_ui_message(
        &self,
        session_id: &str,
        payload: UiMessagePayload,
    ) -> Result<()> {
        let component_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        
        let (component_type, properties) = match &payload {
            UiMessagePayload::Progress(component) => {
                ("progress".to_string(), serde_json::to_value(component)?)
            }
            UiMessagePayload::Table(component) => {
                ("table".to_string(), serde_json::to_value(component)?)
            }
            UiMessagePayload::FileTree(component) => {
                ("file_tree".to_string(), serde_json::to_value(component)?)
            }
            UiMessagePayload::Form(component) => {
                ("form".to_string(), serde_json::to_value(component)?)
            }
            UiMessagePayload::StatusGrid(component) => {
                ("status_grid".to_string(), serde_json::to_value(component)?)
            }
            UiMessagePayload::Update(update) => {
                // Handle component updates
                self.update_component(session_id, &update.target, &update.props)
                    .await?;
                return Ok(());
            }
            UiMessagePayload::Stream(stream) => {
                // Handle streaming data
                ("stream".to_string(), serde_json::to_value(stream)?)
            }
        };
        
        let component_state = ComponentState {
            component_type: component_type.clone(),
            properties,
            created_at: now,
            updated_at: now,
        };
        
        {
            let mut sessions = self.sessions.write().await;
            if let Some(session) = sessions.get_mut(session_id) {
                session.components.insert(component_id.clone(), component_state);
            }
        }
        
        println!(
            "Component created in session {}: {} ({})",
            session_id, component_id, component_type
        );
        
        Ok(())
    }
    
    async fn update_component(
        &self,
        session_id: &str,
        component_id: &str,
        props: &serde_json::Value,
    ) -> Result<()> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            if let Some(component) = session.components.get_mut(component_id) {
                component.properties = props.clone();
                component.updated_at = chrono::Utc::now();
                
                println!(
                    "Component updated in session {}: {}",
                    session_id, component_id
                );
            }
        }
        
        Ok(())
    }
}

impl MessageRouter {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }
    
    pub fn add_handler(&mut self, message_type: MessageType, handler: Box<dyn MessageHandler>) {
        self.handlers.insert(message_type, handler);
    }
    
    pub async fn route_message(&self, message: MessageEnvelope) -> Result<Option<MessageEnvelope>> {
        if let Some(handler) = self.handlers.get(&message.message_type) {
            handler.handle_message(message).await
        } else {
            Ok(None)
        }
    }
}

/// Default message handler implementation
pub struct DefaultMessageHandler {
    sessions: Arc<RwLock<HashMap<String, Session>>>,
}

impl DefaultMessageHandler {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl MessageHandler for DefaultMessageHandler {
    async fn handle_message(&self, message: MessageEnvelope) -> Result<Option<MessageEnvelope>> {
        match message.payload {
            MessagePayload::Control(payload) => {
                match payload {
                    ControlPayload::SessionStart { .. } => {
                        println!("Handling session start");
                        // Handle session start logic here
                    }
                    ControlPayload::SessionEnd { .. } => {
                        println!("Handling session end");
                        // Handle session end logic here
                    }
                }
            }
            MessagePayload::UiMessage(payload) => {
                println!("Handling UI message: {:?}", payload);
                // Handle UI message logic here
            }
            MessagePayload::Response(payload) => {
                println!("Handling response: {:?}", payload);
                // Handle response logic here
            }
            MessagePayload::Event(payload) => {
                println!("Handling event: {:?}", payload);
                // Handle event logic here
            }
            _ => {
                println!("Unhandled message type: {:?}", message.message_type);
            }
        }
        
        Ok(None)
    }
}

/// Builder for Hub server configuration
pub struct HubServerBuilder {
    listeners: Vec<(String, TransportConfig)>,
    event_handlers: Vec<Box<dyn SessionEventHandler>>,
}

impl HubServerBuilder {
    pub fn new() -> Self {
        Self {
            listeners: Vec::new(),
            event_handlers: Vec::new(),
        }
    }
    
    pub fn with_unix_socket(mut self, name: String, path: std::path::PathBuf) -> Self {
        self.listeners.push((
            name,
            TransportConfig::UnixSocket { path },
        ));
        self
    }
    
    pub fn with_tcp_socket(mut self, name: String, host: String, port: u16) -> Self {
        self.listeners.push((
            name,
            TransportConfig::TcpSocket { host, port },
        ));
        self
    }
    
    pub fn with_event_handler(mut self, handler: Box<dyn SessionEventHandler>) -> Self {
        self.event_handlers.push(handler);
        self
    }
    
    pub async fn build(self) -> Result<HubServer> {
        let mut server = HubServer::new();
        
        for (name, config) in self.listeners {
            server.add_listener(name, config).await?;
        }
        
        for handler in self.event_handlers {
            server.add_session_event_handler(handler);
        }
        
        Ok(server)
    }
}

impl Default for HubServerBuilder {
    fn default() -> Self {
        Self::new()
    }
}