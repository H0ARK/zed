Hub Terminal Restructuring Plan

  Phase 1: Clean Slate (Priority: HIGH)

  Goal: Remove complexity, establish foundation

  1.1 Delete Server Architecture

  - Remove hub-server/ folder entirely
  - Remove hub-cli/ folder entirely
  - Remove crates/hub_protocol/ crate
  - Remove crates/hub_sdk/ crate
  - Update workspace Cargo.toml to remove deleted dependencies

  1.2 Preserve Core Assets

  - Extract useful types from hub_core/types.rs into new location
  - Keep existing JSON schema files (terminal_schema.json, example_terminal_state.json)
  - Archive current hub_terminal_panel.rs as reference

  Estimated Time: 2-3 hours
  Risk: Low - just deletion and file moves

  ---
  Phase 2: Grid Semantic Parser (Priority: HIGH)

  Goal: Build the core intelligence that reads terminal grid

  2.1 Create Semantic Parser Foundation

  - Create crates/workspace/src/terminal_semantic_parser.rs
  - Implement basic grid content extraction from Alacritty
  - Add command pattern detection (shell prompts, command boundaries)
  - Test with simple commands (ls, git status)

  2.2 Command Detection Logic

  - Implement prompt pattern matching (user@host:$, ❯, etc.)
  - Add command extraction from prompt lines
  - Build output collection until next prompt
  - Add OSC 133 sequence detection for enhanced shells

  Estimated Time: 1-2 days
  Risk: Medium - core logic that everything depends on

  ---
  Phase 3: JSON State Integration (Priority: HIGH)

  Goal: Convert parsed commands to structured JSON

  3.1 Connect Parser to JSON Schema

  - Use existing terminal_state.rs structures
  - Build commands from semantic parser output
  - Implement real-time JSON state updates
  - Add command status tracking (running→completed)

  3.2 Test JSON Generation

  - Verify JSON matches schema
  - Test with multiple commands
  - Add error handling for parsing failures

  Estimated Time: 1 day
  Risk: Low - mostly connecting existing pieces

  ---
  Phase 4: Simplified UI Integration (Priority: MEDIUM)

  Goal: Render enhanced UI from JSON state

  4.1 Rebuild Hub Terminal Panel

  - Create new simplified HubTerminalView
  - Remove all protocol/client code
  - Connect semantic parser to UI updates
  - Render command blocks from JSON state

  4.2 Command Block UI

  - Reuse existing block rendering components
  - Show command + output in minimal horizontal design
  - Add status indicators (running, success, error)
  - Implement real-time updates as commands execute

  Estimated Time: 1-2 days
  Risk: Medium - UI integration complexity

  ---
  Phase 5: Polish & Enhancement (Priority: LOW)

  Goal: Add the advanced features that make it special

  5.1 Advanced Parsing

  - Handle complex command patterns
  - Add metadata extraction (exit codes, timing)
  - Improve output parsing accuracy
  - Add support for interactive commands

  5.2 AI Integration Foundation

  - Add command context to JSON state
  - Prepare structured data for AI features
  - Add hooks for AI suggestions/analysis

  Estimated Time: 2-3 days
  Risk: Low - enhancement features

  ---
  Success Criteria

  Phase 1 Complete: Codebase is 75% smaller, no server complexity
  Phase 2 Complete: Can detect ls and git status commands from grid
  Phase 3 Complete: Commands appear in structured JSON format
  Phase 4 Complete: Enhanced UI shows command blocks in real-time
  Phase 5 Complete: Ready for AI integration and advanced features

  Recommendation: Start with Phase 1

  Delete the server complexity first - it's the safest change and immediately simplifies everything. Once you have a clean foundation, the
  semantic parser becomes the core piece that everything else builds on.

  Should I start with Phase 1 and begin removing the server architecture?
