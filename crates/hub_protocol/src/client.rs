//! Hub protocol client implementation

use crate::messages::*;
use crate::transport::*;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};

/// Hub protocol client
pub struct HubClient {
    transport: Option<Box<dyn Transport>>,
    session_id: String,
    sequence_counter: u64,
    message_sender: mpsc::UnboundedSender<MessageEnvelope>,
    response_receiver: Arc<RwLock<mpsc::UnboundedReceiver<MessageEnvelope>>>,
    capabilities: SessionCapabilities,
}

/// Client builder for easy configuration
pub struct HubClientBuilder {
    transport_config: Option<TransportConfig>,
    capabilities: SessionCapabilities,
    auto_discover: bool,
}

impl Default for HubClientBuilder {
    fn default() -> Self {
        Self {
            transport_config: None,
            capabilities: SessionCapabilities {
                ui_components: vec![
                    "progress".to_string(),
                    "table".to_string(),
                    "file_tree".to_string(),
                    "form".to_string(),
                    "status_grid".to_string(),
                ],
                interactions: vec![
                    "click".to_string(),
                    "select".to_string(),
                    "input".to_string(),
                ],
                ai_integration: true,
            },
            auto_discover: true,
        }
    }
}

impl HubClientBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    
    pub fn transport(mut self, config: TransportConfig) -> Self {
        self.transport_config = Some(config);
        self.auto_discover = false;
        self
    }
    
    pub fn capabilities(mut self, capabilities: SessionCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
    
    pub fn with_ui_component(mut self, component: &str) -> Self {
        if !self.capabilities.ui_components.contains(&component.to_string()) {
            self.capabilities.ui_components.push(component.to_string());
        }
        self
    }
    
    pub fn with_ai_integration(mut self, enabled: bool) -> Self {
        self.capabilities.ai_integration = enabled;
        self
    }
    
    pub async fn connect(self) -> Result<HubClient> {
        let transport_config = if let Some(config) = self.transport_config {
            config
        } else if self.auto_discover {
            HubDiscovery::discover()
                .await?
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("No Hub instance found"))?
        } else {
            return Err(anyhow::anyhow!("No transport configuration provided"));
        };
        
        let transport = match transport_config {
            TransportConfig::UnixSocket { path } => {
                Box::new(UnixSocketTransport::connect(path).await?) as Box<dyn Transport>
            }
            TransportConfig::TcpSocket { host, port } => {
                Box::new(TcpSocketTransport::connect(host, port).await?) as Box<dyn Transport>
            }
            TransportConfig::InMemory { .. } => {
                return Err(anyhow::anyhow!("In-memory transport not supported for client"));
            }
        };
        
        let (message_sender, message_receiver) = mpsc::unbounded_channel();
        let (response_sender, response_receiver) = mpsc::unbounded_channel();
        
        let mut client = HubClient {
            transport: Some(transport),
            session_id: uuid::Uuid::new_v4().to_string(),
            sequence_counter: 0,
            message_sender,
            response_receiver: Arc::new(RwLock::new(response_receiver)),
            capabilities: self.capabilities,
        };
        
        // Start message processing loop
        client.start_message_loop(message_receiver, response_sender).await?;
        
        Ok(client)
    }
}

impl HubClient {
    pub fn builder() -> HubClientBuilder {
        HubClientBuilder::new()
    }
    
    pub async fn try_connect() -> Result<Option<HubClient>> {
        if HubDiscovery::is_hub_available().await {
            Ok(Some(HubClient::builder().connect().await?))
        } else {
            Ok(None)
        }
    }
    
    pub fn is_connected(&self) -> bool {
        self.transport
            .as_ref()
            .map(|t| t.is_connected())
            .unwrap_or(false)
    }
    
    pub async fn start_session(
        &mut self,
        command: String,
        args: Vec<String>,
        cwd: String,
    ) -> Result<()> {
        let message = MessageEnvelope::session_start(
            self.session_id.clone(),
            self.next_sequence(),
            command,
            args,
            cwd,
            self.capabilities.clone(),
        );
        
        self.send_message(message).await
    }
    
    pub async fn end_session(
        &mut self,
        exit_code: i32,
        duration_ms: u64,
        summary: Option<String>,
    ) -> Result<()> {
        let message = MessageEnvelope::session_end(
            self.session_id.clone(),
            self.next_sequence(),
            exit_code,
            duration_ms,
            summary,
        );
        
        self.send_message(message).await
    }
    
    pub async fn show_progress(
        &mut self,
        current: u64,
        total: u64,
        message: String,
    ) -> Result<()> {
        let message = MessageEnvelope::progress(
            self.session_id.clone(),
            self.next_sequence(),
            current,
            total,
            message,
        );
        
        self.send_message(message).await
    }
    
    pub async fn show_table(
        &mut self,
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    ) -> Result<()> {
        let table_headers: Vec<TableHeader> = headers
            .into_iter()
            .map(|text| TableHeader {
                text,
                width: "flex".to_string(),
            })
            .collect();
        
        let table_rows: Vec<TableRow> = rows
            .into_iter()
            .enumerate()
            .map(|(i, cells)| TableRow {
                id: format!("row-{}", i),
                cells,
                actions: vec![],
                status: None,
            })
            .collect();
        
        let message = MessageEnvelope::new(
            MessageType::UiMessage,
            self.session_id.clone(),
            self.next_sequence(),
            MessagePayload::UiMessage(UiMessagePayload::Table(TableComponent {
                props: TableProps {
                    headers: table_headers,
                    rows: table_rows,
                    sortable: true,
                    filterable: true,
                    selectable: SelectionMode::Multiple,
                },
            })),
        );
        
        self.send_message(message).await
    }
    
    pub async fn show_file_tree(
        &mut self,
        root: String,
        entries: Vec<FileEntry>,
    ) -> Result<()> {
        let message = MessageEnvelope::new(
            MessageType::UiMessage,
            self.session_id.clone(),
            self.next_sequence(),
            MessagePayload::UiMessage(UiMessagePayload::FileTree(FileTreeComponent {
                props: FileTreeProps {
                    root,
                    entries,
                    show_hidden: false,
                    icons: true,
                },
            })),
        );
        
        self.send_message(message).await
    }
    
    pub async fn show_form(&mut self, form: FormComponent) -> Result<()> {
        let message = MessageEnvelope::new(
            MessageType::UiMessage,
            self.session_id.clone(),
            self.next_sequence(),
            MessagePayload::UiMessage(UiMessagePayload::Form(form)),
        );
        
        self.send_message(message).await
    }
    
    pub async fn show_status_grid(&mut self, cards: Vec<StatusCard>) -> Result<()> {
        let message = MessageEnvelope::new(
            MessageType::UiMessage,
            self.session_id.clone(),
            self.next_sequence(),
            MessagePayload::UiMessage(UiMessagePayload::StatusGrid(StatusGridComponent {
                props: StatusGridProps { cards },
            })),
        );
        
        self.send_message(message).await
    }
    
    pub async fn send_stream_data(
        &mut self,
        stream_id: String,
        data: StreamDataType,
    ) -> Result<()> {
        let message = MessageEnvelope::new(
            MessageType::UiMessage,
            self.session_id.clone(),
            self.next_sequence(),
            MessagePayload::UiMessage(UiMessagePayload::Stream(StreamData {
                stream_id,
                data,
            })),
        );
        
        self.send_message(message).await
    }
    
    pub async fn update_component(
        &mut self,
        target: String,
        props: serde_json::Value,
    ) -> Result<()> {
        let message = MessageEnvelope::new(
            MessageType::UiMessage,
            self.session_id.clone(),
            self.next_sequence(),
            MessagePayload::UiMessage(UiMessagePayload::Update(ComponentUpdate {
                target,
                props,
            })),
        );
        
        self.send_message(message).await
    }
    
    pub async fn send_error(
        &mut self,
        error_code: String,
        message: String,
        details: Option<serde_json::Value>,
    ) -> Result<()> {
        let message = MessageEnvelope::error(
            self.session_id.clone(),
            self.next_sequence(),
            error_code,
            message,
            details,
        );
        
        self.send_message(message).await
    }
    
    pub async fn wait_for_response(&self) -> Result<Option<MessageEnvelope>> {
        let mut receiver = self.response_receiver.write().await;
        Ok(receiver.recv().await)
    }
    
    pub async fn close(&mut self) -> Result<()> {
        if let Some(mut transport) = self.transport.take() {
            transport.close().await?;
        }
        Ok(())
    }
    
    async fn send_message(&self, message: MessageEnvelope) -> Result<()> {
        self.message_sender
            .send(message)
            .map_err(|_| anyhow::anyhow!("Failed to send message to transport"))?;
        
        Ok(())
    }
    
    fn next_sequence(&mut self) -> u64 {
        self.sequence_counter += 1;
        self.sequence_counter
    }
    
    async fn start_message_loop(
        &mut self,
        mut message_receiver: mpsc::UnboundedReceiver<MessageEnvelope>,
        response_sender: mpsc::UnboundedSender<MessageEnvelope>,
    ) -> Result<()> {
        let transport = self.transport.take()
            .ok_or_else(|| anyhow::anyhow!("No transport available"))?;
        
        // Outbound message loop
        let outbound_transport = Arc::new(RwLock::new(transport));
        let outbound_transport_clone = Arc::clone(&outbound_transport);
        
        tokio::spawn(async move {
            while let Some(message) = message_receiver.recv().await {
                let mut transport = outbound_transport_clone.write().await;
                if let Err(e) = transport.send(message).await {
                    eprintln!("Failed to send message: {}", e);
                    break;
                }
            }
        });
        
        // Inbound message loop
        tokio::spawn(async move {
            loop {
                let message_result = {
                    let mut transport = outbound_transport.write().await;
                    transport.receive().await
                };
                
                match message_result {
                    Ok(Some(message)) => {
                        if response_sender.send(message).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break, // Connection closed
                    Err(e) => {
                        eprintln!("Failed to receive message: {}", e);
                        break;
                    }
                }
            }
        });
        
        Ok(())
    }
}

/// High-level client interface for common operations
pub struct HubHelper {
    client: HubClient,
}

impl HubHelper {
    pub async fn new() -> Result<Option<Self>> {
        if let Some(client) = HubClient::try_connect().await? {
            Ok(Some(Self { client }))
        } else {
            Ok(None)
        }
    }
    
    pub fn is_available(&self) -> bool {
        self.client.is_connected()
    }
    
    pub async fn with_session<F, R>(
        mut self,
        command: String,
        args: Vec<String>,
        cwd: String,
        f: F,
    ) -> Result<R>
    where
        F: FnOnce(&mut HubClient) -> futures::future::BoxFuture<'_, Result<R>>,
    {
        let start_time = std::time::Instant::now();
        
        self.client.start_session(command, args, cwd).await?;
        
        let result = f(&mut self.client).await;
        
        let duration = start_time.elapsed();
        let exit_code = if result.is_ok() { 0 } else { 1 };
        
        self.client
            .end_session(exit_code, duration.as_millis() as u64, None)
            .await?;
        
        result
    }
    
    pub async fn show_simple_progress<F, R>(
        &mut self,
        total_items: u64,
        message_prefix: String,
        mut f: F,
    ) -> Result<R>
    where
        F: FnMut(u64, &mut dyn FnMut(String) -> Result<()>) -> Result<R>,
    {
        let mut update_progress = |current: u64, message: String| -> Result<()> {
            futures::executor::block_on(async {
                self.client.show_progress(current, total_items, message).await
            })
        };
        
        f(total_items, &mut |msg| update_progress(0, format!("{}: {}", message_prefix, msg)))
    }
    
    pub async fn show_command_output_table(
        &mut self,
        headers: Vec<String>,
        data: Vec<HashMap<String, String>>,
    ) -> Result<()> {
        let rows: Vec<Vec<String>> = data
            .into_iter()
            .map(|row| {
                headers
                    .iter()
                    .map(|header| row.get(header).unwrap_or(&String::new()).clone())
                    .collect()
            })
            .collect();
        
        self.client.show_table(headers, rows).await
    }
}