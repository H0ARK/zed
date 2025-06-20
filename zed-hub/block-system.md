# Block System

**Revolutionary command session management with interactive, persistent blocks**

## Block System Philosophy

The Block System is the heart of The Hub's innovation—transforming the linear, ephemeral nature of traditional terminals into a rich, interactive workspace where each command becomes a persistent, manipulable object. This fundamentally changes how developers interact with command-line tools, moving from "execute and forget" to "execute and enhance."

## Core Concepts

### 1. Command Blocks as First-Class Objects

Every command execution creates a **Command Block**—a rich, interactive container that encapsulates:

```rust
struct CommandBlock {
    // Core command data
    id: BlockId,
    command: String,
    args: Vec<String>,
    working_directory: PathBuf,
    environment: HashMap<String, String>,
    
    // Execution metadata
    start_time: DateTime<Utc>,
    end_time: Option<DateTime<Utc>>,
    exit_code: Option<i32>,
    process_id: Option<u32>,
    
    // Content and output
    output: BlockOutput,
    error_output: String,
    input_history: Vec<String>,
    
    // Interactive state
    state: BlockState,
    user_annotations: Vec<Annotation>,
    ai_insights: Vec<AIInsight>,
    
    // Visual presentation
    display_mode: DisplayMode,
    collapsed: bool,
    pinned: bool,
    tags: Vec<String>,
    
    // Relationships
    parent_block: Option<BlockId>,
    child_blocks: Vec<BlockId>,
    dependencies: Vec<BlockId>,
}

enum BlockState {
    Preparing,     // Command being set up
    Running,       // Currently executing
    Completed,     // Finished successfully
    Failed,        // Finished with error
    Cancelled,     // User cancelled
    Suspended,     // Paused/backgrounded
}

enum DisplayMode {
    Compact,       // Minimal space usage
    Normal,        // Standard detail level
    Expanded,      // Full detail view
    Interactive,   // Rich UI components
    AI,           // AI-enhanced view
}
```

### 2. Block Lifecycle Management

**Creation and Initialization**
```rust
impl BlockManager {
    fn create_block(&mut self, command: &str, context: &ExecutionContext) -> Result<BlockId> {
        let block = CommandBlock {
            id: BlockId::new(),
            command: command.to_string(),
            args: parse_command_args(command),
            working_directory: context.cwd.clone(),
            environment: context.environment.clone(),
            start_time: Utc::now(),
            state: BlockState::Preparing,
            display_mode: DisplayMode::Normal,
            output: BlockOutput::new(),
            ..Default::default()
        };
        
        // AI analysis during creation
        if let Ok(analysis) = self.ai_system.analyze_command_intent(&block) {
            block.ai_insights.push(analysis);
            
            // Suggest optimal display mode
            block.display_mode = analysis.suggested_display_mode;
            
            // Pre-populate expected output structure
            if let Some(structure) = analysis.expected_output_structure {
                block.output.set_expected_structure(structure);
            }
        }
        
        let block_id = block.id;
        self.blocks.insert(block_id, block);
        self.notify_block_created(block_id);
        
        Ok(block_id)
    }
    
    fn start_execution(&mut self, block_id: BlockId) -> Result<()> {
        let block = self.get_block_mut(block_id)?;
        block.state = BlockState::Running;
        block.start_time = Utc::now();
        
        // Set up real-time output streaming
        self.setup_output_streaming(block_id)?;
        
        // Configure AI monitoring
        self.ai_system.monitor_execution(block_id)?;
        
        self.notify_block_state_changed(block_id);
        Ok(())
    }
}
```

**Real-Time Updates**
```rust
impl BlockManager {
    fn handle_output_chunk(&mut self, block_id: BlockId, chunk: OutputChunk) -> Result<()> {
        let block = self.get_block_mut(block_id)?;
        
        match chunk.stream_type {
            StreamType::Stdout => {
                block.output.append_stdout(&chunk.data);
                
                // AI-powered output analysis
                if let Ok(analysis) = self.ai_system.analyze_output_chunk(&chunk) {
                    match analysis.insight_type {
                        InsightType::ErrorDetected => {
                            block.ai_insights.push(AIInsight::error_suggestion(analysis));
                            self.notify_error_detected(block_id);
                        },
                        InsightType::ProgressUpdate => {
                            block.update_progress(analysis.progress_info);
                        },
                        InsightType::StructuredData => {
                            block.output.set_structured_data(analysis.parsed_data);
                            block.display_mode = DisplayMode::Interactive;
                        },
                    }
                }
            },
            StreamType::Stderr => {
                block.error_output.push_str(&chunk.data);
            },
        }
        
        // Trigger real-time UI updates
        self.notify_block_updated(block_id);
        Ok(())
    }
    
    fn complete_execution(&mut self, block_id: BlockId, exit_code: i32) -> Result<()> {
        let block = self.get_block_mut(block_id)?;
        block.state = if exit_code == 0 { BlockState::Completed } else { BlockState::Failed };
        block.end_time = Some(Utc::now());
        block.exit_code = Some(exit_code);
        
        // Final AI analysis
        let final_analysis = self.ai_system.analyze_completed_block(block)?;
        block.ai_insights.extend(final_analysis.insights);
        
        // Auto-organize based on AI analysis
        if final_analysis.should_auto_collapse {
            block.collapsed = true;
        }
        
        if let Some(optimal_mode) = final_analysis.optimal_display_mode {
            block.display_mode = optimal_mode;
        }
        
        self.notify_block_completed(block_id);
        Ok(())
    }
}
```

## Block Output System

### 1. Multi-Format Output Handling

**Rich Output Structure**
```rust
struct BlockOutput {
    // Raw output streams
    raw_stdout: String,
    raw_stderr: String,
    
    // Parsed and structured content
    structured_data: Option<StructuredOutput>,
    
    // Real-time streaming data
    live_streams: HashMap<StreamId, LiveStream>,
    
    // AI-enhanced content
    ai_enhancements: Vec<AIEnhancement>,
    
    // Interactive elements
    interactive_elements: Vec<InteractiveElement>,
}

enum StructuredOutput {
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
        metadata: TableMetadata,
    },
    Tree {
        root: TreeNode,
        expanded_nodes: HashSet<NodeId>,
    },
    Progress {
        current: u64,
        total: Option<u64>,
        steps: Vec<ProgressStep>,
    },
    JSON {
        data: serde_json::Value,
        schema: Option<JsonSchema>,
    },
    Logs {
        entries: Vec<LogEntry>,
        filters: LogFilters,
    },
    Files {
        entries: Vec<FileEntry>,
        view_mode: FileViewMode,
    },
}

struct LiveStream {
    stream_type: StreamType,
    buffer: RingBuffer<String>,
    processors: Vec<Box<dyn StreamProcessor>>,
    ui_components: Vec<UIComponent>,
}
```

**Intelligent Output Processing**
```rust
impl OutputProcessor {
    fn process_output_chunk(&mut self, chunk: &str, context: &CommandContext) -> ProcessedOutput {
        let mut processed = ProcessedOutput::new();
        
        // Detect output patterns
        if let Some(table_data) = self.detect_table_format(chunk) {
            processed.structured_data = Some(StructuredOutput::Table(table_data));
        } else if let Some(json_data) = self.detect_json_format(chunk) {
            processed.structured_data = Some(StructuredOutput::JSON(json_data));
        } else if let Some(progress_data) = self.detect_progress_format(chunk) {
            processed.structured_data = Some(StructuredOutput::Progress(progress_data));
        }
        
        // AI-powered enhancement
        if let Ok(ai_enhancement) = self.ai_system.enhance_output(chunk, context) {
            processed.ai_enhancements.push(ai_enhancement);
        }
        
        // Interactive element detection
        if let Some(interactive) = self.detect_interactive_elements(chunk) {
            processed.interactive_elements.extend(interactive);
        }
        
        processed
    }
    
    fn detect_table_format(&self, text: &str) -> Option<TableData> {
        // Detect various table formats
        if self.looks_like_csv(text) {
            return self.parse_csv(text);
        }
        
        if self.looks_like_aligned_table(text) {
            return self.parse_aligned_table(text);
        }
        
        if self.looks_like_json_array(text) {
            return self.parse_json_table(text);
        }
        
        None
    }
    
    fn detect_progress_format(&self, text: &str) -> Option<ProgressData> {
        // Common progress patterns
        let progress_patterns = [
            r"(\d+)/(\d+)", // "5/10"
            r"(\d+)%",      // "75%"
            r"\[([=\-\s]*)\]", // "[====    ]"
            r"Progress: (\d+\.?\d*)%", // "Progress: 75.5%"
        ];
        
        for pattern in &progress_patterns {
            if let Some(matches) = self.regex_match(pattern, text) {
                return self.parse_progress_from_matches(matches);
            }
        }
        
        None
    }
}
```

### 2. Interactive Output Components

**Dynamic Component Creation**
```rust
impl BlockManager {
    fn create_interactive_component(&mut self, 
                                  block_id: BlockId, 
                                  component_type: ComponentType, 
                                  data: ComponentData) -> Result<ComponentId> {
        let component = match component_type {
            ComponentType::Table => {
                InteractiveTable::new(data.into_table_data()?)
                    .with_sorting(true)
                    .with_filtering(true)
                    .with_actions(vec![
                        Action::new("export", "Export to CSV"),
                        Action::new("refresh", "Refresh Data"),
                    ])
                    .build()
            },
            ComponentType::FileTree => {
                FileTreeComponent::new(data.into_file_data()?)
                    .with_icons(true)
                    .with_preview(true)
                    .with_actions(vec![
                        Action::new("open", "Open File"),
                        Action::new("edit", "Edit File"),
                        Action::new("delete", "Delete File"),
                    ])
                    .build()
            },
            ComponentType::ProgressTracker => {
                ProgressComponent::new(data.into_progress_data()?)
                    .with_eta(true)
                    .with_steps(true)
                    .with_cancellation(true)
                    .build()
            },
        };
        
        let component_id = ComponentId::new();
        let block = self.get_block_mut(block_id)?;
        block.output.interactive_elements.push(InteractiveElement {
            id: component_id,
            component: Box::new(component),
            position: ElementPosition::Inline,
            visible: true,
        });
        
        Ok(component_id)
    }
}
```

## Block Relationships and Dependencies

### 1. Parent-Child Hierarchies

**Command Pipelines**
```rust
struct BlockHierarchy {
    root_blocks: Vec<BlockId>,
    parent_child_map: HashMap<BlockId, Vec<BlockId>>,
    dependency_graph: Graph<BlockId, DependencyType>,
}

enum DependencyType {
    Pipeline,      // Output of one feeds into another
    Sequential,    // Must run after another completes
    Parallel,      // Can run simultaneously
    Conditional,   // Runs based on another's result
}

impl BlockManager {
    fn create_pipeline(&mut self, commands: Vec<String>) -> Result<Vec<BlockId>> {
        let mut block_ids = Vec::new();
        let mut previous_block: Option<BlockId> = None;
        
        for command in commands {
            let block_id = self.create_block(&command, &ExecutionContext::default())?;
            
            if let Some(parent_id) = previous_block {
                self.set_block_dependency(block_id, parent_id, DependencyType::Pipeline)?;
                
                // Set up output piping
                self.setup_output_pipe(parent_id, block_id)?;
            }
            
            block_ids.push(block_id);
            previous_block = Some(block_id);
        }
        
        Ok(block_ids)
    }
    
    fn setup_output_pipe(&mut self, source_block: BlockId, target_block: BlockId) -> Result<()> {
        let pipe = OutputPipe {
            source: source_block,
            target: target_block,
            filter: None,
            transform: None,
        };
        
        self.output_pipes.insert((source_block, target_block), pipe);
        Ok(())
    }
}
```

### 2. Block Collections and Workspaces

**Project-Level Block Organization**
```rust
struct BlockWorkspace {
    id: WorkspaceId,
    name: String,
    description: Option<String>,
    
    // Block organization
    block_collections: HashMap<CollectionId, BlockCollection>,
    pinned_blocks: Vec<BlockId>,
    recent_blocks: VecDeque<BlockId>,
    
    // Workspace-level AI context
    ai_context: WorkspaceAIContext,
    
    // Persistent state
    saved_sessions: Vec<SessionSnapshot>,
    auto_save_enabled: bool,
}

struct BlockCollection {
    id: CollectionId,
    name: String,
    blocks: Vec<BlockId>,
    collection_type: CollectionType,
    metadata: CollectionMetadata,
}

enum CollectionType {
    TaskFlow,      // Related commands for a specific task
    Debug,         // Commands used for debugging
    Deployment,    // Deployment-related commands
    Testing,       // Test execution commands
    Analysis,      // Data analysis and reporting
    Custom(String), // User-defined category
}
```

## Block Persistence and Recovery

### 1. Session Management

**Automatic Session Persistence**
```rust
struct SessionManager {
    active_session: Session,
    saved_sessions: HashMap<SessionId, SavedSession>,
    auto_save_interval: Duration,
    max_saved_sessions: usize,
}

struct Session {
    id: SessionId,
    workspace: BlockWorkspace,
    active_blocks: Vec<BlockId>,
    block_states: HashMap<BlockId, BlockState>,
    ui_state: UIState,
    ai_learning_data: AILearningData,
}

impl SessionManager {
    fn auto_save_session(&mut self) -> Result<()> {
        let snapshot = SessionSnapshot {
            timestamp: Utc::now(),
            workspace_state: self.active_session.workspace.clone(),
            block_states: self.active_session.block_states.clone(),
            ui_state: self.active_session.ui_state.clone(),
            ai_context: self.active_session.ai_learning_data.extract_context(),
        };
        
        self.persistence_layer.save_snapshot(snapshot)?;
        
        // Clean up old snapshots
        self.cleanup_old_snapshots()?;
        
        Ok(())
    }
    
    fn restore_session(&mut self, session_id: SessionId) -> Result<()> {
        let saved_session = self.saved_sessions.get(&session_id)
            .ok_or(Error::SessionNotFound)?;
        
        // Restore block states
        for (block_id, state) in &saved_session.block_states {
            if let Ok(block) = self.block_manager.get_block_mut(*block_id) {
                *block = state.clone();
            }
        }
        
        // Restore UI state
        self.ui_manager.restore_state(&saved_session.ui_state)?;
        
        // Restore AI context
        self.ai_system.restore_context(&saved_session.ai_context)?;
        
        self.active_session = Session::from_saved(saved_session);
        Ok(())
    }
}
```

### 2. Block State Recovery

**Intelligent Recovery**
```rust
impl BlockRecoveryManager {
    fn recover_interrupted_blocks(&mut self) -> Result<Vec<BlockId>> {
        let mut recovered_blocks = Vec::new();
        
        for (block_id, block) in &self.block_manager.blocks {
            match block.state {
                BlockState::Running => {
                    // Check if process is still alive
                    if let Some(pid) = block.process_id {
                        if !self.process_manager.is_process_alive(pid) {
                            self.mark_block_interrupted(*block_id)?;
                            recovered_blocks.push(*block_id);
                        }
                    }
                },
                BlockState::Suspended => {
                    // Offer to resume suspended blocks
                    self.offer_resume_option(*block_id)?;
                    recovered_blocks.push(*block_id);
                },
                _ => {}
            }
        }
        
        Ok(recovered_blocks)
    }
    
    fn offer_resume_option(&mut self, block_id: BlockId) -> Result<()> {
        let block = self.block_manager.get_block(block_id)?;
        
        let resume_prompt = ResumePrompt {
            block_id,
            command: block.command.clone(),
            last_output: block.output.get_last_lines(5),
            suggested_action: self.ai_system.suggest_resume_action(block)?,
        };
        
        self.ui_manager.show_resume_prompt(resume_prompt)?;
        Ok(())
    }
}
```

## AI-Enhanced Block Intelligence

### 1. Smart Block Analysis

**Contextual Intelligence**
```rust
struct BlockAI {
    pattern_matcher: PatternMatcher,
    content_analyzer: ContentAnalyzer,
    suggestion_engine: SuggestionEngine,
    learning_system: LearningSystem,
}

impl BlockAI {
    fn analyze_block_content(&self, block: &CommandBlock) -> Result<BlockAnalysis> {
        let mut analysis = BlockAnalysis::new();
        
        // Content pattern analysis
        if let Some(patterns) = self.pattern_matcher.find_patterns(&block.output.raw_stdout) {
            analysis.detected_patterns = patterns;
            
            // Suggest appropriate display modes
            for pattern in &patterns {
                match pattern.pattern_type {
                    PatternType::TabularData => {
                        analysis.suggested_display_mode = Some(DisplayMode::Interactive);
                        analysis.suggested_components.push(ComponentType::Table);
                    },
                    PatternType::FileList => {
                        analysis.suggested_components.push(ComponentType::FileTree);
                    },
                    PatternType::ProgressIndicator => {
                        analysis.suggested_components.push(ComponentType::ProgressTracker);
                    },
                }
            }
        }
        
        // Error analysis
        if !block.error_output.is_empty() {
            analysis.error_insights = self.analyze_errors(&block.error_output)?;
        }
        
        // Performance analysis
        if let Some(end_time) = block.end_time {
            let duration = end_time - block.start_time;
            analysis.performance_insights = self.analyze_performance(duration, &block.command)?;
        }
        
        Ok(analysis)
    }
    
    fn suggest_follow_up_actions(&self, block: &CommandBlock) -> Result<Vec<SuggestedAction>> {
        let mut suggestions = Vec::new();
        
        match block.state {
            BlockState::Completed if block.exit_code == Some(0) => {
                // Successful command - suggest related actions
                suggestions.extend(self.suggest_success_follow_ups(block)?);
            },
            BlockState::Failed => {
                // Failed command - suggest debugging actions
                suggestions.extend(self.suggest_error_resolutions(block)?);
            },
            BlockState::Running => {
                // Running command - suggest monitoring actions
                suggestions.extend(self.suggest_monitoring_actions(block)?);
            },
            _ => {}
        }
        
        Ok(suggestions)
    }
}
```

### 2. Learning and Adaptation

**Block Pattern Learning**
```rust
struct BlockLearningSystem {
    user_patterns: HashMap<UserId, UserPatterns>,
    global_patterns: GlobalPatterns,
    feedback_tracker: FeedbackTracker,
}

struct UserPatterns {
    command_sequences: Vec<CommandSequence>,
    preferred_display_modes: HashMap<CommandType, DisplayMode>,
    frequently_used_actions: HashMap<ActionType, u32>,
    error_resolution_preferences: HashMap<ErrorType, ResolutionStrategy>,
}

impl BlockLearningSystem {
    fn learn_from_block_interaction(&mut self, 
                                   user_id: UserId, 
                                   block_id: BlockId, 
                                   interaction: UserInteraction) -> Result<()> {
        let patterns = self.user_patterns.entry(user_id).or_default();
        
        match interaction {
            UserInteraction::DisplayModeChange { from, to } => {
                patterns.preferred_display_modes.insert(
                    self.classify_command_type(&block_id)?,
                    to
                );
            },
            UserInteraction::ActionUsed(action) => {
                *patterns.frequently_used_actions.entry(action).or_default() += 1;
            },
            UserInteraction::ErrorResolution { error_type, resolution } => {
                patterns.error_resolution_preferences.insert(error_type, resolution);
            },
        }
        
        // Update global patterns (anonymized)
        self.update_global_patterns(&interaction)?;
        
        Ok(())
    }
    
    fn predict_user_preferences(&self, 
                               user_id: UserId, 
                               context: &BlockContext) -> UserPreferencePrediction {
        let patterns = self.user_patterns.get(&user_id);
        
        UserPreferencePrediction {
            preferred_display_mode: patterns
                .and_then(|p| p.preferred_display_modes.get(&context.command_type))
                .copied()
                .unwrap_or(DisplayMode::Normal),
            likely_actions: patterns
                .map(|p| p.get_likely_actions_for_context(context))
                .unwrap_or_default(),
            error_handling_preference: patterns
                .and_then(|p| p.error_resolution_preferences.get(&context.error_type))
                .copied(),
        }
    }
}
```

## Block UI Integration

### 1. Dynamic Layout Management

**Intelligent Block Layout**
```rust
struct BlockLayoutManager {
    layout_strategy: LayoutStrategy,
    viewport_manager: ViewportManager,
    animation_system: AnimationSystem,
}

enum LayoutStrategy {
    Chronological,  // Blocks in time order
    Grouped,        // Blocks grouped by type/project
    Importance,     // AI-determined importance order
    Custom(Box<dyn CustomLayoutStrategy>),
}

impl BlockLayoutManager {
    fn compute_optimal_layout(&self, blocks: &[CommandBlock]) -> Layout {
        let mut layout = Layout::new();
        
        // AI-driven layout optimization
        let layout_analysis = self.ai_system.analyze_optimal_layout(blocks);
        
        for (block_id, position_hint) in layout_analysis.suggestions {
            let block = blocks.iter().find(|b| b.id == block_id).unwrap();
            
            let layout_item = LayoutItem {
                block_id,
                position: self.compute_position(position_hint, block),
                size: self.compute_size(block),
                z_index: self.compute_z_index(block),
                visibility: self.compute_visibility(block),
            };
            
            layout.add_item(layout_item);
        }
        
        layout
    }
    
    fn animate_layout_change(&mut self, from: &Layout, to: &Layout) -> Result<()> {
        let animation = LayoutAnimation {
            duration: Duration::from_millis(300),
            easing: EasingFunction::EaseInOut,
            changes: self.compute_layout_changes(from, to),
        };
        
        self.animation_system.start_animation(animation)?;
        Ok(())
    }
}
```

### 2. Block Interaction Patterns

**Rich Interaction Model**
```rust
enum BlockInteraction {
    Select(BlockId),
    MultiSelect(Vec<BlockId>),
    Expand(BlockId),
    Collapse(BlockId),
    Pin(BlockId),
    Unpin(BlockId),
    Archive(BlockId),
    Duplicate(BlockId),
    EditCommand(BlockId),
    RerunCommand(BlockId),
    ExportOutput(BlockId, ExportFormat),
    ShareBlock(BlockId, ShareOptions),
    AddAnnotation(BlockId, Annotation),
    TagBlock(BlockId, Tag),
    CreateCollection(Vec<BlockId>, String),
}

impl BlockInteractionHandler {
    fn handle_interaction(&mut self, interaction: BlockInteraction) -> Result<InteractionResult> {
        match interaction {
            BlockInteraction::RerunCommand(block_id) => {
                let original_block = self.block_manager.get_block(block_id)?;
                
                // Create new block with same command
                let new_block_id = self.block_manager.create_block(
                    &original_block.command,
                    &ExecutionContext {
                        cwd: original_block.working_directory.clone(),
                        environment: original_block.environment.clone(),
                        ..Default::default()
                    }
                )?;
                
                // Link to original block
                self.block_manager.set_block_relationship(
                    new_block_id, 
                    block_id, 
                    RelationshipType::Rerun
                )?;
                
                // Start execution
                self.block_manager.start_execution(new_block_id)?;
                
                Ok(InteractionResult::BlockCreated(new_block_id))
            },
            
            BlockInteraction::EditCommand(block_id) => {
                let block = self.block_manager.get_block(block_id)?;
                
                // Show command editor with AI suggestions
                let editor_result = self.ui_manager.show_command_editor(CommandEditor {
                    original_command: block.command.clone(),
                    working_directory: block.working_directory.clone(),
                    ai_suggestions: self.ai_system.suggest_command_improvements(&block)?,
                })?;
                
                if let Some(new_command) = editor_result.new_command {
                    let new_block_id = self.create_edited_block(block_id, new_command)?;
                    Ok(InteractionResult::BlockCreated(new_block_id))
                } else {
                    Ok(InteractionResult::Cancelled)
                }
            },
            
            BlockInteraction::CreateCollection(block_ids, name) => {
                let collection = BlockCollection {
                    id: CollectionId::new(),
                    name,
                    blocks: block_ids.clone(),
                    collection_type: CollectionType::Custom("user_defined".to_string()),
                    metadata: CollectionMetadata::default(),
                };
                
                self.workspace.add_collection(collection)?;
                Ok(InteractionResult::CollectionCreated)
            },
            
            _ => self.handle_standard_interaction(interaction),
        }
    }
}
```

## Performance and Scalability

### 1. Efficient Block Storage

**Memory-Optimized Block Management**
```rust
struct BlockStorage {
    // Hot storage for active blocks
    active_blocks: HashMap<BlockId, CommandBlock>,
    
    // Warm storage for recent blocks
    recent_blocks: LruCache<BlockId, CommandBlock>,
    
    // Cold storage for archived blocks
    archived_blocks: Box<dyn ArchiveStorage>,
    
    // Memory usage tracking
    memory_monitor: MemoryMonitor,
}

impl BlockStorage {
    fn get_block(&self, block_id: BlockId) -> Result<&CommandBlock> {
        // Check hot storage first
        if let Some(block) = self.active_blocks.get(&block_id) {
            return Ok(block);
        }
        
        // Check warm storage
        if let Some(block) = self.recent_blocks.get(&block_id) {
            return Ok(block);
        }
        
        // Load from cold storage
        let block = self.archived_blocks.load_block(block_id)?;
        
        // Promote to warm storage if frequently accessed
        if self.should_promote_to_warm(&block) {
            self.recent_blocks.put(block_id, block.clone());
        }
        
        Ok(block)
    }
    
    fn archive_old_blocks(&mut self) -> Result<()> {
        let cutoff_time = Utc::now() - Duration::days(7);
        let mut blocks_to_archive = Vec::new();
        
        for (block_id, block) in &self.active_blocks {
            if block.end_time.map_or(false, |t| t < cutoff_time) {
                blocks_to_archive.push(*block_id);
            }
        }
        
        for block_id in blocks_to_archive {
            if let Some(block) = self.active_blocks.remove(&block_id) {
                self.archived_blocks.store_block(block)?;
            }
        }
        
        Ok(())
    }
}
```

### 2. Streaming and Real-Time Updates

**Efficient Update Propagation**
```rust
struct BlockUpdateSystem {
    update_channels: HashMap<BlockId, UpdateChannel>,
    batch_processor: BatchProcessor,
    rate_limiter: RateLimiter,
}

impl BlockUpdateSystem {
    fn start_real_time_updates(&mut self, block_id: BlockId) -> Result<UpdateChannel> {
        let (sender, receiver) = bounded(1000);
        
        let channel = UpdateChannel {
            block_id,
            sender,
            receiver,
            rate_limit: RateLimit::new(100, Duration::from_secs(1)), // 100 updates/sec max
        };
        
        self.update_channels.insert(block_id, channel.clone());
        
        // Start background update processor
        let processor = self.batch_processor.clone();
        tokio::spawn(async move {
            Self::process_updates(channel, processor).await
        });
        
        Ok(channel)
    }
    
    async fn process_updates(channel: UpdateChannel, processor: BatchProcessor) -> Result<()> {
        let mut batch = Vec::new();
        let mut last_flush = Instant::now();
        
        while let Ok(update) = channel.receiver.recv_timeout(Duration::from_millis(10)) {
            batch.push(update);
            
            // Flush batch if it's full or enough time has passed
            if batch.len() >= 50 || last_flush.elapsed() > Duration::from_millis(100) {
                processor.process_batch(batch.drain(..).collect()).await?;
                last_flush = Instant::now();
            }
        }
        
        // Flush remaining updates
        if !batch.is_empty() {
            processor.process_batch(batch).await?;
        }
        
        Ok(())
    }
}
```

The Block System represents the core innovation of Zed-Hub—transforming the traditional terminal experience into a rich, interactive workspace where every command becomes a first-class object that can be manipulated, enhanced, and learned from. This system provides the foundation for all other Zed-Hub features while maintaining compatibility with existing CLI tools and workflows.