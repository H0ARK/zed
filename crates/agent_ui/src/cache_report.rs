use agent::{
    CacheUsageTotals, MeasuredCacheUsage,
    cache_replay::{
        ReplayCancelled, ReplayCleanupFailed, ReplayOptions, ReplayReport, replay_json,
    },
};
use agent_client_protocol::schema::v1 as acp;
use anyhow::Result;
use gpui::{Task, prelude::*};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use ui::{Tooltip, prelude::*};

pub(crate) fn snapshot_json(thread: agent::DbThread) -> Result<Vec<u8>> {
    #[derive(serde::Serialize)]
    struct Snapshot {
        version: &'static str,
        #[serde(flatten)]
        thread: agent::DbThread,
    }

    // DbThread alone omits the version envelope used by saved conversations.
    Ok(serde_json::to_vec(&Snapshot {
        version: agent::DbThread::VERSION,
        thread,
    })?)
}

fn measured_cache_totals_text(totals: Option<CacheUsageTotals>) -> String {
    let Some(totals) = totals else {
        return "unavailable".into();
    };
    if totals.requests == 0 {
        return "not reported".into();
    }
    let percentage = totals
        .cache_read_percentage()
        .map(|rate| format!("{rate:.2}%"))
        .unwrap_or_else(|| "N/A".into());
    format!(
        "{percentage} ({}/{} input tokens cached)",
        totals.cached_tokens, totals.input_tokens
    )
}

fn measured_cache_usage_text(usage: MeasuredCacheUsage) -> String {
    if usage.agent.requests == 0 && usage.summary.requests == 0 {
        return "Measured cache reuse: not reported".into();
    }
    format!(
        "Measured cache reuse: agent {} · summaries {} · combined {}",
        measured_cache_totals_text(Some(usage.agent)),
        measured_cache_totals_text(Some(usage.summary)),
        measured_cache_totals_text(usage.combined()),
    )
}

pub(crate) fn render_measured_cache_usage(usage: MeasuredCacheUsage) -> impl IntoElement {
    let combined_requests = usage
        .combined()
        .map(|totals| totals.requests.to_string())
        .unwrap_or_else(|| "unavailable".into());
    div()
        .id("measured-cache-reuse")
        .w_full()
        .flex_none()
        .px_2()
        .pb_1()
        .child(
            Label::new(measured_cache_usage_text(usage))
                .size(LabelSize::Small)
                .color(Color::Muted),
        )
        .tooltip(Tooltip::text(format!(
            "Cached / total input tokens; total input includes cached tokens. Reported requests: agent {}, summaries {}, combined {}. Only observed requests with explicit provider cache counters are measured, including live usage updates and memory-summary retries. Older history and requests from models without cache details are excluded. Not billing savings or an offline estimate. If not reported, Zed is waiting for usage or the provider does not support cache details. N/A means a percentage cannot be computed from the reported totals.",
            usage.agent.requests, usage.summary.requests, combined_requests,
        )))
}

pub(crate) struct CacheReport {
    thread_id: acp::SessionId,
    title: SharedString,
    state: ReportState,
    dismissed: bool,
    cancellation: Arc<AtomicBool>,
}

enum ReportState {
    Running,
    Cancelled,
    Failed(&'static str),
    Complete(Box<ReplayReport>),
}

impl CacheReport {
    pub(crate) fn new(
        thread_id: acp::SessionId,
        title: SharedString,
        input: Task<Result<Vec<u8>>>,
        cx: &mut Context<Self>,
        on_cleanup_failure: impl FnOnce(&mut App) + 'static,
    ) -> Self {
        let cancellation = Arc::new(AtomicBool::new(false));
        let flag = cancellation.clone();
        if !cfg!(unix) {
            return Self {
                thread_id,
                title,
                state: ReportState::Failed(
                    "Offline cache replay currently requires macOS or Linux for private temporary journal permissions.",
                ),
                dismissed: false,
                cancellation,
            };
        }
        // Keep consuming the worker result after dismissal so cleanup failures can
        // still warn the user. Dropping the entity cancels work via the shared flag.
        cx.spawn(async move |this, cx| {
            let state = match input.await {
                Ok(bytes) if !flag.load(Ordering::Relaxed) => {
                    let result = cx
                        .background_spawn(async move {
                            replay_json(&bytes, &ReplayOptions::default(), &flag)
                        })
                        .await;
                    match result {
                        Ok(report) => ReportState::Complete(Box::new(report)),
                        Err(error) if error.is::<ReplayCleanupFailed>() => {
                            cx.update(on_cleanup_failure);
                            ReportState::Failed("Replay could not remove its private temporary journal. Sensitive replay data may remain on disk; check Zed's log for the cleanup path.")
                        }
                        Err(error) if error.is::<ReplayCancelled>() => ReportState::Cancelled,
                        Err(error) if error.is::<std::io::Error>() => ReportState::Failed(
                            "Replay failed while reading or writing private temporary journals. Check available disk space and permissions.",
                        ),
                        Err(_) => ReportState::Failed(
                            "Replay failed: the snapshot contains unsupported or invalid conversation data. The offline CLI can provide detailed diagnostics.",
                        ),
                    }
                }
                Ok(_) => ReportState::Cancelled,
                Err(_) => ReportState::Failed(
                    "Could not load the conversation snapshot. It may have been deleted or its saved data could not be decoded.",
                ),
            };
            if let Some(this) = this.upgrade() {
                this.update(cx, |this, cx| {
                    if !this.cancellation.load(Ordering::Relaxed) {
                        this.state = state;
                    }
                    cx.notify();
                });
            }
        }).detach();
        Self {
            thread_id,
            title,
            state: ReportState::Running,
            dismissed: false,
            cancellation,
        }
    }

    pub(crate) fn warn_cleanup_failure(
        workspace: &gpui::WeakEntity<workspace::Workspace>,
        cx: &mut App,
    ) {
        let workspace = workspace.clone();
        cx.defer(move |cx| {
            if let Some(workspace) = workspace.upgrade() {
                workspace.update(cx, |workspace, cx| {
                    struct ReplayCleanupWarning;
                    workspace.show_toast(
                        workspace::Toast::new(
                            workspace::notifications::NotificationId::unique::<ReplayCleanupWarning>(),
                            "Offline replay could not remove its private journal. Sensitive data may remain in your temporary directory. Check Zed's log for the cleanup path.",
                        ),
                        cx,
                    );
                });
            }
        });
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        self.cancellation.store(true, Ordering::Relaxed);
        self.state = ReportState::Cancelled;
        cx.notify();
    }
}

impl Drop for CacheReport {
    fn drop(&mut self) {
        self.cancellation.store(true, Ordering::Relaxed);
    }
}

fn percentage(rate: Option<f64>) -> String {
    rate.map(|rate| format!("{rate:.2}%"))
        .unwrap_or_else(|| "N/A".into())
}

impl Render for CacheReport {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.dismissed {
            return div().into_any_element();
        }
        let running = matches!(self.state, ReportState::Running);
        v_flex()
            .flex_none()
            .w_full()
            .p_2()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                h_flex()
                    .gap_1()
                    .justify_between()
                    .child(
                        div().id("cache-report-source").flex_1().min_w_0()
                            .child(Label::new(format!("Offline estimate · {}", self.title))
                                .size(LabelSize::Small)
                                .truncate())
                            .tooltip(Tooltip::text(format!(
                                "Snapshot of thread {}. This report remains tied to that snapshot when you switch threads.",
                                self.thread_id
                            ))),
                    )
                    .child(
                        Button::new("close-cache-report", if running { "Cancel" } else { "Close" })
                            .label_size(LabelSize::Small)
                            .style(ButtonStyle::Subtle)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel(cx);
                                this.dismissed = true;
                            })),
                    ),
            )
            .child(match &self.state {
                ReportState::Running => Label::new("Replaying snapshot locally… No model calls or tools are executed.")
                    .size(LabelSize::Small)
                    .color(Color::Muted)
                    .into_any_element(),
                ReportState::Cancelled => Label::new("Replay cancelled.")
                    .size(LabelSize::Small)
                    .color(Color::Muted)
                    .into_any_element(),
                ReportState::Failed(message) => Label::new(*message)
                    .size(LabelSize::Small)
                    .color(Color::Error)
                    .into_any_element(),
                ReportState::Complete(report) => v_flex()
                    .gap_1()
                    .child(Label::new(format!(
                        "Cache reuse: agent {} · summaries {} · combined {}",
                        percentage(report.agent().warm.cache_read_percentage),
                        percentage(report.summary().warm.cache_read_percentage),
                        percentage(report.combined().warm.cache_read_percentage),
                    )).size(LabelSize::Small))
                    .child(Label::new(format!(
                        "{} agent requests · {} summary requests · {} archived records · {} tree nodes",
                        report.agent().warm.requests,
                        report.summary().warm.requests,
                        report.metrics.engine.archived_records,
                        report.metrics.engine.total_nodes,
                    )).size(LabelSize::Small).color(Color::Muted))
                    .child(Label::new("UTF-8 byte estimate, not provider tokens or billing savings. Extractive stand-in summaries; Anthropic-style 5-minute cache, 20-block lookup, 1-second gaps, separate summary-model scope. Original system/tools, token eligibility, concurrency and model latency are not recovered or modeled.")
                        .size(LabelSize::XSmall).color(Color::Muted))
                    .into_any_element(),
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context as _;
    use gpui::TestAppContext;
    use serde_json::json;

    #[test]
    fn measured_cache_usage_distinguishes_zero_from_not_reported() {
        assert_eq!(
            measured_cache_usage_text(MeasuredCacheUsage::default()),
            "Measured cache reuse: not reported"
        );
        let usage = MeasuredCacheUsage {
            agent: CacheUsageTotals {
                requests: 1,
                input_tokens: 100,
                cached_tokens: 0,
            },
            summary: CacheUsageTotals::default(),
        };
        assert_eq!(
            measured_cache_usage_text(usage),
            "Measured cache reuse: agent 0.00% (0/100 input tokens cached) · summaries not reported · combined 0.00% (0/100 input tokens cached)"
        );
    }

    #[test]
    fn measured_cache_usage_combined_is_token_weighted() {
        let usage = MeasuredCacheUsage {
            agent: CacheUsageTotals {
                requests: 2,
                input_tokens: 100,
                cached_tokens: 80,
            },
            summary: CacheUsageTotals {
                requests: 3,
                input_tokens: 900,
                cached_tokens: 90,
            },
        };
        assert_eq!(
            measured_cache_usage_text(usage),
            "Measured cache reuse: agent 80.00% (80/100 input tokens cached) · summaries 10.00% (90/900 input tokens cached) · combined 17.00% (170/1000 input tokens cached)"
        );
    }

    #[test]
    fn measured_cache_usage_summary_only_keeps_agent_unreported() {
        let usage = MeasuredCacheUsage {
            agent: CacheUsageTotals::default(),
            summary: CacheUsageTotals {
                requests: 1,
                input_tokens: 3,
                cached_tokens: 1,
            },
        };
        assert_eq!(
            measured_cache_usage_text(usage),
            "Measured cache reuse: agent not reported · summaries 33.33% (1/3 input tokens cached) · combined 33.33% (1/3 input tokens cached)"
        );
    }

    #[test]
    fn measured_cache_usage_zero_input_has_counts_but_no_percentage() {
        assert_eq!(
            measured_cache_totals_text(Some(CacheUsageTotals {
                requests: 1,
                input_tokens: 0,
                cached_tokens: 0,
            })),
            "N/A (0/0 input tokens cached)"
        );
    }

    #[test]
    fn measured_cache_usage_overflow_is_unavailable_not_zero() {
        let usage = MeasuredCacheUsage {
            agent: CacheUsageTotals {
                requests: 1,
                input_tokens: u64::MAX,
                cached_tokens: 0,
            },
            summary: CacheUsageTotals {
                requests: 1,
                input_tokens: 1,
                cached_tokens: 0,
            },
        };
        assert!(measured_cache_usage_text(usage).ends_with("combined unavailable"));
    }

    fn input() -> Vec<u8> {
        serde_json::to_vec(&json!({
            "version": "0.3.0",
            "title": "Test snapshot",
            "updated_at": "2026-10-08T00:00:00Z",
            "messages": [
                {"User": {"id": "u", "content": [{"Text": "hello"}]}},
                {"Agent": {"content": [{"Text": "hi"}], "tool_results": {}}}
            ]
        }))
        .expect("fixture JSON")
    }

    #[test]
    fn current_snapshot_includes_the_saved_format_version() -> Result<()> {
        let original = input();
        let mut saved: serde_json::Value = serde_json::from_slice(&original)?;
        saved["infinite_context"] = json!(true);
        saved["memory_archived"] = json!(true);
        saved["measured_cache_usage"] = json!({
            "agent": {"requests": 1, "input_tokens": 100, "cached_tokens": 80},
            "summary": {"requests": 0, "input_tokens": 0, "cached_tokens": 0}
        });
        let source = serde_json::to_vec(&saved)?;
        let snapshot = agent::DbThread::from_json(&source)?;
        let messages = serde_json::to_value(&snapshot.messages)?;
        let bytes = snapshot_json(snapshot)?;
        let serialized: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(serialized["version"], agent::DbThread::VERSION);
        assert_eq!(serialized["messages"], messages);
        assert_eq!(serialized["infinite_context"], true);
        assert_eq!(serialized["memory_archived"], true);
        assert_eq!(
            serialized["measured_cache_usage"],
            saved["measured_cache_usage"]
        );
        let result = replay_json(&bytes, &ReplayOptions::default(), &AtomicBool::new(false))?;
        assert_eq!(result.source_format_version, agent::DbThread::VERSION);
        assert_eq!(result.agent().warm.requests, 1);
        assert_eq!(source, serde_json::to_vec(&saved)?);
        Ok(())
    }

    #[gpui::test]
    async fn report_finishes_with_immutable_source_identity(cx: &mut TestAppContext) {
        let bytes = input();
        let original = bytes.clone();
        let report = cx.new(|cx| {
            CacheReport::new(
                acp::SessionId::new("snapshot-thread"),
                "Snapshot title".into(),
                Task::ready(Ok(bytes)),
                cx,
                |_| {},
            )
        });
        cx.run_until_parked();
        report.read_with(cx, |report, _| {
            assert_eq!(report.thread_id, acp::SessionId::new("snapshot-thread"));
            assert_eq!(report.title.as_ref(), "Snapshot title");
            let ReportState::Complete(result) = &report.state else {
                panic!("report did not finish")
            };
            assert_eq!(result.agent().warm.requests, 1);
        });
        assert_eq!(original, input());
    }

    #[gpui::test]
    async fn cancel_does_not_allow_late_completion_to_replace_state(cx: &mut TestAppContext) {
        let report = cx.new(|cx| {
            CacheReport::new(
                acp::SessionId::new("cancelled"),
                "Cancelled".into(),
                Task::ready(Ok(input())),
                cx,
                |_| {},
            )
        });
        report.update(cx, |report, cx| report.cancel(cx));
        cx.run_until_parked();
        assert!(report.read_with(cx, |report, _| matches!(
            report.state,
            ReportState::Cancelled
        )));
    }

    #[gpui::test]
    async fn errors_are_visible_without_exposing_transcript(cx: &mut TestAppContext) {
        let report = cx.new(|cx| {
            CacheReport::new(
                acp::SessionId::new("invalid"),
                "Invalid".into(),
                Task::ready(Ok(b"private transcript".to_vec())),
                cx,
                |_| {},
            )
        });
        cx.run_until_parked();
        report.read_with(cx, |report, _| {
            let ReportState::Failed(message) = report.state else {
                panic!("error not surfaced")
            };
            assert!(!message.contains("private transcript"));
        });
    }

    #[gpui::test]
    async fn snapshot_load_errors_are_visible_without_exposing_details(cx: &mut TestAppContext) {
        let report = cx.new(|cx| {
            CacheReport::new(
                acp::SessionId::new("missing"),
                "Missing".into(),
                Task::ready(Err(anyhow::anyhow!("PRIVATE_SNAPSHOT_CONTENT"))),
                cx,
                |_| {},
            )
        });
        cx.run_until_parked();
        report.read_with(cx, |report, _| {
            let ReportState::Failed(message) = report.state else {
                panic!("load error not surfaced")
            };
            assert!(message.contains("Could not load"));
            assert!(!message.contains("PRIVATE_SNAPSHOT_CONTENT"));
        });
    }

    #[gpui::test]
    async fn cancellation_while_loading_ignores_a_late_snapshot(cx: &mut TestAppContext) {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let load =
            cx.background_spawn(async move { receiver.await.context("snapshot test input") });
        let report = cx.new(|cx| {
            CacheReport::new(
                acp::SessionId::new("delayed"),
                "Delayed".into(),
                load,
                cx,
                |_| {},
            )
        });
        cx.run_until_parked();
        report.update(cx, |report, cx| report.cancel(cx));
        sender.send(input()).expect("snapshot receiver still alive");
        cx.run_until_parked();
        report.read_with(cx, |report, _| {
            assert!(matches!(report.state, ReportState::Cancelled));
            assert_eq!(report.thread_id, acp::SessionId::new("delayed"));
            assert_eq!(report.title.as_ref(), "Delayed");
        });
    }

    #[gpui::test]
    async fn dropping_report_keeps_consuming_a_pending_load(cx: &mut TestAppContext) {
        let (sender, receiver) = futures::channel::oneshot::channel();
        let load =
            cx.background_spawn(async move { receiver.await.context("snapshot test input") });
        let report = cx.new(|cx| {
            CacheReport::new(
                acp::SessionId::new("dropped-loading"),
                "Dropped".into(),
                load,
                cx,
                |_| {},
            )
        });
        cx.run_until_parked();
        let cancellation = report.read_with(cx, |report, _| report.cancellation.clone());
        cx.update(|_| drop(report));
        cx.run_until_parked();
        assert!(cancellation.load(Ordering::Relaxed));
        sender
            .send(input())
            .expect("detached task must still consume the input");
        cx.run_until_parked();
    }

    #[gpui::test]
    fn dropping_report_cancels_worker(cx: &mut TestAppContext) {
        let report = cx.new(|cx| {
            CacheReport::new(
                acp::SessionId::new("dropped"),
                "Dropped".into(),
                Task::ready(Ok(input())),
                cx,
                |_| {},
            )
        });
        let flag = report.read_with(cx, |report, _| report.cancellation.clone());
        cx.update(|_| drop(report));
        cx.run_until_parked();
        assert!(flag.load(Ordering::Relaxed));
    }
}
