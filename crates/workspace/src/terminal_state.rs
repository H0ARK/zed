//! Terminal state management with JSON serialization
//! 
//! Simple JSON structure representing terminal commands and output for live streaming updates

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalState {
    pub terminal_id: u32,
    pub working_directory: Option<String>,
    pub status: TerminalStatus,
    pub commands: Vec<Command>,
    pub last_updated: String, // ISO timestamp
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalStatus {
    Initializing,
    Ready,
    Busy,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub command: String,
    pub status: CommandStatus,
    pub output: Vec<String>,
    pub exit_code: Option<i32>,
    pub started_at: String, // ISO timestamp
    pub completed_at: Option<String>, // ISO timestamp
    pub duration_ms: Option<u64>,
    pub metadata: Option<CommandMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CommandStatus {
    Running,
    Success,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandMetadata {
    pub prompt: Option<String>,
    pub shell: Option<String>,
    pub environment: Option<std::collections::HashMap<String, String>>,
}

impl TerminalState {
    pub fn new(terminal_id: u32) -> Self {
        Self {
            terminal_id,
            working_directory: None,
            status: TerminalStatus::Initializing,
            commands: Vec::new(),
            last_updated: Self::current_timestamp(),
        }
    }
    
    pub fn add_command(&mut self, command: String) -> String {
        let command_id = format!("cmd_{}_{}", self.terminal_id, self.commands.len() + 1);
        
        let cmd = Command {
            id: command_id.clone(),
            command,
            status: CommandStatus::Running,
            output: Vec::new(),
            exit_code: None,
            started_at: Self::current_timestamp(),
            completed_at: None,
            duration_ms: None,
            metadata: None,
        };
        
        self.commands.push(cmd);
        self.status = TerminalStatus::Busy;
        self.update_timestamp();
        
        command_id
    }
    
    pub fn update_command_output(&mut self, command_id: &str, new_output: Vec<String>) {
        if let Some(cmd) = self.commands.iter_mut().find(|c| c.id == command_id) {
            cmd.output.extend(new_output);
            self.update_timestamp();
        }
    }
    
    pub fn complete_command(&mut self, command_id: &str, exit_code: i32) {
        if let Some(cmd) = self.commands.iter_mut().find(|c| c.id == command_id) {
            cmd.status = if exit_code == 0 { CommandStatus::Success } else { CommandStatus::Error };
            cmd.exit_code = Some(exit_code);
            cmd.completed_at = Some(Self::current_timestamp());
            
            // Calculate duration (simplified - just using current time)
            if let (Ok(started), Ok(completed)) = (
                cmd.started_at.parse::<u64>(),
                cmd.completed_at.as_ref().unwrap().parse::<u64>()
            ) {
                cmd.duration_ms = Some((completed - started) * 1000); // Convert to milliseconds
            }
            
            self.update_timestamp();
        }
        
        // Update terminal status
        if !self.commands.iter().any(|c| matches!(c.status, CommandStatus::Running)) {
            self.status = TerminalStatus::Ready;
        }
    }
    
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
    
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
    
    fn current_timestamp() -> String {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string()
    }
    
    fn update_timestamp(&mut self) {
        self.last_updated = Self::current_timestamp();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_terminal_state_creation() {
        let state = TerminalState::new(1);
        assert_eq!(state.terminal_id, 1);
        assert!(matches!(state.status, TerminalStatus::Initializing));
        assert!(state.commands.is_empty());
    }
    
    #[test]
    fn test_add_command() {
        let mut state = TerminalState::new(1);
        let cmd_id = state.add_command("ls -la".to_string());
        
        assert_eq!(state.commands.len(), 1);
        assert_eq!(state.commands[0].command, "ls -la");
        assert!(matches!(state.commands[0].status, CommandStatus::Running));
        assert!(matches!(state.status, TerminalStatus::Busy));
        assert_eq!(cmd_id, "cmd_1_1");
    }
    
    #[test]
    fn test_json_serialization() {
        let mut state = TerminalState::new(1);
        state.add_command("echo hello".to_string());
        
        let json = state.to_json().unwrap();
        let deserialized = TerminalState::from_json(&json).unwrap();
        
        assert_eq!(state.terminal_id, deserialized.terminal_id);
        assert_eq!(state.commands.len(), deserialized.commands.len());
    }
}