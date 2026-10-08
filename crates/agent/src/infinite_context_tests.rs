use super::tests::{create_test_project, init_test_settings, setup_test_environment};
use super::*;
use anyhow::Context as _;
use futures::{future::BoxFuture, stream::BoxStream};
use gpui::TestAppContext;
use language_model::{
    LanguageModelImage, LanguageModelName, LanguageModelProviderId, LanguageModelProviderName,
    LanguageModelToolChoice,
    fake_provider::{FakeLanguageModel, FakeLanguageModelProvider},
};
use serde_json::json;
use std::path::PathBuf;
use workspace::Workspace;

macro_rules! gpui_result_test {
    ($name:ident, $cx:ident, $body:block) => {
        #[gpui::test]
        async fn $name($cx: &mut TestAppContext) {
            let result: Result<()> = async $body.await;
            result.unwrap_or_else(|error| panic!("{} failed: {error:#}", stringify!($name)));
        }
    };
}

struct ToolCapableModel {
    inner: Arc<dyn LanguageModel>,
    cache_usage: Vec<LanguageModelCacheUsage>,
}

impl LanguageModel for ToolCapableModel {
    fn id(&self) -> LanguageModelId {
        self.inner.id()
    }

    fn name(&self) -> LanguageModelName {
        self.inner.name()
    }

    fn provider_id(&self) -> LanguageModelProviderId {
        self.inner.provider_id()
    }

    fn provider_name(&self) -> LanguageModelProviderName {
        self.inner.provider_name()
    }

    fn telemetry_id(&self) -> String {
        self.inner.telemetry_id()
    }

    fn supports_images(&self) -> bool {
        self.inner.supports_images()
    }

    fn supports_tools(&self) -> bool {
        true
    }

    fn supports_tool_choice(&self, choice: LanguageModelToolChoice) -> bool {
        self.inner.supports_tool_choice(choice)
    }

    fn max_token_count(&self) -> u64 {
        self.inner.max_token_count()
    }

    fn count_tokens(
        &self,
        request: LanguageModelRequest,
        cx: &App,
    ) -> BoxFuture<'static, Result<u64>> {
        self.inner.count_tokens(request, cx)
    }

    fn stream_completion(
        &self,
        request: LanguageModelRequest,
        cx: &AsyncApp,
    ) -> BoxFuture<
        'static,
        Result<
            BoxStream<'static, Result<LanguageModelCompletionEvent, LanguageModelCompletionError>>,
            LanguageModelCompletionError,
        >,
    > {
        let completion = self.inner.stream_completion(request, cx);
        let cache_usage = self.cache_usage.clone();
        async move {
            let stream = completion.await?;
            Ok(stream
                .chain(futures::stream::iter(cache_usage.into_iter().map(
                    |usage| Ok(LanguageModelCompletionEvent::CacheUsageUpdate(usage)),
                )))
                .boxed())
        }
        .boxed()
    }

    fn as_fake(&self) -> &FakeLanguageModel {
        self.inner.as_fake()
    }
}

struct MemoryDirectory(PathBuf);

impl MemoryDirectory {
    fn new() -> Result<Self> {
        let path = std::env::temp_dir().join(format!("zed-infinite-context-{}", Uuid::new_v4()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }
}

impl Drop for MemoryDirectory {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0) {
            log::error!(
                "Failed to clean up infinite-context test directory {:?}: {error}",
                self.0
            );
        }
    }
}

struct Fixture {
    thread: Entity<Thread>,
    model: Arc<dyn LanguageModel>,
    _workspace: Entity<Workspace>,
    _thread_store: Entity<ThreadStore>,
    directory: MemoryDirectory,
}

impl Fixture {
    async fn new(cx: &mut TestAppContext) -> Result<Self> {
        init_test_settings(cx);
        let project = create_test_project(cx, json!({"test.rs": "fn main() {}"})).await;
        let (workspace, thread_store, thread, _, model) = setup_test_environment(cx, project).await;
        let model: Arc<dyn LanguageModel> = Arc::new(ToolCapableModel {
            inner: model,
            cache_usage: Vec::new(),
        });
        let provider = Arc::new(FakeLanguageModelProvider);
        cx.update(|cx| {
            LanguageModelRegistry::global(cx).update(cx, |registry, cx| {
                registry.set_default_model(
                    Some(ConfiguredModel {
                        provider: provider.clone(),
                        model: model.clone(),
                    }),
                    cx,
                );
                registry.set_thread_summary_model(
                    Some(ConfiguredModel {
                        provider: provider.clone(),
                        model: model.clone(),
                    }),
                    cx,
                );
            });
        });
        let directory = MemoryDirectory::new()?;
        thread.update(cx, |thread, _| -> Result<()> {
            // Install isolated storage before any operation can open the user's data directory.
            thread.infinite_context = Some(InfiniteContext::open(directory.0.clone())?);
            thread.summary = ThreadSummary::Ready("Infinite-context regression test".into());
            thread.configured_model = Some(ConfiguredModel {
                provider,
                model: model.clone(),
            });
            Ok(())
        })?;
        Ok(Self {
            thread,
            model,
            _workspace: workspace,
            _thread_store: thread_store,
            directory,
        })
    }

    fn enable_without_background(&self, cx: &mut TestAppContext) -> Result<()> {
        self.thread.update(cx, |thread, cx| {
            thread.set_infinite_context_enabled(true, cx)?;
            thread.cancel_memory_summaries(cx);
            anyhow::Ok(())
        })
    }

    fn close(&self, cx: &mut TestAppContext) {
        self.thread.update(cx, |thread, cx| {
            thread.cancel_last_completion(None, cx);
            thread.cancel_memory_summaries(cx);
            thread.infinite_context_enabled = false;
            thread.infinite_context = None;
        });
        for request in self.model.as_fake().pending_completions() {
            self.model.as_fake().end_completion_stream(&request);
        }
        cx.run_until_parked();
    }
}

#[test]
fn measured_cache_usage_replaces_snapshots_and_counts_requests_once() -> Result<()> {
    let mut totals = CacheUsageTotals::default();
    let mut first = None;
    assert_eq!(totals.cache_read_percentage(), None);
    let zero = LanguageModelCacheUsage {
        input_tokens: 100,
        cached_tokens: 0,
    };
    totals.update(&mut first, zero)?;
    assert_eq!(totals.cache_read_percentage(), Some(0.0));
    totals.update(&mut first, zero)?;
    assert_eq!(totals.requests, 1);
    let mut second = None;
    totals.update(
        &mut second,
        LanguageModelCacheUsage {
            input_tokens: 200,
            cached_tokens: 100,
        },
    )?;
    totals.update(
        &mut first,
        LanguageModelCacheUsage {
            input_tokens: 50,
            cached_tokens: 25,
        },
    )?;
    assert_eq!(
        totals,
        CacheUsageTotals {
            requests: 2,
            input_tokens: 250,
            cached_tokens: 125
        }
    );
    assert_eq!(totals.cache_read_percentage(), Some(50.0));
    let before = totals;
    let previous = first;
    assert!(
        totals
            .update(
                &mut first,
                LanguageModelCacheUsage {
                    input_tokens: 10,
                    cached_tokens: 11
                }
            )
            .is_err()
    );
    assert_eq!(totals, before);
    assert_eq!(first, previous);
    assert!(
        totals
            .update(
                &mut first,
                LanguageModelCacheUsage {
                    input_tokens: u64::MAX,
                    cached_tokens: 0
                }
            )
            .is_err()
    );
    assert_eq!(totals, before);
    Ok(())
}

gpui_result_test!(
    measured_cache_usage_streams_agent_and_summary_and_survives_reopen,
    cx,
    {
        let fixture = Fixture::new(cx).await?;
        let usage = LanguageModelCacheUsage {
            input_tokens: 100,
            cached_tokens: 80,
        };
        let model: Arc<dyn LanguageModel> = Arc::new(ToolCapableModel {
            inner: fixture.model.clone(),
            cache_usage: vec![usage, usage],
        });
        fixture.thread.update(cx, |thread, cx| {
            insert_user(thread, "Measured request", cx);
            thread.send_to_model(model.clone(), CompletionIntent::UserPrompt, None, cx);
        });
        cx.run_until_parked();
        let request = model
            .as_fake()
            .pending_completions()
            .into_iter()
            .find(|request| request.intent == Some(CompletionIntent::UserPrompt))
            .context("Missing measured agent request")?;
        model
            .as_fake()
            .stream_completion_response(&request, "Measured reply");
        model.as_fake().end_completion_stream(&request);
        cx.run_until_parked();
        fixture.thread.read_with(cx, |thread, _| {
            assert_eq!(
                thread.measured_cache_usage().agent,
                CacheUsageTotals {
                    requests: 1,
                    input_tokens: 100,
                    cached_tokens: 80
                }
            );
            assert_eq!(
                thread.measured_cache_usage().summary,
                CacheUsageTotals::default()
            );
            assert_eq!(
                thread.cumulative_token_usage(),
                TokenUsage::default(),
                "Cache measurement events must not duplicate accounting usage"
            );
        });
        let summary_model = model.clone();
        let summary_thread = fixture.thread.downgrade();
        let task = cx.spawn(async move |cx| {
            summarize_memory_node(
                summary_model,
                LanguageModelRequest::default(),
                summary_thread,
                &cx,
            )
            .await
        });
        cx.run_until_parked();
        let request = model
            .as_fake()
            .pending_completions()
            .into_iter()
            .find(|request| request.intent != Some(CompletionIntent::UserPrompt))
            .context("Missing measured summary request")?;
        model
            .as_fake()
            .stream_completion_response(&request, "A short summary");
        model.as_fake().end_completion_stream(&request);
        assert_eq!(task.await?, "A short summary");
        let serialized = fixture
            .thread
            .update(cx, |thread, cx| thread.serialize(cx))
            .await?;
        assert_eq!(serialized.measured_cache_usage.summary.requests, 1);
        assert_eq!(
            serialized
                .measured_cache_usage
                .combined()
                .context("Combined totals overflow")?,
            CacheUsageTotals {
                requests: 2,
                input_tokens: 200,
                cached_tokens: 160
            }
        );
        let bytes = serde_json::to_vec(&serialized)?;
        let saved = SerializedThread::from_json(&bytes)?;
        assert_eq!(saved.measured_cache_usage, serialized.measured_cache_usage);
        let (project, tools, builder, project_context) =
            fixture.thread.read_with(cx, |thread, _| {
                (
                    thread.project.clone(),
                    thread.tools.clone(),
                    thread.prompt_builder.clone(),
                    thread.project_context.clone(),
                )
            });
        let restored = cx.new(|cx| {
            Thread::deserialize(
                ThreadId::new(),
                saved,
                project,
                tools,
                builder,
                project_context,
                None,
                cx,
            )
        });
        assert_eq!(
            restored.read_with(cx, |thread, _| thread.measured_cache_usage()),
            serialized.measured_cache_usage
        );
        let mut legacy = serde_json::from_slice::<serde_json::Value>(&bytes)?;
        legacy
            .as_object_mut()
            .context("Missing saved object")?
            .remove("measured_cache_usage");
        let legacy = SerializedThread::from_json(&serde_json::to_vec(&legacy)?)?;
        assert_eq!(legacy.measured_cache_usage, MeasuredCacheUsage::default());
        fixture.close(cx);
        Ok(())
    }
);

fn memory(thread: &mut Thread) -> Result<&mut InfiniteContext> {
    thread
        .infinite_context
        .as_mut()
        .context("Test memory is not open")
}

fn insert_user(thread: &mut Thread, text: &str, cx: &mut Context<Thread>) -> MessageId {
    thread.insert_user_message(text, ContextLoadResult::default(), None, Vec::new(), cx)
}

gpui_result_test!(
    infinite_context_cache_replay_snapshot_is_immediate_and_read_only,
    cx,
    {
        let fixture = Fixture::new(cx).await?;
        fixture.thread.update(cx, |thread, cx| {
            insert_user(thread, "Captured message", cx);
        });
        let before = std::fs::read_dir(&fixture.directory.0)?.count();
        let snapshot = fixture
            .thread
            .read_with(cx, |thread, cx| thread.snapshot_for_cache_replay(cx));
        fixture.thread.update(cx, |thread, cx| {
            insert_user(thread, "Later message", cx);
        });
        assert_eq!(snapshot.messages.len(), 1);
        assert_eq!(
            snapshot
                .messages
                .first()
                .context("Missing snapshot message")?
                .segments,
            vec![SerializedMessageSegment::Text {
                text: "Captured message".into()
            }]
        );
        assert_eq!(
            fixture
                .thread
                .read_with(cx, |thread, _| thread.messages().len()),
            2
        );
        assert_eq!(std::fs::read_dir(&fixture.directory.0)?.count(), before);
        let report = crate::cache_replay::replay_json(
            &serde_json::to_vec(&snapshot)?,
            &crate::cache_replay::ReplayOptions::default(),
            &std::sync::atomic::AtomicBool::new(false),
        )?;
        assert_eq!(report.source_counts.users, 1);
        fixture.close(cx);
        Ok(())
    }
);

fn drain_with_stub_summaries(memory: &mut InfiniteContext) -> Result<()> {
    while let Some(job) = memory.next_job()? {
        memory.complete_job(
            job.key,
            format!("Bounded summary at {}:{}", job.key.l, job.key.i),
        )?;
    }
    assert!(memory.all_summarized_before(memory.message_count()));
    Ok(())
}

fn insert_completed_tool(
    thread: &mut Thread,
    message_id: MessageId,
    id: &str,
    output: &str,
    model: Arc<dyn LanguageModel>,
    cx: &Context<Thread>,
) {
    let tool_id = LanguageModelToolUseId::from(id.to_string());
    thread.tool_use.request_tool_use(
        message_id,
        LanguageModelToolUse {
            id: tool_id.clone(),
            name: "test_tool".into(),
            raw_input: "{\"query\":\"test\"}".into(),
            input: json!({"query": "test"}),
            is_input_complete: true,
        },
        ToolUseMetadata {
            model,
            thread_id: thread.id.clone(),
            prompt_id: thread.last_prompt_id.clone(),
        },
        cx,
    );
    assert!(
        thread
            .tool_use
            .insert_tool_output(
                tool_id,
                "test_tool".into(),
                Ok(output.to_string().into()),
                None,
            )
            .is_some()
    );
}

gpui_result_test!(infinite_context_is_opt_in_for_new_threads, cx, {
    let fixture = Fixture::new(cx).await?;
    fixture.thread.read_with(cx, |thread, _| {
        assert!(!thread.infinite_context_enabled());
        assert!(!thread.memory_history_is_append_only());
        assert_eq!(thread.memory_turn_start, None);
    });

    cx.update(|cx| {
        AgentSettings::override_global(
            AgentSettings {
                infinite_context: true,
                ..AgentSettings::get_global(cx).clone()
            },
            cx,
        );
    });
    let (project, tools, prompt_builder, project_context) =
        fixture.thread.read_with(cx, |thread, _| {
            (
                thread.project.clone(),
                thread.tools.clone(),
                thread.prompt_builder.clone(),
                thread.project_context.clone(),
            )
        });
    let new_thread = cx.new(|cx| Thread::new(project, tools, prompt_builder, project_context, cx));
    new_thread.read_with(cx, |thread, _| {
        assert!(thread.infinite_context_enabled());
        assert!(thread.infinite_context.is_none());
    });
    fixture
        .thread
        .read_with(cx, |thread, _| assert!(!thread.infinite_context_enabled()));
    fixture.close(cx);
    Ok(())
});

gpui_result_test!(
    infinite_context_imports_text_not_thoughts_and_tracks_new_turn,
    cx,
    {
        let fixture = Fixture::new(cx).await?;
        fixture.thread.update(cx, |thread, cx| {
            insert_user(thread, "Archived user text", cx);
            thread.insert_assistant_message(
                vec![
                    MessageSegment::Thinking {
                        text: "PRIVATE_THOUGHT".into(),
                        signature: Some("PRIVATE_SIGNATURE".into()),
                    },
                    MessageSegment::RedactedThinking("PRIVATE_REDACTED".into()),
                    MessageSegment::Text("Archived assistant text".into()),
                ],
                cx,
            );
        });
        fixture.enable_without_background(cx)?;
        fixture.thread.update(cx, |thread, cx| -> Result<()> {
            assert_eq!(thread.memory_turn_start, None);
            assert_eq!(memory(thread)?.message_count(), 2);
            assert!(
                memory(thread)?.next_job()?.is_none(),
                "Short nodes must not need a model call"
            );
            let view = memory(thread)?.render_view()?;
            assert!(view.contains("user: Archived user text"));
            assert!(view.contains("unii: Archived assistant text"));
            assert!(!view.contains("PRIVATE_"));
            assert!(!memory(thread)?.zoom(1, 1, 0)?.contains("PRIVATE_"));
            thread.sync_memory()?;
            assert_eq!(
                memory(thread)?.message_count(),
                2,
                "Sync must be idempotent"
            );

            let first = insert_user(thread, "First new turn", cx);
            assert_eq!(thread.memory_turn_start, Some((first, 2)));
            assert_eq!(memory(thread)?.message_count(), 3);
            thread.insert_assistant_message(
                vec![MessageSegment::Text("First new response".into())],
                cx,
            );
            let second = insert_user(thread, "Second new turn", cx);
            assert_eq!(thread.memory_turn_start, Some((second, 4)));
            assert_eq!(memory(thread)?.message_count(), 5);
            assert!(memory(thread)?.next_job()?.is_none());
            assert!(memory(thread)?.all_summarized_before(5));
            Ok(())
        })?;
        assert_eq!(fixture.model.as_fake().completion_count(), 0);
        fixture.close(cx);
        Ok(())
    }
);

gpui_result_test!(
    infinite_context_request_has_bounded_history_and_paired_current_tools,
    cx,
    {
        let fixture = Fixture::new(cx).await?;
        let archived_user = format!("ARCHIVED_USER_RAW {}", "old user details ".repeat(150));
        let archived_assistant = format!(
            "ARCHIVED_ASSISTANT_RAW {}",
            "old assistant details ".repeat(150)
        );
        fixture.thread.update(cx, |thread, cx| {
            insert_user(thread, &archived_user, cx);
            let assistant = thread.insert_assistant_message(
                vec![MessageSegment::Text(archived_assistant.clone())],
                cx,
            );
            insert_completed_tool(
                thread,
                assistant,
                "archived-tool",
                "Archived tool output",
                fixture.model.clone(),
                cx,
            );
        });
        fixture.enable_without_background(cx)?;
        let request = fixture.thread.update(cx, |thread, cx| -> Result<_> {
            drain_with_stub_summaries(memory(thread)?)?;
            let start = memory(thread)?.message_count();
            let user = insert_user(thread, "CURRENT_USER_RAW", cx);
            assert_eq!(thread.memory_turn_start, Some((user, start)));
            let assistant = thread.insert_assistant_message(
                vec![MessageSegment::Text("CURRENT_ASSISTANT_RAW".into())],
                cx,
            );
            insert_completed_tool(
                thread,
                assistant,
                "current-tool",
                "CURRENT_TOOL_OUTPUT",
                fixture.model.clone(),
                cx,
            );
            thread.sync_memory()?;
            Ok(thread.to_completion_request(
                fixture.model.clone(),
                CompletionIntent::ToolResults,
                cx,
            ))
        })?;
        for name in ["zoom", "date"] {
            assert_eq!(
                request
                    .tools
                    .iter()
                    .filter(|tool| tool.name == name)
                    .count(),
                1
            );
        }
        let contents = request
            .messages
            .iter()
            .map(|message| message.string_contents())
            .collect::<Vec<_>>();
        let text = contents.join("\n");
        assert!(text.contains("Bounded summary at"));
        assert!(!text.contains(&archived_user));
        assert!(!text.contains(&archived_assistant));
        assert_eq!(text.matches("CURRENT_USER_RAW").count(), 1);
        assert_eq!(text.matches("CURRENT_ASSISTANT_RAW").count(), 1);
        assert_eq!(text.matches("CURRENT_TOOL_OUTPUT").count(), 1);
        let mut uses = Vec::new();
        let mut results = Vec::new();
        for message in &request.messages {
            for content in &message.content {
                match content {
                    MessageContent::ToolUse(tool) => {
                        assert_eq!(message.role, Role::Assistant);
                        uses.push(tool.id.clone());
                    }
                    MessageContent::ToolResult(result) => {
                        assert_eq!(message.role, Role::User);
                        results.push(result.tool_use_id.clone());
                    }
                    _ => {}
                }
            }
        }
        assert_eq!(
            uses,
            vec![LanguageModelToolUseId::from("current-tool".to_string())]
        );
        assert_eq!(results, uses);
        assert_eq!(fixture.model.as_fake().completion_count(), 0);
        fixture.close(cx);
        Ok(())
    }
);

gpui_result_test!(infinite_context_retrieves_zoom_date_and_images, cx, {
    let fixture = Fixture::new(cx).await?;
    let image: LanguageModelImage = serde_json::from_value(json!({
        "source": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aZ1sAAAAASUVORK5CYII=",
        "size": {"width": 1, "height": 1}
    }))?;
    fixture.thread.update(cx, |thread, cx| {
        thread.insert_message(
            Role::User,
            vec![MessageSegment::Text("Attached image".into())],
            LoadedContext {
                images: vec![image.clone()],
                ..Default::default()
            },
            Vec::new(),
            false,
            cx,
        );
    });
    fixture.enable_without_background(cx)?;
    fixture.thread.update(cx, |thread, _| -> Result<()> {
        assert!(memory(thread)?.next_job()?.is_none());
        let output = thread.memory_tool_output("zoom", &json!({"id": 0, "n": 1}))?;
        match output.content {
            assistant_tool::ToolResultContent::Text(text) => {
                assert!(text.contains("Attached image"))
            }
            _ => anyhow::bail!("Plain zoom should return text"),
        }
        let output = thread.memory_tool_output("date", &json!({"id": 0}))?;
        match output.content {
            assistant_tool::ToolResultContent::Text(text) => {
                DateTime::parse_from_rfc3339(&text)?;
            }
            _ => anyhow::bail!("Date should return text"),
        }
        let output = thread.memory_tool_output("zoom", &json!({"id": 0, "n": 1, "image": 0}))?;
        match output.content {
            assistant_tool::ToolResultContent::Image(actual) => assert!(actual == image),
            _ => anyhow::bail!("Image zoom should return the original image"),
        }
        Ok(())
    })?;
    fixture.close(cx);
    Ok(())
});

gpui_result_test!(infinite_context_rejects_bad_retrieval_inputs, cx, {
    let fixture = Fixture::new(cx).await?;
    fixture.thread.update(cx, |thread, cx| {
        insert_user(thread, "Retrievable message", cx);
    });
    fixture.enable_without_background(cx)?;
    fixture.thread.update(cx, |thread, _| -> Result<()> {
        assert!(memory(thread)?.next_job()?.is_none());
        for (name, input) in [
            ("unknown", json!({"id": 0, "n": 1})),
            ("zoom", json!({})),
            ("zoom", json!({"id": -1, "n": 1})),
            ("zoom", json!({"id": "0", "n": 1})),
            ("zoom", json!({"id": 0, "n": 0})),
            ("zoom", json!({"id": 0, "n": 3})),
            ("zoom", json!({"id": 99, "n": 1})),
            ("zoom", json!({"id": 0, "n": 1, "page": -1})),
            ("zoom", json!({"id": 0, "n": 1, "page": 99})),
            ("zoom", json!({"id": 0, "n": 1, "image": 0})),
            ("zoom", json!({"id": 0, "n": 1, "image": -1})),
            ("date", json!({"id": 99})),
            ("date", json!({"id": -1})),
        ] {
            assert!(
                thread.memory_tool_output(name, &input).is_err(),
                "Expected error for {name} {input}"
            );
        }
        Ok(())
    })?;
    fixture.close(cx);
    Ok(())
});

gpui_result_test!(
    infinite_context_serialization_preserves_choice_boundary_and_legacy_defaults,
    cx,
    {
        let fixture = Fixture::new(cx).await?;
        fixture.thread.update(cx, |thread, cx| {
            insert_user(thread, "Archived message", cx);
        });
        fixture.enable_without_background(cx)?;
        let boundary = fixture.thread.update(cx, |thread, cx| {
            let user = insert_user(thread, "Current message", cx);
            (user, 1)
        });
        let serialized = fixture
            .thread
            .update(cx, |thread, cx| thread.serialize(cx))
            .await?;
        assert!(serialized.infinite_context);
        assert!(serialized.memory_archived);
        assert_eq!(serialized.memory_turn_start, Some(boundary));
        let mut legacy_json = serde_json::to_value(&serialized)?;
        let object = legacy_json
            .as_object_mut()
            .context("Serialized thread must be an object")?;
        object.remove("infinite_context");
        object.remove("memory_archived");
        object.remove("memory_turn_start");
        let legacy: SerializedThread = serde_json::from_value(legacy_json)?;
        assert!(!legacy.infinite_context);
        assert!(!legacy.memory_archived);
        assert_eq!(legacy.memory_turn_start, None);

        let (project, tools, prompt_builder, project_context) =
            fixture.thread.read_with(cx, |thread, _| {
                (
                    thread.project.clone(),
                    thread.tools.clone(),
                    thread.prompt_builder.clone(),
                    thread.project_context.clone(),
                )
            });
        let restored = cx.new(|cx| {
            Thread::deserialize(
                ThreadId::new(),
                serialized,
                project.clone(),
                tools.clone(),
                prompt_builder.clone(),
                project_context.clone(),
                None,
                cx,
            )
        });
        restored.read_with(cx, |thread, _| {
            assert!(thread.infinite_context_enabled());
            assert!(thread.memory_history_is_append_only());
            assert_eq!(thread.memory_turn_start, Some(boundary));
        });
        fixture.close(cx);
        restored.update(cx, |thread, cx| -> Result<()> {
            thread.infinite_context = Some(InfiniteContext::open(fixture.directory.0.clone())?);
            assert_eq!(memory(thread)?.message_count(), 2);
            thread.sync_memory()?;
            assert_eq!(
                memory(thread)?.message_count(),
                2,
                "Restoring must not duplicate archived messages"
            );
            assert_eq!(thread.memory_turn_start, Some(boundary));
            assert!(memory(thread)?.next_job()?.is_none());
            let request = thread.to_completion_request(
                fixture.model.clone(),
                CompletionIntent::UserPrompt,
                cx,
            );
            assert_eq!(
                request
                    .messages
                    .iter()
                    .filter(|message| message.string_contents() == "Current message")
                    .count(),
                1
            );
            thread.infinite_context_enabled = false;
            thread.infinite_context = None;
            Ok(())
        })?;
        cx.update(|cx| {
            AgentSettings::override_global(
                AgentSettings {
                    infinite_context: true,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });
        let restored_legacy = cx.new(|cx| {
            Thread::deserialize(
                ThreadId::new(),
                legacy,
                project,
                tools,
                prompt_builder,
                project_context,
                None,
                cx,
            )
        });
        restored_legacy.read_with(cx, |thread, _| {
            assert!(
                !thread.infinite_context_enabled(),
                "Legacy threads must not inherit a new opt-in default"
            );
            assert!(!thread.memory_history_is_append_only());
        });
        fixture.close(cx);
        Ok(())
    }
);

gpui_result_test!(
    infinite_context_archived_history_stays_append_only_after_disabling,
    cx,
    {
        let fixture = Fixture::new(cx).await?;
        let first = fixture.thread.update(cx, |thread, cx| {
            let first = insert_user(thread, "Keep original message", cx);
            thread.insert_assistant_message(
                vec![MessageSegment::Text("Keep original response".into())],
                cx,
            );
            first
        });
        fixture.enable_without_background(cx)?;
        fixture.thread.update(cx, |thread, cx| -> Result<()> {
            let original = thread.text();
            for enabled in [true, false] {
                thread.set_infinite_context_enabled(enabled, cx)?;
                thread.cancel_memory_summaries(cx);
                assert!(thread.memory_history_is_append_only());
                assert!(!thread.edit_message(
                    first,
                    Role::User,
                    vec![MessageSegment::Text("Destructive replacement".into())],
                    Vec::new(),
                    None,
                    None,
                    cx
                ));
                assert!(!thread.delete_message(first, cx));
                thread.truncate(first, cx);
                assert_eq!(thread.text(), original);
                assert_eq!(thread.messages.len(), 2);
                assert_eq!(memory(thread)?.message_count(), 2);
            }
            insert_user(thread, "Append a correction instead", cx);
            assert_eq!(thread.messages.len(), 3);
            Ok(())
        })?;
        let serialized = fixture
            .thread
            .update(cx, |thread, cx| thread.serialize(cx))
            .await?;
        assert!(!serialized.infinite_context);
        assert!(
            serialized.memory_archived,
            "Protection must survive reopening with memory off"
        );
        fixture.close(cx);
        Ok(())
    }
);

gpui_result_test!(
    infinite_context_disable_cancels_summary_and_releases_claimed_jobs,
    cx,
    {
        let fixture = Fixture::new(cx).await?;
        fixture.thread.update(cx, |thread, cx| -> Result<()> {
            insert_user(thread, &"Large archived input ".repeat(100), cx);
            thread.set_infinite_context_enabled(true, cx)
        })?;
        cx.run_until_parked();
        let pending = fixture.model.as_fake().pending_completions();
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending.first().map(|request| request.intent),
            Some(Some(CompletionIntent::ThreadContextSummarization))
        );
        fixture.thread.update(cx, |thread, cx| -> Result<()> {
            assert!(thread.memory_summaries_running());
            thread.set_infinite_context_enabled(false, cx)?;
            assert!(!thread.memory_summaries_running());
            assert!(!thread.memory_preparing);
            let job = memory(thread)?
                .next_job()?
                .context("Canceled summary job must be immediately available again")?;
            assert_eq!(job.key, crate::infinite_context::NodeKey { l: 0, i: 0 });
            memory(thread)?.complete_job(job.key, "Recovered summary".into())?;
            assert!(memory(thread)?.all_summarized_before(1));
            Ok(())
        })?;
        fixture.close(cx);
        Ok(())
    }
);

gpui_result_test!(
    infinite_context_empty_history_sends_without_waiting_for_current_summary,
    cx,
    {
        let fixture = Fixture::new(cx).await?;
        fixture.enable_without_background(cx)?;
        fixture.thread.update(cx, |thread, cx| {
            insert_user(thread, &"Current long prompt ".repeat(100), cx);
            assert_eq!(thread.memory_turn_start.map(|(_, end)| end), Some(0));
            thread.send_to_model(
                fixture.model.clone(),
                CompletionIntent::UserPrompt,
                None,
                cx,
            );
        });
        cx.run_until_parked();
        let pending = fixture.model.as_fake().pending_completions();
        let sent_immediately = pending
            .iter()
            .any(|request| request.intent == Some(CompletionIntent::UserPrompt));
        let summaries: Vec<_> = pending
            .iter()
            .filter(|request| request.intent == Some(CompletionIntent::ThreadContextSummarization))
            .cloned()
            .collect();
        assert!(
            !summaries.is_empty(),
            "The long current message should have background summary work"
        );
        // Ending an empty fake stream makes summarization fail without needing a real provider.
        for request in &summaries {
            fixture.model.as_fake().end_completion_stream(request);
        }
        cx.run_until_parked();
        let user_request_survived = fixture
            .model
            .as_fake()
            .pending_completions()
            .iter()
            .any(|request| request.intent == Some(CompletionIntent::UserPrompt));
        let preparing = fixture
            .thread
            .read_with(cx, |thread, _| thread.memory_preparing);
        fixture.close(cx);
        assert!(
            sent_immediately,
            "An empty-history turn must not wait for its current-message summary"
        );
        assert!(
            user_request_survived,
            "Current-message summary failure must not prevent the user completion"
        );
        assert!(!preparing);
        Ok(())
    }
);

gpui_result_test!(
    infinite_context_failed_summary_releases_key_and_can_retry,
    cx,
    {
        let fixture = Fixture::new(cx).await?;
        fixture.thread.update(cx, |thread, cx| -> Result<()> {
            insert_user(
                thread,
                &"Archived input requiring a summary ".repeat(100),
                cx,
            );
            thread.set_infinite_context_enabled(true, cx)
        })?;
        cx.run_until_parked();
        let pending = fixture.model.as_fake().pending_completions();
        assert_eq!(pending.len(), 1);
        let request = pending.first().context("Missing initial summary request")?;
        assert_eq!(
            request.intent,
            Some(CompletionIntent::ThreadContextSummarization)
        );
        fixture.model.as_fake().end_completion_stream(request);
        cx.run_until_parked();
        let result = fixture
            .thread
            .read_with(cx, |thread, _| thread.memory_summaries.clone())
            .await;
        assert!(result.is_err(), "An empty summary stream must fail");
        fixture.thread.update(cx, |thread, cx| -> Result<()> {
            assert!(!thread.memory_summaries_running());
            let job = memory(thread)?
                .next_job()?
                .context("Failed summary key was not released")?;
            assert_eq!(job.key, crate::infinite_context::NodeKey { l: 0, i: 0 });
            memory(thread)?.release_in_flight_jobs()?;
            thread.start_memory_summaries(cx);
            Ok(())
        })?;
        cx.run_until_parked();
        let pending = fixture.model.as_fake().pending_completions();
        assert_eq!(pending.len(), 1);
        let retry = pending.first().context("Missing retried summary request")?;
        assert_eq!(
            retry.intent,
            Some(CompletionIntent::ThreadContextSummarization)
        );
        fixture
            .model
            .as_fake()
            .stream_completion_response(retry, "A bounded retried summary");
        fixture.model.as_fake().end_completion_stream(retry);
        cx.run_until_parked();
        let result = fixture
            .thread
            .read_with(cx, |thread, _| thread.memory_summaries.clone())
            .await;
        result.map_err(|error| anyhow!("Summary retry failed: {error:#}"))?;
        fixture.thread.update(cx, |thread, _| -> Result<()> {
            assert!(!thread.memory_summaries_running());
            assert!(memory(thread)?.all_summarized_before(1));
            assert!(
                memory(thread)?
                    .render_view()?
                    .contains("A bounded retried summary")
            );
            Ok(())
        })?;
        fixture.close(cx);
        Ok(())
    }
);

gpui_result_test!(
    infinite_context_stop_cancels_preparation_and_summary_drain,
    cx,
    {
        let fixture = Fixture::new(cx).await?;
        fixture.thread.update(cx, |thread, cx| {
            insert_user(
                thread,
                &"Prior history requiring a summary ".repeat(100),
                cx,
            );
        });
        fixture.enable_without_background(cx)?;
        fixture.thread.update(cx, |thread, cx| {
            insert_user(thread, &"Current turn requiring a summary ".repeat(100), cx);
            assert_eq!(thread.memory_turn_start.map(|(_, end)| end), Some(1));
            thread.send_to_model(
                fixture.model.clone(),
                CompletionIntent::UserPrompt,
                None,
                cx,
            );
        });
        cx.run_until_parked();
        let pending = fixture.model.as_fake().pending_completions();
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().all(|request| request.intent == Some(CompletionIntent::ThreadContextSummarization)));
        fixture.thread.update(cx, |thread, cx| -> Result<()> {
            assert!(thread.memory_preparing);
            assert!(thread.memory_summaries_running());
            assert!(thread.cancel_last_completion(None, cx));
            assert!(!thread.memory_preparing);
            assert!(!thread.memory_summaries_running());
            assert!(!thread.is_generating());
            assert!(
                thread.infinite_context_enabled(),
                "Stop must not disable the mode"
            );
            let mut recovered = Vec::new();
            while let Some(job) = memory(thread)?.next_job()? {
                recovered.push(job.key);
                memory(thread)?.complete_job(job.key, "Recovered after stop".into())?;
            }
            assert_eq!(recovered.len(), 2);
            for id in [0, 1] {
                assert!(recovered.contains(&crate::infinite_context::NodeKey { l: 0, i: id }));
            }
            assert!(memory(thread)?.all_summarized_before(2));
            assert!(!thread.cancel_last_completion(None, cx));
            Ok(())
        })?;
        for request in &pending {
            fixture.model.as_fake().end_completion_stream(request);
        }
        cx.run_until_parked();
        assert_eq!(
            fixture.model.as_fake().completion_count(),
            0,
            "Canceled preparation must not start a user completion later"
        );
        fixture.thread.read_with(cx, |thread, _| {
            assert!(!thread.memory_preparing);
            assert!(!thread.memory_summaries_running());
            assert!(thread.pending_completions.is_empty());
        });
        fixture.close(cx);
        Ok(())
    }
);

#[test]
fn infinite_context_tool_output_clipping_preserves_utf8_head_and_tail() {
    let at_limit = "🦀".repeat(30_000);
    assert_eq!(clip_memory_output(&at_limit), at_limit);
    let head = "🦀".repeat(15_000);
    let tail = "界".repeat(15_000);
    let input = format!("{head}OMIT_THIS_MIDDLE{tail}");
    let clipped = clip_memory_output(&input);
    assert!(clipped.starts_with(&"🦀".repeat(14_992)));
    assert!(clipped.ends_with(&"界".repeat(14_992)));
    assert!(clipped.contains("[middle omitted]"));
    assert_eq!(clipped.chars().count(), 30_000);
    assert!(!clipped.contains("OMIT_THIS_MIDDLE"));
    assert_eq!(clip_memory_output(""), "");
}

#[test]
fn infinite_context_cache_blocks_preserve_append_prefix() -> Result<()> {
    fn view(count: usize) -> String {
        let lines = (0..count)
            .map(|id| format!("{id}+1|message {id}\n"))
            .collect::<String>();
        format!("<chat>\n{lines}</chat>")
    }
    let mut before = LanguageModelRequest::default();
    append_memory_view(&mut before, view(8));
    let mut after = LanguageModelRequest::default();
    append_memory_view(&mut after, view(10));
    assert_eq!(before.messages.len(), 3);
    assert_eq!(after.messages.len(), 4);
    assert_eq!(before.messages.get(..2), after.messages.get(..2));
    assert!(
        before
            .messages
            .get(1)
            .context("Missing completed cache block")?
            .cache
    );
    assert!(
        after
            .messages
            .get(1)
            .context("Missing preserved cache block")?
            .cache
    );
    assert!(
        !after
            .messages
            .get(2)
            .context("Missing partial block")?
            .cache
    );
    assert_eq!(
        after
            .messages
            .last()
            .context("Missing closing tag")?
            .string_contents(),
        "</chat>"
    );
    assert!(
        after
            .messages
            .iter()
            .all(|message| message.role == Role::User)
    );
    let mut empty = LanguageModelRequest::default();
    append_memory_view(&mut empty, "<chat>\n</chat>".into());
    assert_eq!(empty.messages.len(), 1);
    let message = empty
        .messages
        .first()
        .context("Missing empty memory view")?;
    assert_eq!(message.string_contents(), "<chat>\n</chat>");
    assert!(!message.cache);
    Ok(())
}
