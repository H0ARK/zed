//! Core types and functionality for The Hub
//! 
//! This crate provides the fundamental types, traits, and utilities
//! that form the foundation of The Hub platform.

pub mod types;
pub mod commands;
pub mod blocks;
pub mod settings;

use gpui::App;

pub use types::*;
pub use commands::*;
pub use blocks::*;
pub use settings::*;

/// Initialize the core Hub functionality
pub fn init(_cx: &mut App) {
    // Core initialization will be implemented here
    log::info!("Hub core initialized");
}

/// Core Hub platform version
pub const HUB_VERSION: &str = "0.1.0";

/// Hub protocol version 
pub const PROTOCOL_VERSION: &str = "1.0.0";

/// Check if we're running in Hub mode
pub fn is_hub_mode() -> bool {
    std::env::var("HUB_MODE").is_ok() || std::env::var("HUB_SOCKET").is_ok() || std::env::var("HUB_PORT").is_ok()
}

/// Get Hub configuration from environment
pub fn get_hub_config() -> HubEnvironmentConfig {
    HubEnvironmentConfig {
        socket_path: std::env::var("HUB_SOCKET").ok(),
        port: std::env::var("HUB_PORT")
            .ok()
            .and_then(|s| s.parse().ok()),
        session_id: std::env::var("HUB_SESSION").ok(),
        debug_mode: std::env::var("HUB_DEBUG").is_ok(),
    }
}

/// Hub environment configuration
#[derive(Debug, Clone)]
pub struct HubEnvironmentConfig {
    pub socket_path: Option<String>,
    pub port: Option<u16>,
    pub session_id: Option<String>,
    pub debug_mode: bool,
}