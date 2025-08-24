use gpui::{
    AnyElement, App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    Render, SharedString, Styled, Subscription, Task, WeakEntity, Window, div, prelude::*,
};
use project::{Project, ProjectPath};
use std::path::PathBuf;
use ui::{Button, Label, prelude::*};
use workspace::{
    Item, NavigationState, SplitDirection, Workspace, WorkspaceId,
    item::{ItemEvent, SerializableItem, TabContentParams},
};

pub struct CanvasPane {
    project: Entity<Project>,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    content: String,
    navigation_state: Entity<NavigationState>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone)]
pub enum CanvasEvent {
    FileClicked { path: PathBuf },
}

impl EventEmitter<CanvasEvent> for CanvasPane {}
impl EventEmitter<ItemEvent> for CanvasPane {}

impl CanvasPane {
    pub fn new(
        project: Entity<Project>,
        workspace: WeakEntity<Workspace>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let navigation_state = NavigationState::get_or_init(project.clone(), cx);
        
        Self {
            project,
            workspace,
            focus_handle,
            content: String::new(),
            navigation_state,
            _subscriptions: Vec::new(),
        }
    }

    pub fn set_content(&mut self, content: String, cx: &mut Context<Self>) {
        self.content = content;
        cx.notify();
    }

    fn handle_file_click(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                // Try to find the file in the project
                if let Some(project_path) = self.find_project_path(&path, cx) {
                    // Open the file in a split pane
                    let active_pane = workspace.active_pane().clone();
                    
                    // Create a split to the right of the canvas
                    if let Some(new_pane) = workspace.split_and_clone(
                        active_pane, 
                        SplitDirection::Right, 
                        window, 
                        cx
                    ) {
                        // Open the file in the new pane
                        workspace.open_path(project_path, Some(new_pane.downgrade()), true, window, cx)
                            .detach_and_log_err(cx);
                    }
                }
            });
        }
        
        cx.emit(CanvasEvent::FileClicked { path });
    }

    fn find_project_path(&self, path: &PathBuf, cx: &App) -> Option<ProjectPath> {
        let project = self.project.read(cx);
        
        // Try to find the file in any of the project's worktrees
        for worktree in project.worktrees(cx) {
            let worktree = worktree.read(cx);
            let abs_path = worktree.abs_path();
            
            // Check if the path is relative to this worktree
            if let Ok(relative_path) = path.strip_prefix(abs_path.as_ref()) {
                return Some(ProjectPath {
                    worktree_id: worktree.id(),
                    path: relative_path.into(),
                });
            }
            
            // Also check if the path is already relative
            let potential_absolute = abs_path.join(path);
            if potential_absolute.exists() {
                return Some(ProjectPath {
                    worktree_id: worktree.id(),
                    path: path.clone().into(),
                });
            }
        }
        
        None
    }

    fn render_content(&self, cx: &Context<Self>) -> Div {
        let content = self.content.clone();
        
        div()
            .p_4()
            .child(
                div()
                    .child(Label::new("Agent Canvas"))
                    .mb_4()
                    .text_size(rems(1.2))
                    .font_weight(FontWeight::BOLD)
            )
            .child(
                div()
                    .child(self.render_content_with_links(content, cx))
            )
    }

    fn render_content_with_links(&self, content: String, cx: &Context<Self>) -> AnyElement {
        // Simple file path detection - look for patterns like "src/file.rs" or "/path/to/file.ext"
        match regex::Regex::new(r"(?:^|\s)([a-zA-Z0-9_/.-]+\.[a-zA-Z0-9]+)(?:\s|$)") {
            Ok(file_pattern) => {
                let mut elements = Vec::new();
                let mut last_end = 0;
                
                for mat in file_pattern.find_iter(&content) {
                    // Add text before the match
                    if mat.start() > last_end {
                        let text = content[last_end..mat.start()].to_string();
                        if !text.is_empty() {
                            elements.push(Label::new(text).into_any_element());
                        }
                    }
                    
                    // Add clickable file link
                    let file_path = mat.as_str().trim().to_string();
                    let path_buf = PathBuf::from(file_path.clone());
                    
                    elements.push(
                        Button::new(SharedString::from(format!("file-{}", file_path)), file_path)
                            .color(Color::Accent)
                            .style(ui::ButtonStyle::Subtle)
                            .on_click(cx.listener(move |this, _event, window, cx| {
                                this.handle_file_click(path_buf.clone(), window, cx);
                            }))
                            .into_any_element()
                    );
                    
                    last_end = mat.end();
                }
                
                // Add remaining text
                if last_end < content.len() {
                    let text = content[last_end..].to_string();
                    if !text.is_empty() {
                        elements.push(Label::new(text).into_any_element());
                    }
                }
                
                // If no file patterns found, just return the content as text
                if elements.is_empty() {
                    return Label::new(content).into_any_element();
                }
                
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(elements)
                    .into_any_element()
            }
            Err(_) => {
                // If regex fails, just return the content as plain text
                Label::new(content).into_any_element()
            }
        }
    }
}

impl Focusable for CanvasPane {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CanvasPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .child(self.render_content(cx))
    }
}

impl Item for CanvasPane {
    type Event = ItemEvent;

    fn tab_content(&self, _params: TabContentParams, _window: &Window, _cx: &App) -> AnyElement {
        Label::new("Canvas")
            .color(Color::Default)
            .into_any_element()
    }

    fn tab_content_text(&self, _length: usize, _cx: &App) -> SharedString {
        "Canvas".into()
    }

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("canvas pane")
    }

    fn clone_on_split(&self, _workspace_id: Option<WorkspaceId>, _window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<Self>> {
        Some(cx.new(|cx| Self::new(
            self.project.clone(),
            self.workspace.clone(),
            _window,
            cx,
        )))
    }

    fn is_singleton(&self, _cx: &App) -> bool {
        false
    }

    fn can_save(&self, _cx: &App) -> bool {
        false
    }

    fn has_conflict(&self, _cx: &App) -> bool {
        false
    }

    fn is_dirty(&self, _cx: &App) -> bool {
        false
    }
}

impl SerializableItem for CanvasPane {
    fn serialized_item_kind() -> &'static str {
        "Canvas"
    }

    fn serialize(&mut self, _workspace: &mut Workspace, _item_id: u64, _is_active: bool, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Task<anyhow::Result<()>>> {
        None // Canvas panes don't need to be serialized
    }

    fn should_serialize(&self, _event: &ItemEvent) -> bool {
        false
    }

    fn cleanup(_workspace_id: WorkspaceId, _alive_item_ids: Vec<u64>, _window: &mut Window, _cx: &mut App) -> Task<anyhow::Result<()>> {
        Task::ready(Ok(())) // No cleanup needed
    }

    fn deserialize(
        _project: Entity<Project>,
        _workspace: WeakEntity<Workspace>,
        _workspace_id: WorkspaceId,
        _item_id: u64,
        _window: &mut Window,
        _cx: &mut App,
    ) -> Task<anyhow::Result<Entity<Self>>> {
        Task::ready(Err(anyhow::anyhow!("Canvas panes cannot be deserialized")))
    }
}