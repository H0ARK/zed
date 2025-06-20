//! Enhanced terminal engine for The Hub
//! 
//! This crate extends Zed's existing terminal implementation with additional
//! capabilities needed for The Hub's dual-mode architecture and protocol
//! communication.

pub mod enhanced_terminal;
pub mod protocol_integration;
pub mod block_terminal;

use gpui::App;
use anyhow::Result;

pub use enhanced_terminal::*;
pub use protocol_integration::{HubTerminalManager, HubTerminalExtension, with_hub_terminal_manager, initialize_hub_terminal_manager};
pub use block_terminal::*;

/// Initialize the enhanced terminal engine
pub fn init(cx: &mut App) -> Result<()> {
    // Initialize the global Hub terminal manager
    initialize_hub_terminal_manager()?;
    
    log::info!("Hub terminal engine initialized");
    Ok(())
}