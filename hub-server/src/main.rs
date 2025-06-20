//! Standalone Hub server daemon
//! 
//! This is a simple Hub server that runs as a background process and handles
//! CLI tool connections for The Hub protocol.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use anyhow::Result;

// Import our Hub protocol types
use hub_protocol::{
    MessageEnvelope, MessageType, MessagePayload, 
    ControlPayload, UiMessagePayload, ProgressComponent, ProgressProps, ProgressStyle,
    TcpSocketTransport, Transport
};

/// Simple Hub server that manages CLI sessions
pub struct HubServer {
    /// Active sessions
    sessions: Arc<RwLock<HashMap<String, HubSession>>>,
    /// Server port
    port: u16,
}

/// Represents an active CLI session
#[derive(Debug, Clone)]
pub struct HubSession {
    pub session_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub connected_at: std::time::SystemTime,
}

impl HubServer {
    pub fn new(port: Option<u16>) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            port: port.unwrap_or(7878),
        }
    }
    
    /// Start the Hub server
    pub async fn start(&self) -> Result<()> {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", self.port)).await?;
        println!("🚀 Hub server listening on port {}", self.port);
        println!("📡 Ready to accept CLI connections...");
        
        loop {
            match listener.accept().await {
                Ok((stream, addr)) => {
                    println!("🔌 New connection from {}", addr);
                    let sessions = Arc::clone(&self.sessions);
                    
                    // Handle connection in background
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, sessions).await {
                            eprintln!("❌ Connection error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    eprintln!("❌ Failed to accept connection: {}", e);
                }
            }
        }
    }
    
    /// Handle a single client connection
    async fn handle_connection(
        stream: tokio::net::TcpStream,
        sessions: Arc<RwLock<HashMap<String, HubSession>>>,
    ) -> Result<()> {
        let mut transport = TcpSocketTransport::new(stream).await?;
        
        println!("📥 Waiting for messages...");
        
        // Listen for messages
        loop {
            match transport.receive().await? {
                Some(message) => {
                    println!("📨 Received message: {:?}", message.message_type);
                    Self::handle_message(message, &mut transport, &sessions).await?;
                }
                None => {
                    println!("🔌 Connection closed");
                    break;
                }
            }
        }
        
        Ok(())
    }
    
    /// Handle a protocol message
    async fn handle_message(
        message: MessageEnvelope,
        transport: &mut TcpSocketTransport,
        sessions: &Arc<RwLock<HashMap<String, HubSession>>>,
    ) -> Result<()> {
        match message.message_type {
            MessageType::Control => {
                if let MessagePayload::Control(payload) = message.payload {
                    match payload {
                        ControlPayload::SessionStart { command, args, cwd, capabilities } => {
                            println!("🎯 Session started: {} in {}", command, cwd);
                            
                            // Store session
                            let session = HubSession {
                                session_id: message.session_id.clone(),
                                command: command.clone(),
                                args: args.clone(),
                                cwd: cwd.clone(),
                                connected_at: std::time::SystemTime::now(),
                            };
                            
                            sessions.write().await.insert(message.session_id.clone(), session);
                            
                            // Send welcome message as a progress component
                            let welcome_message = MessageEnvelope::new(
                                MessageType::UiMessage,
                                message.session_id.clone(),
                                1,
                                MessagePayload::UiMessage(UiMessagePayload::Progress(ProgressComponent {
                                    props: ProgressProps {
                                        current: 0,
                                        total: 100,
                                        message: format!("🎉 Welcome to The Hub! Running: {}", command),
                                        show_percentage: true,
                                        show_eta: false,
                                        style: ProgressStyle::Bar,
                                    },
                                })),
                            );
                            
                            transport.send(welcome_message).await?;
                            println!("✅ Sent welcome message to session {}", message.session_id);
                        }
                        ControlPayload::SessionEnd { exit_code, duration_ms, summary } => {
                            println!("🏁 Session ended: {} (exit code: {})", message.session_id, exit_code);
                            sessions.write().await.remove(&message.session_id);
                        }
                        _ => {
                            println!("🔄 Control message: {:?}", payload);
                        }
                    }
                }
            }
            MessageType::UiMessage => {
                println!("🎨 UI message received from {}", message.session_id);
                // For demo purposes, echo UI messages back
                transport.send(message).await?;
            }
            _ => {
                println!("📢 Other message: {:?}", message.message_type);
            }
        }
        
        Ok(())
    }
    
    /// Get active sessions
    pub async fn get_sessions(&self) -> HashMap<String, HubSession> {
        self.sessions.read().await.clone()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    env_logger::init();
    
    println!("🌟 The Hub Server Daemon");
    println!("========================");
    
    let server = HubServer::new(Some(7878));
    
    // Start server
    server.start().await?;
    
    Ok(())
}