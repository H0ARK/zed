//! Terminal Semantic Parser - The heart of grid-based command detection
//!
//! This module implements semantic parsing of terminal grid content to detect:
//! - Command execution (when user presses Enter)
//! - Command boundaries (start/end of output)
//! - Command results and exit codes
//! - Real-time output streaming
//!
//! The parser works by reading the Alacritty terminal grid directly, making it
//! compatible with ANY CLI tool without requiring protocol modifications.

use crate::terminal_state::TerminalState;
use terminal::{Terminal, TerminalContent, alacritty_terminal::term::cell::Cell};
use gpui::{Context, Entity};
use std::collections::HashMap;
use anyhow::Result;

/// Core semantic parser that reads terminal grid and extracts command semantics
pub struct TerminalSemanticParser {
    /// Current terminal state as JSON
    state: TerminalState,
    
    /// Last grid content hash to detect changes
    last_grid_hash: u64,
    
    /// Pending commands that are currently executing
    pending_commands: HashMap<String, PendingCommand>,
    
    /// Command counter for generating unique IDs
    command_counter: usize,
}

/// A command that has been detected but not yet completed
#[derive(Debug, Clone)]
pub struct PendingCommand {
    pub id: String,
    pub command: String,
    pub started_at: std::time::Instant,
    pub output_start_line: Option<usize>,
}

/// Represents a detected command from the grid
#[derive(Debug, Clone)]
pub struct DetectedCommand {
    pub command: String,
    pub output: Vec<String>,
    pub line_start: usize,
    pub line_end: usize,
    pub completed: bool,
}

/// Changes detected in the terminal grid
#[derive(Debug, Clone)]
pub struct GridChanges {
    pub new_commands: Vec<DetectedCommand>,
    pub updated_outputs: Vec<(String, Vec<String>)>, // (command_id, new_output_lines)
    pub completed_commands: Vec<String>, // command_ids
}

impl TerminalSemanticParser {
    /// Create a new semantic parser for a terminal
    pub fn new(terminal_id: u32) -> Self {
        Self {
            state: TerminalState::new(terminal_id),
            last_grid_hash: 0,
            pending_commands: HashMap::new(),
            command_counter: 0,
        }
    }
    
    /// Process terminal grid changes and update JSON state
    pub fn process_terminal_update<T>(&mut self, terminal: &Entity<Terminal>, cx: &Context<T>) -> Option<String> {
        // Wrap entire processing in error handling
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // Read current terminal content
            let terminal_data = terminal.read(cx);
            let content = terminal_data.last_content();
            
            // Extract text lines from terminal grid
            let grid_lines = self.extract_grid_lines(&content);
            
            // Early return if no content
            if grid_lines.is_empty() || grid_lines.iter().all(|line| line.trim().is_empty()) {
                return None;
            }
            
            // Calculate content hash to detect changes
            let current_hash = self.calculate_hash(&grid_lines);
            if current_hash == self.last_grid_hash {
                return None; // No changes
            }
            
            self.last_grid_hash = current_hash;
            
            // Detect semantic changes (commands, outputs)
            let changes = self.detect_grid_changes(&grid_lines);
            
            // Only proceed if there are actual changes
            if changes.new_commands.is_empty() && changes.updated_outputs.is_empty() && changes.completed_commands.is_empty() {
                return None;
            }
            
            // Update JSON state based on detected changes
            self.apply_changes(changes);
            
            // Return updated JSON state
            self.state.to_json().ok()
        }));
        
        match result {
            Ok(json_result) => json_result,
            Err(panic_info) => {
                eprintln!("Semantic parser process_terminal_update panicked: {:?}", panic_info);
                
                // Try to create a minimal error state JSON
                match serde_json::to_string(&serde_json::json!({
                    "terminal_id": self.state.terminal_id,
                    "status": "error", 
                    "commands": [],
                    "error": "Parser encountered a critical error during grid processing"
                })) {
                    Ok(error_json) => Some(error_json),
                    Err(_) => None
                }
            }
        }
    }
    
    /// Extract text lines from terminal grid content
    fn extract_grid_lines(&self, content: &TerminalContent) -> Vec<String> {
        let mut lines = Vec::new();
        
        // Group cells by line number
        let mut current_line = Vec::new();
        let mut current_line_num = None;
        
        for indexed_cell in &content.cells {
            let line_num = indexed_cell.point.line.0;
            
            // Start new line if line number changed
            if current_line_num != Some(line_num) {
                if !current_line.is_empty() {
                    lines.push(self.cells_to_string(&current_line));
                    current_line.clear();
                }
                current_line_num = Some(line_num);
            }
            
            current_line.push(&indexed_cell.cell);
        }
        
        // Add final line
        if !current_line.is_empty() {
            lines.push(self.cells_to_string(&current_line));
        }
        
        lines
    }
    
    /// Convert terminal cells to string, handling spaces and formatting
    fn cells_to_string(&self, cells: &[&Cell]) -> String {
        let mut line = String::new();
        
        for cell in cells {
            line.push(cell.c);
        }
        
        // Remove trailing spaces but preserve internal structure
        line.trim_end().to_string()
    }
    
    /// Calculate hash of grid content for change detection
    fn calculate_hash(&self, lines: &[String]) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        for line in lines {
            line.hash(&mut hasher);
        }
        hasher.finish()
    }
    
    /// Detect semantic changes in grid content
    fn detect_grid_changes(&mut self, grid_lines: &[String]) -> GridChanges {
        let mut changes = GridChanges {
            new_commands: Vec::new(),
            updated_outputs: Vec::new(),
            completed_commands: Vec::new(),
        };
        
        // First pass: Find all command prompt lines
        let mut command_lines = Vec::new();
        for (i, line) in grid_lines.iter().enumerate() {
            if let Some((command, prompt)) = self.extract_command_from_line(line) {
                command_lines.push((i, command, prompt));
            }
        }
        
        // Second pass: For each command, determine its output and completion status
        for (cmd_idx, (line_idx, command, _prompt)) in command_lines.iter().enumerate() {
            // Collect output from the line after command until next prompt or end
            let (output, end_line) = self.collect_command_output(&grid_lines[line_idx + 1..]);
            
            // Command is completed if there's a next command after this one
            let next_prompt_exists = cmd_idx + 1 < command_lines.len();
            let command_completed = next_prompt_exists || 
                // Or if we found output and there's a prompt after it
                (end_line > 0 && line_idx + 1 + end_line < grid_lines.len() && 
                 self.is_shell_prompt(&grid_lines[line_idx + 1 + end_line]));
            
            let detected_command = DetectedCommand {
                command: command.clone(),
                output,
                line_start: *line_idx,
                line_end: line_idx + 1 + end_line,
                completed: command_completed,
            };
            
            changes.new_commands.push(detected_command);
        }
        
        // TODO: Check for updates to existing pending commands
        // TODO: Detect command completion based on new prompts
        
        changes
    }
    
    /// Extract command from a line that contains a shell prompt
    fn extract_command_from_line(&self, line: &str) -> Option<(String, String)> {
        // Common shell prompt patterns
        let patterns = [
            r"(.+[$❯>#]\s+)(.+)",  // "user@host:~/path$ command args"
            r"(\d+\s+[❯>]\s+)(.+)", // "1 ❯ command args" (fish with line numbers)
            r"([❯>]\s+)(.+)",       // "❯ command args" (minimal prompts)
            r"(.+:\s+)(.+)",        // "directory: command args"
        ];
        
        for pattern in &patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if let Some(caps) = re.captures(line) {
                    if let (Some(prompt), Some(command)) = (caps.get(1), caps.get(2)) {
                        let prompt_text = prompt.as_str().to_string();
                        let command_text = command.as_str().trim().to_string();
                        
                        // Skip empty commands
                        if !command_text.is_empty() {
                            return Some((command_text, prompt_text));
                        }
                    }
                }
            }
        }
        
        None
    }
    
    /// Collect command output lines until the next prompt appears
    fn collect_command_output(&self, lines: &[String]) -> (Vec<String>, usize) {
        let mut output = Vec::new();
        let mut end_line = 0;
        
        for (i, line) in lines.iter().enumerate() {
            // Stop if we hit another shell prompt
            if self.is_shell_prompt(line) {
                break;
            }
            
            // Skip empty lines at start of output
            if output.is_empty() && line.trim().is_empty() {
                continue;
            }
            
            output.push(line.clone());
            end_line = i + 1;
        }
        
        (output, end_line)
    }
    
    /// Check if a line looks like a shell prompt (with or without command)
    fn is_shell_prompt(&self, line: &str) -> bool {
        // Look for common shell prompt indicators (including prompts with commands)
        let prompt_indicators = [
            r".+@.+:.+[$❯>#]\s*",    // user@host:path$ [command]
            r"\d+\s+[❯>]\s*",        // 1 ❯ [command] (fish line numbers)
            r"^[❯>]\s*",             // ❯ [command] (minimal prompt)
            r".+:\s*$",              // directory: (with or without space at end)
            r".+:\s+",               // directory: [command] (with space and command)
        ];
        
        for pattern in &prompt_indicators {
            if let Ok(re) = regex::Regex::new(pattern) {
                if re.is_match(line) {
                    return true;
                }
            }
        }
        
        false
    }
    
    /// Apply detected changes to the internal JSON state
    fn apply_changes(&mut self, changes: GridChanges) {
        println!("Applying {} new commands, {} updated outputs, {} completed commands",
                 changes.new_commands.len(), changes.updated_outputs.len(), changes.completed_commands.len());
        
        for detected_command in changes.new_commands {
            self.command_counter += 1;
            let command_id = self.state.add_command(detected_command.command.clone());
            
            println!("Added command '{}' with ID '{}'", detected_command.command, command_id);
            
            // Add output if available
            if !detected_command.output.is_empty() {
                self.state.update_command_output(&command_id, detected_command.output.clone());
                println!("Added {} output lines to command '{}'", detected_command.output.len(), command_id);
            }
            
            // Mark as completed if detection indicates completion
            if detected_command.completed {
                self.state.complete_command(&command_id, 0); // TODO: Detect actual exit code
                println!("Marked command '{}' as completed", command_id);
            } else {
                // Track as pending for future updates
                println!("Tracking command '{}' as pending", command_id);
                self.pending_commands.insert(command_id.clone(), PendingCommand {
                    id: command_id,
                    command: detected_command.command,
                    started_at: std::time::Instant::now(),
                    output_start_line: Some(detected_command.line_start),
                });
            }
        }
        
        // Apply output updates to existing commands
        for (command_id, new_output) in changes.updated_outputs {
            self.state.update_command_output(&command_id, new_output.clone());
            println!("Updated output for command '{}' with {} new lines", command_id, new_output.len());
        }
        
        // Complete previously pending commands
        for command_id in changes.completed_commands {
            self.state.complete_command(&command_id, 0); // TODO: Detect actual exit code
            self.pending_commands.remove(&command_id);
            println!("Completed pending command '{}'", command_id);
        }
        
        // Update terminal status based on pending commands
        let has_running_commands = !self.pending_commands.is_empty();
        self.state.status = if has_running_commands {
            crate::terminal_state::TerminalStatus::Busy
        } else {
            crate::terminal_state::TerminalStatus::Ready
        };
        
        println!("Terminal state now has {} total commands, {} pending",
                 self.state.commands.len(), self.pending_commands.len());
    }
    
    /// Get current JSON state
    pub fn get_json_state(&self) -> Result<String> {
        Ok(self.state.to_json()?)
    }
    
    /// Get terminal state for direct access
    pub fn get_state(&self) -> &TerminalState {
        &self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_command_extraction() {
        let parser = TerminalSemanticParser::new(1);
        
        // Test various prompt formats
        let test_cases = [
            ("user@host:~/code$ ls -la", Some(("ls -la".to_string(), "user@host:~/code$ ".to_string()))),
            ("❯ git status", Some(("git status".to_string(), "❯ ".to_string()))),
            ("1 ❯ cargo build", Some(("cargo build".to_string(), "1 ❯ ".to_string()))),
            ("~/projects: npm test", Some(("npm test".to_string(), "~/projects: ".to_string()))),
            ("just output text", None),
            ("user@host:~/code$ ", None), // Empty command
        ];
        
        for (input, expected) in test_cases {
            let result = parser.extract_command_from_line(input);
            assert_eq!(result, expected, "Failed for input: {}", input);
        }
    }
    
    #[test]
    fn test_prompt_detection() {
        let parser = TerminalSemanticParser::new(1);
        
        let test_cases = [
            ("user@host:~/code$", true),
            ("❯", true),
            ("1 ❯", true),
            ("~/projects:", true),
            ("README.md", false),
            ("total 24", false),
            ("", false),
        ];
        
        for (input, expected) in test_cases {
            let result = parser.is_shell_prompt(input);
            assert_eq!(result, expected, "Failed for input: {}", input);
        }
    }
    
    #[test]
    fn test_output_collection() {
        let parser = TerminalSemanticParser::new(1);
        
        let terminal_lines = vec![
            "README.md".to_string(),
            "src/".to_string(),
            "Cargo.toml".to_string(),
            "user@host:~/code$".to_string(),
        ];
        
        let (output, end_line) = parser.collect_command_output(&terminal_lines);
        
        assert_eq!(output, vec!["README.md", "src/", "Cargo.toml"]);
        assert_eq!(end_line, 3);
    }
    
    #[test]
    fn test_grid_changes_detection() {
        let mut parser = TerminalSemanticParser::new(1);
        
        let grid_lines = vec![
            "user@host:~/code$ ls -la".to_string(),
            "total 24".to_string(),
            "drwxr-xr-x  3 user user 4096 Dec 20 10:30 .".to_string(),
            "-rw-r--r--  1 user user  220 Dec 20 10:25 README.md".to_string(),
            "user@host:~/code$".to_string(),
        ];
        
        let changes = parser.detect_grid_changes(&grid_lines);
        
        assert_eq!(changes.new_commands.len(), 1);
        assert_eq!(changes.new_commands[0].command, "ls -la");
        assert_eq!(changes.new_commands[0].output.len(), 3);
        assert!(changes.new_commands[0].completed);
    }
    
    #[test]
    fn test_json_state_generation() {
        let mut parser = TerminalSemanticParser::new(1);
        
        // Simulate processing terminal with commands
        let grid_lines = vec![
            "user@host:~/code$ ls".to_string(),
            "README.md".to_string(),
            "src/".to_string(),
            "user@host:~/code$ git status".to_string(),
            "On branch main".to_string(),
            "nothing to commit".to_string(),
            "user@host:~/code$".to_string(),
        ];
        
        let changes = parser.detect_grid_changes(&grid_lines);
        parser.apply_changes(changes);
        
        // Verify JSON state
        let json_state = parser.get_json_state().unwrap();
        let state: crate::terminal_state::TerminalState = 
            crate::terminal_state::TerminalState::from_json(&json_state).unwrap();
        
        // Should have detected 2 commands
        assert_eq!(state.commands.len(), 2);
        
        // First command: ls
        assert_eq!(state.commands[0].command, "ls");
        assert_eq!(state.commands[0].output, vec!["README.md", "src/"]);
        assert!(matches!(state.commands[0].status, crate::terminal_state::CommandStatus::Success));
        
        // Second command: git status  
        assert_eq!(state.commands[1].command, "git status");
        assert_eq!(state.commands[1].output, vec!["On branch main", "nothing to commit"]);
        assert!(matches!(state.commands[1].status, crate::terminal_state::CommandStatus::Success));
        
        // Terminal should be ready (no pending commands)
        assert!(matches!(state.status, crate::terminal_state::TerminalStatus::Ready));
    }
    
    #[test]
    fn test_pending_command_tracking() {
        let mut parser = TerminalSemanticParser::new(1);
        
        // Simulate a command that hasn't completed yet (no next prompt)
        let grid_lines = vec![
            "user@host:~/code$ npm install".to_string(),
            "npm WARN deprecated package@1.0.0".to_string(),
            "Installing dependencies...".to_string(),
        ];
        
        let changes = parser.detect_grid_changes(&grid_lines);
        parser.apply_changes(changes);
        
        // Should have 1 pending command
        assert_eq!(parser.pending_commands.len(), 1);
        
        let json_state = parser.get_json_state().unwrap();
        let state: crate::terminal_state::TerminalState = 
            crate::terminal_state::TerminalState::from_json(&json_state).unwrap();
        
        assert_eq!(state.commands.len(), 1);
        assert_eq!(state.commands[0].command, "npm install");
        assert!(matches!(state.commands[0].status, crate::terminal_state::CommandStatus::Running));
        assert!(matches!(state.status, crate::terminal_state::TerminalStatus::Busy));
    }
}