//! Protocol layer for CLI-UI communication in The Hub
//! 
//! This crate implements the standardized protocol that allows CLI tools
//! to communicate rich UI components and interactions to The Hub.

pub mod messages;
pub mod transport;
pub mod client;
pub mod server;

use gpui::App;
use anyhow::Result;

pub use messages::*;
pub use transport::*;
pub use client::*;
pub use server::*;

/// Initialize the protocol layer
pub fn init(_cx: &mut App) {
    // Protocol initialization will be implemented here
}

/// Default protocol port for local communication
pub const DEFAULT_PROTOCOL_PORT: u16 = 8765;

/// Protocol magic bytes for message identification
pub const PROTOCOL_MAGIC: &[u8] = b"HUB1";