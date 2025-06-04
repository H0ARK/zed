use anyhow::Result;
use chrono::{DateTime, Utc};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, Task, WeakEntity, Window, actions, div,
};
use serde::{Deserialize, Serialize};

use ui::{ButtonLike, IconButton, IconName, Label, Tooltip, prelude::*};
use util::ResultExt;
use workspace::{Panel, Workspace, dock::{PanelEvent, DockPosition}};
use project::Project;
use gpui::{FontWeight, Pixels};

use crate::session::{
    AgentSessionManager, OrchestratorConfig, OrchestratorEvent, SessionConfig, SessionId,
    SessionOrchestrator, SessionStatus,
};
use crate::{AgentPanel, ThreadStore};
use assistant_tool::ToolWorkingSet;
use prompt_store::PromptBuilder;
use std::sync::Arc;

actions!(
    agent_management,
    [
        ToggleFocus,
        NewSession,
        CoordinateAll,
        SendMessage,
        ShowSessionDetails
    ]
);

#[allow(dead_code)]
const AGENT_MANAGEMENT_PANEL_KEY: &str = "AgentManagementPanel";

pub fn init(cx: &mut App) {
    cx.observe_new(|workspace: &mut Workspace, _, _| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<AgentManagementPanel>(window, cx);
        });
    })
    .detach();
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SerializedAgentManagementPanel {
    width: Option<f32>,
    collapsed_sections: Vec<Section>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum Section {
    ActiveSessions,
    Coordination,
    Communication,
    Settings,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct SessionInfo {
    pub id: SessionId,
    pub name: String,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub message_count: usize,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum ListItem {
    Header(Section),
    Session(SessionInfo),
    CoordinationHistory(String),
    MessageChannel {
        from: SessionId,
        to: SessionId,
        message: String,
    },
    EmptyState(String),
}

pub struct AgentManagementPanel {
    focus_handle: FocusHandle,
    workspace: WeakEntity<Workspace>,
    session_orchestrator: Option<Entity<SessionOrchestrator>>,
    session_manager: Option<Entity<AgentSessionManager>>,
    #[allow(dead_code)]
    thread_store: Entity<ThreadStore>,
    width: Option<f32>,
    pending_serialization: Option<Task<Result<()>>>,

    // UI State
    sessions: Vec<SessionInfo>,
    coordination_history: Vec<String>,
    collapsed_sections: Vec<Section>,
    selected_session: Option<SessionId>,
    show_coordination_panel: bool,
    #[allow(dead_code)]
    show_communication_panel: bool,

    // Communication
    #[allow(dead_code)]
    pending_message: String,
    #[allow(dead_code)]
    message_from: Option<SessionId>,
    #[allow(dead_code)]
    message_to: Option<SessionId>,

    _subscriptions: Vec<gpui::Subscription>,
}

impl AgentManagementPanel {
    pub fn new(
        workspace: WeakEntity<workspace::Workspace>,
        thread_store: Entity<ThreadStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        // This method should only be used when we can safely access the workspace
        // In contexts where the workspace is already being updated, use new_with_project instead
        let project = workspace.upgrade()
            .expect("Workspace should be available")
            .read(cx)
            .project()
            .clone();
        
        Self::new_with_project(workspace, thread_store, project, cx)
    }

    pub fn new_with_project(
        workspace: WeakEntity<workspace::Workspace>,
        thread_store: Entity<ThreadStore>,
        project: Entity<Project>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        // Create new session manager
        let session_manager = Some(cx.new(|cx| {
            AgentSessionManager::new(
                thread_store.clone(),
                project,
                cx,
            )
        }));

        let session_orchestrator = session_manager.as_ref().map(|manager| {
            cx.new(|cx| {
                SessionOrchestrator::new(
                    manager.clone(),
                    thread_store.clone(),
                    OrchestratorConfig::default(),
                    cx,
                )
            })
        });

        let mut subscriptions = Vec::new();

        if let Some(orchestrator) = &session_orchestrator {
            subscriptions.push(cx.subscribe(orchestrator, Self::handle_orchestrator_event));
        }

        if let Some(manager) = &session_manager {
            subscriptions.push(cx.subscribe(manager, Self::handle_session_manager_event));
        }

        let mut panel = Self {
            focus_handle,
            workspace,
            session_orchestrator,
            session_manager,
            thread_store,
            width: None,
            pending_serialization: None,
            sessions: Vec::new(),
            coordination_history: Vec::new(),
            collapsed_sections: Vec::new(),
            selected_session: None,
            show_coordination_panel: false,
            show_communication_panel: false,
            pending_message: String::new(),
            message_from: None,
            message_to: None,
            _subscriptions: subscriptions,
        };

        panel.update_sessions(cx);
        panel
    }

    pub async fn load(
        workspace: WeakEntity<Workspace>,
        prompt_builder: Arc<PromptBuilder>,
        mut cx: gpui::AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        let tools = cx.new(|_| ToolWorkingSet::default())?;
        let (thread_store, project) = workspace
            .update(&mut cx, |workspace, cx| {
                let project = workspace.project().clone();
                let thread_store_future = ThreadStore::load(
                    project.clone(),
                    tools.clone(),
                    None,
                    prompt_builder.clone(),
                    cx,
                );
                (thread_store_future, project)
            })?;
        
        let thread_store = thread_store.await?;

        // Create the panel directly without trying to update the workspace again
        // The workspace is already being updated in the calling context
        cx.new(|cx| Self::new_with_project(workspace.clone(), thread_store, project, cx))
    }

    #[allow(dead_code)]
    fn serialization_key(&self) -> String {
        AGENT_MANAGEMENT_PANEL_KEY.to_string()
    }

    #[allow(dead_code)]
    fn serialize(&self, _cx: &Context<Self>) -> SerializedAgentManagementPanel {
        SerializedAgentManagementPanel {
            width: self.width,
            collapsed_sections: self.collapsed_sections.clone(),
        }
    }

    fn update_sessions(&mut self, cx: &mut Context<Self>) {
        self.sessions.clear();

        if let Some(manager) = &self.session_manager {
            let session_ids = manager.read(cx).list_sessions();
            let active_session = manager.read(cx).active_session_id();

            for session_id in session_ids {
                if let Some(session) = manager.read(cx).get_session(&session_id) {
                    let session_data = session.read(cx);
                    let info = SessionInfo {
                        id: session_id.clone(),
                        name: session_data.name().to_string(),
                        status: session_data.status().clone(),
                        created_at: session_data.metadata.created_at,
                        last_active: session_data.metadata.last_active,
                        message_count: session_data.metadata.message_count,
                        is_active: active_session == Some(&session_id),
                    };
                    self.sessions.push(info);
                }
            }
        }

        cx.notify();
    }

    #[allow(dead_code)]
    fn toggle_focus(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if self.focus_handle.is_focused(window) {
            window.focus(&self.focus_handle);
        } else {
            self.focus_handle.focus(window);
        }
    }

    fn new_session(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(orchestrator) = &self.session_orchestrator {
            let session_name = format!("Session {}", self.sessions.len() + 1);
            let config = SessionConfig::default();

            orchestrator.update(cx, |orchestrator, cx| {
                orchestrator
                    .create_session(session_name, config, cx)
                    .log_err();
            });
        }
    }

    fn remove_session(
        &mut self,
        session_id: &SessionId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(orchestrator) = &self.session_orchestrator {
            orchestrator.update(cx, |orchestrator, cx| {
                orchestrator.remove_session(session_id, cx).log_err();
            });
        }
    }

    fn switch_session(
        &mut self,
        session_id: &SessionId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(orchestrator) = &self.session_orchestrator {
            orchestrator.update(cx, |orchestrator, cx| {
                orchestrator
                    .activate_session(session_id.clone(), cx)
                    .log_err();
            });
        }
        self.selected_session = Some(session_id.clone());

        // Also notify the agent panel to switch to this session
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                if let Some(_agent_panel) = workspace.panel::<AgentPanel>(cx) {
                    // The AgentPanel should listen for session changes through the orchestrator
                    cx.notify();
                }
            });
        }
    }

    fn coordinate_all_sessions(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(orchestrator) = &self.session_orchestrator {
            let task = format!(
                "Analyze and coordinate {} active sessions",
                self.sessions.len()
            );
            orchestrator.update(cx, |orchestrator, cx| {
                orchestrator.coordinate_sessions(task, cx).log_err();
            });
        }
        self.show_coordination_panel = true;
        cx.notify();
    }

    fn toggle_section(&mut self, section: Section, cx: &mut Context<Self>) {
        if self.collapsed_sections.contains(&section) {
            self.collapsed_sections.retain(|&s| s != section);
        } else {
            self.collapsed_sections.push(section);
        }
        cx.notify();
    }

    fn is_section_collapsed(&self, section: Section) -> bool {
        self.collapsed_sections.contains(&section)
    }

    fn handle_orchestrator_event(
        &mut self,
        _orchestrator: Entity<SessionOrchestrator>,
        event: &OrchestratorEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            OrchestratorEvent::SessionCreated(_)
            | OrchestratorEvent::SessionRemoved(_)
            | OrchestratorEvent::SessionActivated(_) => {
                self.update_sessions(cx);
            }
            OrchestratorEvent::CoordinationStarted => {
                self.coordination_history
                    .push("🧠 Coordination started...".to_string());
                cx.notify();
            }
            OrchestratorEvent::CoordinationCompleted => {
                self.coordination_history
                    .push("✅ Coordination completed".to_string());
                cx.notify();
            }
            OrchestratorEvent::CoordinationFailed(error) => {
                self.coordination_history
                    .push(format!("❌ Coordination failed: {}", error));
                cx.notify();
            }
            OrchestratorEvent::MainAIResponse(response) => {
                self.coordination_history
                    .push(format!("🤖 Main AI: {}", response));
                cx.notify();
            }
            OrchestratorEvent::SessionStatusChanged(_session_id, _status) => {
                self.update_sessions(cx);
            }
        }
    }

    fn handle_session_manager_event(
        &mut self,
        _manager: Entity<AgentSessionManager>,
        _event: &crate::session::SessionManagerEvent,
        cx: &mut Context<Self>,
    ) {
        self.update_sessions(cx);
    }

    fn render_section_header(&self, section: Section, cx: &mut Context<Self>) -> impl IntoElement {
        let is_collapsed = self.is_section_collapsed(section);
        let title = match section {
            Section::ActiveSessions => "Active Sessions",
            Section::Coordination => "AI Coordination",
            Section::Communication => "Session Communication",
            Section::Settings => "Settings",
        };

        ButtonLike::new(("section_header", section as usize))
            .style(ButtonStyle::Subtle)
            .full_width()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        IconButton::new(
                            ("toggle_section", section as usize),
                            if is_collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            },
                        )
                        .icon_size(IconSize::XSmall),
                    )
                    .child(Label::new(title).weight(FontWeight::BOLD)),
            )
            .on_click(cx.listener(move |this, _event, _window, cx| {
                this.toggle_section(section, cx);
            }))
    }

    fn render_session(&self, session: &SessionInfo, cx: &mut Context<Self>) -> Div {
        let status_icon = match session.status {
            SessionStatus::Thinking => "🤔",
            SessionStatus::Responding => "💬",
            SessionStatus::WaitingForUser => "⏳",
            SessionStatus::Error(_) => "❌",
            SessionStatus::Idle => "💤",
        };

        div().child(
            ButtonLike::new(("session", session.id.to_string().len()))
            .style(if session.is_active {
                ButtonStyle::Filled
            } else {
                ButtonStyle::Subtle
            })
            .full_width()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Label::new(status_icon).size(LabelSize::Small))
                            .child(Label::new(&session.name).weight(if session.is_active {
                                FontWeight::BOLD
                            } else {
                                FontWeight::NORMAL
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                                        IconButton::new(
                            ("switch", session.id.to_string().len()),
                                    IconName::ArrowRight,
                                )
                                .icon_size(IconSize::XSmall)
                                .tooltip(Tooltip::text("Switch to session"))
                                .on_click({
                                    let session_id = session.id.clone();
                                    cx.listener(move |this, _event, window, cx| {
                                        this.switch_session(&session_id, window, cx);
                                    })
                                }),
                            )
                            .when(!session.is_active, |element| {
                                element.child(
                                                                         IconButton::new(("remove", session.id.to_string().len()), IconName::X)
                                        .icon_size(IconSize::XSmall)
                                        .tooltip(Tooltip::text("Remove session"))
                                        .on_click({
                                            let session_id = session.id.clone();
                                            cx.listener(move |this, _event, window, cx| {
                                                this.remove_session(&session_id, window, cx);
                                            })
                                        }),
                                )
                            }),
                    ),
            )
        )
    }

    fn render_coordination_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .bg(cx.theme().colors().surface_background)
            .border_1()
            .border_color(cx.theme().colors().border)
            .rounded_md()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(Label::new("🧠 AI Coordination").weight(FontWeight::BOLD))
                    .child(
                        IconButton::new("coordinate_all", IconName::Play)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Start coordination"))
                            .on_click(cx.listener(|this, _event, window, cx| {
                                this.coordinate_all_sessions(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .max_h_32()
                    .overflow_y_hidden()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(
                        self.coordination_history
                            .iter()
                            .rev()
                            .take(10)
                            .map(|entry| {
                                div()
                                    .p_2()
                                    .bg(cx.theme().colors().ghost_element_background)
                                    .rounded_md()
                                    .child(Label::new(entry).size(LabelSize::Small))
                            }),
                    )
                    .when(self.coordination_history.is_empty(), |element| {
                        element.child(
                            div().flex().items_center().justify_center().p_4().child(
                                Label::new("No coordination history yet").color(Color::Muted),
                            ),
                        )
                    }),
            )
    }
}

impl Focusable for AgentManagementPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for AgentManagementPanel {}

impl Panel for AgentManagementPanel {
    fn persistent_name() -> &'static str {
        "AgentManagementPanel"
    }

    fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
        DockPosition::Left
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(
            position,
            DockPosition::Left | DockPosition::Right
        )
    }

    fn set_position(&mut self, _position: DockPosition, _window: &mut Window, cx: &mut Context<Self>) {
        // Position is managed by workspace
        cx.notify();
    }

    fn size(&self, _window: &Window, _cx: &App) -> Pixels {
        self.width.unwrap_or(300.0).into()
    }

    fn set_size(&mut self, size: Option<Pixels>, _window: &mut Window, cx: &mut Context<Self>) {
        self.width = size.map(|s| s.0);
        self.pending_serialization = Some(cx.background_executor().spawn(async { Ok(()) }));
        cx.notify();
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<IconName> {
        Some(IconName::Brain)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Agent Management")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        3
    }
}

impl Render for AgentManagementPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().colors().panel_background)
            .child(
                // Header
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .p_2()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(Label::new("Agent Management").weight(FontWeight::BOLD))
                    .child(
                        IconButton::new("new_session", IconName::Plus)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("New Session"))
                            .on_click(cx.listener(|this, _event, window, cx| {
                                this.new_session(window, cx);
                            })),
                    ),
            )
            .child(
                // Content
                div().flex().flex_col().flex_1().overflow_hidden().child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .p_2()
                        .overflow_y_hidden()
                        // Active Sessions Section
                        .child(self.render_section_header(Section::ActiveSessions, cx))
                        .when(
                            !self.is_section_collapsed(Section::ActiveSessions),
                            |element| {
                                element.child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap_1()
                                        .pl_4()
                                                                .children({
                            let sessions = self.sessions.clone();
                            sessions.into_iter().map(|session| {
                                self.render_session(&session, cx)
                            }).collect::<Vec<_>>()
                        })
                                        .when(self.sessions.is_empty(), |element| {
                                            element.child(
                                                div().p_4().child(
                                                    Label::new("No active sessions")
                                                        .color(Color::Muted),
                                                ),
                                            )
                                        }),
                                )
                            },
                        )
                        // Coordination Section
                        .child(self.render_section_header(Section::Coordination, cx))
                        .when(
                            !self.is_section_collapsed(Section::Coordination),
                            |element| {
                                element
                                    .child(div().pl_4().child(self.render_coordination_panel(cx)))
                            },
                        ),
                ),
            )
    }
}

// Actions are now defined in the actions! macro above
