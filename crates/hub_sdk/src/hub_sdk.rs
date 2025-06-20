//! SDK and developer tools for building CLI applications with The Hub
//! 
//! This crate provides the tools, libraries, and utilities that CLI developers
//! need to add rich UI capabilities to their applications.

pub mod client_builder;
pub mod ui_components;
pub mod macros;
pub mod testing;

use anyhow::Result;

pub use client_builder::*;
pub use ui_components::*;
pub use macros::*;
pub use testing::*;

/// Initialize the SDK
pub fn init() -> Result<()> {
    // SDK initialization will be implemented here
    Ok(())
}

/// Check if running in Hub mode vs traditional CLI mode
pub fn is_hub_mode() -> bool {
    std::env::var("HUB_MODE").is_ok()
}

/// Get the Hub protocol port from environment
pub fn get_hub_port() -> u16 {
    std::env::var("HUB_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(hub_protocol::DEFAULT_PROTOCOL_PORT)
}