//! The Hub: CLI App Platform
//! 
//! A revolutionary platform that bridges the gap between command-line tools 
//! and modern graphical interfaces, creating rich, interactive applications
//! that work both as traditional CLI tools and as rich UI experiences.

pub mod application;

use gpui::{App, Context, EventEmitter, IntoElement, ParentElement, Render};
use anyhow::Result;

pub use hub_core::*;
pub use hub_protocol::*;
pub use hub_blocks::*;
pub use hub_terminal_engine::*;

/// Initialize The Hub platform
pub fn init(cx: &mut App) -> Result<()> {
    hub_core::init(cx);
    hub_protocol::init(cx);
    hub_blocks::init(cx);
    hub_terminal_engine::init(cx)?;
    
    // Initialize AI integration if available
    #[cfg(feature = "ai")]
    hub_ai::init(cx);
    
    Ok(())
}

/// The main Hub application entity
pub struct HubApplication {
    // Core application state will be defined here
}

impl HubApplication {
    pub fn new() -> Result<Self> {
        Ok(Self {})
    }
}

impl EventEmitter<()> for HubApplication {}

impl Render for HubApplication {
    fn render(&mut self, _window: &mut gpui::Window, _cx: &mut Context<Self>) -> impl IntoElement {
        gpui::div().child("The Hub - Coming Soon")
    }
}