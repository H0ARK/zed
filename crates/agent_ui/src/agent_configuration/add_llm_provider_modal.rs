use std::sync::Arc;

use anyhow::Result;
use collections::HashSet;
use fs::Fs;
use futures::AsyncReadExt;
use gpui::{DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Render, Task};
use http_client::{AsyncBody, http};
use language_model::LanguageModelRegistry;
use language_models::{
    AllLanguageModelSettings, OpenAiCompatibleSettingsContent,
    provider::open_ai_compatible::AvailableModel,
};
use settings::update_settings_file;
use ui::{Banner, KeyBinding, Modal, ModalFooter, ModalHeader, Section, prelude::*};
use ui_input::SingleLineInput;
use workspace::{ModalView, Workspace};

#[derive(Clone, Copy)]
pub enum LlmCompatibleProvider {
    OpenAi,
}

impl LlmCompatibleProvider {
    fn name(&self) -> &'static str {
        match self {
            LlmCompatibleProvider::OpenAi => "OpenAI",
        }
    }

    fn api_url(&self) -> &'static str {
        match self {
            LlmCompatibleProvider::OpenAi => "https://api.openai.com/v1",
        }
    }
}

struct AddLlmProviderInput {
    provider_name: Entity<SingleLineInput>,
    api_url: Entity<SingleLineInput>,
    api_key: Entity<SingleLineInput>,
    models: Vec<ModelInput>,
}

impl AddLlmProviderInput {
    fn new(provider: LlmCompatibleProvider, window: &mut Window, cx: &mut App) -> Self {
        let provider_name = single_line_input("Provider Name", provider.name(), None, window, cx);
        let api_url = single_line_input("API URL", provider.api_url(), None, window, cx);
        let api_key = single_line_input(
            "API Key",
            "000000000000000000000000000000000000000000000000",
            None,
            window,
            cx,
        );

        Self {
            provider_name,
            api_url,
            api_key,
            models: vec![ModelInput::new(window, cx)],
        }
    }

    fn add_model(&mut self, window: &mut Window, cx: &mut App) {
        self.models.push(ModelInput::new(window, cx));
    }

    fn remove_model(&mut self, index: usize) {
        self.models.remove(index);
    }
}

struct ModelInput {
    name: Entity<SingleLineInput>,
    max_completion_tokens: Entity<SingleLineInput>,
    max_output_tokens: Entity<SingleLineInput>,
    max_tokens: Entity<SingleLineInput>,
}

impl ModelInput {
    fn new(window: &mut Window, cx: &mut App) -> Self {
        let model_name = single_line_input(
            "Model Name",
            "e.g. gpt-4o, claude-opus-4, gemini-2.5-pro",
            None,
            window,
            cx,
        );
        let max_completion_tokens =
            single_line_input("Max Completion Tokens", "200000", None, window, cx);
        let max_output_tokens =
            single_line_input("Max Output Tokens", "Max Output Tokens", None, window, cx);
        let max_tokens = single_line_input("Max Tokens", "Max Tokens", None, window, cx);
        Self {
            name: model_name,
            max_completion_tokens,
            max_output_tokens,
            max_tokens,
        }
    }

    fn parse(&self, cx: &App) -> Result<AvailableModel, SharedString> {
        let name = self.name.read(cx).text(cx);
        if name.is_empty() {
            return Err(SharedString::from("Model Name cannot be empty"));
        }

        // Optional fields: allow empty -> None
        let max_completion_tokens =
            {
                let s = self.max_completion_tokens.read(cx).text(cx);
                if s.trim().is_empty() {
                    None
                } else {
                    Some(s.parse::<u64>().map_err(|_| {
                        SharedString::from("Max Completion Tokens must be a number")
                    })?)
                }
            };

        let max_output_tokens = {
            let s = self.max_output_tokens.read(cx).text(cx);
            if s.trim().is_empty() {
                None
            } else {
                Some(
                    s.parse::<u64>()
                        .map_err(|_| SharedString::from("Max Output Tokens must be a number"))?,
                )
            }
        };

        // Required field: still must be provided
        let max_tokens = self
            .max_tokens
            .read(cx)
            .text(cx)
            .parse::<u64>()
            .map_err(|_| SharedString::from("Max Tokens must be a number"))?;

        Ok(AvailableModel {
            name,
            display_name: None,
            max_completion_tokens,
            max_output_tokens,
            max_tokens,
        })
    }
}

fn single_line_input(
    label: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    text: Option<&str>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SingleLineInput> {
    cx.new(|cx| {
        let input = SingleLineInput::new(window, cx, placeholder).label(label);
        if let Some(text) = text {
            input
                .editor()
                .update(cx, |editor, cx| editor.set_text(text, window, cx));
        }
        input
    })
}

fn save_provider_to_settings(
    input: &AddLlmProviderInput,
    cx: &mut App,
) -> Task<Result<(), SharedString>> {
    let provider_name: Arc<str> = input.provider_name.read(cx).text(cx).into();
    if provider_name.is_empty() {
        return Task::ready(Err("Provider Name cannot be empty".into()));
    }

    if LanguageModelRegistry::read_global(cx)
        .providers()
        .iter()
        .any(|provider| {
            provider.id().0.as_ref() == provider_name.as_ref()
                || provider.name().0.as_ref() == provider_name.as_ref()
        })
    {
        return Task::ready(Err(
            "Provider Name is already taken by another provider".into()
        ));
    }

    let api_url = input.api_url.read(cx).text(cx);
    if api_url.is_empty() {
        return Task::ready(Err("API URL cannot be empty".into()));
    }

    let api_key = input.api_key.read(cx).text(cx);
    if api_key.is_empty() {
        return Task::ready(Err("API Key cannot be empty".into()));
    }

    let mut models = Vec::new();
    let mut model_names: HashSet<String> = HashSet::default();
    for model in &input.models {
        // Allow empty model rows: skip if name is empty/whitespace
        let name = model.name.read(cx).text(cx);
        if name.trim().is_empty() {
            continue;
        }

        match model.parse(cx) {
            Ok(model) => {
                if !model_names.insert(model.name.clone()) {
                    return Task::ready(Err("Model Names must be unique".into()));
                }
                models.push(model)
            }
            Err(err) => return Task::ready(Err(err)),
        }
    }

    let fs = <dyn Fs>::global(cx);
    let task = cx.write_credentials(&api_url, "Bearer", api_key.as_bytes());
    cx.spawn(async move |cx| {
        task.await
            .map_err(|_| "Failed to write API key to keychain")?;
        cx.update(|cx| {
            update_settings_file::<AllLanguageModelSettings>(fs, cx, |settings, _cx| {
                settings.openai_compatible.get_or_insert_default().insert(
                    provider_name,
                    OpenAiCompatibleSettingsContent {
                        api_url,
                        available_models: models,
                    },
                );
            });
        })
        .ok();
        Ok(())
    })
}

pub struct AddLlmProviderModal {
    provider: LlmCompatibleProvider,
    input: AddLlmProviderInput,
    focus_handle: FocusHandle,
    last_error: Option<SharedString>,
}

impl AddLlmProviderModal {
    pub fn toggle(
        provider: LlmCompatibleProvider,
        workspace: &mut Workspace,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        workspace.toggle_modal(window, cx, |window, cx| Self::new(provider, window, cx));
    }

    fn new(provider: LlmCompatibleProvider, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            input: AddLlmProviderInput::new(provider, window, cx),
            provider,
            last_error: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn confirm(&mut self, _: &menu::Confirm, _: &mut Window, cx: &mut Context<Self>) {
        let task = save_provider_to_settings(&self.input, cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| match result {
                Ok(_) => {
                    cx.emit(DismissEvent);
                }
                Err(error) => {
                    this.last_error = Some(error);
                    cx.notify();
                }
            })
        })
        .detach_and_log_err(cx);
    }

    fn cancel(&mut self, _: &menu::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn render_model_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let refresh_handler = cx.listener(
            |this: &mut Self, _event, window: &mut Window, cx: &mut Context<Self>| {
                let api_url = this.input.api_url.read(cx).text(cx);
                let api_key = this.input.api_key.read(cx).text(cx);

                if api_url.trim().is_empty() || api_key.trim().is_empty() {
                    this.last_error =
                        Some("API URL and API Key are required to fetch models".into());
                    cx.notify();
                    return;
                }

                let base = api_url.trim_end_matches('/');
                let url = if base.ends_with("/v1") {
                    format!("{}/models", base)
                } else {
                    format!("{}/v1/models", base)
                };

                let request = match http::Request::builder()
                    .uri(url)
                    .method(http::Method::GET)
                    .header("Authorization", format!("Bearer {}", api_key))
                    .body(AsyncBody::empty())
                {
                    Ok(req) => req,
                    Err(e) => {
                        this.last_error = Some(format!("Failed to build request: {}", e).into());
                        cx.notify();
                        return;
                    }
                };

                let http_client = cx.http_client();
                cx.spawn_in(window, async move |this, cx| -> anyhow::Result<()> {
                    let mut response = match http_client.send(request).await {
                        Ok(res) => res,
                        Err(err) => {
                            this.update(cx, |this, cx| {
                                this.last_error =
                                    Some(format!("Failed to fetch models: {}", err).into());
                                cx.notify();
                            })?;
                            return Ok(());
                        }
                    };

                    if !response.status().is_success() {
                        let status = response.status();
                        this.update(cx, |this, cx| {
                            this.last_error =
                                Some(format!("Failed to fetch models: HTTP {}", status).into());
                            cx.notify();
                        })?;
                        return Ok(());
                    }

                    let mut body = Vec::new();
                    response.body_mut().read_to_end(&mut body).await.ok();

                    let models = serde_json::from_slice::<serde_json::Value>(&body)
                        .ok()
                        .and_then(|v| {
                            let arr = if let Some(a) = v.get("data").and_then(|d| d.as_array()) {
                                Some(a.clone())
                            } else if let Some(a) = v.get("models").and_then(|d| d.as_array()) {
                                Some(a.clone())
                            } else if let Some(a) = v.as_array() {
                                Some(a.clone())
                            } else {
                                None
                            }?;
                            Some(
                                arr.into_iter()
                                    .filter_map(|item| {
                                        let id = item
                                            .get("id")
                                            .or_else(|| item.get("name"))
                                            .and_then(|s| s.as_str())
                                            .map(|s| s.to_string())?;
                                        let get_num = |k: &str| {
                                            item.get(k)
                                                .and_then(|v| v.as_u64())
                                                .or_else(|| {
                                                    item.get(k).and_then(|v| {
                                                        v.as_str().and_then(|s| s.parse::<u64>().ok())
                                                    })
                                                })
                                        };
                                        let max_tokens = get_num("max_tokens")
                                            .or_else(|| get_num("context_window"))
                                            .or_else(|| get_num("context_length"))
                                            .or_else(|| get_num("max_context_tokens"));
                                        let max_output_tokens = get_num("max_output_tokens")
                                            .or_else(|| get_num("output_token_limit"));
                                        let max_completion_tokens = get_num("max_completion_tokens")
                                            .or_else(|| get_num("input_token_limit"));
                                        Some((id, max_tokens, max_completion_tokens, max_output_tokens))
                                    })
                                    .collect::<Vec<(String, Option<u64>, Option<u64>, Option<u64>)>>(),
                            )
                        })
                        .unwrap_or_default();

                    this.update_in(cx, |this, inner_window, cx| {
                        if models.is_empty() {
                            this.last_error = Some("No models returned from provider".into());
                            cx.notify();
                            return;
                        }

                        let mut new_models = Vec::new();
                        for (name, max_tokens, max_completion_tokens, max_output_tokens) in models {
                            let mi = ModelInput::new(inner_window, cx);
                            mi.name.update(cx, |input, cx| {
                                input.editor().update(cx, |editor, cx| {
                                    editor.set_text(name.as_str(), inner_window, cx);
                                });
                            });
                            if let Some(v) = max_tokens {
                                mi.max_tokens.update(cx, |input, cx| {
                                    input.editor().update(cx, |editor, cx| {
                                        editor.set_text(v.to_string().as_str(), inner_window, cx);
                                    });
                                });
                            }
                            if let Some(v) = max_completion_tokens {
                                mi.max_completion_tokens.update(cx, |input, cx| {
                                    input.editor().update(cx, |editor, cx| {
                                        editor.set_text(v.to_string().as_str(), inner_window, cx);
                                    });
                                });
                            }
                            if let Some(v) = max_output_tokens {
                                mi.max_output_tokens.update(cx, |input, cx| {
                                    input.editor().update(cx, |editor, cx| {
                                        editor.set_text(v.to_string().as_str(), inner_window, cx);
                                    });
                                });
                            }
                            new_models.push(mi);
                        }
                        this.input.models = new_models;
                        this.last_error = None;
                        cx.notify();
                    })
                    .ok();

                    Ok(())
                })
                .detach_and_log_err(cx);
            },
        );

        v_flex()
            .mt_1()
            .gap_2()
            .child(
                h_flex()
                    .justify_between()
                    .child(Label::new("Models").size(LabelSize::Small))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("refresh-models", "Refresh")
                                    .label_size(LabelSize::Small)
                                    .style(ButtonStyle::Subtle)
                                    .on_click(refresh_handler),
                            )
                            .child(
                                Button::new("add-model", "Add Model")
                                    .icon(IconName::Plus)
                                    .icon_position(IconPosition::Start)
                                    .icon_size(IconSize::XSmall)
                                    .icon_color(Color::Muted)
                                    .label_size(LabelSize::Small)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.input.add_model(window, cx);
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .children(
                self.input
                    .models
                    .iter()
                    .enumerate()
                    .map(|(ix, _)| self.render_model(ix, cx)),
            )
    }

    fn render_model(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let has_more_than_one_model = self.input.models.len() > 1;
        let model = &self.input.models[ix];

        v_flex()
            .p_2()
            .gap_2()
            .rounded_sm()
            .border_1()
            .border_dashed()
            .border_color(cx.theme().colors().border.opacity(0.6))
            .bg(cx.theme().colors().element_active.opacity(0.15))
            .child(model.name.clone())
            .child(
                h_flex()
                    .gap_2()
                    .child(model.max_completion_tokens.clone())
                    .child(model.max_output_tokens.clone()),
            )
            .child(model.max_tokens.clone())
            .when(has_more_than_one_model, |this| {
                this.child(
                    Button::new(("remove-model", ix), "Remove Model")
                        .icon(IconName::Trash)
                        .icon_position(IconPosition::Start)
                        .icon_size(IconSize::XSmall)
                        .icon_color(Color::Muted)
                        .label_size(LabelSize::Small)
                        .style(ButtonStyle::Subtle)
                        .full_width()
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            this.input.remove_model(ix);
                            cx.notify();
                        })),
                )
            })
    }
}

impl EventEmitter<DismissEvent> for AddLlmProviderModal {}

impl Focusable for AddLlmProviderModal {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for AddLlmProviderModal {}

impl Render for AddLlmProviderModal {
    fn render(&mut self, window: &mut ui::Window, cx: &mut ui::Context<Self>) -> impl IntoElement {
        let focus_handle = self.focus_handle(cx);

        div()
            .id("add-llm-provider-modal")
            .key_context("AddLlmProviderModal")
            .w(rems(34.))
            .elevation_3(cx)
            .on_action(cx.listener(Self::cancel))
            .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                this.focus_handle(cx).focus(window);
            }))
            .child(
                Modal::new("configure-context-server", None)
                    .header(ModalHeader::new().headline("Add LLM Provider"))
                    .when_some(self.last_error.clone(), |this, error| {
                        this.section(
                            Section::new().child(
                                Banner::new()
                                    .severity(ui::Severity::Warning)
                                    .child(div().text_xs().child(error)),
                            ),
                        )
                    })
                    .child(
                        v_flex()
                            .id("modal_content")
                            .size_full()
                            .max_h_128()
                            .overflow_y_scroll()
                            .px(DynamicSpacing::Base12.rems(cx))
                            .gap(DynamicSpacing::Base04.rems(cx))
                            .child(self.input.provider_name.clone())
                            .child(self.input.api_url.clone())
                            .child(self.input.api_key.clone())
                            .child(self.render_model_section(cx)),
                    )
                    .footer(
                        ModalFooter::new().end_slot(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new("cancel", "Cancel")
                                        .key_binding(
                                            KeyBinding::for_action_in(
                                                &menu::Cancel,
                                                &focus_handle,
                                                window,
                                                cx,
                                            )
                                            .map(|kb| kb.size(rems_from_px(12.))),
                                        )
                                        .on_click(cx.listener(|this, _event, window, cx| {
                                            this.cancel(&menu::Cancel, window, cx)
                                        })),
                                )
                                .child(
                                    Button::new("save-server", "Save Provider")
                                        .key_binding(
                                            KeyBinding::for_action_in(
                                                &menu::Confirm,
                                                &focus_handle,
                                                window,
                                                cx,
                                            )
                                            .map(|kb| kb.size(rems_from_px(12.))),
                                        )
                                        .on_click(cx.listener(|this, _event, window, cx| {
                                            this.confirm(&menu::Confirm, window, cx)
                                        })),
                                ),
                        ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use editor::EditorSettings;
    use fs::FakeFs;
    use gpui::{TestAppContext, VisualTestContext};
    use language::language_settings;
    use language_model::{
        LanguageModelProviderId, LanguageModelProviderName,
        fake_provider::FakeLanguageModelProvider,
    };
    use project::Project;
    use settings::{Settings as _, SettingsStore};
    use util::path;

    #[gpui::test]
    async fn test_save_provider_invalid_inputs(cx: &mut TestAppContext) {
        let cx = setup_test(cx).await;

        assert_eq!(
            save_provider_validation_errors("", "someurl", "somekey", vec![], cx,).await,
            Some("Provider Name cannot be empty".into())
        );

        assert_eq!(
            save_provider_validation_errors("someprovider", "", "somekey", vec![], cx,).await,
            Some("API URL cannot be empty".into())
        );

        assert_eq!(
            save_provider_validation_errors("someprovider", "someurl", "", vec![], cx,).await,
            Some("API Key cannot be empty".into())
        );

        assert_eq!(
            save_provider_validation_errors(
                "someprovider",
                "someurl",
                "somekey",
                vec![("", "200000", "200000", "32000")],
                cx,
            )
            .await,
            Some("Model Name cannot be empty".into())
        );

        assert_eq!(
            save_provider_validation_errors(
                "someprovider",
                "someurl",
                "somekey",
                vec![("somemodel", "abc", "200000", "32000")],
                cx,
            )
            .await,
            Some("Max Tokens must be a number".into())
        );

        assert_eq!(
            save_provider_validation_errors(
                "someprovider",
                "someurl",
                "somekey",
                vec![("somemodel", "200000", "abc", "32000")],
                cx,
            )
            .await,
            Some("Max Completion Tokens must be a number".into())
        );

        assert_eq!(
            save_provider_validation_errors(
                "someprovider",
                "someurl",
                "somekey",
                vec![("somemodel", "200000", "200000", "abc")],
                cx,
            )
            .await,
            Some("Max Output Tokens must be a number".into())
        );

        assert_eq!(
            save_provider_validation_errors(
                "someprovider",
                "someurl",
                "somekey",
                vec![
                    ("somemodel", "200000", "200000", "32000"),
                    ("somemodel", "200000", "200000", "32000"),
                ],
                cx,
            )
            .await,
            Some("Model Names must be unique".into())
        );
    }

    #[gpui::test]
    async fn test_save_provider_name_conflict(cx: &mut TestAppContext) {
        let cx = setup_test(cx).await;

        cx.update(|_window, cx| {
            LanguageModelRegistry::global(cx).update(cx, |registry, cx| {
                registry.register_provider(
                    FakeLanguageModelProvider::new(
                        LanguageModelProviderId::new("someprovider"),
                        LanguageModelProviderName::new("Some Provider"),
                    ),
                    cx,
                );
            });
        });

        assert_eq!(
            save_provider_validation_errors(
                "someprovider",
                "someurl",
                "someapikey",
                vec![("somemodel", "200000", "200000", "32000")],
                cx,
            )
            .await,
            Some("Provider Name is already taken by another provider".into())
        );
    }

    async fn setup_test(cx: &mut TestAppContext) -> &mut VisualTestContext {
        cx.update(|cx| {
            let store = SettingsStore::test(cx);
            cx.set_global(store);
            workspace::init_settings(cx);
            Project::init_settings(cx);
            theme::init(theme::LoadThemes::JustBase, cx);
            language_settings::init(cx);
            EditorSettings::register(cx);
            language_model::init_settings(cx);
            language_models::init_settings(cx);
        });

        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| <dyn Fs>::set_global(fs.clone(), cx));
        let project = Project::test(fs, [path!("/dir").as_ref()], cx).await;
        let (_, cx) =
            cx.add_window_view(|window, cx| Workspace::test_new(project.clone(), window, cx));

        cx
    }

    async fn save_provider_validation_errors(
        provider_name: &str,
        api_url: &str,
        api_key: &str,
        models: Vec<(&str, &str, &str, &str)>,
        cx: &mut VisualTestContext,
    ) -> Option<SharedString> {
        fn set_text(
            input: &Entity<SingleLineInput>,
            text: &str,
            window: &mut Window,
            cx: &mut App,
        ) {
            input.update(cx, |input, cx| {
                input.editor().update(cx, |editor, cx| {
                    editor.set_text(text, window, cx);
                });
            });
        }

        let task = cx.update(|window, cx| {
            let mut input = AddLlmProviderInput::new(LlmCompatibleProvider::OpenAi, window, cx);
            set_text(&input.provider_name, provider_name, window, cx);
            set_text(&input.api_url, api_url, window, cx);
            set_text(&input.api_key, api_key, window, cx);

            for (i, (name, max_tokens, max_completion_tokens, max_output_tokens)) in
                models.iter().enumerate()
            {
                if i >= input.models.len() {
                    input.models.push(ModelInput::new(window, cx));
                }
                let model = &mut input.models[i];
                set_text(&model.name, name, window, cx);
                set_text(&model.max_tokens, max_tokens, window, cx);
                set_text(
                    &model.max_completion_tokens,
                    max_completion_tokens,
                    window,
                    cx,
                );
                set_text(&model.max_output_tokens, max_output_tokens, window, cx);
            }
            save_provider_to_settings(&input, cx)
        });

        task.await.err()
    }
}
