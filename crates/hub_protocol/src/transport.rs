//! Transport layer implementations for The Hub protocol

use crate::messages::*;
use anyhow::{Context, Result};
use serde_json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::sync::{mpsc, RwLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Transport layer abstraction
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    async fn send(&mut self, message: MessageEnvelope) -> Result<()>;
    async fn receive(&mut self) -> Result<Option<MessageEnvelope>>;
    async fn close(&mut self) -> Result<()>;
    fn is_connected(&self) -> bool;
}

/// Transport configuration
#[derive(Debug, Clone)]
pub enum TransportConfig {
    UnixSocket { path: PathBuf },
    TcpSocket { host: String, port: u16 },
    InMemory { buffer_size: usize },
}

/// Connection manager for handling multiple transports
pub struct ConnectionManager {
    listeners: HashMap<String, Box<dyn Listener>>,
    connections: Arc<RwLock<HashMap<String, Arc<RwLock<Box<dyn Transport>>>>>>,
    message_handlers: Vec<Box<dyn MessageHandler>>,
}

/// Listener trait for accepting new connections
#[async_trait::async_trait]
pub trait Listener: Send + Sync {
    async fn accept(&mut self) -> Result<Box<dyn Transport>>;
    async fn close(&mut self) -> Result<()>;
}

/// Message handler trait
#[async_trait::async_trait]
pub trait MessageHandler: Send + Sync {
    async fn handle_message(&self, message: MessageEnvelope) -> Result<Option<MessageEnvelope>>;
}

/// Unix socket transport implementation
pub struct UnixSocketTransport {
    stream: UnixStream,
    connected: bool,
}

impl UnixSocketTransport {
    pub async fn connect(path: PathBuf) -> Result<Self> {
        let stream = UnixStream::connect(&path)
            .await
            .with_context(|| format!("Failed to connect to Unix socket: {:?}", path))?;
        
        Ok(Self {
            stream,
            connected: true,
        })
    }
}

#[async_trait::async_trait]
impl Transport for UnixSocketTransport {
    async fn send(&mut self, message: MessageEnvelope) -> Result<()> {
        if !self.connected {
            return Err(anyhow::anyhow!("Transport not connected"));
        }
        
        let json = serde_json::to_string(&message)
            .with_context(|| "Failed to serialize message")?;
        
        // Send message with length prefix
        let data = json.as_bytes();
        let len = data.len() as u32;
        self.stream.write_all(&len.to_le_bytes()).await
            .with_context(|| "Failed to write message length")?;
        self.stream.write_all(data).await
            .with_context(|| "Failed to write message data")?;
        
        Ok(())
    }
    
    async fn receive(&mut self) -> Result<Option<MessageEnvelope>> {
        if !self.connected {
            return Ok(None);
        }
        
        // Read message length first
        let mut len_bytes = [0u8; 4];
        match self.stream.read_exact(&mut len_bytes).await {
            Ok(_) => {
                let len = u32::from_le_bytes(len_bytes) as usize;
                
                // Read message data
                let mut data = vec![0u8; len];
                self.stream.read_exact(&mut data).await
                    .with_context(|| "Failed to read message data")?;
                
                let json = String::from_utf8(data)
                    .with_context(|| "Failed to decode message bytes")?;
                
                let message = serde_json::from_str(&json)
                    .with_context(|| "Failed to deserialize message")?;
                
                Ok(Some(message))
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                self.connected = false;
                Ok(None)
            }
            Err(e) => Err(anyhow::anyhow!("Transport error: {}", e))
        }
    }
    
    async fn close(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }
    
    fn is_connected(&self) -> bool {
        self.connected
    }
}

/// TCP socket transport implementation
pub struct TcpSocketTransport {
    stream: TcpStream,
    connected: bool,
}

impl TcpSocketTransport {
    pub async fn connect(host: String, port: u16) -> Result<Self> {
        let stream = TcpStream::connect(format!("{}:{}", host, port))
            .await
            .with_context(|| format!("Failed to connect to TCP socket: {}:{}", host, port))?;
        
        Ok(Self {
            stream,
            connected: true,
        })
    }
    
    pub async fn new(stream: TcpStream) -> Result<Self> {
        Ok(Self {
            stream,
            connected: true,
        })
    }
}

#[async_trait::async_trait]
impl Transport for TcpSocketTransport {
    async fn send(&mut self, message: MessageEnvelope) -> Result<()> {
        if !self.connected {
            return Err(anyhow::anyhow!("Transport not connected"));
        }
        
        let json = serde_json::to_string(&message)
            .with_context(|| "Failed to serialize message")?;
        
        // Send message with length prefix
        let data = json.as_bytes();
        let len = data.len() as u32;
        self.stream.write_all(&len.to_le_bytes()).await
            .with_context(|| "Failed to write message length")?;
        self.stream.write_all(data).await
            .with_context(|| "Failed to write message data")?;
        
        Ok(())
    }
    
    async fn receive(&mut self) -> Result<Option<MessageEnvelope>> {
        if !self.connected {
            return Ok(None);
        }
        
        // Read message length first
        let mut len_bytes = [0u8; 4];
        match self.stream.read_exact(&mut len_bytes).await {
            Ok(_) => {
                let len = u32::from_le_bytes(len_bytes) as usize;
                
                // Read message data
                let mut data = vec![0u8; len];
                self.stream.read_exact(&mut data).await
                    .with_context(|| "Failed to read message data")?;
                
                let json = String::from_utf8(data)
                    .with_context(|| "Failed to decode message bytes")?;
                
                let message = serde_json::from_str(&json)
                    .with_context(|| "Failed to deserialize message")?;
                
                Ok(Some(message))
            }
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                self.connected = false;
                Ok(None)
            }
            Err(e) => Err(anyhow::anyhow!("Transport error: {}", e))
        }
    }
    
    async fn close(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }
    
    fn is_connected(&self) -> bool {
        self.connected
    }
}

/// In-memory transport for testing
pub struct InMemoryTransport {
    sender: mpsc::UnboundedSender<MessageEnvelope>,
    receiver: mpsc::UnboundedReceiver<MessageEnvelope>,
    connected: bool,
}

impl InMemoryTransport {
    pub fn new_pair() -> (Self, Self) {
        let (tx1, rx1) = mpsc::unbounded_channel();
        let (tx2, rx2) = mpsc::unbounded_channel();
        
        let transport1 = Self {
            sender: tx2,
            receiver: rx1,
            connected: true,
        };
        
        let transport2 = Self {
            sender: tx1,
            receiver: rx2,
            connected: true,
        };
        
        (transport1, transport2)
    }
}

#[async_trait::async_trait]
impl Transport for InMemoryTransport {
    async fn send(&mut self, message: MessageEnvelope) -> Result<()> {
        if !self.connected {
            return Err(anyhow::anyhow!("Transport not connected"));
        }
        
        self.sender
            .send(message)
            .map_err(|_| anyhow::anyhow!("Failed to send message"))?;
        
        Ok(())
    }
    
    async fn receive(&mut self) -> Result<Option<MessageEnvelope>> {
        if !self.connected {
            return Ok(None);
        }
        
        match self.receiver.recv().await {
            Some(message) => Ok(Some(message)),
            None => {
                self.connected = false;
                Ok(None)
            }
        }
    }
    
    async fn close(&mut self) -> Result<()> {
        self.connected = false;
        Ok(())
    }
    
    fn is_connected(&self) -> bool {
        self.connected
    }
}

/// Unix socket listener
pub struct UnixSocketListener {
    listener: UnixListener,
}

impl UnixSocketListener {
    pub async fn bind(path: PathBuf) -> Result<Self> {
        // Remove existing socket file if it exists
        if path.exists() {
            std::fs::remove_file(&path)
                .with_context(|| format!("Failed to remove existing socket: {:?}", path))?;
        }
        
        let listener = UnixListener::bind(&path)
            .with_context(|| format!("Failed to bind Unix socket: {:?}", path))?;
        
        Ok(Self { listener })
    }
}

#[async_trait::async_trait]
impl Listener for UnixSocketListener {
    async fn accept(&mut self) -> Result<Box<dyn Transport>> {
        let (stream, _) = self.listener
            .accept()
            .await
            .with_context(|| "Failed to accept Unix socket connection")?;
        
        Ok(Box::new(UnixSocketTransport {
            stream,
            connected: true,
        }))
    }
    
    async fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// TCP socket listener
pub struct TcpSocketListener {
    listener: TcpListener,
}

impl TcpSocketListener {
    pub async fn bind(host: String, port: u16) -> Result<Self> {
        let listener = TcpListener::bind(format!("{}:{}", host, port))
            .await
            .with_context(|| format!("Failed to bind TCP socket: {}:{}", host, port))?;
        
        Ok(Self { listener })
    }
}

#[async_trait::async_trait]
impl Listener for TcpSocketListener {
    async fn accept(&mut self) -> Result<Box<dyn Transport>> {
        let (stream, _) = self.listener
            .accept()
            .await
            .with_context(|| "Failed to accept TCP socket connection")?;
        
        Ok(Box::new(TcpSocketTransport {
            stream,
            connected: true,
        }))
    }
    
    async fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self {
            listeners: HashMap::new(),
            connections: Arc::new(RwLock::new(HashMap::new())),
            message_handlers: Vec::new(),
        }
    }
    
    pub async fn add_listener(
        &mut self,
        name: String,
        config: TransportConfig,
    ) -> Result<()> {
        let listener: Box<dyn Listener> = match config {
            TransportConfig::UnixSocket { path } => {
                Box::new(UnixSocketListener::bind(path).await?)
            }
            TransportConfig::TcpSocket { host, port } => {
                Box::new(TcpSocketListener::bind(host, port).await?)
            }
            TransportConfig::InMemory { .. } => {
                return Err(anyhow::anyhow!("In-memory transport cannot be used as listener"));
            }
        };
        
        self.listeners.insert(name, listener);
        Ok(())
    }
    
    pub fn add_message_handler(&mut self, handler: Box<dyn MessageHandler>) {
        self.message_handlers.push(handler);
    }
    
    pub async fn start(&mut self) -> Result<()> {
        let listeners = std::mem::take(&mut self.listeners);
        for (name, mut listener) in listeners {
            let connections = Arc::clone(&self.connections);
            let connection_name = name;
            
            tokio::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok(transport) => {
                            let mut connections = connections.write().await;
                            connections.insert(
                                connection_name.clone(),
                                Arc::new(RwLock::new(transport)),
                            );
                        }
                        Err(e) => {
                            eprintln!("Failed to accept connection: {}", e);
                            break;
                        }
                    }
                }
            });
        }
        
        Ok(())
    }
    
    pub async fn broadcast(&self, message: MessageEnvelope) -> Result<()> {
        let connections = self.connections.read().await;
        
        for transport in connections.values() {
            let mut transport = transport.write().await;
            if let Err(e) = transport.send(message.clone()).await {
                eprintln!("Failed to send message: {}", e);
            }
        }
        
        Ok(())
    }
}

/// Auto-discovery for Hub instances
pub struct HubDiscovery;

impl HubDiscovery {
    /// Discover Hub instances using standard locations
    pub async fn discover() -> Result<Vec<TransportConfig>> {
        let mut configs = Vec::new();
        
        // Check standard Unix socket location
        let socket_path = dirs::home_dir()
            .unwrap_or_default()
            .join(".the-hub")
            .join("socket");
        
        if socket_path.exists() {
            configs.push(TransportConfig::UnixSocket { path: socket_path });
        }
        
        // Check environment variables
        if let Ok(socket_path) = std::env::var("HUB_SOCKET") {
            configs.push(TransportConfig::UnixSocket {
                path: PathBuf::from(socket_path),
            });
        }
        
        if let Ok(port_str) = std::env::var("HUB_PORT") {
            if let Ok(port) = port_str.parse::<u16>() {
                configs.push(TransportConfig::TcpSocket {
                    host: "localhost".to_string(),
                    port,
                });
            }
        }
        
        // Always try the default Hub server port
        configs.push(TransportConfig::TcpSocket {
            host: "localhost".to_string(),
            port: 7878,
        });
        
        Ok(configs)
    }
    
    /// Check if The Hub is available
    pub async fn is_hub_available() -> bool {
        if let Ok(configs) = Self::discover().await {
            for config in configs {
                match config {
                    TransportConfig::UnixSocket { path } => {
                        if UnixSocketTransport::connect(path).await.is_ok() {
                            return true;
                        }
                    }
                    TransportConfig::TcpSocket { host, port } => {
                        if TcpSocketTransport::connect(host, port).await.is_ok() {
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }
        
        false
    }
}