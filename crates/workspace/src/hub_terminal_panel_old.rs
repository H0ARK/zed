//! Hub Terminal Panel - Tabbed terminal interface for The Hub

use gpui::{
    div, prelude::*, AnyElement, Context, Entity, EventEmitter, FocusHandle, Render, 
    Window, InteractiveElement, ParentElement, Styled, WeakEntity, Subscription, Timer,
};
use std::collections::HashMap;
use theme::ActiveTheme;
use ui::{h_flex, v_flex, ButtonStyle, ButtonCommon, Clickable, IconButton, IconName, Tooltip};
use terminal::Terminal;
use project::terminals::TerminalKind;
use util::ResultExt;
use anyhow::Result;
use std::time::Duration;


/// Interactive terminal element that can receive focus and input
pub struct InteractiveTerminalElement {
    terminal: Entity<Terminal>,
    _focus_handle: FocusHandle,
    focused: bool,
}

impl InteractiveTerminalElement {
    pub fn new(terminal: Entity<Terminal>, focus_handle: FocusHandle, focused: bool) -> Self {
        Self {
            terminal,
            _focus_handle: focus_handle,
            focused,
        }
    }
}

impl EventEmitter<()> for InteractiveTerminalElement {}

impl Render for InteractiveTerminalElement {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let terminal = self.terminal.read(cx);
        let content = terminal.last_content();
        
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().colors().terminal_background)
            .p_4()
            .child(
                div()
                    .text_color(cx.theme().colors().text)
                    .child(format!("Terminal - Bounds: {}x{}", 
                        content.terminal_bounds.bounds.size.width.0 as u32,
                        content.terminal_bounds.bounds.size.height.0 as u32
                    ))
            )
            .child(
                div()
                    .text_color(cx.theme().colors().text_muted)
                    .text_size(gpui::rems(0.8))
                    .child("🖱️ Click here and type commands - Hub blocks will appear above")
            )
            .child(
                div()
                    .mt_2()
                    .text_color(cx.theme().colors().text_accent)
                    .text_size(gpui::rems(0.75))
                    .child("💡 Hub Terminal: Enhanced command line interface")
            )
            .when(self.focused, |div| {
                div.border_2().border_color(cx.theme().colors().border_focused)
            })
            .when(!self.focused, |div| {
                div.border_1().border_color(cx.theme().colors().border)
            })
    }
}

// Helper function to convert key strings to bytes for terminal input
#[allow(dead_code)]
fn key_to_bytes(key: &str) -> Option<Vec<u8>> {
    match key {
        "enter" => Some(b"\r".to_vec()),
        "backspace" => Some(b"\x7f".to_vec()),
        "tab" => Some(b"\t".to_vec()),
        "escape" => Some(b"\x1b".to_vec()),
        key if key.len() == 1 => {
            let char = key.chars().next()?;
            Some(char.to_string().into_bytes())
        }
        _ => None,
    }
}

/// Hub command tracking for detecting commands sent to terminal
#[derive(Clone)]
pub struct HubCommandInput {
    last_command: String,
    _focus_handle: FocusHandle,
}

impl HubCommandInput {
    pub fn new(cx: &mut Context<HubTerminalView>) -> Self {
        Self {
            last_command: String::new(),
            _focus_handle: cx.focus_handle(),
        }
    }
}

/// Hub command block for displaying command results
#[derive(Clone)]
pub struct HubCommandBlock {
    command: String,
    output: Vec<String>,
    status: CommandStatus,
    _block_id: String,
}

#[derive(Clone, Debug)]
pub enum CommandStatus {
    Running,
    Success,
    #[allow(dead_code)]
    Error,
}

impl HubCommandBlock {
    pub fn new(command: String, block_id: String) -> Self {
        Self {
            command,
            output: Vec::new(),
            status: CommandStatus::Running,
            _block_id: block_id,
        }
    }
    
    pub fn add_output(&mut self, line: String) {
        self.output.push(line);
    }
    
    pub fn set_status(&mut self, status: CommandStatus) {
        self.status = status;
    }
}

/// Hub-enhanced terminal view that wraps a TerminalView entity
pub struct HubTerminalView {
    terminal: Option<Entity<Terminal>>,
    terminal_id: usize,
    _hub_enabled: bool,
    _focus_handle: FocusHandle,
    workspace: WeakEntity<crate::Workspace>,
    _subscriptions: Vec<Subscription>,
    command_input: HubCommandInput,
    command_blocks: Vec<HubCommandBlock>,
    next_block_id: usize,
}

impl HubTerminalView {
    #[allow(dead_code)]
    pub fn new(
        terminal: Entity<Terminal>, 
        terminal_id: usize,
        focus_handle: FocusHandle,
        workspace: WeakEntity<crate::Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&terminal, |_, _, cx| {
                cx.notify();
            })
        ];
        
        let command_input = HubCommandInput::new(cx);
        
        let mut view = Self { 
            terminal: Some(terminal), 
            terminal_id,
            _hub_enabled: true,
            _focus_handle: focus_handle,
            workspace,
            _subscriptions: subscriptions,
            command_input,
            command_blocks: Vec::new(),
            next_block_id: 1,
        };
        
        // Generate demo commands immediately
        view.generate_demo_commands(cx);
        view
    }
    
    pub fn new_deferred(
        terminal_id: usize,
        focus_handle: FocusHandle,
        workspace: WeakEntity<crate::Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let command_input = HubCommandInput::new(cx);
        
        let mut view = Self { 
            terminal: None, 
            terminal_id,
            _hub_enabled: true,
            _focus_handle: focus_handle,
            workspace,
            _subscriptions: Vec::new(),
            command_input,
            command_blocks: Vec::new(),
            next_block_id: 1,
        };
        
        // Generate demo commands immediately for deferred terminals too
        view.generate_demo_commands(cx);
        view
    }
    
    #[allow(dead_code)]
    pub fn terminal(&self) -> Option<&Entity<Terminal>> {
        self.terminal.as_ref()
    }
    
    fn ensure_terminal_created(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.is_none() {
            if let Some(workspace) = self.workspace.upgrade() {
                let project = workspace.read(cx).project().clone();
                let working_directory = workspace.read(cx).project().read(cx).active_project_directory(cx)
                    .map(|path| path.to_path_buf());
                
                let window_handle = window.window_handle();
                
                let terminal_task = project.update(cx, |project, cx| {
                    println!("Creating terminal with working directory: {:?}", working_directory);
                    project.create_terminal(
                        TerminalKind::Shell(working_directory),
                        window_handle,
                        cx,
                    )
                });
                
                // Store the task and handle it asynchronously
                let _terminal_id = self.terminal_id;
                let _workspace = self.workspace.clone();
                cx.spawn(async move |this, cx| {
                    if let Ok(terminal) = terminal_task.await {
                        this.update(cx, |this, cx| {
                            // Subscribe to terminal updates and detect commands
                            let subscription = cx.observe(&terminal, |this, terminal, cx| {
                                println!("Terminal updated, detecting commands...");
                                this.detect_commands_from_terminal(&terminal, cx);
                                cx.notify();
                            });
                            this.terminal = Some(terminal.clone());
                            this._subscriptions.push(subscription);
                            
                            // Try to activate/focus the terminal to start shell process
                            println!("Attempting to activate terminal...");
                            terminal.update(cx, |terminal, _| {
                                // Check if terminal has any methods to start or activate
                                let content = terminal.last_content();
                                println!("Terminal initial state: {} cells, cursor point: ({}, {})", 
                                    content.cells.len(), content.cursor.point.line.0, content.cursor.point.column.0);
                            });
                            
                            // Auto-generate demo command blocks now that terminal is available
                            println!("Terminal created and available, generating demo commands...");
                            this.generate_demo_commands(cx);
                            
                            // Start periodic monitoring of terminal content with increasing intervals
                            let terminal_for_monitoring = terminal.clone();
                            cx.spawn(async move |this, cx| {
                                for i in 0..5 { // Check for 5 seconds
                                    Timer::after(Duration::from_millis(1000)).await;
                                    
                                    this.update(cx, |this, cx| {
                                        println!("Periodic terminal check #{}/5...", i + 1);
                                        this.detect_commands_from_terminal(&terminal_for_monitoring, cx);
                                        
                                        // If we get content, reduce checking frequency
                                        let content = terminal_for_monitoring.read(cx).last_content();
                                        if content.cells.len() > 0 {
                                            println!("Terminal now has content! Reducing check frequency.");
                                        }
                                    }).log_err();
                                }
                                
                                println!("Stopping periodic terminal checks after 5 seconds");
                            }).detach();
                            
                            cx.notify();
                        }).log_err();
                    }
                }).detach();
            }
        }
    }
    
    fn generate_demo_commands(&mut self, cx: &mut Context<Self>) {
        // Auto-execute a few commands since stdin is not available
        if self.command_blocks.is_empty() {
            // Wait for terminal to be ready, then send commands
            if let Some(terminal) = &self.terminal {
                println!("Trying to send initial newline to wake up terminal...");
                terminal.update(cx, |terminal, _| {
                    // Send a newline first to potentially trigger a prompt
                    terminal.input("\n".as_bytes());
                });
            }
            
            // Create command blocks but don't send commands immediately
            self.create_demo_command_blocks(cx);
        }
    }
    
    fn create_demo_command_blocks(&mut self, cx: &mut Context<Self>) {
        // Create command blocks that will be executed when terminal is ready
        let block_id = format!("block_{}", self.next_block_id);
        self.next_block_id += 1;
        let mut ls_block = HubCommandBlock::new("ls".to_string(), block_id);
        ls_block.add_output("$ ls".to_string());
        ls_block.add_output("Waiting for terminal to be ready...".to_string());
        self.command_blocks.push(ls_block);
        
        let block_id = format!("block_{}", self.next_block_id);
        self.next_block_id += 1;
        let mut git_block = HubCommandBlock::new("git status".to_string(), block_id);
        git_block.add_output("$ git status".to_string());
        git_block.add_output("Waiting for terminal to be ready...".to_string());
        self.command_blocks.push(git_block);
        
        cx.notify();
    }
    
    fn execute_pending_commands_if_ready(&mut self, terminal_lines: &[String], cx: &mut Context<Self>) {
        // Check if terminal has a prompt (indicates it's ready for commands)
        let has_prompt = terminal_lines.iter().any(|line| {
            line.contains("$") || line.contains("❯") || line.contains(">") || line.contains("#")
        });
        
        if has_prompt {
            println!("Terminal appears ready (found prompt), executing pending commands...");
            
            // Execute commands for blocks that are still waiting
            for block in &mut self.command_blocks {
                if block.output.iter().any(|line| line.contains("Waiting for terminal to be ready")) {
                    println!("Executing pending command: {}", block.command);
                    
                    // Update the block to show it's executing
                    block.output.retain(|line| !line.contains("Waiting for terminal to be ready"));
                    block.add_output("Executing...".to_string());
                    
                    // Send the actual command
                    if let Some(terminal) = &self.terminal {
                        let command_bytes = format!("{}\n", block.command);
                        terminal.update(cx, |terminal, _| {
                            terminal.input(command_bytes.into_bytes());
                        });
                    }
                }
            }
            
            cx.notify();
        }
    }
    
    fn auto_execute_command(&mut self, command: String, cx: &mut Context<Self>) {
        // Send real command to terminal and create command block for real output
        if let Some(terminal) = &self.terminal {
            println!("Executing command: '{}'", command);
            
            // Send the command to the actual terminal with newline
            let command_bytes = format!("{}\n", command);
            println!("Sending bytes to terminal: {:?}", command_bytes.as_bytes());
            
            terminal.update(cx, |terminal, _| {
                println!("Terminal input method called");
                terminal.input(command_bytes.into_bytes());
                
                // Also try to read current content to debug
                let content = terminal.last_content();
                println!("Terminal state after input: {} cells", content.cells.len());
            });
            
            // Create command block that will be updated with real output
            let block_id = format!("block_{}", self.next_block_id);
            self.next_block_id += 1;
            
            let mut command_block = HubCommandBlock::new(command.clone(), block_id);
            command_block.add_output(format!("$ {}", command));
            command_block.add_output("Executing...".to_string());
            // Status remains Running until we get real output
            
            self.command_blocks.push(command_block);
            self.command_input.last_command = command;
            
            println!("Created command block, total blocks: {}", self.command_blocks.len());
            
            cx.notify();
        } else {
            println!("No terminal available for command: '{}'", command);
        }
    }
    
    #[allow(dead_code)]
    fn create_hub_output_json(&self, command: &str, output_lines: &[String], status: &CommandStatus) -> String {
        // Create structured JSON output for Hub protocol
        let output_data = serde_json::json!({
            "command": command,
            "output": output_lines,
            "status": match status {
                CommandStatus::Running => "running",
                CommandStatus::Success => "success",
                CommandStatus::Error => "error"
            },
            "timestamp": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            "block_type": "command_output",
            "metadata": {
                "terminal_id": self.terminal_id,
                "execution_context": "hub_terminal"
            }
        });
        
        output_data.to_string()
    }
    
    fn detect_commands_from_terminal(&mut self, terminal: &Entity<Terminal>, cx: &mut Context<Self>) {
        // Read terminal content and extract all text
        let content = terminal.read(cx).last_content();
        
        // Debug: Print raw terminal info
        println!("Terminal bounds: {}x{}", 
            content.terminal_bounds.bounds.size.width.0, 
            content.terminal_bounds.bounds.size.height.0);
        println!("Terminal mode: {:?}", content.mode);
        println!("Display offset: {}", content.display_offset);
        
        // Convert terminal cells to text lines
        let mut lines = Vec::new();
        let mut current_line = String::new();
        let mut last_line_idx = -1;
        
        for cell in &content.cells {
            let line_idx = cell.point.line.0;
            
            // If we've moved to a new line, save the current line and start a new one
            if line_idx != last_line_idx && !current_line.is_empty() {
                lines.push(current_line.trim().to_string());
                current_line.clear();
            }
            
            // Add the character from this cell
            current_line.push(cell.cell.c);
            last_line_idx = line_idx;
        }
        
        // Don't forget the last line
        if !current_line.is_empty() {
            lines.push(current_line.trim().to_string());
        }
        
        // Debug: Print terminal content to see what we're getting
        println!("Terminal content: {} cells, {} lines", content.cells.len(), lines.len());
        if lines.is_empty() {
            println!("  No lines found in terminal");
        } else {
            println!("Terminal lines:");
            for (i, line) in lines.iter().enumerate() {
                println!("  {}: '{}'", i, line);
            }
            
            // If terminal now has content and we have pending commands, execute them
            self.execute_pending_commands_if_ready(&lines, cx);
        }
        
        // Update existing command blocks with terminal output
        self.update_command_blocks_with_terminal_output(&lines, cx);
        
        // Look for new commands in the terminal
        for line in lines.iter().rev().take(5) {
            if self.is_command_line(line) && !self.command_already_exists(line) {
                // Found a new command - create a command block
                self.execute_command_from_terminal(line.clone(), cx);
                break; // Only process one new command at a time
            }
        }
    }
    
    fn update_command_blocks_with_terminal_output(&mut self, terminal_lines: &[String], cx: &mut Context<Self>) {
        // Find running command blocks and update them with terminal output
        for block in &mut self.command_blocks {
            if matches!(block.status, CommandStatus::Running) {
                // Look for output related to this command
                let command = block.command.clone();
                println!("Looking for output for command: '{}'", command);
                
                // Find lines that come after the command was executed
                let mut collecting_output = false;
                let mut new_output = Vec::new();
                
                for line in terminal_lines {
                    // Look for the command prompt line
                    if line.contains(&format!("$ {}", command)) || line.contains(&command) {
                        println!("Found command line: '{}'", line);
                        collecting_output = true;
                        continue;
                    }
                    
                    // If we're collecting and hit another command, stop
                    if collecting_output && (line.starts_with('$') || line.contains("$ ")) {
                        println!("Hit another command, stopping collection");
                        break;
                    }
                    
                    // Collect output lines
                    if collecting_output && !line.trim().is_empty() {
                        println!("Collecting output line: '{}'", line);
                        new_output.push(line.clone());
                    }
                }
                
                // Update the command block with new output
                if !new_output.is_empty() {
                    println!("Updating command block with {} lines of output", new_output.len());
                    // Clear the "Executing..." message
                    block.output.retain(|line| !line.contains("Executing"));
                    
                    // Add new output
                    for line in &new_output {
                        block.add_output(line.clone());
                    }
                    
                    // Mark as completed if we have substantial output
                    if block.output.len() > 2 { // More than just the command line
                        block.set_status(CommandStatus::Success);
                    }
                    
                    // Note: Hub JSON output generation would happen here
                    log::info!("Hub Block updated with {} lines of output", new_output.len());
                    
                    cx.notify();
                } else {
                    println!("No new output found for command '{}'", command);
                }
            }
        }
    }
    
    fn is_command_line(&self, line: &str) -> bool {
        // Detect lines that look like commands
        // Look for common shell prompts followed by commands
        if line.contains("$ ") || line.contains("❯ ") || line.contains("> ") {
            return true;
        }
        
        // Also look for lines that start with common commands
        let trimmed = line.trim();
        for cmd_prefix in &["npm ", "git ", "cargo ", "ls", "pwd", "cd ", "cat ", "grep ", "find "] {
            if trimmed.starts_with(cmd_prefix) {
                return true;
            }
        }
        
        false
    }
    
    fn command_already_exists(&self, line: &str) -> bool {
        // Extract just the command part (after prompt if present)
        let command = self.extract_command_from_line(line);
        
        // Check if we already have this command in our blocks
        self.command_blocks.iter().any(|block| block.command == command)
    }
    
    fn extract_command_from_line(&self, line: &str) -> String {
        // Extract command from line (remove shell prompts)
        if let Some(pos) = line.find("$ ") {
            line[pos + 2..].trim().to_string()
        } else if let Some(pos) = line.find("❯ ") {
            line[pos + 2..].trim().to_string()
        } else if let Some(pos) = line.find("> ") {
            line[pos + 2..].trim().to_string()
        } else {
            line.trim().to_string()
        }
    }
    
    fn execute_command_from_terminal(&mut self, line: String, cx: &mut Context<Self>) {
        let command = self.extract_command_from_line(&line);
        
        if command.is_empty() {
            return;
        }
        
        // Create a new command block with just the command initially
        let block_id = format!("block_{}", self.next_block_id);
        self.next_block_id += 1;
        
        let mut command_block = HubCommandBlock::new(command.clone(), block_id);
        command_block.add_output(format!("$ {}", command));
        command_block.add_output("Waiting for output...".to_string());
        // Leave status as Running initially
        
        self.command_blocks.push(command_block);
        self.command_input.last_command = command;
        
        // TODO: In a full implementation, we would:
        // 1. Monitor terminal output for this command's results
        // 2. Update the command block with real output
        // 3. Set final status based on command success/failure
        
        cx.notify();
    }
    
    #[allow(dead_code)]
    fn execute_command(&mut self, command: String, cx: &mut Context<Self>) {
        if command.trim().is_empty() {
            return;
        }
        
        // Create a new command block
        let block_id = format!("block_{}", self.next_block_id);
        self.next_block_id += 1;
        
        let mut command_block = HubCommandBlock::new(command.clone(), block_id);
        
        // For now, simulate command execution with mock output
        command_block.add_output(format!("$ {}", command));
        
        match command.trim() {
            "npm run legion" => {
                command_block.add_output("> legion@1.0.0 legion".to_string());
                command_block.add_output("> node dist/index.js".to_string());
                command_block.add_output("".to_string());
                command_block.add_output("✓ Server started on port 3000".to_string());
                command_block.add_output("✓ Database connected".to_string());
                command_block.add_output("✓ Ready to accept connections".to_string());
                command_block.set_status(CommandStatus::Success);
            }
            "git status" => {
                command_block.add_output("On branch main".to_string());
                command_block.add_output("Your branch is ahead of 'origin/main' by 2 commits.".to_string());
                command_block.add_output("  (use \"git push\" to publish your local commits)".to_string());
                command_block.add_output("".to_string());
                command_block.add_output("Changes not staged for commit:".to_string());
                command_block.add_output("  modified:   src/components/Terminal.tsx".to_string());
                command_block.add_output("  modified:   package.json".to_string());
                command_block.set_status(CommandStatus::Success);
            }
            "ls" => {
                command_block.add_output("README.md".to_string());
                command_block.add_output("src/".to_string());
                command_block.add_output("Cargo.toml".to_string());
                command_block.set_status(CommandStatus::Success);
            }
            "pwd" => {
                command_block.add_output("/Users/conrad/Documents/github/zed".to_string());
                command_block.set_status(CommandStatus::Success);
            }
            "cargo build" => {
                command_block.add_output("   Compiling hub_core v0.1.0".to_string());
                command_block.add_output("   Compiling hub_protocol v0.1.0".to_string());
                command_block.add_output("   Compiling hub_blocks v0.1.0".to_string());
                command_block.add_output("   Compiling workspace v0.1.0".to_string());
                command_block.add_output("    Finished release [optimized] target(s) in 1m 23s".to_string());
                command_block.set_status(CommandStatus::Success);
            }
            "git log --oneline" => {
                command_block.add_output("d280c95 agent: Suggest turning burn mode on when close to context window limit".to_string());
                command_block.add_output("fcf5042 anthropic: Reorder Model variants in descending order".to_string());
                command_block.add_output("cb9beb8 anthropic: Refactor a bit".to_string());
                command_block.add_output("29f3e62 ui: Refactor the Callout component".to_string());
                command_block.add_output("aa1cb9c editor: Fix inline blame show/hide not working".to_string());
                command_block.set_status(CommandStatus::Success);
            }
            _ => {
                command_block.add_output(format!("Hub: Command '{}' executed", command));
                command_block.set_status(CommandStatus::Success);
            }
        }
        
        self.command_blocks.push(command_block);
        self.command_input.last_command = command;
        
        // TODO: Send command to actual terminal process
        // TODO: Set up Hub protocol stream for this terminal
        
        cx.notify();
    }
    
    fn render_command_block(&self, block: &HubCommandBlock, cx: &Context<Self>) -> impl IntoElement {
        div()
            .mb_1()
            .child(
                // Minimal horizontal header - matches the screenshot design
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(cx.theme().colors().surface_background)
                    .px_3()
                    .py_2()
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded(gpui::rems(0.25))
                    .child(
                        // Left side: Command
                        div()
                            .text_color(cx.theme().colors().text)
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_size(gpui::rems(0.875))
                            .child(block.command.clone())
                    )
                    .child(
                        // Right side: Action buttons (placeholders)
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_color(cx.theme().colors().text_muted)
                                    .text_size(gpui::rems(0.875))
                                    .child("✨") // Enhance
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().colors().text_muted)
                                    .text_size(gpui::rems(0.875))
                                    .child("📥") // Download
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().colors().text_muted)
                                    .text_size(gpui::rems(0.875))
                                    .child("🔍") // Filter
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().colors().text_muted)
                                    .text_size(gpui::rems(0.875))
                                    .child("⋯") // More
                            )
                    )
            )
            .child(
                // Command output - clean, minimal
                div()
                    .bg(cx.theme().colors().terminal_background)
                    .px_3()
                    .py_2()
                    .border_l_1()
                    .border_color(cx.theme().colors().border_variant)
                    .children(
                        block.output.iter().skip(1).map(|line| { // Skip the "$ command" line
                            div()
                                .text_color(cx.theme().colors().terminal_foreground)
                                .font_family("monospace")
                                .text_size(gpui::rems(0.8))
                                .child(line.clone())
                        })
                    )
                    .when(block.output.len() <= 1, |div_el| {
                        div_el.child(
                            div()
                                .text_color(cx.theme().colors().text_muted)
                                .text_size(gpui::rems(0.8))
                                .child("Running...")
                        )
                    })
            )
    }
}

impl EventEmitter<()> for HubTerminalView {}

impl Render for HubTerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Ensure terminal is created
        self.ensure_terminal_created(window, cx);
        
        let focused = self._focus_handle.is_focused(window);
        
        // Render the terminal with Hub block capabilities
        div()
            .flex()
            .flex_col()
            .size_full()
            .border_1()
            .border_color(cx.theme().colors().border)
            .child(
                // Terminal area (takes most space)
                div()
                    .flex_1()
                    .bg(cx.theme().colors().terminal_background)
                    .when_some(self.terminal.clone(), |div, terminal| {
                        div.child(
                            cx.new(|_cx| InteractiveTerminalElement::new(
                                terminal,
                                self._focus_handle.clone(),
                                focused
                            ))
                        )
                    })
                    .when(self.terminal.is_none(), |el| {
                        el.flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_color(cx.theme().colors().text_muted)
                                    .child(format!("Creating Terminal {}...", self.terminal_id))
                            )
                    })
            )
            .child(
                // Hub blocks area (bottom of screen) - Command Blocks
                div()
                    .flex()
                    .flex_col()
                    .max_h(gpui::rems(20.0))
                    .overflow_y_hidden()
                    .bg(cx.theme().colors().editor_background)
                    .border_t_1() // Top border instead of bottom since it's at bottom
                    .border_color(cx.theme().colors().border)
                    .p_2()
                    .children(
                        self.command_blocks.iter().map(|block| {
                            self.render_command_block(block, cx)
                        })
                    )
                    .when(self.command_blocks.is_empty(), |div_el| {
                        div_el.child(
                            div()
                                .text_color(cx.theme().colors().text_muted)
                                .text_size(gpui::rems(0.8))
                                .px_3()
                                .py_2()
                                .child("Commands will appear here...")
                        )
                    })
            )
            .child(
                // Hub status indicator (minimal)
                div()
                    .absolute()
                    .top(gpui::rems(0.5))
                    .right(gpui::rems(0.5))
                    .px_2()
                    .py_1()
                    .bg(cx.theme().colors().surface_background)
                    .border_1()
                    .border_color(cx.theme().colors().border)
                    .rounded(gpui::rems(0.25))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_color(cx.theme().colors().text_accent)
                                    .text_size(gpui::rems(0.75))
                                    .child("🧩")
                            )
                            .child(
                                div()
                                    .text_color(cx.theme().colors().text_muted)
                                    .text_size(gpui::rems(0.7))
                                    .child("Hub Active")
                            )
                    )
            )
    }
}

/// Hub Terminal Panel manages multiple terminal instances in tabs
pub struct HubTerminalPanel {
    terminal_tabs: HashMap<usize, Entity<HubTerminalView>>,
    active_terminal_id: Option<usize>,
    next_terminal_id: usize,
    _focus_handle: FocusHandle,
    workspace: WeakEntity<crate::Workspace>,
    pending_terminals: usize,
}

/// Events emitted by the Hub Terminal Panel
#[derive(Clone, Debug, PartialEq)]
pub enum HubTerminalEvent {
    TerminalAdded(usize),
    TerminalRemoved(usize),
    ActiveTerminalChanged(Option<usize>),
}

impl EventEmitter<HubTerminalEvent> for HubTerminalPanel {}

impl HubTerminalPanel {
    pub fn new(workspace: WeakEntity<crate::Workspace>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        
        let mut panel = Self {
            terminal_tabs: HashMap::new(),
            active_terminal_id: None,
            next_terminal_id: 1,
            _focus_handle: focus_handle,
            workspace,
            pending_terminals: 0,
        };
        
        // Create the first terminal automatically
        panel.add_terminal(cx);
        
        panel
    }
    
    pub fn add_terminal(&mut self, cx: &mut Context<Self>) -> usize {
        let terminal_id = self.next_terminal_id;
        self.next_terminal_id += 1;
        
        // Create a task to build the real terminal asynchronously
        let workspace = self.workspace.clone();
        self.pending_terminals += 1;
        
        cx.spawn(async move |hub_panel, cx| {
            let result = Self::create_terminal(workspace, terminal_id, cx).await;
            
            hub_panel.update(cx, |hub_panel, cx| {
                hub_panel.pending_terminals = hub_panel.pending_terminals.saturating_sub(1);
                
                match result {
                    Ok(hub_terminal_view) => {
                        hub_panel.terminal_tabs.insert(terminal_id, hub_terminal_view);
                        hub_panel.active_terminal_id = Some(terminal_id);
                        
                        cx.emit(HubTerminalEvent::TerminalAdded(terminal_id));
                        cx.emit(HubTerminalEvent::ActiveTerminalChanged(hub_panel.active_terminal_id));
                        cx.notify();
                    }
                    Err(err) => {
                        log::error!("Failed to create terminal {}: {}", terminal_id, err);
                        // Create a placeholder showing the error
                        // For now, we'll just log it
                    }
                }
            })
        }).detach();
        
        terminal_id
    }
    
    async fn create_terminal(
        workspace: WeakEntity<crate::Workspace>,
        terminal_id: usize,
        cx: &mut gpui::AsyncApp,
    ) -> Result<Entity<HubTerminalView>> {
        // For now, create a placeholder HubTerminalView that will create the real terminal later
        // This avoids the window handle issue in async context
        let hub_terminal_view = workspace.update(cx, |workspace, cx| {
            let focus_handle = cx.focus_handle();
            let workspace_weak = workspace.weak_handle();
            
            // Create HubTerminalView with deferred terminal creation
            cx.new(|cx| {
                HubTerminalView::new_deferred(
                    terminal_id,
                    focus_handle,
                    workspace_weak,
                    cx,
                )
            })
        })?;
        
        Ok(hub_terminal_view)
    }
    
    pub fn remove_terminal(&mut self, terminal_id: usize, cx: &mut Context<Self>) {
        if self.terminal_tabs.remove(&terminal_id).is_some() {
            // If we removed the active terminal, switch to another one
            if self.active_terminal_id == Some(terminal_id) {
                self.active_terminal_id = self.terminal_tabs.keys().next().copied();
            }
            
            cx.emit(HubTerminalEvent::TerminalRemoved(terminal_id));
            cx.emit(HubTerminalEvent::ActiveTerminalChanged(self.active_terminal_id));
            cx.notify();
        }
    }
    
    pub fn set_active_terminal(&mut self, terminal_id: usize, cx: &mut Context<Self>) {
        if self.terminal_tabs.contains_key(&terminal_id) {
            self.active_terminal_id = Some(terminal_id);
            cx.emit(HubTerminalEvent::ActiveTerminalChanged(self.active_terminal_id));
            cx.notify();
        }
    }
    
    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .bg(cx.theme().colors().tab_bar_background)
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                h_flex()
                    .flex_1()
                    .children(
                        self.terminal_tabs
                            .keys()
                            .map(|&terminal_id| {
                                let is_active = self.active_terminal_id == Some(terminal_id);
                                
                                div()
                                    .flex()
                                    .items_center()
                                    .px_3()
                                    .py_1()
                                    .border_r_1()
                                    .border_color(cx.theme().colors().border)
                                    .when(is_active, |div| {
                                        div.bg(cx.theme().colors().tab_active_background)
                                    })
                                    .when(!is_active, |div| {
                                        div.bg(cx.theme().colors().tab_inactive_background)
                                            .hover(|div| div.bg(cx.theme().colors().element_hover))
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .cursor_pointer()
                                            .child(format!("Terminal {}", terminal_id))
                                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                this.set_active_terminal(terminal_id, cx);
                                            }))
                                    )
                                    .child(
                                        IconButton::new(("close_tab", terminal_id), IconName::Close)
                                            .style(ButtonStyle::Transparent)
                                            .tooltip(Tooltip::text("Close Terminal"))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.remove_terminal(terminal_id, cx);
                                            }))
                                    )
                            })
                    )
            )
            .child(
                IconButton::new("add_terminal", IconName::Plus)
                    .style(ButtonStyle::Transparent)
                    .tooltip(Tooltip::text("New Terminal"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.add_terminal(cx);
                    }))
            )
    }
    
    fn render_active_terminal(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(terminal_id) = self.active_terminal_id {
            if let Some(hub_terminal_view) = self.terminal_tabs.get(&terminal_id) {
                return Some(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .overflow_hidden()
                        .child(hub_terminal_view.clone())
                        .into_any_element()
                );
            }
        }
        
        None
    }
}

impl Render for HubTerminalPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .child(
                // Terminal tabs (thinner)
                self.render_tabs(cx)
            )
            .child(
                // Active terminal content
                div()
                    .flex_1()
                    .overflow_hidden()
                    .when_some(self.render_active_terminal(cx), |div, terminal| {
                        div.child(terminal)
                    })
                    .when(self.active_terminal_id.is_none(), |this| {
                        this.flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_color(cx.theme().colors().text_muted)
                                    .child("No terminal open. Click + to add a new terminal.")
                            )
                    })
            )
    }
}