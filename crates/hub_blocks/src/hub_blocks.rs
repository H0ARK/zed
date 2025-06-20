//! Block-based UI system for The Hub
//! 
//! This crate implements the rich, interactive block system that allows
//! CLI commands to display structured, interactive output within The Hub.

pub mod block;
pub mod block_manager;
pub mod renderers;
pub mod interactions;

use gpui::App;
use anyhow::Result;

pub use block::*;
pub use block_manager::*;
pub use renderers::*;
pub use interactions::*;

/// Initialize the block system
pub fn init(_cx: &mut App) {
    // Block system initialization will be implemented here
}