//! Command definitions and management

use crate::types::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Command definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub args: Vec<CommandArg>,
    pub supports_hub_mode: bool,
    pub version: String,
}

/// Command argument definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandArg {
    pub name: String,
    pub description: String,
    pub arg_type: ArgType,
    pub required: bool,
    pub default_value: Option<String>,
}

/// Argument types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ArgType {
    String,
    Integer,
    Float,
    Boolean,
    Path,
    Choice { options: Vec<String> },
}

/// Command execution context
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub working_directory: std::path::PathBuf,
    pub environment: HashMap<String, String>,
    pub user_id: Option<String>,
    pub session_id: Option<SessionId>,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            working_directory: std::env::current_dir().unwrap_or_default(),
            environment: std::env::vars().collect(),
            user_id: None,
            session_id: None,
        }
    }
}

/// Command registry for managing available commands
pub struct CommandRegistry {
    commands: HashMap<String, Command>,
}

impl CommandRegistry {
    pub fn new() -> Self {
        Self {
            commands: HashMap::new(),
        }
    }
    
    pub fn register(&mut self, command: Command) {
        self.commands.insert(command.name.clone(), command);
    }
    
    pub fn get(&self, name: &str) -> Option<&Command> {
        self.commands.get(name)
    }
    
    pub fn list(&self) -> Vec<&Command> {
        self.commands.values().collect()
    }
    
    pub fn list_hub_compatible(&self) -> Vec<&Command> {
        self.commands
            .values()
            .filter(|cmd| cmd.supports_hub_mode)
            .collect()
    }
    
    pub fn search(&self, query: &str) -> Vec<&Command> {
        let query = query.to_lowercase();
        self.commands
            .values()
            .filter(|cmd| {
                cmd.name.to_lowercase().contains(&query)
                    || cmd.description.to_lowercase().contains(&query)
            })
            .collect()
    }
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Command result and status
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration: std::time::Duration,
    pub blocks: Vec<BlockId>,
}

impl CommandResult {
    pub fn success(stdout: String, duration: std::time::Duration) -> Self {
        Self {
            exit_code: 0,
            stdout,
            stderr: String::new(),
            duration,
            blocks: Vec::new(),
        }
    }
    
    pub fn failure(exit_code: i32, stderr: String, duration: std::time::Duration) -> Self {
        Self {
            exit_code,
            stdout: String::new(),
            stderr,
            duration,
            blocks: Vec::new(),
        }
    }
    
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }
    
    pub fn with_blocks(mut self, blocks: Vec<BlockId>) -> Self {
        self.blocks = blocks;
        self
    }
}