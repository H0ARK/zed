use std::sync::Arc;

use serde::{ Deserialize, Serialize };

use crate::{ NewTextThread, NewThread, OpenHistory };
use agent::{
    ThreadEvent, // Added ThreadEvent import
    thread_store::{ TextThreadStore, ThreadStore },
};
use anyhow::{ Context as AnyhowContext, Result };
use assistant_slash_command::SlashCommandWorkingSet;
use gpui::{
    App,
    AppContext,
    AsyncWindowContext,
    Context,
    Entity,
    EventEmitter,
    FocusHandle,
    Focusable,
    IntoElement,
    Render,
    SharedString,
    Subscription,
    Task,
    WeakEntity,
    Window,
};
use prompt_store::{ PromptBuilder, PromptStore };
use ui::Icon;
use workspace::{ Workspace, item::{ Item, ItemEvent } };
use zed_actions::agent::OpenSettings;

// Import the AgentPanel implementation to reuse its logic
use crate::agent_panel::AgentPanel;

#[derive(Serialize, Deserialize)]
struct SerializedAgentItem {
    // Add any serialization fields needed
}

pub struct AgentItem {
    // Reuse the same internal structure as AgentPanel
    panel: Entity<AgentPanel>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>, // Track subscriptions for title updates
}

impl AgentItem {
    pub fn new(
        workspace: &Workspace,
        thread_store: Entity<ThreadStore>,
        context_store: Entity<TextThreadStore>,
        prompt_store: Option<Entity<PromptStore>>,
        window: &mut Window,
        cx: &mut Context<Self>
    ) -> Self {
        let focus_handle = cx.focus_handle();

        // Create the underlying AgentPanel
        let panel = cx.new(|cx| {
            AgentPanel::new_internal(workspace, thread_store, context_store, prompt_store, window, cx)
        });

        let mut subscriptions = Vec::new();

        // Subscribe to the panel to get notifications when the active thread changes
        let panel_subscription = cx.subscribe(&panel, |this, _panel, _event, cx| {
            // When panel events occur, refresh subscriptions to the current thread
            this.refresh_thread_subscriptions(cx);
        });
        subscriptions.push(panel_subscription);

        let mut item = Self {
            panel,
            focus_handle,
            _subscriptions: subscriptions,
        };

        // Initial setup of thread subscriptions
        item.refresh_thread_subscriptions(cx);

        item
    }

    fn refresh_thread_subscriptions(&mut self, cx: &mut Context<Self>) {
        // Clear existing thread subscriptions (keep panel subscription)
        self._subscriptions.truncate(1);

        // Get the current active thread from the panel
        if let Some(thread) = self.panel.read(cx).active_thread() {
            let thread_subscription = cx.subscribe(&thread, |_this, _thread, event, cx| {
                match event {
                    ThreadEvent::SummaryGenerated | ThreadEvent::SummaryChanged => {
                        // Emit UpdateTab when thread summary changes
                        cx.emit(ItemEvent::UpdateTab);
                    }
                    _ => {}
                }
            });
            self._subscriptions.push(thread_subscription);
        }
    }

    pub fn load(
        workspace: WeakEntity<Workspace>,
        prompt_builder: Arc<PromptBuilder>,
        cx: AsyncWindowContext
    ) -> Task<Result<Entity<Self>>> {
        cx.spawn(async move |cx| {
            let workspace = workspace.upgrade().with_context(|| "workspace was dropped")?;

            // Get the prompt store - we'll pass None for now since we need an App context
            let prompt_store = None;

            // Load the thread store and context store
            let (thread_store, context_store) = workspace.update(cx, |workspace, cx| {
                let tools = cx.new(|_cx| assistant_tool::ToolWorkingSet::default());
                let thread_store_task = agent::thread_store::ThreadStore::load(
                    workspace.project().clone(),
                    tools,
                    prompt_store.clone(),
                    prompt_builder.clone(),
                    cx
                );
                let context_store_task = assistant_context::ContextStore::new(
                    workspace.project().clone(),
                    prompt_builder.clone(),
                    Arc::new(SlashCommandWorkingSet::default()),
                    cx
                );
                (thread_store_task, context_store_task)
            })?;

            let thread_store = thread_store.await?;
            let context_store = context_store.await?;

            let agent_item = cx.update(|window, cx| {
                workspace.update(cx, |workspace, cx| {
                    cx.new(|cx| { Self::new(workspace, thread_store, context_store, prompt_store, window, cx) })
                })
            })?;

            Ok(agent_item)
        })
    }
}

impl Focusable for AgentItem {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<ItemEvent> for AgentItem {}

impl Item for AgentItem {
    type Event = ItemEvent;

    fn to_item_events(event: &Self::Event, mut f: impl FnMut(ItemEvent)) {
        f(*event)
    }

    fn tab_content_text(&self, _detail: usize, cx: &App) -> SharedString {
        // Try to get the current thread summary for a more descriptive tab title
        if let Some(thread) = self.panel.read(cx).active_thread() {
            let summary: SharedString = thread.read(cx).summary().or_default();
            // Truncate summary to a reasonable tab length
            const MAX_TAB_TITLE_LEN: usize = 48;
            return util::truncate_and_trailoff(&summary, MAX_TAB_TITLE_LEN).into();
        }

        // Fallback to default title when there is no active thread
        "Agent".into()
    }

    fn tab_icon(&self, _window: &Window, _cx: &App) -> Option<Icon> {
        Some(Icon::new(ui::IconName::ZedAssistant))
    }

    fn tab_tooltip_text(&self, _cx: &App) -> Option<SharedString> {
        Some("Agent Panel".into())
    }

    fn telemetry_event_text(&self) -> Option<&'static str> {
        Some("agent panel")
    }

    fn is_singleton(&self, _cx: &App) -> bool {
        false
    }

    fn clone_on_split(
        &self,
        _workspace_id: Option<workspace::WorkspaceId>,
        _window: &mut Window,
        _cx: &mut Context<Self>
    ) -> Option<Entity<Self>>
        where Self: Sized
    {
        None // Agent panel should not be cloned on split
    }

    fn added_to_workspace(&mut self, workspace: &mut Workspace, _window: &mut Window, _cx: &mut Context<Self>) {
        // Register actions that were previously in the panel init
        workspace
            .register_action(|workspace, _: &crate::NewAgentTab, window, cx| {
                // Spawn a fresh AgentItem and add it to the active pane
                let fs = workspace.app_state().fs.clone();
                let prompt_builder = PromptBuilder::load(fs, false, cx);
                let task = Self::load(workspace.weak_handle(), prompt_builder, window.to_async(cx));
                cx.spawn_in(window, async move |workspace, cx| {
                    if let Ok(agent_item) = task.await {
                        workspace.update_in(cx, |workspace, window, cx| {
                            workspace.add_item_to_active_pane(Box::new(agent_item), None, true, window, cx);
                        })?;
                    }
                    anyhow::Ok(())
                }).detach_and_log_err(cx);
            })
            .register_action(|workspace, action: &NewThread, window, cx| {
                if let Some(item) = workspace.active_item(cx).and_then(|item| item.downcast::<AgentItem>()) {
                    item.update(cx, |item, cx| {
                        item.panel.update(cx, |panel, cx| panel.new_thread(action, window, cx));
                        // Refresh subscriptions after creating new thread
                        item.refresh_thread_subscriptions(cx);
                    });
                }
            })
            .register_action(|workspace, _: &OpenHistory, window, cx| {
                if let Some(item) = workspace.active_item(cx).and_then(|item| item.downcast::<AgentItem>()) {
                    item.update(cx, |item, cx| {
                        item.panel.update(cx, |panel, cx| panel.open_history(window, cx));
                    });
                }
            })
            .register_action(|workspace, _: &OpenSettings, window, cx| {
                if let Some(item) = workspace.active_item(cx).and_then(|item| item.downcast::<AgentItem>()) {
                    item.update(cx, |item, cx| {
                        item.panel.update(cx, |panel, cx| panel.open_configuration(window, cx));
                    });
                }
            })
            .register_action(|workspace, _: &NewTextThread, window, cx| {
                if let Some(item) = workspace.active_item(cx).and_then(|item| item.downcast::<AgentItem>()) {
                    item.update(cx, |item, cx| {
                        item.panel.update(cx, |panel, cx| panel.new_prompt_editor(window, cx));
                        // Refresh subscriptions after creating new text thread
                        item.refresh_thread_subscriptions(cx);
                    });
                }
            });
    }
}

impl Render for AgentItem {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // Render the actual AgentPanel
        self.panel.clone()
    }
}
