//! Client builder for easy Hub integration
//!
//! This module provides utilities for CLI applications to easily connect
//! to The Hub and send rich UI components.

use anyhow::Result;
use hub_protocol::{HubClient, HubClientBuilder, TransportConfig};

/// Builder for creating Hub clients with sensible defaults
pub struct SimpleHubClientBuilder {
    auto_discover: bool,
    transport_config: Option<TransportConfig>,
}

impl Default for SimpleHubClientBuilder {
    fn default() -> Self {
        Self {
            auto_discover: true,
            transport_config: None,
        }
    }
}

impl SimpleHubClientBuilder {
    /// Create a new client builder
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Enable or disable auto-discovery of Hub instances
    pub fn auto_discover(mut self, enabled: bool) -> Self {
        self.auto_discover = enabled;
        self
    }
    
    /// Set a specific transport configuration
    pub fn transport(mut self, config: TransportConfig) -> Self {
        self.transport_config = Some(config);
        self
    }
    
    /// Build the Hub client
    pub async fn build(self) -> Result<Option<HubClient>> {
        let mut builder = HubClientBuilder::default();
        
        if let Some(config) = self.transport_config {
            builder = builder.transport(config);
        }
        
        // Try to connect, but return None if no Hub is available instead of erroring
        match builder.connect().await {
            Ok(client) => Ok(Some(client)),
            Err(_) if self.auto_discover => Ok(None), // Hub not available, that's ok
            Err(e) => Err(e), // Actual error occurred
        }
    }
}

/// Quick helper to create a Hub client if one is available
pub async fn create_hub_client() -> Result<Option<HubClient>> {
    SimpleHubClientBuilder::new().build().await
}