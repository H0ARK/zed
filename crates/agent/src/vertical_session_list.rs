use anyhow::Result;
use chrono::{DateTime, Utc};
use gpui::{
    actions, div, App, Context, Entity, EventEmitter, Focusable, FocusHandle,
    IntoElement, ParentElement, Render, Styled, WeakEntity, Window, MouseButton, FontWeight,
};
use project::Project;
use serde::{Deserialize, Serialize};
use workspace::Workspace;

use ui::{prelude::*, IconButton, IconName, Label, Tooltip};
use util::ResultExt;

use crate::session::{
    AgentSessionManager, OrchestratorConfig, OrchestratorEvent, SessionConfig, SessionId,
    SessionOrchestrator, SessionStatus,
};
use crate::ThreadStore;

actions!(
    vertical_session_list,
    [
        NewSession,
        RemoveSession,
        SwitchSession,
        ToggleOrchestratorMode,
        ShowSessionThread
    ]
);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListItem {
    pub id: SessionId,
    pub name: String,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub message_count: usize,
    pub is_active: bool,
    pub is_orchestrator: bool,
    pub thread_preview: Option<String>,
}

#[derive(Debug, Clone)]
pub enum SessionListEvent {
    SessionSelected(SessionId),
    SessionCreated(SessionId),
    SessionRemoved(SessionId),
    OrchestratorModeToggled(bool),
    ShowThreadRequested(SessionId),
}

impl EventEmitter<SessionListEvent> for VerticalSessionList {}

pub struct VerticalSessionList {
    focus_handle: FocusHandle,
    session_orchestrator: Option<Entity<SessionOrchestrator>>,
    session_manager: Option<Entity<AgentSessionManager>>,
    
    // UI State
    sessions: Vec<SessionListItem>,
    selected_session: Option<SessionId>,
    orchestrator_mode: bool,
    main_session_id: Option<SessionId>, // Always exists - the main session
    
    // Subscriptions
    _subscriptions: Vec<gpui::Subscription>,
}

impl VerticalSessionList {
    pub fn new(
        _workspace: WeakEntity<Workspace>,
        thread_store: Entity<ThreadStore>,
        project: Entity<Project>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        
        // Create session manager
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

        let mut list = Self {
            focus_handle,
            session_orchestrator,
            session_manager,
            sessions: Vec::new(),
            selected_session: None,
            orchestrator_mode: false,
            main_session_id: None,
            _subscriptions: subscriptions,
        };

        // Create the main session (always exists)
        list.ensure_main_session(cx);
        list.update_sessions(cx);
        
        list
    }

    fn ensure_main_session(&mut self, cx: &mut Context<Self>) {
        if let Some(manager) = &self.session_manager {
            let sessions = manager.read(cx).list_sessions();
            
            if sessions.is_empty() {
                // Create the main session
                let main_session_id = manager.update(cx, |manager, cx| {
                    manager.create_session("Main Session", SessionConfig::default(), cx)
                }).unwrap_or_else(|_| SessionId::new());
                
                self.main_session_id = Some(main_session_id.clone());
                self.selected_session = Some(main_session_id);
            } else {
                // Use the first session as main if no main session is set
                if self.main_session_id.is_none() {
                    self.main_session_id = sessions.first().cloned();
                    self.selected_session = self.main_session_id.clone();
                }
            }
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
                    let is_orchestrator = self.orchestrator_mode && session_id != *self.main_session_id.as_ref().unwrap_or(&session_id);
                    
                    let item = SessionListItem {
                        id: session_id.clone(),
                        name: session_data.name().to_string(),
                        status: session_data.status().clone(),
                        created_at: session_data.metadata.created_at,
                        last_active: session_data.metadata.last_active,
                        message_count: session_data.metadata.message_count,
                        is_active: active_session == Some(&session_id),
                        is_orchestrator,
                        thread_preview: None, // TODO: Implement thread preview
                    };
                    self.sessions.push(item);
                }
            }
        }

        // Ensure main session is always first
        if let Some(main_id) = &self.main_session_id {
            if let Some(main_index) = self.sessions.iter().position(|s| &s.id == main_id) {
                if main_index != 0 {
                    let main_session = self.sessions.remove(main_index);
                    self.sessions.insert(0, main_session);
                }
            }
        }

        cx.notify();
    }

    #[allow(dead_code)]
    fn get_thread_preview(&self, _session_id: &SessionId, _cx: &mut Context<Self>) -> Option<String> {
        // TODO: Implement thread preview extraction
        // This would get the latest message or current work from the session
        None
    }

    fn render_session_status(&self, status: &SessionStatus, _cx: &mut Context<Self>) -> impl IntoElement {
        let (icon, color) = match status {
            SessionStatus::Thinking => (IconName::LoadCircle, Color::Info),
            SessionStatus::Responding => (IconName::MessageBubbles, Color::Success),
            SessionStatus::WaitingForUser => (IconName::CountdownTimer, Color::Warning),
            SessionStatus::Error(_) => (IconName::XCircle, Color::Error),
            SessionStatus::Idle => (IconName::Circle, Color::Muted),
        };

        div()
            .flex()
            .items_center()
            .child(
                ui::Icon::new(icon)
                    .size(IconSize::XSmall)
                    .color(color)
            )
    }

    pub fn new_session(&mut self, cx: &mut Context<Self>) -> Result<SessionId> {
        if let Some(orchestrator) = &self.session_orchestrator {
            let session_name = if self.orchestrator_mode {
                format!("Agent {}", self.sessions.len())
            } else {
                format!("Session {}", self.sessions.len() + 1)
            };
            
            let config = SessionConfig::default();
            let session_id = orchestrator.update(cx, |orchestrator, cx| {
                orchestrator.create_session(session_name, config, cx)
            })?;

            cx.emit(SessionListEvent::SessionCreated(session_id.clone()));
            Ok(session_id)
        } else {
            Err(anyhow::anyhow!("No orchestrator available"))
        }
    }

    pub fn remove_session(&mut self, session_id: &SessionId, cx: &mut Context<Self>) -> Result<()> {
        // Don't allow removing the main session
        if Some(session_id) == self.main_session_id.as_ref() {
            return Err(anyhow::anyhow!("Cannot remove the main session"));
        }

        if let Some(orchestrator) = &self.session_orchestrator {
            orchestrator.update(cx, |orchestrator, cx| {
                orchestrator.remove_session(session_id, cx)
            })?;

            // If this was the selected session, switch to main
            if self.selected_session.as_ref() == Some(session_id) {
                self.selected_session = self.main_session_id.clone();
            }

            cx.emit(SessionListEvent::SessionRemoved(session_id.clone()));
            Ok(())
        } else {
            Err(anyhow::anyhow!("No orchestrator available"))
        }
    }

    pub fn switch_session(&mut self, session_id: &SessionId, cx: &mut Context<Self>) -> Result<()> {
        if let Some(orchestrator) = &self.session_orchestrator {
            orchestrator.update(cx, |orchestrator, cx| {
                orchestrator.activate_session(session_id.clone(), cx)
            })?;

            self.selected_session = Some(session_id.clone());
            cx.emit(SessionListEvent::SessionSelected(session_id.clone()));
            Ok(())
        } else {
            Err(anyhow::anyhow!("No orchestrator available"))
        }
    }

    pub fn toggle_orchestrator_mode(&mut self, cx: &mut Context<Self>) {
        self.orchestrator_mode = !self.orchestrator_mode;
        self.update_sessions(cx);
        cx.emit(SessionListEvent::OrchestratorModeToggled(self.orchestrator_mode));
    }

    pub fn show_session_thread(&mut self, session_id: &SessionId, cx: &mut Context<Self>) {
        cx.emit(SessionListEvent::ShowThreadRequested(session_id.clone()));
    }

    pub fn is_orchestrator_mode(&self) -> bool {
        self.orchestrator_mode
    }

    pub fn main_session_id(&self) -> Option<&SessionId> {
        self.main_session_id.as_ref()
    }

    pub fn selected_session_id(&self) -> Option<&SessionId> {
        self.selected_session.as_ref()
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
            | OrchestratorEvent::SessionActivated(_)
            | OrchestratorEvent::SessionStatusChanged(_, _) => {
                self.update_sessions(cx);
            }
            _ => {}
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

    #[allow(dead_code)]
    fn render_session_item(&self, session: &SessionListItem, cx: &mut Context<Self>) -> impl IntoElement {
        let session_id = session.id.clone();
        let is_selected = self.selected_session.as_ref() == Some(&session.id);
        let is_main = self.main_session_id.as_ref() == Some(&session.id);
        let is_orchestrator_session = session.is_orchestrator;

        div()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .when(is_selected, |div| {
                div.bg(cx.theme().colors().element_selected)
            })
            .when(!is_selected, |div| {
                div.hover(|div| div.bg(cx.theme().colors().element_hover))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(Label::new(session.name.clone()).weight(FontWeight::MEDIUM))
                                    .when(is_main, |div| {
                                        div.child(
                                            Label::new("MAIN")
                                                .size(LabelSize::XSmall)
                                                .color(Color::Accent)
                                        )
                                    })
                                    .when(is_orchestrator_session, |div| {
                                        div.child(
                                            Label::new("AGENT")
                                                .size(LabelSize::XSmall)
                                                .color(Color::Info)
                                        )
                                    })
                            )
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(self.render_session_status(&session.status, cx))
                            .when(session.thread_preview.is_some(), |div| {
                                div.child(
                                    IconButton::new(("show_thread", session.id.to_string().len()), IconName::MessageBubbles)
                                        .icon_size(IconSize::XSmall)
                                        .tooltip(Tooltip::text("Show thread"))
                                        .on_click({
                                            let session_id = session_id.clone();
                                            cx.listener(move |this, _event, _window, cx| {
                                                this.show_session_thread(&session_id, cx);
                                            })
                                        })
                                )
                            })
                            .when(!is_main, |div| {
                                div.child(
                                    IconButton::new(("remove", session.id.to_string().len()), IconName::Trash)
                                        .icon_size(IconSize::XSmall)
                                        .tooltip(Tooltip::text("Remove session"))
                                        .on_click({
                                            let session_id = session_id.clone();
                                            cx.listener(move |this, _event, _window, cx| {
                                                this.remove_session(&session_id, cx).log_err();
                                            })
                                        })
                                )
                            })
                    )
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        Label::new(format!("{} messages", session.message_count))
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                    )
                    .child(
                        Label::new(session.last_active.format("%H:%M").to_string())
                            .size(LabelSize::Small)
                            .color(Color::Muted)
                    )
            )
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, cx.listener(move |this, _event, _window, cx| {
                this.switch_session(&session_id, cx).log_err();
            }))
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .p_2()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(Label::new("Sessions").weight(FontWeight::MEDIUM))
                    .when(self.orchestrator_mode, |div| {
                        div.child(
                            Label::new("ORCHESTRATOR")
                                .size(LabelSize::XSmall)
                                .color(Color::Info)
                        )
                    })
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        IconButton::new("new_session", IconName::Plus)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("New session"))
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.new_session(cx).log_err();
                            }))
                    )
                    .child(
                        IconButton::new("toggle_orchestrator", IconName::Brain)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text(if self.orchestrator_mode {
                                "Exit orchestrator mode"
                            } else {
                                "Enter orchestrator mode"
                            }))
                            .toggle_state(self.orchestrator_mode)
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.toggle_orchestrator_mode(cx);
                            }))
                    )
            )
    }
}

impl Focusable for VerticalSessionList {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for VerticalSessionList {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let session_items: Vec<_> = self.sessions
            .iter()
            .map(|session| {
                let session_id = session.id.clone();
                let is_selected = self.selected_session.as_ref() == Some(&session.id);
                let is_main = self.main_session_id.as_ref() == Some(&session.id);
                let is_orchestrator_session = session.is_orchestrator;

                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .p_2()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .when(is_selected, |div| {
                        div.bg(cx.theme().colors().element_selected)
                    })
                    .when(!is_selected, |div| {
                        div.hover(|div| div.bg(cx.theme().colors().element_hover))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(Label::new(session.name.clone()).weight(FontWeight::MEDIUM))
                                            .when(is_main, |div| {
                                                div.child(
                                                    Label::new("MAIN")
                                                        .size(LabelSize::XSmall)
                                                        .color(Color::Accent)
                                                )
                                            })
                                            .when(is_orchestrator_session, |div| {
                                                div.child(
                                                    Label::new("AGENT")
                                                        .size(LabelSize::XSmall)
                                                        .color(Color::Info)
                                                )
                                            })
                                    )
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(self.render_session_status(&session.status, cx))
                                    .when(session.thread_preview.is_some(), |div| {
                                        div.child(
                                            IconButton::new(("show_thread", session.id.to_string().len()), IconName::MessageBubbles)
                                                .icon_size(IconSize::XSmall)
                                                .tooltip(Tooltip::text("Show thread"))
                                                .on_click({
                                                    let session_id = session_id.clone();
                                                    cx.listener(move |this, _event, _window, cx| {
                                                        this.show_session_thread(&session_id, cx);
                                                    })
                                                })
                                        )
                                    })
                                    .when(!is_main, |div| {
                                        div.child(
                                            IconButton::new(("remove", session.id.to_string().len()), IconName::Trash)
                                                .icon_size(IconSize::XSmall)
                                                .tooltip(Tooltip::text("Remove session"))
                                                .on_click({
                                                    let session_id = session_id.clone();
                                                    cx.listener(move |this, _event, _window, cx| {
                                                        this.remove_session(&session_id, cx).log_err();
                                                    })
                                                })
                                        )
                                    })
                            )
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                Label::new(format!("{} messages", session.message_count))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted)
                            )
                            .child(
                                Label::new(session.last_active.format("%H:%M").to_string())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted)
                            )
                    )
                    .cursor_pointer()
                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _event, _window, cx| {
                        this.switch_session(&session_id, cx).log_err();
                    }))
            })
            .collect();

        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_header(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .overflow_hidden()
                    .children(session_items)
            )
    }
} 