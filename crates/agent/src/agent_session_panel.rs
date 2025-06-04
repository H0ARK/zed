use anyhow::Result;
use gpui::{
    actions, div, App, Context, Entity, EventEmitter, Focusable, FocusHandle,
    IntoElement, ParentElement, Render, Styled, WeakEntity, Window, FontWeight,
};
use project::Project;
use serde::{Deserialize, Serialize};
use ui::{prelude::*, IconButton, IconName, Label, Tooltip};
use workspace::dock::{DockPosition, Panel, PanelEvent};
use workspace::Workspace;

use crate::session::SessionId;
use crate::vertical_session_list::{VerticalSessionList, SessionListEvent};
use crate::{AgentPanel, ThreadStore};

actions!(
    agent_session_panel,
    [
        ToggleFocus,
        ShowSessionThread,
        ToggleOrchestratorMode
    ]
);

#[derive(Serialize, Deserialize)]
struct SerializedAgentSessionPanel {
    width: Option<f32>,
    session_list_width: Option<f32>,
}

pub struct AgentSessionPanel {
    focus_handle: FocusHandle,
    session_list: Entity<VerticalSessionList>,
    agent_panel: Option<Entity<AgentPanel>>,
    width: Option<f32>,
    session_list_width: Option<f32>,
    _subscriptions: Vec<gpui::Subscription>,
}

impl AgentSessionPanel {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        thread_store: Entity<ThreadStore>,
        project: Entity<Project>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        
        // Create the vertical session list
        let session_list = cx.new(|cx| {
            VerticalSessionList::new(workspace.clone(), thread_store.clone(), project, cx)
        });

        // Subscribe to session list events
        let subscriptions = vec![cx.subscribe(&session_list, Self::handle_session_list_event)];

        Self {
            focus_handle,
            session_list,
            agent_panel: None, // Will be set later when needed
            width: None,
            session_list_width: Some(300.0),
            _subscriptions: subscriptions,
        }
    }

    pub async fn load(
        workspace: WeakEntity<Workspace>,
        thread_store: Entity<ThreadStore>,
        project: Entity<Project>,
        mut cx: gpui::AsyncWindowContext,
    ) -> Result<Entity<Self>> {
        let panel = cx.new(|cx| {
            let focus_handle = cx.focus_handle();
            
            // Create the vertical session list
            let session_list = cx.new(|cx| {
                VerticalSessionList::new(workspace.clone(), thread_store.clone(), project, cx)
            });

            // Subscribe to session list events
            let subscriptions = vec![cx.subscribe(&session_list, Self::handle_session_list_event)];

            Self {
                focus_handle,
                session_list,
                agent_panel: None, // Will be set later when needed
                width: None,
                session_list_width: Some(300.0),
                _subscriptions: subscriptions,
            }
        })?;

        Ok(panel)
    }

    pub fn set_agent_panel(&mut self, agent_panel: Entity<AgentPanel>, cx: &mut Context<Self>) {
        self.agent_panel = Some(agent_panel);
        cx.notify();
    }

    fn handle_session_list_event(
        &mut self,
        _session_list: Entity<VerticalSessionList>,
        event: &SessionListEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            SessionListEvent::SessionSelected(session_id) => {
                self.switch_to_session(session_id, cx);
            }
            SessionListEvent::ShowThreadRequested(session_id) => {
                self.show_session_thread(session_id, cx);
            }
            SessionListEvent::OrchestratorModeToggled(enabled) => {
                self.handle_orchestrator_mode_toggle(*enabled, cx);
            }
            _ => {}
        }
    }

    fn switch_to_session(&mut self, _session_id: &SessionId, cx: &mut Context<Self>) {
        // Update the agent panel to show the selected session's thread
        // This would involve coordinating with the agent panel to switch contexts
        cx.notify();
    }

    fn show_session_thread(&mut self, _session_id: &SessionId, cx: &mut Context<Self>) {
        // Show the thread for the specified session in the agent panel
        // This could open a new view or switch the current view
        cx.notify();
    }

    fn handle_orchestrator_mode_toggle(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if let Some(agent_panel) = &self.agent_panel {
            if enabled {
                // In orchestrator mode, the main session becomes read-only for editing
                // but can still create and communicate with other agents
                agent_panel.update(cx, |panel, cx| {
                    panel.set_orchestrator_mode(true, cx);
                });
            } else {
                // Exit orchestrator mode - restore full editing capabilities
                agent_panel.update(cx, |panel, cx| {
                    panel.set_orchestrator_mode(false, cx);
                });
            }
        }
        cx.notify();
    }

    pub fn toggle_orchestrator_mode(&mut self, cx: &mut Context<Self>) {
        self.session_list.update(cx, |list, cx| {
            list.toggle_orchestrator_mode(cx);
        });
    }

    pub fn is_orchestrator_mode(&self, cx: &App) -> bool {
        self.session_list.read(cx).is_orchestrator_mode()
    }



    fn render_session_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .w(px(self.session_list_width.unwrap_or(300.0)))
            .h_full()
            .border_r_1()
            .border_color(cx.theme().colors().border)
            .child(self.session_list.clone())
    }

    fn render_agent_panel(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .h_full()
            .child(
                if let Some(agent_panel) = &self.agent_panel {
                    agent_panel.clone().into_any_element()
                } else {
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h_full()
                        .child(Label::new("No agent panel available"))
                        .into_any_element()
                }
            )
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
                    .child(Label::new("Agent Sessions").weight(FontWeight::BOLD))
                    .when(self.is_orchestrator_mode(cx), |div| {
                        div.child(Label::new("ORCHESTRATOR MODE").size(LabelSize::Small).color(Color::Info))
                    })
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        IconButton::new("toggle_orchestrator", IconName::Brain)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text(if self.is_orchestrator_mode(cx) {
                                "Exit orchestrator mode"
                            } else {
                                "Enter orchestrator mode"
                            }))
                            .toggle_state(self.is_orchestrator_mode(cx))
                            .on_click(cx.listener(|this, _event, _window, cx| {
                                this.toggle_orchestrator_mode(cx);
                            }))
                    )
            )
    }
}

impl Focusable for AgentSessionPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PanelEvent> for AgentSessionPanel {}

impl Panel for AgentSessionPanel {
    fn persistent_name() -> &'static str {
        "AgentSessionPanel"
    }

    fn position(&self, _window: &Window, _cx: &App) -> DockPosition {
        DockPosition::Right
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        matches!(
            position,
            DockPosition::Left | DockPosition::Right
        )
    }

    fn set_position(&mut self, _position: DockPosition, _window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
    }

    fn size(&self, _window: &Window, _cx: &App) -> gpui::Pixels {
        self.width.map(px).unwrap_or(px(400.))
    }

    fn set_size(&mut self, size: Option<gpui::Pixels>, _window: &mut Window, cx: &mut Context<Self>) {
        self.width = size.map(|s| s.0);
        cx.notify();
    }

    fn icon(&self, _window: &Window, _cx: &App) -> Option<IconName> {
        Some(IconName::MessageBubbles)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Agent Sessions")
    }

    fn toggle_action(&self) -> Box<dyn gpui::Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        2
    }
}

impl Render for AgentSessionPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(self.render_header(cx))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .child(self.render_session_list(cx))
                    .child(self.render_agent_panel(cx))
            )
    }
}

pub fn init(cx: &mut App) {
    cx.observe_new::<Workspace>(|workspace: &mut Workspace, _window, _cx| {
        workspace.register_action(|workspace, _: &ToggleFocus, window, cx| {
            workspace.toggle_panel_focus::<AgentSessionPanel>(window, cx);
        });
    })
    .detach();
} 