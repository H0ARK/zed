//! Protocol integration layer for Zed's terminal system

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use gpui::{Entity, WeakEntity};
use terminal::Terminal;

use crate::enhanced_terminal::{HubTerminal, HubBlock};
use hub_protocol::HubServer;

/// Manager for Hub-enhanced terminals
pub struct HubTerminalManager {
    /// Map of terminal entities to their Hub enhancements
    enhanced_terminals: HashMap<WeakEntity<Terminal>, Arc<HubTerminal>>,
    
    /// Hub server for handling protocol communications
    hub_server: Option<Arc<HubServer>>,
    
    /// Active Hub sessions
    active_sessions: HashMap<String, WeakEntity<Terminal>>,
}

impl HubTerminalManager {
    /// Create a new Hub terminal manager
    pub fn new() -> Self {
        Self {
            enhanced_terminals: HashMap::new(),
            hub_server: None,
            active_sessions: HashMap::new(),
        }
    }
    
    /// Initialize the Hub server
    pub async fn initialize_server(&mut self) -> Result<()> {
        let server = HubServer::new();
        
        // Start server on default port
        let server_arc = Arc::new(server);
        
        // TODO: Start server listening on socket/TCP
        // This would be done in the background
        
        self.hub_server = Some(server_arc);
        log::info!("Hub server initialized");
        
        Ok(())
    }
    
    /// Enhance a terminal with Hub capabilities
    pub fn enhance_terminal(&mut self, terminal: Entity<Terminal>) -> Result<Arc<HubTerminal>> {
        let hub_terminal = Arc::new(HubTerminal::new(terminal.clone()));
        
        // TODO: Initialize Hub connection for this terminal
        // This would be done asynchronously
        
        self.enhanced_terminals.insert(terminal.downgrade(), hub_terminal.clone());
        
        log::info!("Enhanced terminal with Hub capabilities");
        Ok(hub_terminal)
    }
    
    /// Get Hub enhancement for a terminal
    pub fn get_enhancement(&self, terminal: &Entity<Terminal>) -> Option<Arc<HubTerminal>> {
        self.enhanced_terminals.get(&terminal.downgrade()).cloned()
    }
    
    /// Process terminal input through Hub enhancement
    pub fn process_terminal_input(
        &mut self, 
        terminal: &Entity<Terminal>, 
        input: &[u8]
    ) -> Result<Vec<u8>> {
        if let Some(enhancement) = self.get_enhancement(terminal) {
            // Process through Hub enhancement
            if let Ok(mut hub_terminal) = Arc::try_unwrap(enhancement.clone()) {
                hub_terminal.process_input(input)
            } else {
                // Multiple references, use clone
                Ok(input.to_vec())
            }
        } else {
            // No enhancement, pass through
            Ok(input.to_vec())
        }
    }
    
    /// Process terminal output through Hub enhancement
    pub fn process_terminal_output(
        &mut self, 
        terminal: &Entity<Terminal>, 
        output: &[u8]
    ) -> Result<Vec<u8>> {
        if let Some(enhancement) = self.get_enhancement(terminal) {
            // Process through Hub enhancement
            if let Ok(mut hub_terminal) = Arc::try_unwrap(enhancement.clone()) {
                hub_terminal.process_output(output)
            } else {
                // Multiple references, use clone
                Ok(output.to_vec())
            }
        } else {
            // No enhancement, pass through
            Ok(output.to_vec())
        }
    }
    
    /// Get all active Hub blocks across all terminals
    pub fn get_all_active_blocks(&self) -> HashMap<String, (WeakEntity<Terminal>, HubBlock)> {
        let mut all_blocks = HashMap::new();
        
        for (terminal_weak, enhancement) in &self.enhanced_terminals {
            for (block_id, block) in enhancement.active_blocks() {
                all_blocks.insert(block_id.clone(), (terminal_weak.clone(), block.clone()));
            }
        }
        
        all_blocks
    }
    
    /// Clean up dead terminal references
    pub fn cleanup(&mut self) {
        self.enhanced_terminals.retain(|terminal_weak, _| {
            terminal_weak.upgrade().is_some()
        });
        
        self.active_sessions.retain(|_, terminal_weak| {
            terminal_weak.upgrade().is_some()
        });
    }
}

/// Extension trait for Terminals to add Hub capabilities
pub trait HubTerminalExtension {
    /// Enhance this terminal with Hub capabilities
    fn enable_hub(&self, manager: &mut HubTerminalManager) -> Result<Arc<HubTerminal>>;
    
    /// Check if this terminal has Hub enhancement
    fn has_hub_enhancement(&self, manager: &HubTerminalManager) -> bool;
    
    /// Get Hub blocks for this terminal
    fn get_hub_blocks(&self, manager: &HubTerminalManager) -> Option<HashMap<String, HubBlock>>;
}

impl HubTerminalExtension for Entity<Terminal> {
    fn enable_hub(&self, manager: &mut HubTerminalManager) -> Result<Arc<HubTerminal>> {
        manager.enhance_terminal(self.clone())
    }
    
    fn has_hub_enhancement(&self, manager: &HubTerminalManager) -> bool {
        manager.get_enhancement(self).is_some()
    }
    
    fn get_hub_blocks(&self, manager: &HubTerminalManager) -> Option<HashMap<String, HubBlock>> {
        manager.get_enhancement(self)
            .map(|enhancement| enhancement.active_blocks().clone())
    }
}

/// Global Hub terminal manager instance
static HUB_TERMINAL_MANAGER: OnceLock<std::sync::Mutex<HubTerminalManager>> = OnceLock::new();

/// Initialize the global Hub terminal manager
pub fn initialize_hub_terminal_manager() -> Result<()> {
    HUB_TERMINAL_MANAGER.get_or_init(|| {
        std::sync::Mutex::new(HubTerminalManager::new())
    });
    Ok(())
}

/// Get reference to the global Hub terminal manager
pub fn with_hub_terminal_manager<F, R>(f: F) -> Option<R>
where
    F: FnOnce(&mut HubTerminalManager) -> R,
{
    HUB_TERMINAL_MANAGER.get()
        .and_then(|manager| manager.lock().ok())
        .map(|mut manager| f(&mut manager))
}

// Hub terminal creation helper
// Note: This function is temporarily commented out until we resolve the TerminalKind import
// pub fn create_hub_enhanced_terminal(
//     terminals: &mut Terminals,
//     kind: TerminalKind,
//     window: gpui::AnyWindowHandle,
//     cx: &mut Context<Terminals>,
// ) -> Result<Entity<Terminal>> {
//     // Create standard terminal
//     let terminal = terminals.create_terminal_with_venv(kind, None, window, cx)?;
//     
//     // Enhance with Hub capabilities
//     with_hub_terminal_manager(|manager| {
//         let _ = terminal.enable_hub(manager);
//     });
//     
//     Ok(terminal)
// }