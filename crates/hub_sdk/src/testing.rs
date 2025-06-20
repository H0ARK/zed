//! Testing utilities for Hub-enabled CLI applications
//!
//! This module provides utilities for testing CLI applications that integrate with The Hub.

use anyhow::Result;
use hub_protocol::{InMemoryTransport, HubClient, MessageEnvelope};
use tokio::sync::mpsc;

/// Test client that captures Hub messages for verification
pub struct TestHubClient {
    sent_messages: Vec<MessageEnvelope>,
    transport: Option<InMemoryTransport>,
}

impl TestHubClient {
    /// Create a new test client
    pub fn new() -> (Self, InMemoryTransport) {
        let (transport1, transport2) = InMemoryTransport::new_pair();
        
        (
            Self {
                sent_messages: Vec::new(),
                transport: Some(transport1),
            },
            transport2,
        )
    }
    
    /// Get all messages that were sent to the Hub
    pub fn sent_messages(&self) -> &[MessageEnvelope] {
        &self.sent_messages
    }
    
    /// Check if a specific message type was sent
    pub fn has_message_type(&self, message_type: &hub_protocol::messages::MessageType) -> bool {
        self.sent_messages
            .iter()
            .any(|msg| &msg.message_type == message_type)
    }
    
    /// Count messages of a specific type
    pub fn count_message_type(&self, message_type: &hub_protocol::messages::MessageType) -> usize {
        self.sent_messages
            .iter()
            .filter(|msg| &msg.message_type == message_type)
            .count()
    }
    
    /// Clear captured messages
    pub fn clear_messages(&mut self) {
        self.sent_messages.clear();
    }
}

impl Default for TestHubClient {
    fn default() -> Self {
        let (client, _) = Self::new();
        client
    }
}

/// Test environment for Hub CLI applications
pub struct TestEnvironment {
    pub client: TestHubClient,
    pub transport: InMemoryTransport,
}

impl TestEnvironment {
    /// Create a new test environment
    pub fn new() -> Self {
        let (client, transport) = TestHubClient::new();
        Self { client, transport }
    }
    
    /// Set Hub environment variables
    pub fn enable_hub_mode(&self) {
        unsafe {
            std::env::set_var("HUB_MODE", "1");
        }
    }
    
    /// Clear Hub environment variables
    pub fn disable_hub_mode(&self) {
        unsafe {
            std::env::remove_var("HUB_MODE");
            std::env::remove_var("HUB_SOCKET");
            std::env::remove_var("HUB_PORT");
        }
    }
}

impl Default for TestEnvironment {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper macro for testing Hub integration
#[macro_export]
macro_rules! test_hub_integration {
    ($test_name:ident, $test_body:block) => {
        #[tokio::test]
        async fn $test_name() {
            let test_env = TestEnvironment::new();
            test_env.enable_hub_mode();
            
            $test_body
            
            test_env.disable_hub_mode();
        }
    };
}

pub use test_hub_integration;