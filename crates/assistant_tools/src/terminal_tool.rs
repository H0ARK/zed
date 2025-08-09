use crate::{ schema::json_schema_for, ui::{ COLLAPSED_LINES, ToolOutputPreview } };
use anyhow::{ Context as _, Result, anyhow };
use assistant_tool::{ ActionLog, Tool, ToolCard, ToolResult, ToolUseStatus };
use futures::{ FutureExt as _, future::Shared };
use gpui::{
    AnyWindowHandle,
    App,
    AppContext,
    Entity,
    EntityId,
    Task,
    TextStyleRefinement,
    WeakEntity,
    Window,
    Animation,
    AnimationExt,
    Transformation,
    percentage,
};
use language::LineEnding;
use language_model::{ LanguageModel, LanguageModelRequest, LanguageModelToolSchemaFormat };
use markdown::{ Markdown, MarkdownElement, MarkdownStyle };
use portable_pty::{ CommandBuilder, PtySize, native_pty_system };
use project::{ Project, terminals::TerminalKind };
use schemars::JsonSchema;
use serde::{ Deserialize, Serialize };
use settings::Settings;
use std::{ collections::HashMap, env, path::{ Path, PathBuf }, process::ExitStatus, sync::{Arc, Mutex}, time::{ Duration, Instant } };
use terminal::Terminal;
use terminal_view::TerminalView;
use theme::ThemeSettings;
use ui::{ Disclosure, Tooltip, prelude::* };
use util::{
    ResultExt,
    get_system_shell,
    markdown::MarkdownInlineCode,
    size::format_file_size,
    time::duration_alt_display,
};
use workspace::Workspace;

const COMMAND_OUTPUT_LIMIT: usize = 16 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, Default)]
pub struct TerminalToolInput {
    /// The one-liner command to execute.
    command: String,
    /// Preferred working directory for the command. If it is a project root name or a path
    /// inside a project worktree, it will be used. If it is an absolute path outside the
    /// project, it will also be honored when it exists; otherwise the tool falls back to
    /// the project's default working directory.
    cd: String,
    /// Keep a persistent shell session for this thread and directory.
    #[serde(default)]
    persist: bool,
    /// Optional action mode. Defaults to "run".
    #[serde(default)]
    mode: Option<TerminalToolMode>,
    /// Input to send when mode is send_input.
    #[serde(default)]
    input: Option<String>,
    /// Optional explicit session identifier. When omitted, the session key is derived from the thread_id and cd.
    #[serde(default)]
    session: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum TerminalToolMode {
    Run,
    SendInput,
    Detach,
}

pub struct TerminalTool {
    determine_shell: Shared<Task<String>>,
    sessions: Arc<Mutex<HashMap<SessionKey, WeakEntity<Terminal>>>>,
}

impl TerminalTool {
    pub const NAME: &str = "terminal";

    pub(crate) fn new(cx: &mut App) -> Self {
        let determine_shell = cx.background_spawn(async move {
            if cfg!(windows) {
                return get_system_shell();
            }

            // Prefer bash for better shell integration support
            if which::which("bash").is_ok() {
                log::info!("agent selected bash for terminal tool with shell integration");
                "bash".into()
            } else {
                let shell = get_system_shell();
                log::info!("agent selected {shell} for terminal tool");
                shell
            }
        });
        Self {
            determine_shell: determine_shell.shared(),
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Tool for TerminalTool {
    fn name(&self) -> String {
        Self::NAME.to_string()
    }

    fn needs_confirmation(&self, _: &serde_json::Value, _: &App) -> bool {
        true
    }

    fn may_perform_edits(&self) -> bool {
        false
    }

    fn description(&self) -> String {
        include_str!("./terminal_tool/description.md").to_string()
    }

    fn icon(&self) -> IconName {
        IconName::Terminal
    }

    fn input_schema(&self, format: LanguageModelToolSchemaFormat) -> Result<serde_json::Value> {
        json_schema_for::<TerminalToolInput>(format)
    }

    fn ui_text(&self, input: &serde_json::Value) -> String {
        match serde_json::from_value::<TerminalToolInput>(input.clone()) {
            Ok(input) => {
                let mut lines = input.command.lines();
                let first_line = lines.next().unwrap_or_default();
                let remaining_line_count = lines.count();
                match remaining_line_count {
                    0 => MarkdownInlineCode(&first_line).to_string(),
                    1 =>
                        MarkdownInlineCode(&format!("{} - {} more line", first_line, remaining_line_count)).to_string(),
                    n => MarkdownInlineCode(&format!("{} - {} more lines", first_line, n)).to_string(),
                }
            }
            Err(_) => "Run terminal command".to_string(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: serde_json::Value,
        request: Arc<LanguageModelRequest>,
        project: Entity<Project>,
        _action_log: Entity<ActionLog>,
        _model: Arc<dyn LanguageModel>,
        window: Option<AnyWindowHandle>,
        cx: &mut App
    ) -> ToolResult {
        let input: TerminalToolInput = match serde_json::from_value(input) {
            Ok(input) => input,
            Err(err) => {
                return Task::ready(Err(anyhow!(err))).into();
            }
        };

        let working_dir = match working_dir(&input, &project, cx) {
            Ok(dir) => dir,
            Err(err) => {
                return Task::ready(Err(err)).into();
            }
        };
        let working_dir_for_session = working_dir.clone();
        let working_dir_for_card_outer = working_dir.clone();
        let persist = input.persist || matches!(input.mode, Some(TerminalToolMode::SendInput) | Some(TerminalToolMode::Detach));
        let mode = input.mode.clone().unwrap_or(TerminalToolMode::Run);
        let program = self.determine_shell.clone();
        let command = if cfg!(windows) {
            // Windows PowerShell with shell integration
            format!("$null | & {{{}}}", input.command.replace("\"", "'"))
        } else {
            // Unix shells with OSC 133 shell integration sequences
            let shell_integration_wrapper = format!(
                r#"
# OSC 133 Shell Integration - Command Start
printf '\033]133;C\007'

# Change to working directory if specified
{}

# Execute the actual command
{}

# OSC 133 Shell Integration - Command End with exit code
exit_code=$?
printf '\033]133;D;%s\007' "$exit_code"
exit $exit_code
"#,
                if let Some(cwd) = working_dir.as_ref().and_then(|cwd| cwd.as_os_str().to_str()) {
                    format!("cd '{}'", cwd.replace("'", "'\"'\"'"))
                } else {
                    "# No directory change needed".to_string()
                },
                input.command
            );
            shell_integration_wrapper
        };
        let args = vec!["-c".into(), command.clone()];

        let cwd = working_dir.clone();
        let env = match &working_dir {
            Some(dir) => project.update(cx, |project, cx| { project.directory_environment(dir.as_path().into(), cx) }),
            None => Task::ready(None).shared(),
        };

        let env = cx.spawn(async move |_| {
            let mut env = env.await.unwrap_or_default();
            if cfg!(unix) {
                env.insert("PAGER".into(), "cat".into());
            }
            env
        });

        // Headless mode (no window): only support one-off non-interactive execution
        let Some(window) = window else {
            // Headless setup, a test or eval. Our terminal subsystem requires a workspace,
            // so bypass it and provide a convincing imitation using a pty.
            if persist || matches!(mode, TerminalToolMode::SendInput | TerminalToolMode::Detach) {
                return Task::ready(Err(anyhow!("Interactive or persistent sessions require a windowed environment"))).into();
            }
            let task = cx.background_spawn(async move {
                let env = env.await;
                let pty_system = native_pty_system();
                let program = program.await;
                let mut cmd = CommandBuilder::new(program);
                cmd.args(args);
                for (k, v) in env {
                    cmd.env(k, v);
                }
                if let Some(cwd) = cwd {
                    cmd.cwd(cwd);
                }
                let pair = pty_system.openpty(PtySize {
                    rows: 24,
                    cols: 80,
                    ..Default::default()
                })?;
                let mut child = pair.slave.spawn_command(cmd)?;
                let mut reader = pair.master.try_clone_reader()?;
                drop(pair);
                let mut content = String::new();
                reader.read_to_string(&mut content)?;
                // Massage the pty output a bit to try to match what the terminal codepath gives us
                LineEnding::normalize(&mut content);
                content = content
                    .chars()
                    .filter(|c| (c.is_ascii_whitespace() || !c.is_ascii_control()))
                    .collect();
                let content = content.trim_start().trim_start_matches("^D");
                let exit_status = child.wait()?;
                let (processed_content, _) = process_content(content, &input.command, Some(exit_status));
                Ok(processed_content.into())
            });
            return ToolResult {
                output: task,
                card: None,
            };
        };

        // Session-aware path
        let terminal: Task<Result<Entity<Terminal>>> = if persist {
            let session_key = SessionKey::new(
                input.session.clone().or_else(|| request.thread_id.clone()),
                working_dir_for_session.clone(),
            );
            let existing = {
                let sessions = self.sessions.lock().unwrap();
                sessions.get(&session_key).cloned()
            };

            let project_clone = project.downgrade();
            let sessions_arc = self.sessions.clone();
            let program_task = program.clone();
            let term_task: Task<Result<Entity<Terminal>>> = cx.spawn(async move |cx| {
                // Try to upgrade existing session
                if let Some(weak) = existing {
                    if let Some(strong) = weak.upgrade() {
                        return Ok(strong);
                    }
                }
                let _ = program_task.await;
                let _ = env.await;
                let terminal = project_clone.update(cx, |project, cx| {
                    let wd_for_shell = working_dir_for_session.clone().or_else(|| {
                        project_clone
                            .update(cx, |project, cx| project.active_project_directory(cx).map(|p| p.as_ref().to_path_buf()))
                            .ok()
                            .flatten()
                    });
                    project.create_terminal(
                        TerminalKind::Shell(wd_for_shell),
                        window,
                        cx,
                    )
                })?.await?;
                // Store weak handle
                let weak = terminal.downgrade();
                let mut sessions = sessions_arc.lock().unwrap();
                sessions.insert(session_key, weak);
                Ok(terminal)
            });
            term_task
        } else {
            // One-off task path (existing behavior)
            let t: Task<Result<Entity<Terminal>>> = cx.spawn({
                let project = project.downgrade();
                async move |cx| {
                    let program = program.await;
                    let env = env.await;
                    let terminal = project.update(cx, |project, cx| {
                        project.create_terminal(
                            TerminalKind::Task(task::SpawnInTerminal {
                                command: program,
                                args: vec!["-c".into(), command.clone()],
                                cwd,
                                env,
                                ..Default::default()
                            }),
                            window,
                            cx,
                        )
                    })?.await?;
                    Ok(terminal)
                }
            });
            t
        };

        let cmd_text2 = format!("```bash\n{}\n```", input.command.clone());
        let command_markdown = cx.new(|cx| Markdown::new(cmd_text2.into(), None, None, cx));

        let wd_for_card = working_dir_for_card_outer.clone();
        let card = cx.new(|cx| TerminalToolCard::new(command_markdown.clone(), wd_for_card, cx.entity_id()));

        log::info!("Created terminal tool card for command: {}", input.command);

        let output = cx.spawn({
            let card = card.clone();
            let input_clone = input.clone();
            async move |cx| {
                let terminal = terminal.await?;
                let workspace = window
                    .downcast::<Workspace>()
                    .and_then(|handle| handle.entity(cx).ok())
                    .context("no workspace entity in root of window")?;

                let terminal_view = window.update(cx, |_, window, cx| {
                    cx.new(|cx| {
                        let mut view = TerminalView::new(
                            terminal.clone(),
                            workspace.downgrade(),
                            None,
                            project.downgrade(),
                            window,
                            cx
                        );
                        view.set_embedded_mode(None, cx);
                        view
                    })
                })?;

                card.update(cx, |card, cx| {
                    card.terminal = Some(terminal_view.clone());
                    card.start_instant = Instant::now();
                    cx.notify(); // Notify the card to re-render now that terminal is available
                    log::info!("Terminal attached to card and card notified for re-render");
                }).log_err();
                if !persist {
                    // Original one-off behavior: wait for task completion and process whole content
                    let exit_status = terminal.update(cx, |terminal, cx| terminal.wait_for_completed_task(cx))?.await;
                    let (content, content_line_count) = terminal.read_with(cx, |terminal, _| {
                        (terminal.get_content(), terminal.total_lines())
                    })?;
                    let previous_len = content.len();
                    let (processed_content, finished_with_empty_output) = process_content(
                        &content,
                        &input_clone.command,
                        exit_status.map(portable_pty::ExitStatus::from),
                    );
                    card.update(cx, |card, _| {
                        card.command_finished = true;
                        card.exit_status = exit_status;
                        card.was_content_truncated = processed_content.len() < previous_len;
                        card.original_content_len = previous_len;
                        card.content_line_count = content_line_count;
                        card.finished_with_empty_output = finished_with_empty_output;
                        card.elapsed_time = Some(card.start_instant.elapsed());
                    }).log_err();
                    Ok(processed_content.into())
                } else {
                    // Persistent session behavior
                    match mode {
                        TerminalToolMode::SendInput => {
                            // Send raw input and return immediately
                            if let Some(text) = input_clone.input.clone() {
                                let mut send = text;
                                if !send.ends_with('\n') && !send.ends_with('\r') { send.push('\n'); }
                                terminal.update(cx, |terminal, _| terminal.input(send.into_bytes()))?;
                                card.update(cx, |card, _| {
                                    card.command_finished = true;
                                    card.elapsed_time = Some(card.start_instant.elapsed());
                                }).ok();
                                Ok("Input sent.".to_string().into())
                            } else {
                                return Err(anyhow::anyhow!("No input provided for send_input mode").into());
                            }
                        }
                        TerminalToolMode::Detach => {
                            // Inject the command but do not wait for completion
                            inject_bracketed_command(&terminal, &input_clone.command, working_dir.clone(), cx)?;
                            card.update(cx, |card, _| {
                                card.command_finished = true;
                                card.elapsed_time = Some(card.start_instant.elapsed());
                            }).ok();
                            Ok("Command started in background.".to_string().into())
                        }
                        TerminalToolMode::Run => {
                            // Capture baseline, inject, then wait for OSC 133;D and slice
                            let baseline_len = terminal.read_with(cx, |terminal, _| terminal.get_content().len())?;
                            inject_bracketed_command(&terminal, &input_clone.command, working_dir.clone(), cx)?;

                            let terminal_weak = terminal.downgrade();
                            let (processed, exit_code) = wait_for_bracketed_completion_and_collect(
                                terminal_weak,
                                baseline_len,
                                &input_clone.command,
                                cx,
                            ).await?;

                            card.update(cx, |card, _| {
                                card.command_finished = true;
                                card.exit_status = exit_code.map(|code| {
                                    #[cfg(unix)]
                                    { std::os::unix::process::ExitStatusExt::from_raw(code) }
                                    #[cfg(windows)]
                                    { std::os::windows::process::ExitStatusExt::from_raw(code as u32) }
                                });
                                card.was_content_truncated = false;
                                card.original_content_len = processed.len();
                                card.content_line_count = 0;
                                card.finished_with_empty_output = processed.trim().is_empty();
                                card.elapsed_time = Some(card.start_instant.elapsed());
                            }).log_err();

                            Ok(processed.into())
                        }
                    }
                }
            }
        });

        ToolResult {
            output,
            card: Some(card.into()),
        }
    }
}

fn strip_shell_integration_markers(input: &str) -> String {
    // Remove any lines containing OSC 133 sequences, whether with ESC or with ESC stripped
    let mut out = String::with_capacity(input.len());
    for (idx, line) in input.lines().enumerate() {
        if line.contains("\x1b]133;") || line.contains("]133;C") || line.contains("]133;D") {
            continue;
        }
        if idx > 0 { out.push('\n'); }
        out.push_str(line);
    }
    out
}

fn process_content(content: &str, command: &str, exit_status: Option<portable_pty::ExitStatus>) -> (String, bool) {
    let should_truncate = content.len() > COMMAND_OUTPUT_LIMIT;

    let slice = if should_truncate {
        let mut end_ix = COMMAND_OUTPUT_LIMIT.min(content.len());
        while !content.is_char_boundary(end_ix) { end_ix -= 1; }
        end_ix = content[..end_ix].rfind('\n').unwrap_or(end_ix);
        &content[..end_ix]
    } else {
        content
    };

    let mut cleaned = strip_shell_integration_markers(slice).trim().to_string();
    let is_empty = cleaned.is_empty();
    let mut wrapped = format!("```\n{cleaned}\n```");
    if should_truncate {
        wrapped = format!("Command output too long. The first {} bytes:\n\n{}", wrapped.len(), wrapped);
    }

    let content = match exit_status {
        Some(exit_status) if exit_status.success() => {
            if is_empty { "Command executed successfully.".to_string() } else { wrapped.to_string() }
        }
        Some(exit_status) => {
            if is_empty {
                format!("Command \"{command}\" failed with exit code {}.", exit_status.exit_code())
            } else {
                format!("Command \"{command}\" failed with exit code {}.\n\n{wrapped}", exit_status.exit_code())
            }
        }
        None => { format!("Command failed or was interrupted.\nPartial output captured:\n\n{}", wrapped) }
    };
    (content, is_empty)
}

fn working_dir(input: &TerminalToolInput, project: &Entity<Project>, cx: &mut App) -> Result<Option<PathBuf>> {
    let project = project.read(cx);
    let cd = &input.cd;

    if cd == "." || cd == "" {
        // Accept "." or "" as meaning "the one worktree" if we only have one worktree.
        let mut worktrees = project.worktrees(cx);

        match worktrees.next() {
            Some(worktree) => {
                anyhow::ensure!(
                    worktrees.next().is_none(),
                    "'.' is ambiguous in multi-root workspaces. Please specify a root directory explicitly."
                );
                Ok(Some(worktree.read(cx).abs_path().to_path_buf()))
            }
            None => Ok(None),
        }
    } else {
        let input_path = Path::new(cd);

        if input_path.is_absolute() {
            // Allow absolute paths outside the project as long as they exist.
            return Ok(input_path.is_dir().then_some(input_path.to_path_buf()));
        } else {
            if let Some(worktree) = project.worktree_for_root_name(cd, cx) {
                return Ok(Some(worktree.read(cx).abs_path().to_path_buf()));
            }
        }

        anyhow::bail!("`cd` directory {cd:?} was not in any of the project's worktrees.");
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct SessionKey {
    thread_or_session: Option<String>,
    cwd: Option<PathBuf>,
}

impl SessionKey {
    fn new(thread_or_session: Option<String>, cwd: Option<PathBuf>) -> Self {
        Self { thread_or_session, cwd }
    }
}

fn inject_bracketed_command(
    terminal: &Entity<Terminal>,
    command: &str,
    cwd: Option<PathBuf>,
    cx: &mut gpui::AsyncApp,
) -> anyhow::Result<()> {
    let mut script = String::new();
    script.push_str("printf '\x1b]133;C\x07'\n");
    if let Some(cwd) = cwd.as_ref() {
        let path = cwd.display().to_string().replace('\'', "'\"'\"'");
        script.push_str(&format!("cd '{}'\n", path));
    }
    script.push_str(command);
    script.push_str("\nexit_code=$?\n");
    script.push_str("printf '\x1b]133;D;%s\x07' \"$exit_code\"\n");
    terminal.update(cx, |t, _| t.input(script.into_bytes()))?;
    Ok(())
}

async fn wait_for_bracketed_completion_and_collect(
    terminal: WeakEntity<Terminal>,
    baseline_len: usize,
    command: &str,
    cx: &mut gpui::AsyncApp,
) -> anyhow::Result<(String, Option<i32>)> {
    let mut exit_code: Option<i32> = None;
    // Poll for OSC 133;D;{code}
    loop {
        cx.background_executor().timer(Duration::from_millis(200)).await;
        if let Some(t) = terminal.upgrade() {
            let finished: (bool, Option<i32>) = t.read_with(cx, |t, _| {
                let content = t.get_content();
                if let Some(ix) = content.rfind("\x1b]133;D;") {
                    // Try to parse exit code from the suffix after the marker
                    let tail = &content[ix + 7..]; // after "]133;D;"
                    let code = tail.chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect::<String>();
                    if let Ok(code) = code.parse::<i32>() { (true, Some(code)) } else { (true, None) }
                } else {
                    (false, None)
                }
            })?;
            if finished.0 { exit_code = finished.1; break; }
        } else {
            break;
        }
    }
    // Slice output after baseline
    let t = terminal
        .upgrade()
        .ok_or_else(|| anyhow::anyhow!("terminal session vanished"))?;
    let content = t.read_with(cx, |t, _| t.get_content())?;
    let sliced = if baseline_len <= content.len() { &content[baseline_len..] } else { "" };
    let (processed, _) = process_content(sliced, command, None);
    Ok((processed, exit_code))
}

struct TerminalToolCard {
    input_command: Entity<Markdown>,
    working_dir: Option<PathBuf>,
    entity_id: EntityId,
    exit_status: Option<ExitStatus>,
    terminal: Option<Entity<TerminalView>>,
    command_finished: bool,
    was_content_truncated: bool,
    finished_with_empty_output: bool,
    content_line_count: usize,
    original_content_len: usize,
    preview_expanded: bool,
    start_instant: Instant,
    elapsed_time: Option<Duration>,
}

impl TerminalToolCard {
    pub fn new(input_command: Entity<Markdown>, working_dir: Option<PathBuf>, entity_id: EntityId) -> Self {
        Self {
            input_command,
            working_dir,
            entity_id,
            exit_status: None,
            terminal: None,
            command_finished: false,
            was_content_truncated: false,
            finished_with_empty_output: false,
            original_content_len: 0,
            content_line_count: 0,
            preview_expanded: true,
            start_instant: Instant::now(),
            elapsed_time: None,
        }
    }
}

impl ToolCard for TerminalToolCard {
    fn render(
        &mut self,
        status: &ToolUseStatus,
        window: &mut Window,
        _workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>
    ) -> impl IntoElement {
        log::info!("TerminalToolCard::render called, terminal available: {}", self.terminal.is_some());

        let Some(terminal) = self.terminal.as_ref() else {
            log::info!("Terminal not yet available, showing loading state");
            // Show a loading state instead of empty when terminal is not yet available
            return div()
                .p_2()
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Icon::new(IconName::ArrowCircle)
                                .size(IconSize::Small)
                                .color(Color::Accent)
                                .with_animation(
                                    "arrow-circle",
                                    Animation::new(Duration::from_secs(2)).repeat(),
                                    |icon, delta| { icon.transform(Transformation::rotate(percentage(delta))) }
                                )
                        )
                        .child(
                            Label::new("Starting terminal...")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .buffer_font(cx)
                        )
                )
                .into_any();
        };

        // Check if command is still running by looking at the terminal content for shell integration
        let command_still_running = !self.command_finished && {
            let terminal_handle = terminal.read(cx).terminal().clone();
            let content = terminal_handle.read(cx).get_content();
            // If we see OSC 133;C (command started) but not OSC 133;D (command finished),
            // the command is still running
            content.contains("\x1b]133;C") && !content.contains("\x1b]133;D")
        };

        // Show spinner if command is still running
        if command_still_running {
            log::info!("Command still running, showing spinner");
            return div()
                .p_2()
                .child(
                    h_flex()
                        .gap_1()
                        .child(
                            Icon::new(IconName::ArrowCircle)
                                .size(IconSize::Small)
                                .color(Color::Accent)
                                .with_animation(
                                    "command-running",
                                    Animation::new(Duration::from_secs(1)).repeat(),
                                    |icon, delta| { icon.transform(Transformation::rotate(percentage(delta))) }
                                )
                        )
                        .child(
                            Label::new("Running command...")
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .buffer_font(cx)
                        )
                )
                .into_any();
        }

        log::info!("Terminal available, rendering terminal view");

        let tool_failed = matches!(status, ToolUseStatus::Error(_));

        let command_failed = self.command_finished && self.exit_status.is_none_or(|code| !code.success());

        if (tool_failed || command_failed) && self.elapsed_time.is_none() {
            self.elapsed_time = Some(self.start_instant.elapsed());
        }
        let time_elapsed = self.elapsed_time.unwrap_or_else(|| self.start_instant.elapsed());

        let header_bg = cx
            .theme()
            .colors()
            .element_background.blend(cx.theme().colors().editor_foreground.opacity(0.025));

        let border_color = cx.theme().colors().border.opacity(0.6);

        let path = self.working_dir
            .as_ref()
            .cloned()
            .or_else(|| env::current_dir().ok())
            .map(|path| format!("{}", path.display()))
            .unwrap_or_else(|| "current directory".to_string());

        // Pre-build action buttons to avoid borrow conflicts on cx
        let stop_btn = {
            let terminal_for_stop = self.terminal.clone();
            IconButton::new(("terminal-stop", self.entity_id), IconName::StopFilled)
                .icon_size(IconSize::XSmall)
                .icon_color(Color::Error)
                .tooltip(Tooltip::text("Stop (Ctrl-C)"))
                .on_click(cx.listener(move |_this, _e, _w, cx| {
                    if let Some(terminal_view) = &terminal_for_stop {
                        let _ = terminal_view.update(cx, |view, cx| {
                            view.terminal().update(cx, |t, cx| {
                                t.input(vec![0x03]);
                            })
                        });
                    }
                }))
        };
        let send_btn = {
            let terminal_for_send = self.terminal.clone();
            IconButton::new(("terminal-send-input", self.entity_id), IconName::Terminal)
                .icon_size(IconSize::XSmall)
                .icon_color(Color::Muted)
                .tooltip(Tooltip::text("Send Input (Enter)"))
                .on_click(cx.listener(move |_this, _e, _w, cx| {
                    if let Some(terminal_view) = &terminal_for_send {
                        let _ = terminal_view.update(cx, |view, cx| {
                            view.terminal().update(cx, |t, cx| {
                                t.input("\r".as_bytes());
                            })
                        });
                    }
                }))
        };

        let header = h_flex()
            .flex_none()
            .gap_1()
            .justify_between()
            .rounded_t_md()
            .child(
                div()
                    .id(("command-target-path", self.entity_id))
                    .w_full()
                    .max_w_full()
                    .overflow_x_scroll()
                    .child(Label::new(path).buffer_font(cx).size(LabelSize::XSmall).color(Color::Muted))
            )
            .child(h_flex().gap_1().child(stop_btn).child(send_btn))
            .when(self.was_content_truncated, |header| {
                let tooltip = if self.content_line_count + 10 > terminal::MAX_SCROLL_HISTORY_LINES {
                    "Output exceeded terminal max lines and was \
                        truncated, the model received the first 16 KB.".to_string()
                } else {
                    format!(
                        "Output is {} long, to avoid unexpected token usage, \
                            only 16 KB was sent back to the model.",
                        format_file_size(self.original_content_len as u64, true)
                    )
                };
                header.child(
                    h_flex()
                        .id(("terminal-tool-truncated-label", self.entity_id))
                        .tooltip(Tooltip::text(tooltip))
                        .gap_1()
                        .child(Icon::new(IconName::Info).size(IconSize::XSmall).color(Color::Ignored))
                        .child(Label::new("Truncated").color(Color::Muted).size(LabelSize::Small))
                )
            })
            .when(time_elapsed > Duration::from_secs(10), |header| {
                header.child(
                    Label::new(format!("({})", duration_alt_display(time_elapsed)))
                        .buffer_font(cx)
                        .color(Color::Muted)
                        .size(LabelSize::Small)
                )
            })
            .when(tool_failed || command_failed, |header| {
                header.child(
                    div()
                        .id(("terminal-tool-error-code-indicator", self.entity_id))
                        .child(Icon::new(IconName::Close).size(IconSize::Small).color(Color::Error))
                        .when(command_failed && self.exit_status.is_some(), |this| {
                            this.tooltip(
                                Tooltip::text(
                                    format!(
                                        "Exited with code {}",
                                        self.exit_status.and_then(|status| status.code()).unwrap_or(-1)
                                    )
                                )
                            )
                        })
                        .when(!command_failed && tool_failed && status.error().is_some(), |this| {
                            this.tooltip(Tooltip::text(format!("Error: {}", status.error().unwrap())))
                        })
                )
            })
            .when(!self.finished_with_empty_output, |header| {
                header.child(
                    Disclosure::new(("terminal-tool-disclosure", self.entity_id), self.preview_expanded)
                        .opened_icon(IconName::ChevronUp)
                        .closed_icon(IconName::ChevronDown)
                        .on_click(
                            cx.listener(move |this, _event, _window, _cx| {
                                this.preview_expanded = !this.preview_expanded;
                            })
                        )
                )
            });

        v_flex()
            .mb_2()
            .border_1()
            .when(tool_failed || command_failed, |card| card.border_dashed())
            .border_color(border_color)
            .rounded_lg()
            .overflow_hidden()
            .child(
                v_flex()
                    .p_2()
                    .gap_0p5()
                    .bg(header_bg)
                    .text_xs()
                    .child(header)
                    .child(
                        MarkdownElement::new(
                            self.input_command.clone(),
                            markdown_style(window, cx)
                        ).code_block_renderer(markdown::CodeBlockRenderer::Default {
                            copy_button: false,
                            copy_button_on_hover: true,
                            border: false,
                        })
                    )
            )
            .when(self.preview_expanded && !self.finished_with_empty_output, |this| {
                this.child(
                    div()
                        .pt_2()
                        .border_t_1()
                        .border_color(border_color)
                        .bg(cx.theme().colors().editor_background)
                        .rounded_b_md()
                        .text_ui_sm(cx)
                        .child({
                            let content_mode = terminal.read(cx).content_mode(window, cx);

                            if content_mode.is_scrollable() {
                                div().h_72().child(terminal.clone()).into_any_element()
                            } else {
                                ToolOutputPreview::new(terminal.clone().into_any_element(), terminal.entity_id())
                                    .with_total_lines(self.content_line_count)
                                    .toggle_state(!content_mode.is_limited())
                                    .on_toggle({
                                        let terminal = terminal.clone();
                                        move |is_expanded, _, cx| {
                                            terminal.update(cx, |terminal, cx| {
                                                terminal.set_embedded_mode(
                                                    if is_expanded {
                                                        None
                                                    } else {
                                                        Some(COLLAPSED_LINES)
                                                    },
                                                    cx
                                                );
                                            });
                                        }
                                    })
                                    .into_any_element()
                            }
                        })
                )
            })
            .into_any()
    }
}

fn markdown_style(window: &Window, cx: &App) -> MarkdownStyle {
    let theme_settings = ThemeSettings::get_global(cx);
    let buffer_font_size = TextSize::Default.rems(cx);
    let mut text_style = window.text_style();

    text_style.refine(
        &(TextStyleRefinement {
            font_family: Some(theme_settings.buffer_font.family.clone()),
            font_fallbacks: theme_settings.buffer_font.fallbacks.clone(),
            font_features: Some(theme_settings.buffer_font.features.clone()),
            font_size: Some(buffer_font_size.into()),
            color: Some(cx.theme().colors().text),
            ..Default::default()
        })
    );

    MarkdownStyle {
        base_text_style: text_style.clone(),
        selection_background_color: cx.theme().players().local().selection,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use editor::EditorSettings;
    use fs::RealFs;
    use gpui::{ BackgroundExecutor, TestAppContext };
    use language_model::fake_provider::FakeLanguageModel;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use settings::{ Settings, SettingsStore };
    use terminal::terminal_settings::TerminalSettings;
    use theme::ThemeSettings;
    use util::{ ResultExt as _, test::TempTree };

    use super::*;

    fn init_test(executor: &BackgroundExecutor, cx: &mut TestAppContext) {
        zlog::init_test();

        executor.allow_parking();
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            language::init(cx);
            Project::init_settings(cx);
            workspace::init_settings(cx);
            ThemeSettings::register(cx);
            TerminalSettings::register(cx);
            EditorSettings::register(cx);
        });
    }

    #[gpui::test]
    async fn test_interactive_command(executor: BackgroundExecutor, cx: &mut TestAppContext) {
        if cfg!(windows) {
            return;
        }

        init_test(&executor, cx);

        let fs = Arc::new(RealFs::new(None, executor));
        let tree = TempTree::new(json!({
            "project": {},
        }));
        let project: Entity<Project> = Project::test(fs, [tree.path().join("project").as_path()], cx).await;
        let action_log = cx.update(|cx| cx.new(|_| ActionLog::new(project.clone())));
        let model = Arc::new(FakeLanguageModel::default());

        let input = TerminalToolInput { command: "cat".to_owned(), cd: tree.path().join("project").as_path().to_string_lossy().to_string(), ..Default::default() };
        let result = cx.update(|cx| {
            TerminalTool::run(
                Arc::new(TerminalTool::new(cx)),
                serde_json::to_value(input).unwrap(),
                Arc::default(),
                project.clone(),
                action_log.clone(),
                model,
                None,
                cx
            )
        });

        let output = result.output.await.log_err().unwrap().content;
        assert_eq!(output.as_str().unwrap(), "Command executed successfully.");
    }

    #[gpui::test]
    async fn test_working_directory(executor: BackgroundExecutor, cx: &mut TestAppContext) {
        if cfg!(windows) {
            return;
        }

        init_test(&executor, cx);

        let fs = Arc::new(RealFs::new(None, executor));
        let tree = TempTree::new(json!({
            "project": {},
            "other-project": {},
        }));
        let project: Entity<Project> = Project::test(fs, [tree.path().join("project").as_path()], cx).await;
        let action_log = cx.update(|cx| cx.new(|_| ActionLog::new(project.clone())));
        let model = Arc::new(FakeLanguageModel::default());

        let check = |input, expected, cx: &mut App| {
            let headless_result = TerminalTool::run(
                Arc::new(TerminalTool::new(cx)),
                serde_json::to_value(input).unwrap(),
                Arc::default(),
                project.clone(),
                action_log.clone(),
                model.clone(),
                None,
                cx
            );
            cx.spawn(async move |_| {
                let output = headless_result.output.await.map(|output| output.content);
                assert_eq!(
                    output.ok().and_then(|content| content.as_str().map(ToString::to_string)),
                    expected
                );
            })
        };

        cx.update(|cx| {
            check(
                TerminalToolInput { command: "pwd".into(), cd: ".".into(), ..Default::default() },
                Some(format!("```\n{}\n```", tree.path().join("project").display())),
                cx
            )
        }).await;

        cx.update(|cx| {
            check(
                TerminalToolInput { command: "pwd".into(), cd: "other-project".into(), ..Default::default() },
                None, // other-project is a dir, but *not* a worktree (yet)
                cx
            )
        }).await;

        // Absolute path above the worktree root
        cx.update(|cx| {
            check(
                TerminalToolInput { command: "pwd".into(), cd: tree.path().to_string_lossy().into(), ..Default::default() },
                None,
                cx
            )
        }).await;

        project
            .update(cx, |project, cx| { project.create_worktree(tree.path().join("other-project"), true, cx) }).await
            .unwrap();

        cx.update(|cx| {
            check(
                TerminalToolInput { command: "pwd".into(), cd: "other-project".into(), ..Default::default() },
                Some(format!("```\n{}\n```", tree.path().join("other-project").display())),
                cx
            )
        }).await;

        cx.update(|cx| {
            check(
                TerminalToolInput { command: "pwd".into(), cd: ".".into(), ..Default::default() },
                None,
                cx
            )
        }).await;
    }
}
