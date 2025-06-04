use anyhow::Result;
use chrono::{DateTime, Utc};
use gpui::{
    actions, div, App, Context, Entity, EventEmitter, Focusable, FocusHandle,
    IntoElement, ParentElement, Render, Styled, Window,
};
use ui::{prelude::*, ButtonLike, IconButton, IconName, Label, Tooltip};
use util::ResultExt;

use crate::session::{AgentSessionManager, SessionConfig, SessionId, SessionStatus};
use crate::ThreadStore;

actions!(session_manager_ui, [NewSession, RemoveSession, SwitchSession, CoordinateAll]);

#[derive(Debug, Clone)]
pub struct SessionTab {
    pub id: SessionId,
    pub name: String,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub message_count: usize,
}

pub struct SessionManagerUI {
    focus_handle: FocusHandle,
    session_manager: Entity<AgentSessionManager>,
    #[allow(dead_code)]
    thread_store: Entity<ThreadStore>,
    session_tabs: Vec<SessionTab>,
    active_session_id: Option<SessionId>,
    show_coordination_panel: bool,
    coordination_history: Vec<String>,
    #[allow(dead_code)]
    creating_session: bool,
    #[allow(dead_code)]
    new_session_name: String,
}

pub enum SessionUIEvent {
    SessionSwitched(SessionId),
    SessionCreated(SessionId),
    SessionRemoved(SessionId),
    CoordinationRequested(String),
}

impl EventEmitter<SessionUIEvent> for SessionManagerUI {}

impl SessionManagerUI {
    pub fn new(
        session_manager: Entity<AgentSessionManager>,
        thread_store: Entity<ThreadStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        let mut ui = Self {
            focus_handle,
            session_manager,
            thread_store,
            session_tabs: Vec::new(),
            active_session_id: None,
            show_coordination_panel: false,
            coordination_history: Vec::new(),
            creating_session: false,
            new_session_name: String::new(),
        };

        ui.update_session_tabs(cx);
        ui
    }

    pub fn create_session(&mut self, name: String, cx: &mut Context<Self>) -> Result<SessionId> {
        let config = SessionConfig::default();
        
        let session_id = self.session_manager.update(cx, |manager, cx| {
            manager.create_session(&name, config, cx)
        })?;

        self.update_session_tabs(cx);
        self.switch_to_session(session_id.clone(), cx)?;
        cx.emit(SessionUIEvent::SessionCreated(session_id.clone()));

        Ok(session_id)
    }

    pub fn remove_session(&mut self, session_id: &SessionId, cx: &mut Context<Self>) -> Result<()> {
        self.session_manager.update(cx, |manager, cx| {
            manager.remove_session(session_id, cx)
        })?;

        if self.active_session_id.as_ref() == Some(session_id) {
            self.active_session_id = None;
            // Switch to first available session
            if let Some(first_tab) = self.session_tabs.first() {
                self.switch_to_session(first_tab.id.clone(), cx)?;
            }
        }

        self.update_session_tabs(cx);
        cx.emit(SessionUIEvent::SessionRemoved(session_id.clone()));
        Ok(())
    }

    pub fn switch_to_session(&mut self, session_id: SessionId, cx: &mut Context<Self>) -> Result<()> {
        self.session_manager.update(cx, |manager, cx| {
            manager.activate_session(session_id.clone(), cx)
        })?;

        self.active_session_id = Some(session_id.clone());
        cx.emit(SessionUIEvent::SessionSwitched(session_id));
        Ok(())
    }

    pub fn active_session_id(&self) -> Option<&SessionId> {
        self.active_session_id.as_ref()
    }

    pub fn session_count(&self) -> usize {
        self.session_tabs.len()
    }

    fn update_session_tabs(&mut self, cx: &mut Context<Self>) {
        self.session_tabs = self.session_manager.read(cx)
            .list_sessions()
            .into_iter()
            .filter_map(|session_id| {
                self.session_manager.read(cx)
                    .get_session(&session_id)
                    .map(|session| {
                        let session_data = session.read(cx);
                        SessionTab {
                            id: session_id,
                            name: session_data.name().to_string(),
                            status: session_data.status().clone(),
                            created_at: session_data.metadata.created_at,
                            last_active: session_data.metadata.last_active,
                            message_count: session_data.metadata.message_count,
                        }
                    })
            })
            .collect();

        // Set active session if none is set and we have sessions
        if self.active_session_id.is_none() && !self.session_tabs.is_empty() {
            if let Some(first_tab) = self.session_tabs.first() {
                self.active_session_id = Some(first_tab.id.clone());
            }
        }
    }

    pub fn coordinate_sessions(&mut self, task: String, cx: &mut Context<Self>) {
        self.coordination_history.push(format!("🧠 Coordinating: {}", task));
        cx.emit(SessionUIEvent::CoordinationRequested(task));
        cx.notify();
    }

    fn render_session_tabs(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.session_tabs.is_empty() {
            return div().into_any();
        }

        div()
            .flex()
            .flex_row()
            .gap_1()
            .p_2()
            .bg(cx.theme().colors().title_bar_background)
            .child(
                IconButton::new("new_session", IconName::Plus)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("New Session"))
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        let session_name = format!("Session {}", this.session_tabs.len() + 1);
                        this.create_session(session_name, cx).log_err();
                    }))
            )
            .children(
                self.session_tabs.iter().enumerate().map(|(index, tab)| {
                    let is_active = self.active_session_id.as_ref() == Some(&tab.id);
                    let status_icon = match tab.status {
                        SessionStatus::Thinking => "🤔",
                        SessionStatus::Responding => "💬",
                        SessionStatus::WaitingForUser => "⏳",
                        SessionStatus::Error(_) => "❌",
                        SessionStatus::Idle => "💤",
                    };

                    ButtonLike::new(("session_tab", index))
                        .style(if is_active { 
                            ButtonStyle::Filled 
                        } else { 
                            ButtonStyle::Subtle 
                        })
                        .child(
                            div()
                                .flex()
                                .flex_row()
                                .gap_1()
                                .child(Label::new(status_icon).size(LabelSize::Small))
                                .child(Label::new(&tab.name).size(LabelSize::Small))
                                .when(self.session_tabs.len() > 1, |element| {
                                    element.child(
                                        IconButton::new(("close_session", index), IconName::X)
                                            .icon_size(IconSize::XSmall)
                                            .on_click({
                                                let session_id = tab.id.clone();
                                                cx.listener(move |this, _event, _window, cx| {
                                                    this.remove_session(&session_id, cx).log_err();
                                                })
                                            })
                                    )
                                })
                        )
                        .on_click({
                            let session_id = tab.id.clone();
                            cx.listener(move |this, _event, _window, cx| {
                                this.switch_to_session(session_id.clone(), cx).log_err();
                            })
                        })
                })
            )
            .child(
                IconButton::new("coordinate_all", IconName::Play)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Coordinate All Sessions"))
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        this.show_coordination_panel = !this.show_coordination_panel;
                        if this.show_coordination_panel {
                            let task = format!("Analyze and coordinate {} active sessions", this.session_tabs.len());
                            this.coordinate_sessions(task, cx);
                        }
                        cx.notify();
                    }))
            )
            .into_any()
    }

    fn render_coordination_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.show_coordination_panel {
            return div().into_any();
        }

        div()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .bg(cx.theme().colors().surface_background)
            .border_1()
            .border_color(cx.theme().colors().border)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_between()
                    .child(Label::new("🧠 AI Coordination"))
                    .child(
                        IconButton::new("close_coordination", IconName::X)
                            .icon_size(IconSize::XSmall)
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.show_coordination_panel = false;
                                cx.notify();
                            }))
                    )
            )
            .child(
                div()
                    .max_h_32()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .children(
                                self.coordination_history.iter().rev().take(5).map(|entry| {
                                    div()
                                        .p_2()
                                        .bg(cx.theme().colors().ghost_element_background)
                                        .rounded_md()
                                        .child(Label::new(entry).size(LabelSize::Small))
                                })
                            )
                    )
            )
            .when(self.coordination_history.is_empty(), |element| {
                element.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .p_4()
                        .child(Label::new("Click the coordination button to start").color(Color::Muted))
                )
            })
            .into_any()
    }

    pub fn render_session_info(&self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.session_tabs.is_empty() {
            return div()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .items_center()
                        .child(Label::new("🚀 Agent-First Zed"))
                        .child(Label::new("Create your first session to begin").color(Color::Muted))
                        .child(
                            ButtonLike::new("create_first_session")
                                .style(ButtonStyle::Filled)
                                .child(Label::new("Create Session"))
                                .on_click(cx.listener(|this, _event, _window, cx| {
                                    this.create_session("Main Session".to_string(), cx).log_err();
                                }))
                        )
                )
                .into_any();
        }

        div()
            .child(
                Label::new(format!(
                    "Managing {} sessions • Active: {}",
                    self.session_tabs.len(),
                    self.active_session_id
                        .as_ref()
                        .and_then(|id| self.session_tabs.iter().find(|tab| &tab.id == id))
                        .map(|tab| tab.name.as_str())
                        .unwrap_or("None")
                ))
                .size(LabelSize::Small)
                .color(Color::Muted)
            )
            .into_any()
    }
}

impl Focusable for SessionManagerUI {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SessionManagerUI {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_session_tabs(cx))
            .child(self.render_coordination_panel(cx))
            .child(self.render_session_info(cx))
    }
}

