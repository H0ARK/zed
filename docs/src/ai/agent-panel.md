# Agent Panel

The Agent Panel provides you with a way to interact with LLMs.
You can use it for various tasks, such as generating code, asking questions about your code base, and general inquiries such as emails and documentation.

To open the Agent Panel, use the `agent: new thread` action in [the Command Palette](../getting-started.md#command-palette) or click the ✨ (sparkles) icon in the status bar.

If you're using the Agent Panel for the first time, you'll need to [configure at least one LLM provider](./configuration.md).

## Overview {#overview}

After you've configured one or more LLM providers, type at the message editor and hit `enter` to submit your prompt.
If you need extra room to type, you can expand the message editor with {#kb agent::ExpandMessageEditor}.

You should start to see the responses stream in with indications of [which tools](./tools.md) the AI is using to fulfill your prompt.

### Editing Messages {#editing-messages}

Any message that you send to the AI is editable.
You can click on the card that contains your message and re-submit it with an adjusted prompt and/or new pieces of context.
In [Infinite memory mode](#infinite-memory), destructive message editing and deletion are disabled to preserve the transcript.

### Checkpoints {#checkpoints}

Every time the AI performs an edit, you should see a "Restore Checkpoint" button to the top of your message, allowing you to return your codebase to the state it was in prior to that message.

The checkpoint button appears even if you interrupt the thread midway through an edit attempt, as this is likely a moment when you've identified that the agent is not heading in the right direction and you want to revert back.

### Navigating History {#navigating-history}

To quickly navigate through recently opened threads, use the {#kb agent::ToggleNavigationMenu} binding, when focused on the panel's editor, or click the hamburger icon button at the top left of the panel to open the dropdown that shows you the six most recent threads.

The items in this menu function similarly to tabs, and closing them doesn’t delete the thread; instead, it simply removes them from the recent list.

To view all historical conversations, reach for the `View All` option from within the same menu or via the {#kb agent::OpenHistory} binding.

### Following the Agent {#following-the-agent}

Zed is built with collaboration natively integrated.
This approach extends to collaboration with AI as well.
To follow the agent reading through your codebase and performing edits, click on the "crosshair" icon button at the bottom left of the panel.

### Get Notified {#get-notified}

If you send a prompt to the Agent and then move elsewhere, thus putting Zed in the background, you can be notified of whether its response is finished either via:

- a visual notification that appears in the top right of your screen
- or a sound notification

Both notification methods can be used together or individually according to your preference.

You can customize their behavior, including turning them off entirely, by using the `agent.notify_when_agent_waiting` and `agent.play_sound_when_agent_done` settings keys.

### Reviewing Changes {#reviewing-changes}

Once the agent has made changes to your project, the panel will surface which files, and how many of them, have been edited.

To see which files specifically have been edited, expand the accordion bar that shows up right above the message editor or click the `Review Changes` button ({#kb agent::OpenAgentDiff}), which opens a multi-buffer tab with all changes.

You're able to reject or accept each individual change hunk, or the whole set of changes made by the agent.

Edit diffs also appear in individual buffers.
So, if your active tab had edits made by the AI, you'll see diffs with the same accept/reject controls as in the multi-buffer.

## Adding Context {#adding-context}

Although Zed's agent is very efficient at reading through your codebase to autonomously pick up relevant files, directories, and other context, manually adding context is still encouraged as a way to speed up and improve the AI's response quality.

If you have a tab open when opening the Agent Panel, that tab appears as a suggested context in form of a dashed button.
You can also add other forms of context by either mentioning them with `@` or hitting the `+` icon button.

You can even add previous threads as context by mentioning them with `@thread`, or by selecting the "New From Summary" option from the top-right menu to continue a longer conversation, keeping it within the context window.

Pasting images as context is also supported by the Agent Panel.

### Token Usage {#token-usage}

Zed surfaces how many tokens you are consuming for your currently active thread in the panel's toolbar.
Depending on how many pieces of context you add, your token consumption can grow rapidly.

With that in mind, once you get close to the model's context window, a banner appears below the message editor suggesting to start a new thread with the current one summarized and added as context.
You can also do this at any time with an ongoing thread via the "Agent Options" menu on the top right.

### Infinite Memory (Experimental) {#infinite-memory}

Infinite memory is an opt-in mode for agent threads, disabled by default.
Click **Infinite memory** above the conversation to enable or disable it for the current thread; the button is selected when the mode is enabled.
The choice is saved with the thread and does not change your default for new threads.
The button is disabled while the agent is generating a response or has pending tool uses.

To enable the mode by default for **new threads**, add this to your settings:

```json
{
  "agent": {
    "infinite_context": true
  }
}
```

This setting does not change existing threads, and the mode is not available for text threads.

Infinite memory preserves conversation text in append-only daily journals at `<data_dir>/agent/infinite_context/<thread_id>` within Zed's data directory.
Completed replies and tool exchanges are archived; reasoning is displayed but excluded from the memory journals.
Tool output longer than 30,000 characters is explicitly clipped to its head and tail; other long text is split losslessly, and attached images are stored separately.
Instead of replacing the transcript, it builds an immutable binary tree of summaries, each at most 512 UTF-8 bytes.
The saved history view grows to 128,000 bytes, then merges built sibling summaries toward 64,000 bytes, keeping finer detail near the present.
Each new user turn sends that view and the new message; only the current turn's live tool exchanges are replayed in full.
The agent can use `zoom(id, n, page)` to retrieve older text, `zoom({"id": id, "n": 1, "image": index})` to retrieve an attached image by index, and `date(id)` to retrieve a message's timestamp.
Once a thread has an archive, destructive message editing and deletion remain disabled even if memory mode is switched off; send a new correction instead.
Disabling the mode cancels its background work and returns requests to ordinary full-history behavior without deleting the archive.

**This does not disable context limits or give the model unlimited tokens.**
Each request still has to fit the selected model's context window; older details remain available through summaries and retrieval rather than all being sent on every request.

Background summarization makes additional model calls, which consume tokens and may incur costs or count toward provider usage limits.
These calls use the model configured by `agent.thread_summary_model`, falling back to your default model when that setting is not specified.
Conversation content used for summarization is sent to that model's provider, so choose it with your cost and privacy requirements in mind.
A tool-capable conversation model is required for retrieval. Both conversation and summary requests are checked against their respective model limits.
Background work is limited to eight simultaneous calls; failed summaries can be retried on the next message. Stopping generation also cancels the active summary drain.
The compactor's separate persisted context targets 16,000 bytes and starts reducing near 27,000 bytes, reserving space below its 32,000-byte limit for in-flight work.

This implementation does not yet provide the design's cross-request cache-write coordination or named-subagent chat retrieval.
Assistant text is journaled when its response finishes or is canceled, not on every streamed chunk, so a process crash during an unfinished response can lose that response's unarchived tail.

#### Measured cache reuse

The **Measured cache reuse** row above the conversation, below the Infinite memory controls, shows live provider-reported cache reuse for the current thread, whether or not Infinite memory is enabled.
It shows agent, memory-summary, and combined percentages alongside raw **cached / total input token** counts. Total input includes cached tokens; the combined percentage is token-weighted (`100 × combined cached tokens / combined total input tokens`), not an average of request percentages. Hover over the row for reported-request counts and measurement scope.

Only observed requests with explicit cache counters contribute to these totals, including live usage updates and memory-summary retries that report counters. Repeated usage updates for a request replace that request's previous counters rather than counting another request.
Older thread history without measured counters and requests from other models or providers without cache details are not measured; legacy token-usage fields are not used to infer cache reuse.
An explicitly reported zero displays **0.00%** when total input is positive. Missing details display **not reported**, not zero. If no requests have reported counters, the row reads **Measured cache reuse: not reported**: Zed may be waiting for usage, or the provider may not support cache details. A reported zero total input displays raw counts with **N/A** instead of a percentage.
Switching threads reads the newly active thread's measured totals, independently of any open offline report.
These are provider-token measurements for the observed cache-reporting requests only, **not billing savings**, and displaying them makes no extra API calls.

In this checkout, OpenAI **Chat Completions** support reads the optional `usage.prompt_tokens_details.cached_tokens` counter and uses `stream_options.include_usage: true` for streaming requests, including usage-only final chunks. Missing or null cache details remain unreported.
The label is provider-neutral: cloud and compatible-provider paths that use the same event mapper can report counters too, as can other mappers that emit explicit cache usage.
This checkout has no OpenAI **Responses** integration or Responses diagnostics. The installed `openai-subscribed` provider's implementation is absent from this checkout and is not changed by this feature; its cache-reporting behavior is not established here.

#### Replaying a saved conversation locally

The offline cache benchmark can replay a saved agent conversation without calling a model or executing its recorded tools.
It supports this checkout's version `0.2.0` thread JSON and version `0.3.0` externally tagged `User`/`Agent` threads.

Click **Cache report** next to **Infinite memory**, or run `agent: replay thread cache`, to replay a snapshot of the active thread. This is available whether or not Infinite memory is enabled, and is disabled while the thread is generating or has pending tools.

For previous conversations, open history (`agent: open history`), hover or select a thread, and click its **Offline cache report** icon next to Delete. This reads the saved record directly, without opening, upgrading, or modifying the conversation.

The panel displays agent, summary, and combined byte-weighted cache reuse, request counts, and tree statistics under **Offline estimate**, labeled with the source thread. Switching threads does not retarget an existing report. **Cancel** stops a running replay; starting another report cancels the previous one. Parsing, serialization, and journal work run in the background after an in-memory snapshot is captured. No provider requests are made, no recorded tools are executed, and transcript text is not shown in the report. Private temporary journals are deleted after completion, errors, or cancellation. A cleanup failure raises a separate warning even if you have closed the report. Replay currently requires macOS or Linux; its controls are disabled on Windows until private temporary journal permissions are supported.

The command-line benchmark also supports custom replay parameters and full JSON reports. First list saved thread ids, then export a selected thread:

```sh
python3 script/export-agent-thread.py --list
mkdir -p target/cache-replay
python3 script/export-agent-thread.py --thread-id THREAD_ID --output target/cache-replay/thread.json
```

The exporter opens `threads.db` read-only and handles JSON or zstd-compressed records; compressed records require the `zstd` CLI.
Use `--database PATH` for a custom Zed data directory.
Snapshots contain private conversation data, are created with owner-only permissions on Unix, and must not be committed or shared without review.
The exporter refuses to overwrite existing files. `target/` is ignored by Git.

Then replay the snapshot:

```sh
cargo run -p agent --example infinite_context_replay --features gpui/runtime_shaders --offline -- \
  --input target/cache-replay/thread.json --report target/cache-replay/report.json
```

The example uses the real memory tree and production cache-block formatting, but deterministic extractive stand-ins for model-generated summaries.
Its cache model has a five-minute TTL, a 20-content-block lookup window, separate conversation/summary model scopes, and one-second gaps between recorded agent responses by default.
It reports byte-weighted prefix-cache reuse for agent and summary calls separately, plus a cold-cache comparison and frontier rewrites.
These are **structural estimates, not provider-token hit rates, billing savings, summary-quality validation, or a claim about the original thread's provider**.
Provider token eligibility thresholds, concurrency, and model duration are not modeled; the original full system prompt and tool schemas are not recovered automatically.
Reasoning is omitted. Saved token-usage counters are included only as unmodified baseline metadata, not mixed with replay byte metrics.

For sensitivity checks, add `--pause-after-request 36 --pause-seconds 600` to simulate cache expiry, or `--summary-bytes 256` to change the stand-in summary size.
An optional `--prefix-json FILE` supplies additional system blocks and tool schemas as `{"system": ["..."], "tools": [{"name": "...", "description": "...", "input_schema": {}}]}`.
`--min-cache-bytes N` is a byte proxy for cache eligibility, not a provider's token threshold.
Report files also refuse overwrites. Temporary replay journals are private on Unix and deleted after the run; currently the example rejects non-Unix hosts.

## Changing Models {#changing-models}

After you've configured your LLM providers—either via [a custom API key](./configuration.md#use-your-own-keys) or through [Zed's hosted models](./models.md)—you can switch between them by clicking on the model selector on the message editor or by using the {#kb agent::ToggleModelSelector} keybinding.

## Using Tools {#using-tools}

The new Agent Panel supports tool calling, which enables agentic editing.
Zed comes with [several built-in tools](./tools.md) that allow models to perform tasks such as searching through your codebase, editing files, running commands, and others.

You can also extend the set of available tools via [MCP Servers](./mcp.md).

### Profiles {#profiles}

Profiles act as a way to group tools.
Zed offers three built-in profiles and you can create as many custom ones as you want.

#### Built-in Profiles {#built-in-profiles}

- `Write`: A profile with tools to allow the LLM to write to your files and run terminal commands. This one essentially has all built-in tools turned on.
- `Ask`: A profile with read-only tools. Best for asking questions about your code base without the concern of the agent making changes.
- `Minimal`: A profile with no tools. Best for general conversations with the LLM where no knowledge of your code base is necessary.

You can explore the exact tools enabled in each profile by clicking on the profile selector button > `Configure Profiles…` > the one you want to check out.

#### Custom Profiles {#custom-profiles}

You can create a custom profile via the `Configure Profiles…` option in the profile selector.
From here, you can choose to `Add New Profile` or fork an existing one with a custom name and your preferred set of tools.

You can also override built-in profiles.
With a built-in profile selected, in the profile selector, navigate to `Configure Tools`, and select the tools you'd like.

Zed will store this profile in your settings using the same profile name as the default you overrode.

All custom profiles can be edited via the UI or by hand under the `assistant.profiles` key in your `settings.json` file.

### Model Support {#model-support}

Tool calling needs to be individually supported by each model and model provider.
Therefore, despite the presence of tools, some models may not have the ability to pick them up yet in Zed.
You should see a "No tools" label if you select a model that falls into this case.

We want to support all of them, though!
We may prioritize which ones to focus on based on popularity and user feedback, so feel free to help and contribute to fast-track those that don't fit this bill.

All [Zed's hosted models](./models.md) support tool calling out-of-the-box.

### MCP Servers {#mcp-servers}

Similarly to the built-in tools, some models may not support all tools included in a given MCP Server.
Zed's UI will inform about this via a warning icon that appears close to the model selector.

## Text Threads {#text-threads}

["Text threads"](./text-threads.md) present your conversation with the LLM in a different format—as raw text.
With text threads, you have full control over the conversation data.
You can remove and edit responses from the LLM, swap roles, and include more context earlier in the conversation.

For users who have been with us for some time, you'll notice that text threads are our original assistant panel—users love it for the control it offers.
We do not plan to deprecate text threads, but it should be noted that if you want the AI to write to your code base autonomously, that's only available in the newer, and now default, "Threads".

### Text Thread History {#text-thread-history}

Content from text thread are saved to your file system.
Visit [the dedicated docs](./text-threads.md#history) for more info.

## Errors and Debugging {#errors-and-debugging}

In case of any error or strange LLM response behavior, the best way to help the Zed team debug is by reaching for the `agent: open thread as markdown` action and attaching that data as part of your issue on GitHub.

This action exposes the entire thread in the form of Markdown and allows for deeper understanding of what each tool call was doing.

You can also open threads as Markdown by clicking on the file icon button, to the right of the thumbs down button, when focused on the panel's editor.

## Feedback {#feedback}

Every change we make to Zed's system prompt and tool set, needs to be backed by an eval with good scores.

Every time the LLM performs a weird change or investigates a certain topic in your codebase completely incorrectly, it's an indication that there's an improvement opportunity.

> Note that rating responses will send your data related to that response to Zed's servers.
> See [AI Improvement](./ai-improvement.md) and [Privacy and Security](./privacy-and-security.md) for more information about Zed's approach to AI improvement, privacy, and security.
> **_If you don't want data persisted on Zed's servers, don't rate_**. We will not collect data for improving our Agentic offering without you explicitly rating responses.

The best way you can help influence the next change to Zed's system prompt and tools is by rating the LLM's response via the thumbs up/down buttons at the end of every response.
In case of a thumbs down, a new text area will show up where you can add more specifics about what happened.

You can provide feedback on the thread at any point after the agent responds, and multiple times within the same thread.
