# Using hfx

[README](../README.md) · [Providers](providers.md) · [Usage](usage.md) · [Tools & security](tools.md) · [Linux](linux.md) · [Development](development.md)

Projects, chats, attachments, queued follow-ups, and settings are saved locally. Demo mode lets you explore the interface without making model requests or running workspace tools. Shell examples below assume the repository root.

## Projects and attachments

Host commands and MCP require a one-time explicit trust acknowledgement for each project; existing projects also start untrusted after upgrading. Review, grant, or revoke it in **Settings → Tools → Project host access**. Approval is bound to the canonical directory shown in the dialog: if the path or symlink target changes, close the dialog and review the new location before granting trust. File tools do not imply host isolation; see [Tools & security](tools.md#command-access-and-git-authentication).

Add projects with the **+** beside Projects, or **File → Add project**. Enter an existing folder path. Project chats have separate histories and drafts; right-click a chat to rename or delete it. Use the paperclip to choose files, paste copied files/screenshots with **Ctrl+V** (**Cmd+V** on macOS) while the chat input is focused, or drag files into the window. The **+** menu also offers paste and attachment by path. Each attachment has a removable preview; images show thumbnails. Attachments can be sent without a text prompt. Background loading stays with the chat where it started.

Attachments support UTF-8 text/code (256 KiB each) and PNG, JPEG, WebP, or GIF images (10 MiB, at most 32 megapixels each), with up to 8 files per message. Images are normalized to PNG; GIF uses the first frame. Other binary document formats are rejected with a visible error. Text file contents are included as reference data; images use Responses `input_image` for OpenAI/Codex and Chat Completions `image_url` for OpenRouter/llama.cpp. Image inference requires a vision-capable model; llama.cpp also needs its multimodal projector configured. Explicit attachments may be outside the project. Clipboard images use arboard, Wayland file lists use data-control, and Windows file lists use CF_HDROP. Clipboard access happens only after a paste gesture. Platform clipboard/portal support varies; file selection and drag-and-drop remain available.

## Git repository indicator

The composer header checks the selected project's local Git metadata and shows **Git · branch-name**, **Git · detached @ commit**, **Git (bare) · branch-name**, or **Not a Git repo**. An initial branch with no commits and linked worktrees are supported. Long names are truncated visually; hover for details. Missing Git, inaccessible projects, malformed metadata, and ownership errors show **Git unavailable**, not a false non-repository result.

Checks run in the background, refresh every three seconds, and time out after two seconds. Git output is bounded to 64 KiB per stream. Changing projects discards the previous project's results. These fixed read-only Git queries do not run a shell, invoke repository hooks or fsmonitor, fetch from remotes, modify the index, or grant project trust.

## Queueing and steering

While a response is running, **Enter** queues the composer text and attachments as a follow-up instead of discarding or interrupting the current run. The queue appears above the composer with **Steer**, edit, and delete controls. Multiple follow-ups run in FIFO order after successful completion. **Shift+Enter** still inserts a newline.

Click **Steer** on a pending row, or press **Ctrl+Enter** (**Cmd+Enter** on macOS) in the composer, to guide the active run in that chat. Steering is applied at the next completed model-request/tool-round boundary—not halfway through streaming, a running command, an approval, or a tool call/result pair. It continues the same backend run, preserving its context, and records the guidance as a visible user message followed by a new assistant reply. A dispatched row shows **Steering…** until accepted; it cannot be edited or deleted after dispatch. If the run completes before accepting it, it becomes a normal queued follow-up instead of being lost. Demo mode supports queueing but does not perform model steering.

Queues belong to their original chats. Switching chats never redirects a steer; another chat's queue waits until the current run finishes. Deleting an inactive chat leaves the running reply, tools, and queues untouched; stop the active chat before deleting it. The active chat's pending follow-ups are preferred, then other unpaused chats. Queued messages are not model context until sent. Automatic follow-ups use the provider settings current when they start and that chat's workspace; steering uses the active run's existing provider/settings. **Pause/Resume** controls follow-ups without stopping the current run; an already dispatched steer may still be accepted.

**Stop**, generation errors, and application restart pause unsent work. Messages and attachments remain saved, and **Resume** restarts the queue. Adding to a paused queue does not silently unpause it. Queue edits leave the composer draft and existing attachments intact.

## Automatic context compaction

Automatic compaction is enabled by default for Codex, OpenAI, OpenRouter, and llama.cpp. Before the next model request—including continuations after completed tool rounds—hfx checks whether the context has reached **75% of the model's context window**. Provider input/output usage is used when available; otherwise a conservative estimate includes instructions, tool definitions, text, and an image allowance (not the image's base64 length). The circular meter immediately left of the composer’s reasoning selector fills with the selected chat/model’s context usage. Hover to see used / maximum tokens, the percentage consumed, and whether the maximum comes from provider metadata, a saved/manual override, or an unverified fallback. A `~` beside the token count means it is estimated. New chats or newly selected connections show an empty ring until usage is available.

Older context is summarized by the selected model without workspace tools. The latest user request and as much recent context as fits are retained; tool calls and results are never split. Oversized old transcripts are summarized in bounded chunks. Compaction creates a persisted checkpoint for future model input, **not a deletion of visible chat messages, attachments, edits, or tool history**. The UI shows “Compacting context…” while it runs. Summary requests use the same provider credentials and may incur additional inference cost. Stop cancels them too. If summarization fails, the original history remains intact and the turn stops with an error instead of silently discarding context.

**Settings → Providers → Context** lets you disable compaction or set the selected model's context window independently of its maximum output tokens. Model metadata is learned through **Test connection** (OpenRouter/compatible servers and llama.cpp's allocated context) or Codex model discovery. When metadata is unavailable, the fallback is **128,000 tokens**, not a verified model limit: set it to your model's actual window for an accurate threshold. Codex windows vary by model; for a model with a **272,000-token** window, compaction starts at **204,000 tokens**. Provider metadata refreshes separately from manual overrides; use **Use discovered limit / fallback** to remove an override. Limits are stored per provider, endpoint, and model. Output requests are capped at one quarter of the context window to leave room for input; compaction can run sooner if necessary to reserve output space. A single oversized latest request that cannot be safely compacted produces a clear error rather than being truncated.

## Motion and persistence

- UTF-8-safe text reveal smooths bursts from every provider and accelerates to catch up with long responses.
- Messages fade in; the welcome screen fades out; panels slide; hover states and menus fade; the idle mark breathes gently.
- **Settings → Appearance** controls reveal speed, text size, reasoning visibility, and reduced motion.
- Chats, project paths, drafts, queued/sent attachments, action history, edit diffs, and preferences save every five seconds and on clean exit. Interrupted responses restore as stopped.
- Serialization and disk writes run on a background worker with buffered I/O and a single coalesced latest pending snapshot. Large tool outputs and image/context payloads are shared in save snapshots, and the UI renders conversations without copying their history each frame. Action labels are cached instead of repeatedly parsing full file-edit arguments while typing.
- API keys entered in settings remain in memory and are **never serialized**. Use environment variables to avoid re-entering them after relaunch.
- Conversations and file context are saved locally in plaintext; images are stored as base64 in the same local state. On Linux, chat state is normally in `~/.local/share/hfx/chats.json` (or under `XDG_DATA_HOME`); `app.ron` holds lightweight UI state. Legacy chats migrate automatically after the first successful background save. Chat files use mode 0600 on Unix. macOS and Windows use eframe's platform data directory.

## Recovering a chat-state load failure

If `chats.json` is unreadable, malformed, or incompatible, hfx preserves it and shows a recovery dialog plus a persistent **saving is disabled** banner. A legacy or fresh temporary session may be used in memory, but startup saves, background autosaves, clean-exit saves, and legacy-state deletion remain blocked. Continuing without saving means changes in that temporary session are not durable.

After stopping generation, choose one explicit recovery action:

- **Retry loading original:** repair or restore the file first, then reload it. This replaces the temporary session; its unsaved changes are discarded. Restored queues stay paused.
- **Back up original and enable saving:** creates a private, synced `chats-recovery-<uuid>.json` beside the original without deleting it, then enables normal atomic saves for the current session. If backup fails, saving stays disabled and the error remains visible.
- **Continue without saving:** dismisses the dialog without changing the original. Reopen it using **Review recovery** in the banner.

Recovery disk work runs off the UI thread. While a recovery job is running, **Hide recovery progress** only hides the dialog; it does not cancel the requested operation, and saving is enabled only on success. **Continue without saving** is offered only when no recovery job is pending. Existing dangling history symlinks also enter recovery rather than being replaced by fresh history; restore their target before retrying. No recovery or trust decision is automatically accepted by a timer.

## Shutdown and diagnostics

Closing the window / **Quit hfx** is a clean exit, not a hard kill. hfx stops the current run, cancels sign-in, pauses queued work, and waits for the latest chat snapshot to be atomically written and synced to disk. Buffered JSON writes avoid hundreds of thousands of small write calls for escaped tool output. The final snapshot replaces obsolete pending autosaves instead of sitting behind them; an in-progress save may still need to finish.

After saving, asynchronous/background runtime cleanup has a **100 ms grace period** rather than waiting indefinitely for non-cancellable clipboard, file-dialog, or image-decoding jobs. Idle cleanup normally returns immediately; 100 ms is not a mandatory delay. Already-running synchronous work may finish during the save/grace period; any unfinished work ends when the process exits. PNG exports also write to a temporary file before replacement, avoiding truncation of an existing image if interrupted. Notification/chime workers are detached and are not joined on exit.

The durable chat save is **not** timed out or skipped to hide a slow disk. A large history, slow filesystem, or graphics/window-system teardown can still take time. To identify remaining shutdown time from a terminal:

```bash
HFX_SHUTDOWN_TRACE=1 ./target/release/hfx
```

This reports stop/cancel, final chat save, save-worker cleanup, and runtime cleanup timings without logging chat contents or credentials. It also reports the interval from `on_exit` to native return, including subsequent window/graphics teardown (not the preceding eframe UI-state save). A true **SIGKILL / force kill** bypasses cleanup and can lose changes since the last successful save.

A reproducible synthetic-history I/O benchmark (no user history is read):

```bash
cargo test --release --locked profile_shutdown_save_io -- --ignored --nocapture
```

## Shortcuts

In the composer: **Enter** sends/queues, **Shift+Enter** inserts a newline, and **Ctrl/Cmd+Enter** steers the active run in that chat.

| Shortcut | Action |
| --- | --- |
| Enter | Send, or queue a follow-up while running |
| Shift+Enter | New line |
| Ctrl/Cmd+Enter | Steer the active run in this chat |
| Ctrl/Cmd+N | New chat |
| Ctrl/Cmd+, | Settings |
| Ctrl/Cmd+B | Toggle sidebar |
| Ctrl/Cmd+K | Search chats |
| Escape | Close image viewer; otherwise decline pending tool, close a panel, or stop generation |

For command access, review mode, web research, and Git attribution, see [Workspace tools and security](tools.md). Linux-specific rendering and completion-alert behavior are covered in [Linux installation and desktop integration](linux.md).
