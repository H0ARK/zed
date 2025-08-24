use std::{collections::HashMap, sync::Arc, time::Duration};

use agent_settings::AgentSettings;
use assistant_tool::{ToolSource, ToolWorkingSet};

use crate::AddContextServer;

use crate::agent_configuration::{
    ConfigureContextServerModal, extension_only_provides_context_server,
    resolve_extension_for_context_server, show_unable_to_uninstall_extension_with_context_server,
};
use context_server::ContextServerId;
use extension_host::ExtensionStore;
use fs::Fs;
use gpui::{
    Action, Animation, AnimationExt, AnyElement, AnyView, App, Context, Corner, DismissEvent,
    Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle, SharedString, Subscription, Task,
    Transformation, WeakEntity, Window, percentage,
};
use language::LanguageRegistry;
use language_model::{LanguageModelProvider, LanguageModelProviderId, LanguageModelRegistry};
use menu;
use project::context_server_store::{
    ContextServerConfiguration, ContextServerStatus, ContextServerStore,
};
use project::project_settings::{ContextServerSettings, ProjectSettings};
use settings::{Settings, update_settings_file};
use ui::{
    Button, ButtonStyle, Color, ContextMenu, Disclosure, ElevationIndex, Headline, Icon,
    IconButton, IconName, IconPosition, IconSize, Indicator, Label, LabelSize, Modal, ModalHeader,
    PopoverMenu, Scrollbar, ScrollbarState, Section, Switch, SwitchColor, Tab, TabBar, Tooltip,
    prelude::*,
};
use util::ResultExt;
use workspace::{ModalView, Workspace};
use zed_actions::ExtensionCategoryFilter;

#[derive(Clone, Copy, Debug, PartialEq)]
enum SettingsTab {
    General,
    Providers,
    ContextServers,
    Tools,
}

impl SettingsTab {
    fn label(&self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Providers => "Providers",
            Self::ContextServers => "Context Servers",
            Self::Tools => "Tools",
        }
    }

    fn all() -> Vec<Self> {
        vec![
            Self::General,
            Self::Providers,
            Self::ContextServers,
            Self::Tools,
        ]
    }
}

pub struct AgentSettingsModal {
    fs: Arc<dyn Fs>,
    language_registry: Arc<LanguageRegistry>,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    context_server_store: Entity<ContextServerStore>,
    tools: Entity<ToolWorkingSet>,
    active_tab: SettingsTab,
    configuration_views_by_provider: HashMap<LanguageModelProviderId, AnyView>,
    expanded_provider_configurations: HashMap<LanguageModelProviderId, bool>,
    expanded_context_server_tools: HashMap<ContextServerId, bool>,
    scroll_handle: ScrollHandle,
    scrollbar_state: ScrollbarState,
    _registry_subscription: Subscription,
}

impl AgentSettingsModal {
    pub fn new(
        fs: Arc<dyn Fs>,
        context_server_store: Entity<ContextServerStore>,
        tools: Entity<ToolWorkingSet>,
        language_registry: Arc<LanguageRegistry>,
        workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let scroll_handle = ScrollHandle::new();
        let scrollbar_state = ScrollbarState::new(scroll_handle.clone());

        let registry_subscription = cx.observe_global::<LanguageModelRegistry>({
            move |this, cx| {
                // Note: build_provider_configuration_views needs window parameter
                // This will be called when needed in render methods
            }
        });

        let mut modal = Self {
            fs,
            language_registry,
            workspace,
            focus_handle,
            context_server_store,
            tools,
            active_tab: SettingsTab::General,
            configuration_views_by_provider: HashMap::default(),
            expanded_provider_configurations: HashMap::default(),
            expanded_context_server_tools: HashMap::default(),
            scroll_handle,
            scrollbar_state,
            _registry_subscription: registry_subscription,
        };

        modal.build_provider_configuration_views(window, cx);
        modal
    }

    fn set_active_tab(&mut self, tab: SettingsTab, cx: &mut Context<Self>) {
        self.active_tab = tab;
        cx.notify();
    }

    fn build_provider_configuration_views(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for provider in LanguageModelRegistry::read_global(cx).providers() {
            self.add_provider_configuration_view(&provider, window, cx);
        }
    }

    fn add_provider_configuration_view(
        &mut self,
        provider: &Arc<dyn LanguageModelProvider>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let configuration_view = provider.configuration_view(window, cx);
        self.configuration_views_by_provider
            .insert(provider.id(), configuration_view);
    }

    fn render_tab_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = SettingsTab::all();

        TabBar::new("agent-settings-tabs").children(tabs.into_iter().map(|tab| {
            Tab::new(tab.label())
                .toggle_state(tab == self.active_tab)
                .child(Label::new(tab.label()))
                .on_click({
                    let tab = tab;
                    cx.listener(move |this, _event, _window, cx| {
                        this.set_active_tab(tab, cx);
                    })
                })
        }))
    }

    fn render_general_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .flex_1()
            .gap_4()
            .p_4()
            .child(self.render_command_permission(cx))
            .child(self.render_single_file_review(cx))
            .child(self.render_sound_notification(cx))
    }

    fn render_providers_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let providers = LanguageModelRegistry::read_global(cx).providers();

        let mut container = v_flex().w_full().flex_1().gap_4().p_4().child(
            v_flex()
                .gap_2()
                .child(Headline::new("Language Model Providers"))
                .child(Label::new("Configure your language model providers.").color(Color::Muted))
                .child(
                    Button::new("add-llm-provider", "Add Provider")
                        .style(ButtonStyle::Filled)
                        .icon(IconName::Plus)
                        .icon_size(IconSize::Small)
                        .icon_position(IconPosition::Start)
                        .on_click(cx.listener(|this, _event, window, cx| {
                            let workspace = this.workspace.clone();
                            window.defer(cx, move |window, cx| {
                                workspace
                                    .update(cx, |workspace, cx| {
                                        crate::agent_configuration::AddLlmProviderModal::toggle(
                                            crate::agent_configuration::LlmCompatibleProvider::OpenAi,
                                            workspace,
                                            window,
                                            cx,
                                        );
                                    })
                                    .ok();
                            });
                        })),
                ),
        );

        for provider in providers.iter() {
            container = container.child(self.render_provider_configuration_block(provider, cx));
        }

        container
    }

    fn render_context_servers_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let context_server_ids = self.context_server_store.read(cx).all_server_ids().clone();

        let mut context_server_elements: Vec<AnyElement> = Vec::new();
        for context_server_id in context_server_ids {
            let element = self.render_context_server(context_server_id, window, cx);
            context_server_elements.push(element.into_any_element());
        }

        v_flex()
            .w_full()
            .flex_1()
            .gap_2()
            .child(
                v_flex()
                    .gap_0p5()
                    .child(Headline::new("Model Context Protocol (MCP) Servers"))
                    .child(Label::new("Connect to context servers via the Model Context Protocol either via Zed extensions or directly.").color(Color::Muted)),
            )
            .children(context_server_elements)
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .child(
                        h_flex().w_full().child(
                            Button::new("add-context-server", "Add Custom Server")
                                .style(ButtonStyle::Filled)
                                .layer(ElevationIndex::ModalSurface)
                                .full_width()
                                .icon(IconName::Plus)
                                .icon_size(IconSize::Small)
                                .icon_position(IconPosition::Start)
                                .on_click(|_event, window, cx| {
                                    window.dispatch_action(AddContextServer.boxed_clone(), cx)
                                }),
                        ),
                    )
                    .child(
                        h_flex().w_full().child(
                            Button::new(
                                "install-context-server-extensions",
                                "Install MCP Extensions",
                            )
                            .style(ButtonStyle::Filled)
                            .layer(ElevationIndex::ModalSurface)
                            .full_width()
                            .icon(IconName::ToolHammer)
                            .icon_size(IconSize::Small)
                            .icon_position(IconPosition::Start)
                            .on_click(|_event, window, cx| {
                                window.dispatch_action(
                                    zed_actions::Extensions {
                                        category_filter: Some(
                                            ExtensionCategoryFilter::ContextServers,
                                        ),
                                        id: None,
                                    }
                                    .boxed_clone(),
                                    cx,
                                )
                            }),
                        ),
                    ),
            )
    }

    fn render_context_server(
        &mut self,
        context_server_id: ContextServerId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tools_by_source = self.tools.read(cx).tools_by_source(cx);
        let server_status = self
            .context_server_store
            .read(cx)
            .status_for_server(&context_server_id)
            .unwrap_or(ContextServerStatus::Stopped);
        let server_configuration = self
            .context_server_store
            .read(cx)
            .configuration_for_server(&context_server_id);

        let is_running = matches!(server_status, ContextServerStatus::Running);
        let item_id = SharedString::from(context_server_id.0.clone());
        let is_from_extension = server_configuration
            .as_ref()
            .map(|config| {
                matches!(
                    config.as_ref(),
                    ContextServerConfiguration::Extension { .. }
                )
            })
            .unwrap_or(false);

        let error = if let ContextServerStatus::Error(error) = server_status.clone() {
            Some(error)
        } else {
            None
        };

        let are_tools_expanded = self
            .expanded_context_server_tools
            .get(&context_server_id)
            .copied()
            .unwrap_or_default();
        let tools = tools_by_source
            .get(&ToolSource::ContextServer {
                id: context_server_id.0.clone().into(),
            })
            .map_or([].as_slice(), |tools| tools.as_slice());
        let tool_count = tools.len();

        let border_color = cx.theme().colors().border.opacity(0.6);

        let (source_icon, source_tooltip) = if is_from_extension {
            (
                IconName::ZedMcpExtension,
                "This MCP server was installed from an extension.",
            )
        } else {
            (
                IconName::ZedMcpCustom,
                "This custom MCP server was installed directly.",
            )
        };

        let (status_indicator, tooltip_text) = match server_status {
            ContextServerStatus::Starting => (
                Icon::new(IconName::LoadCircle)
                    .size(IconSize::XSmall)
                    .color(Color::Accent)
                    .with_animation(
                        SharedString::from(format!("{}-starting", context_server_id.0.clone(),)),
                        Animation::new(Duration::from_secs(3)).repeat(),
                        |icon, delta| icon.transform(Transformation::rotate(percentage(delta))),
                    )
                    .into_any_element(),
                "Server is starting.",
            ),
            ContextServerStatus::Running => (
                Indicator::dot().color(Color::Success).into_any_element(),
                "Server is active.",
            ),
            ContextServerStatus::Error(_) => (
                Indicator::dot().color(Color::Error).into_any_element(),
                "Server has an error.",
            ),
            ContextServerStatus::Stopped => (
                Indicator::dot().color(Color::Muted).into_any_element(),
                "Server is stopped.",
            ),
        };

        let context_server_configuration_menu = PopoverMenu::new("context-server-config-menu")
            .trigger_with_tooltip(
                IconButton::new("context-server-config-menu", IconName::Settings)
                    .icon_color(Color::Muted)
                    .icon_size(IconSize::Small),
                Tooltip::text("Open MCP server options"),
            )
            .anchor(Corner::TopRight)
            .menu({
                let fs = self.fs.clone();
                let context_server_id = context_server_id.clone();
                let language_registry = self.language_registry.clone();
                let context_server_store = self.context_server_store.clone();
                let workspace = self.workspace.clone();
                move |window, cx| {
                    Some(ContextMenu::build(window, cx, |menu, _window, _cx| {
                        menu.entry("Configure Server", None, {
                            let context_server_id = context_server_id.clone();
                            let language_registry = language_registry.clone();
                            let workspace = workspace.clone();
                            move |window, cx| {
                                ConfigureContextServerModal::show_modal_for_existing_server(
                                    context_server_id.clone(),
                                    language_registry.clone(),
                                    workspace.clone(),
                                    window,
                                    cx,
                                )
                                .detach_and_log_err(cx);
                            }
                        })
                        .separator()
                        .entry("Uninstall", None, {
                            let fs = fs.clone();
                            let context_server_id = context_server_id.clone();
                            let context_server_store = context_server_store.clone();
                            let workspace = workspace.clone();
                            move |_, cx| {
                                let is_provided_by_extension = context_server_store
                                    .read(cx)
                                    .configuration_for_server(&context_server_id)
                                    .as_ref()
                                    .map(|config| {
                                        matches!(
                                            config.as_ref(),
                                            ContextServerConfiguration::Extension { .. }
                                        )
                                    })
                                    .unwrap_or(false);

                                let uninstall_extension_task = match (
                                    is_provided_by_extension,
                                    resolve_extension_for_context_server(&context_server_id, cx),
                                ) {
                                    (true, Some((id, manifest))) => {
                                        if extension_only_provides_context_server(manifest.as_ref())
                                        {
                                            ExtensionStore::global(cx).update(cx, |store, cx| {
                                                store.uninstall_extension(id, cx)
                                            })
                                        } else {
                                            workspace.update(cx, |workspace, cx| {
                                                show_unable_to_uninstall_extension_with_context_server(workspace, context_server_id.clone(), cx);
                                            }).log_err();
                                            Task::ready(Ok(()))
                                        }
                                    }
                                    _ => Task::ready(Ok(())),
                                };

                                cx.spawn({
                                    let fs = fs.clone();
                                    let context_server_id = context_server_id.clone();
                                    async move |cx| {
                                        uninstall_extension_task.await?;
                                        cx.update(|cx| {
                                            update_settings_file::<ProjectSettings>(
                                                fs.clone(),
                                                cx,
                                                {
                                                    let context_server_id =
                                                        context_server_id.clone();
                                                    move |settings, _| {
                                                        settings
                                                            .context_servers
                                                            .remove(&context_server_id.0);
                                                    }
                                                },
                                            )
                                        })
                                    }
                                })
                                .detach_and_log_err(cx);
                            }
                        })
                    }))
                }
            });

        v_flex()
            .id(item_id.clone())
            .border_1()
            .rounded_md()
            .border_color(border_color)
            .bg(cx.theme().colors().background.opacity(0.2))
            .overflow_hidden()
            .child(
                h_flex()
                    .p_1()
                    .justify_between()
                    .when(
                        error.is_some() || are_tools_expanded && tool_count >= 1,
                        |element| element.border_b_1().border_color(border_color),
                    )
                    .child(
                        h_flex()
                            .child(
                                Disclosure::new(
                                    "tool-list-disclosure",
                                    are_tools_expanded || error.is_some(),
                                )
                                .disabled(tool_count == 0)
                                .on_click(cx.listener({
                                    let context_server_id = context_server_id.clone();
                                    move |this, _event, _window, _cx| {
                                        let is_open = this
                                            .expanded_context_server_tools
                                            .entry(context_server_id.clone())
                                            .or_insert(false);

                                        *is_open = !*is_open;
                                    }
                                })),
                            )
                            .child(
                                h_flex()
                                    .id(SharedString::from(format!("tooltip-{}", item_id)))
                                    .h_full()
                                    .w_3()
                                    .mx_1()
                                    .justify_center()
                                    .tooltip(Tooltip::text(tooltip_text))
                                    .child(status_indicator),
                            )
                            .child(Label::new(item_id).ml_0p5())
                            .child(
                                div()
                                    .id("extension-source")
                                    .mt_0p5()
                                    .mx_1()
                                    .tooltip(Tooltip::text(source_tooltip))
                                    .child(
                                        Icon::new(source_icon)
                                            .size(IconSize::Small)
                                            .color(Color::Muted),
                                    ),
                            )
                            .when(is_running, |this| {
                                this.child(
                                    Label::new(if tool_count == 1 {
                                        SharedString::from("1 tool")
                                    } else {
                                        SharedString::from(format!("{} tools", tool_count))
                                    })
                                    .color(Color::Muted)
                                    .size(LabelSize::Small),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(context_server_configuration_menu)
                            .child(
                                Switch::new("context-server-switch", is_running.into())
                                    .color(SwitchColor::Accent)
                                    .on_click({
                                        let context_server_manager =
                                            self.context_server_store.clone();
                                        let context_server_id = context_server_id.clone();
                                        let fs = self.fs.clone();

                                        move |state, _window, cx| {
                                            let is_enabled = match state {
                                                ToggleState::Unselected
                                                | ToggleState::Indeterminate => {
                                                    context_server_manager.update(
                                                        cx,
                                                        |this, cx| {
                                                            this.stop_server(
                                                                &context_server_id,
                                                                cx,
                                                            )
                                                            .log_err();
                                                        },
                                                    );
                                                    false
                                                }
                                                ToggleState::Selected => {
                                                    context_server_manager.update(
                                                        cx,
                                                        |this, cx| {
                                                            if let Some(server) =
                                                                this.get_server(&context_server_id)
                                                            {
                                                                this.start_server(server, cx);
                                                            }
                                                        },
                                                    );
                                                    true
                                                }
                                            };
                                            update_settings_file::<ProjectSettings>(
                                                fs.clone(),
                                                cx,
                                                {
                                                    let context_server_id =
                                                        context_server_id.clone();

                                                    move |settings, _| {
                                                        settings
                                                            .context_servers
                                                            .entry(context_server_id.0)
                                                            .or_insert_with(|| {
                                                                ContextServerSettings::Extension {
                                                                    enabled: is_enabled,
                                                                    settings: serde_json::json!({}),
                                                                }
                                                            })
                                                            .set_enabled(is_enabled);
                                                    }
                                                },
                                            );
                                        }
                                    }),
                            ),
                    ),
            )
            .map(|parent| {
                if let Some(error) = error {
                    return parent.child(
                        h_flex()
                            .p_2()
                            .gap_2()
                            .items_start()
                            .child(
                                h_flex()
                                    .flex_none()
                                    .h(window.line_height() / 1.6_f32)
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::XCircle)
                                            .size(IconSize::XSmall)
                                            .color(Color::Error),
                                    ),
                            )
                            .child(
                                div().w_full().child(
                                    Label::new(error)
                                        .buffer_font(cx)
                                        .color(Color::Muted)
                                        .size(LabelSize::Small),
                                ),
                            ),
                    );
                }

                if !are_tools_expanded || tools.is_empty() {
                    return parent;
                }

                parent.child(v_flex().py_1p5().px_1().gap_1().children(
                    tools.into_iter().enumerate().map(|(ix, tool)| {
                        h_flex()
                            .id(("tool-item", ix))
                            .px_1()
                            .gap_2()
                            .justify_between()
                            .hover(|style| style.bg(cx.theme().colors().element_hover))
                            .rounded_sm()
                            .child(
                                Label::new(tool.name())
                                    .buffer_font(cx)
                                    .size(LabelSize::Small),
                            )
                            .child(
                                Icon::new(IconName::Info)
                                    .size(IconSize::Small)
                                    .color(Color::Ignored),
                            )
                            .tooltip(Tooltip::text(tool.description()))
                    }),
                ))
            })
    }

    fn render_tools_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let tools_by_source = self.tools.read(cx).tools_by_source(cx);

        v_flex()
            .w_full()
            .flex_1()
            .gap_2()
            .child(
                v_flex()
                    .gap_0p5()
                    .child(Headline::new("Available Tools"))
                    .child(Label::new("Configure which tools the agent can use to interact with your environment.").color(Color::Muted))
            )
            .child(
                v_flex()
                    .gap_4()
                    // Native Tools Section
                    .when_some(
                        tools_by_source.get(&ToolSource::Native),
                        |parent, native_tools| {
                            parent.child(
                                v_flex()
                                    .gap_2()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(Headline::new("Built-in Tools").size(HeadlineSize::Small))
                                            .child(Label::new(format!("{} tools", native_tools.len())).color(Color::Muted).size(LabelSize::Small))
                                    )
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .children(
                                                native_tools.iter().map(|tool| {
                                                    h_flex()
                                                        .px_2()
                                                        .py_1()
                                                        .gap_2()
                                                        .justify_between()
                                                        .hover(|style| style.bg(cx.theme().colors().element_hover))
                                                        .rounded_sm()
                                                        .child(
                                                            Label::new(tool.name())
                                                                .buffer_font(cx)
                                                                .size(LabelSize::Small)
                                                        )
                                                        .child(
                                                            Icon::new(IconName::Info)
                                                                .size(IconSize::Small)
                                                                .color(Color::Ignored)
                                                        )
                                                })
                                            )
                                    )
                            )
                        }
                    )
                    // Context Server Tools Section
                    .children(
                        tools_by_source.iter().filter_map(|(source, tools)| {
                            if let ToolSource::ContextServer { id } = source {
                                Some(
                                    v_flex()
                                        .gap_2()
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .child(
                                                    h_flex()
                                                        .gap_1()
                                                        .child(Headline::new(format!("{} Server", id)).size(HeadlineSize::Small))
                                                        .child(
                                                            Icon::new(IconName::ZedMcpExtension)
                                                                .size(IconSize::Small)
                                                                .color(Color::Muted)
                                                        )
                                                )
                                                .child(Label::new(format!("{} tools", tools.len())).color(Color::Muted).size(LabelSize::Small))
                                        )
                                        .child(
                                            v_flex()
                                                .gap_1()
                                                .children(
                                                    tools.iter().map(|tool| {
                                                        h_flex()
                                                            .px_2()
                                                            .py_1()
                                                            .gap_2()
                                                            .justify_between()
                                                            .hover(|style| style.bg(cx.theme().colors().element_hover))
                                                            .rounded_sm()
                                                            .child(
                                                                Label::new(tool.name())
                                                                    .buffer_font(cx)
                                                                    .size(LabelSize::Small)
                                                            )
                                                            .child(
                                                                Icon::new(IconName::Info)
                                                                    .size(IconSize::Small)
                                                                    .color(Color::Ignored)
                                                            )
                                                    })
                                                )
                                        )
                                )
                            } else {
                                None
                            }
                        })
                    )
            )
    }

    fn render_command_permission(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let always_allow_tool_actions = AgentSettings::get_global(cx).always_allow_tool_actions;

        h_flex()
            .gap_4()
            .justify_between()
            .flex_wrap()
            .child(
                v_flex()
                    .gap_0p5()
                    .max_w_5_6()
                    .child(Label::new("Allow running editing tools without asking for confirmation"))
                    .child(
                        Label::new(
                            "The agent can perform potentially destructive actions without asking for your confirmation.",
                        )
                        .color(Color::Muted),
                    ),
            )
            .child(
                Switch::new(
                    "always-allow-tool-actions-switch",
                    always_allow_tool_actions.into(),
                )
                .color(SwitchColor::Accent)
                .on_click({
                    let fs = self.fs.clone();
                    move |state, _window, cx| {
                        let allow = state == &ToggleState::Selected;
                        update_settings_file::<AgentSettings>(
                            fs.clone(),
                            cx,
                            move |settings, _| {
                                settings.set_always_allow_tool_actions(allow);
                            },
                        );
                    }
                }),
            )
    }

    fn render_single_file_review(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let single_file_review = AgentSettings::get_global(cx).single_file_review;

        h_flex()
            .gap_4()
            .justify_between()
            .flex_wrap()
            .child(
                v_flex()
                    .gap_0p5()
                    .max_w_5_6()
                    .child(Label::new("Enable single-file agent reviews"))
                    .child(
                        Label::new(
                            "Agent edits are also displayed in single-file editors for review.",
                        )
                        .color(Color::Muted),
                    ),
            )
            .child(
                Switch::new("single-file-review-switch", single_file_review.into())
                    .color(SwitchColor::Accent)
                    .on_click({
                        let fs = self.fs.clone();
                        move |state, _window, cx| {
                            let allow = state == &ToggleState::Selected;
                            update_settings_file::<AgentSettings>(
                                fs.clone(),
                                cx,
                                move |settings, _| {
                                    settings.set_single_file_review(allow);
                                },
                            );
                        }
                    }),
            )
    }

    fn render_sound_notification(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let play_sound_when_agent_done = AgentSettings::get_global(cx).play_sound_when_agent_done;

        h_flex()
            .gap_4()
            .justify_between()
            .flex_wrap()
            .child(
                v_flex()
                    .gap_0p5()
                    .max_w_5_6()
                    .child(Label::new("Play sound when finished generating"))
                    .child(
                        Label::new(
                            "Hear a notification sound when the agent is done generating changes or needs your input.",
                        )
                        .color(Color::Muted),
                    ),
            )
            .child(
                Switch::new("play-sound-notification-switch", play_sound_when_agent_done.into())
                    .color(SwitchColor::Accent)
                    .on_click({
                        let fs = self.fs.clone();
                        move |state, _window, cx| {
                            let allow = state == &ToggleState::Selected;
                            update_settings_file::<AgentSettings>(
                                fs.clone(),
                                cx,
                                move |settings, _| {
                                    settings.set_play_sound_when_agent_done(allow);
                                },
                            );
                        }
                    }),
            )
    }

    fn render_provider_configuration_block(
        &mut self,
        provider: &Arc<dyn LanguageModelProvider>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let provider_id = provider.id().0.clone();
        let provider_name = provider.name().0.clone();
        let provider_id_string = SharedString::from(format!("provider-disclosure-{provider_id}"));

        let configuration_view = self
            .configuration_views_by_provider
            .get(&provider.id())
            .cloned();

        let is_expanded = self
            .expanded_provider_configurations
            .get(&provider.id())
            .copied()
            .unwrap_or(false);

        v_flex()
            .w_full()
            .py_2()
            .gap_1p5()
            .border_t_1()
            .border_color(cx.theme().colors().border.opacity(0.6))
            .child(
                h_flex().w_full().gap_1().justify_between().child(
                    h_flex()
                        .id(provider_id_string.clone())
                        .cursor_pointer()
                        .py_0p5()
                        .w_full()
                        .justify_between()
                        .rounded_sm()
                        .hover(|hover| hover.bg(cx.theme().colors().element_hover))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Icon::new(provider.icon())
                                        .size(IconSize::Small)
                                        .color(Color::Muted),
                                )
                                .child(Label::new(provider_name.clone()).size(LabelSize::Large))
                                .when(provider.is_authenticated(cx) && !is_expanded, |parent| {
                                    parent.child(Icon::new(IconName::Check).color(Color::Success))
                                }),
                        )
                        .child(
                            Disclosure::new(provider_id_string, is_expanded)
                                .opened_icon(IconName::ChevronUp)
                                .closed_icon(IconName::ChevronDown),
                        )
                        .on_click(cx.listener({
                            let provider_id = provider.id().clone();
                            move |this, _event, _window, _cx| {
                                let is_expanded = this
                                    .expanded_provider_configurations
                                    .entry(provider_id.clone())
                                    .or_insert(false);

                                *is_expanded = !*is_expanded;
                            }
                        })),
                ),
            )
            .when(is_expanded, |parent| match configuration_view {
                Some(configuration_view) => parent.child(configuration_view),
                None => parent.child(Label::new(format!(
                    "No configuration view for {provider_name}",
                ))),
            })
    }

    fn render_tab_content(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        match self.active_tab {
            SettingsTab::General => self.render_general_tab(cx).into_any_element(),
            SettingsTab::Providers => self.render_providers_tab(cx).into_any_element(),
            SettingsTab::ContextServers => self
                .render_context_servers_tab(window, cx)
                .into_any_element(),
            SettingsTab::Tools => self.render_tools_tab(window, cx).into_any_element(),
        }
    }
}

impl EventEmitter<DismissEvent> for AgentSettingsModal {}

impl ModalView for AgentSettingsModal {}

impl Focusable for AgentSettingsModal {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AgentSettingsModal {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Dynamic sizing: narrower width, height constrained to 80% viewport; content scrolls.
        let viewport = window.viewport_size();
        let max_h = viewport.height * 0.8;
        v_flex()
            .key_context("AgentSettingsModal")
            .elevation_3(cx)
            .border_1()
            .rounded_lg()
            .border_color(cx.theme().colors().border)
            .bg(cx.theme().colors().panel_background)
            .w(rems(52.))
            .min_w(rems(44.))
            .max_w(px((viewport.width.0 * 0.75_f32).min(1100.0)))
            .min_h(rems(36.))
            .max_h(max_h)
            .overflow_hidden()
            .child(
                Modal::new("agent-settings-modal", Some(self.scroll_handle.clone()))
                    .header(
                        ModalHeader::new()
                            .show_back_button(false)
                            .show_dismiss_button(true)
                            .child(Headline::new("Agent Settings")),
                    )
                    // The scrolling region is handled internally by `Modal` via the provided scroll handle.
                    .child(
                        v_flex()
                            .flex_1()
                            .key_context("AgentSettingsModalInner")
                            .on_action(cx.listener(|_, _: &menu::Cancel, _, cx| {
                                cx.emit(DismissEvent);
                            }))
                            .child(self.render_tab_bar(cx))
                            .child(
                                // Tab content stretches full width; Section::new() (no internal border box) for cleaner look.
                                v_flex().flex_1().child(
                                    Section::new()
                                        .padded(true)
                                        .child(self.render_tab_content(window, cx)),
                                ),
                            ),
                    ),
            )
            // Custom scrollbar overlay retained but no longer swallows scroll events.
            .child(
                div()
                    .id("agent-settings-scrollbar")
                    .occlude()
                    .absolute()
                    .right(px(2.))
                    .top_0()
                    .bottom_0()
                    .pb_4()
                    .w(px(10.))
                    .cursor_default()
                    .children(Scrollbar::vertical(self.scrollbar_state.clone())),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_settings::AgentSettings;
    use assistant_tool::ToolWorkingSet;
    use gpui::{Entity, TestAppContext};
    use project::Project;
    use project::context_server_store::ContextServerStore;
    use settings::{Settings, SettingsStore};
    use std::sync::Arc;
    use workspace::Workspace;

    #[gpui::test]
    async fn test_agent_settings_modal_creation(cx: &mut TestAppContext) {
        cx.update(|cx| {
            settings::init(cx);
            AgentSettings::register(cx);
        });

        let fs = fs::FakeFs::new(cx.executor().clone());
        let project = Project::test(Arc::new(fs.clone()), [], cx).await;
        let workspace = cx.new(|cx| Workspace::test_new(project.clone(), cx));
        let context_server_store = cx.new(|_| ContextServerStore::new());
        let tools = cx.new(|_| ToolWorkingSet::default());
        let language_registry = Arc::new(language::LanguageRegistry::test(cx.executor().clone()));

        cx.new_window(|window, cx| {
            let modal = AgentSettingsModal::new(
                Arc::new(fs),
                context_server_store,
                tools,
                language_registry,
                workspace.downgrade(),
                window,
                cx,
            );

            // Test that modal is created with default tab
            assert_eq!(modal.active_tab, SettingsTab::General);
            modal
        });
    }

    #[gpui::test]
    async fn test_tab_switching(cx: &mut TestAppContext) {
        cx.update(|cx| {
            settings::init(cx);
            AgentSettings::register(cx);
        });

        let fs = fs::FakeFs::new(cx.executor().clone());
        let project = Project::test(Arc::new(fs.clone()), [], cx).await;
        let workspace = cx.new(|cx| Workspace::test_new(project.clone(), cx));
        let context_server_store = cx.new(|_| ContextServerStore::new());
        let tools = cx.new(|_| ToolWorkingSet::default());
        let language_registry = Arc::new(language::LanguageRegistry::test(cx.executor().clone()));

        cx.new_window(|window, cx| {
            let mut modal = AgentSettingsModal::new(
                Arc::new(fs),
                context_server_store,
                tools,
                language_registry,
                workspace.downgrade(),
                window,
                cx,
            );

            // Test tab switching
            modal.set_active_tab(SettingsTab::Providers, cx);
            assert_eq!(modal.active_tab, SettingsTab::Providers);

            modal.set_active_tab(SettingsTab::ContextServers, cx);
            assert_eq!(modal.active_tab, SettingsTab::ContextServers);

            modal.set_active_tab(SettingsTab::Tools, cx);
            assert_eq!(modal.active_tab, SettingsTab::Tools);

            modal
        });
    }
}
