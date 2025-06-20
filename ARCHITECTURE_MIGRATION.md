# Hub Terminal Architecture Migration

## Overview
This document outlines the migration from the complex Hub server/client architecture to a simplified JSON-based streaming approach for terminal command blocks.

## Current State (Complex Architecture)

### Problems with Current Implementation:
- ❌ **Over-engineered**: Multiple crates (hub_protocol, hub_blocks, hub_terminal_engine)
- ❌ **Complex messaging**: Server/client protocol with async message routing
- ❌ **Terminal issues**: 0 cells, no content, shell process not starting
- ❌ **Circular dependencies**: hub_terminal_engine ↔ terminal_view conflicts
- ❌ **Unnecessary abstraction**: Protocol streams for simple terminal output

### Current Crate Structure:
```
crates/
├── hub_core/           # Core types and utilities
├── hub_protocol/       # Server/client messaging
├── hub_blocks/         # Block rendering system
├── hub_terminal_engine/# Terminal enhancement layer
└── workspace/          # Main UI integration
    └── hub_terminal_panel.rs
```

### Current Data Flow:
```
Terminal → Hub Server → Protocol Messages → Hub Blocks → UI
```

## Target State (Simplified JSON Architecture)

### Benefits of New Approach:
- ✅ **Simple**: Single JSON structure represents entire terminal state
- ✅ **Direct**: Terminal output → JSON → UI (no middleware)
- ✅ **Streaming**: Live updates via JSON patches
- ✅ **Debuggable**: Human-readable JSON state
- ✅ **Efficient**: No protocol overhead or message routing

### Target Structure:
```
crates/
└── workspace/
    ├── hub_terminal_panel.rs  # UI components
    ├── terminal_state.rs      # JSON state management
    └── terminal_monitor.rs    # Terminal content monitoring
```

### Target Data Flow:
```
Terminal Content → Terminal Monitor → JSON State → Command Blocks UI
```

## Migration Plan

### Phase 1: JSON State Foundation ✅
- [x] Create JSON schema (`terminal_schema.json`)
- [x] Implement Rust structures (`terminal_state.rs`)
- [x] Add serialization/deserialization
- [x] Write unit tests

### Phase 2: Terminal Content Monitoring
**Goal**: Replace complex terminal observation with direct content parsing

#### Current Issues to Fix:
```rust
// PROBLEM: Current detection returns 0 cells
fn detect_commands_from_terminal() {
    let content = terminal.read(cx).last_content();
    // content.cells.len() == 0 (no shell process)
}
```

#### Target Implementation:
```rust
// SOLUTION: Direct terminal content monitoring
struct TerminalMonitor {
    state: TerminalState,
    last_content_hash: u64,
}

impl TerminalMonitor {
    fn update_from_terminal(&mut self, terminal: &Terminal) -> Option<String> {
        let content = terminal.last_content();
        
        // Parse terminal cells into text lines
        let lines = self.parse_terminal_content(&content);
        
        // Detect new commands and output
        if let Some(changes) = self.detect_changes(&lines) {
            self.update_json_state(changes);
            return Some(self.state.to_json());
        }
        
        None
    }
}
```

### Phase 3: Command Detection & Output Parsing
**Goal**: Reliable command detection from terminal content

#### Command Detection Strategy:
```rust
fn detect_commands(&self, lines: &[String]) -> Vec<DetectedCommand> {
    let mut commands = Vec::new();
    
    for (i, line) in lines.iter().enumerate() {
        // Look for shell prompts
        if let Some(command) = self.extract_command_from_prompt(line) {
            // Collect output until next prompt
            let output = self.collect_command_output(&lines[i+1..]);
            
            commands.push(DetectedCommand {
                command,
                output,
                line_start: i,
                line_end: i + output.len(),
            });
        }
    }
    
    commands
}

fn extract_command_from_prompt(&self, line: &str) -> Option<String> {
    // Pattern matching for different shell prompts:
    // "user@host:~/path$ command args"
    // "❯ command args" 
    // "> command args"
    
    for pattern in &["$ ", "❯ ", "> ", "# "] {
        if let Some(pos) = line.find(pattern) {
            return Some(line[pos + pattern.len()..].trim().to_string());
        }
    }
    
    None
}
```

### Phase 4: JSON State Management
**Goal**: Live updating JSON that represents terminal state

#### State Updates:
```rust
impl TerminalState {
    fn add_detected_command(&mut self, command: &str) -> String {
        let cmd_id = self.add_command(command.to_string());
        
        // Emit JSON update
        println!("JSON_UPDATE: {}", self.to_json().unwrap());
        
        cmd_id
    }
    
    fn update_command_output(&mut self, cmd_id: &str, new_lines: Vec<String>) {
        self.update_command_output(cmd_id, new_lines);
        
        // Emit JSON update
        println!("JSON_UPDATE: {}", self.to_json().unwrap());
    }
    
    fn complete_command(&mut self, cmd_id: &str, exit_code: i32) {
        self.complete_command(cmd_id, exit_code);
        
        // Emit JSON update  
        println!("JSON_UPDATE: {}", self.to_json().unwrap());
    }
}
```

### Phase 5: UI Integration
**Goal**: Command blocks render from JSON state

#### Current UI (Complex):
```rust
struct HubCommandBlock {
    command: String,
    output: Vec<String>,
    status: CommandStatus,
    block_id: String,
}

// Multiple structs, complex state management
```

#### Target UI (Simple):
```rust
impl HubTerminalView {
    fn render_from_json(&self, json_state: &str) -> impl IntoElement {
        let state: TerminalState = serde_json::from_str(json_state).unwrap();
        
        div()
            .flex_col()
            .children(
                state.commands.iter().map(|cmd| {
                    self.render_command_block(cmd)
                })
            )
    }
    
    fn render_command_block(&self, cmd: &Command) -> impl IntoElement {
        div()
            .child(format!("$ {}", cmd.command))
            .children(
                cmd.output.iter().map(|line| {
                    div().child(line.clone())
                })
            )
            .when(cmd.status == CommandStatus::Running, |div| {
                div.child("● Running...")
            })
    }
}
```

### Phase 6: Streaming Updates
**Goal**: Real-time JSON updates to frontend

#### WebSocket/EventStream:
```rust
// Instead of complex protocol, simple JSON streaming
fn stream_terminal_updates(terminal_id: u32) -> impl Stream<Item = String> {
    // Monitor terminal content changes
    // Emit JSON updates when commands/output change
    // Frontend receives JSON and re-renders blocks
}
```

#### Frontend Integration:
```javascript
// Simple JSON consumption
const terminalStream = new EventSource(`/terminal/${id}/stream`);
terminalStream.onmessage = (event) => {
    const state = JSON.parse(event.data);
    renderCommandBlocks(state.commands);
};
```

## Migration Steps

### Step 1: Remove Hub Server Complexity
```bash
# Remove unnecessary crates
rm -rf crates/hub_protocol
rm -rf crates/hub_blocks  
rm -rf crates/hub_terminal_engine
rm -rf crates/hub_core

# Update Cargo.toml dependencies
# Remove hub_protocol, hub_blocks, etc.
```

### Step 2: Implement Terminal Monitor
```rust
// Create crates/workspace/src/terminal_monitor.rs
pub struct TerminalMonitor {
    terminal_id: u32,
    state: TerminalState,
    last_content_hash: u64,
}

impl TerminalMonitor {
    pub fn new(terminal_id: u32) -> Self { /* ... */ }
    pub fn update(&mut self, terminal: &Terminal) -> Option<String> { /* ... */ }
    pub fn get_json_state(&self) -> String { /* ... */ }
}
```

### Step 3: Replace HubTerminalView
```rust
// Simplify hub_terminal_panel.rs
pub struct HubTerminalView {
    terminal: Option<Entity<Terminal>>,
    monitor: TerminalMonitor,  // Replace complex command blocks
    // Remove: command_blocks, command_input, etc.
}

impl HubTerminalView {
    fn update_from_terminal(&mut self, terminal: &Entity<Terminal>, cx: &mut Context<Self>) {
        if let Some(json_update) = self.monitor.update(terminal) {
            // Stream JSON update (console for now, WebSocket later)
            println!("TERMINAL_UPDATE: {}", json_update);
            cx.notify(); // Re-render UI
        }
    }
}
```

### Step 4: Test with Real Terminal
```rust
// Fix the terminal content issue first
fn ensure_terminal_has_shell(&mut self, terminal: &Entity<Terminal>) {
    // Investigate why terminal.last_content().cells.len() == 0
    // Ensure shell process starts properly
    // Test with minimal commands first
}
```

## Success Metrics

### Before (Current Issues):
- ❌ Terminal content: 0 cells, no shell process
- ❌ Complex codebase: 4+ crates, 1000+ lines
- ❌ Debug difficulty: Complex async message flow
- ❌ Command blocks: Stuck on "Executing..."

### After (Target Goals):
- ✅ Terminal content: Real shell output captured
- ✅ Simple codebase: 1 crate, ~300 lines  
- ✅ Debug friendly: Human-readable JSON state
- ✅ Command blocks: Live updates from real commands

## JSON Example Flow

### 1. Initial State:
```json
{
  "terminal_id": 1,
  "status": "ready", 
  "commands": []
}
```

### 2. User types "ls":
```json
{
  "terminal_id": 1,
  "status": "busy",
  "commands": [
    {
      "id": "cmd_1_1",
      "command": "ls", 
      "status": "running",
      "output": [],
      "started_at": "1703097600"
    }
  ]
}
```

### 3. Command completes:
```json
{
  "terminal_id": 1,
  "status": "ready",
  "commands": [
    {
      "id": "cmd_1_1", 
      "command": "ls",
      "status": "success",
      "output": ["README.md", "src/", "Cargo.toml"],
      "exit_code": 0,
      "started_at": "1703097600",
      "completed_at": "1703097601", 
      "duration_ms": 1000
    }
  ]
}
```

## Implementation Priority

1. **High Priority**: Fix terminal content issue (0 cells problem)
2. **High Priority**: Implement basic TerminalMonitor  
3. **Medium Priority**: Remove Hub server crates
4. **Medium Priority**: JSON state streaming
5. **Low Priority**: WebSocket integration
6. **Low Priority**: Advanced metadata collection

## Risk Mitigation

### Risk: Terminal content still doesn't work
**Mitigation**: Focus on fixing fundamental terminal shell process issue first, before JSON migration

### Risk: Command detection is unreliable  
**Mitigation**: Start with simple prompt patterns, expand gradually

### Risk: Performance issues with JSON serialization
**Mitigation**: Use incremental updates, only serialize changes

### Risk: Breaking existing functionality
**Mitigation**: Migrate incrementally, keep old code until new system works

## Conclusion

This migration simplifies the architecture dramatically while solving the core terminal content issue. The JSON-based approach provides clear state representation, easy debugging, and efficient streaming updates.

**Next Action**: Start with Phase 2 - implement TerminalMonitor and fix the terminal content detection issue.