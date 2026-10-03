# hfx

A native coding workspace in **Rust + egui**, inspired by the supplied Codex screenshots. Charcoal surfaces, a sage accent, quiet typography, and a focused conversation area.

## Run

Install Rust **1.95 or later**, then:

```bash
cargo run --release --locked
```

The app starts in **Demo** mode. Send a message to try scripted reasoning, streaming text, and animation without credentials or a running model. The current working directory becomes the first project.

Linux requires a desktop session and the usual Wayland/X11 and OpenGL runtime libraries. Build dependencies may include a C compiler, `pkg-config`, and development packages for X11, Wayland, and xkbcommon. The optional strict command sandbox requires **bubblewrap 0.12 or later**, user namespaces, and overlay filesystem support; the default trusted host commands do not. The renderer uses OpenGL through eframe's Glow backend.

## Linux installation and desktop launcher

From this checkout:

```bash
./scripts/install-linux.sh --desktop-shortcut
```

This builds the release binary and installs it **for your user only**, without `sudo`:

- Executable: `~/.local/bin/hfx` (or `$XDG_BIN_HOME/hfx`, or `--bin-dir /absolute/path`).
- Application-menu launcher: `$XDG_DATA_HOME/applications/hfx.desktop`, normally `~/.local/share/applications/hfx.desktop`.
- Native window/dock icon and installed SVG/PNG icons in the user’s hicolor theme.
- Optional Desktop shortcut, using the configured/localized `XDG_DESKTOP_DIR` from `user-dirs.dirs` rather than assuming an English folder name.

Omit `--desktop-shortcut` if you only want the application-menu/dock entry. Find **hfx** in your application menu and pin it to the dock. GNOME may require a desktop-icons extension to show shortcuts on the wallpaper, and a shortcut may need **right-click → Allow Launching**. The installer attempts the trust metadata but this depends on desktop support.

Normal Linux launches also register an application-menu entry automatically if there is no existing user/system `hfx.desktop`; this works for portable/source builds without running the installer. That entry points to the executable you launched. Use the installer for a permanent location independent of this checkout. Existing launchers are not hijacked when running a different development build. Previews and headless CLI commands do not auto-register. Set `HFX_NO_DESKTOP_INTEGRATION=1` or pass `--no-desktop-integration` to opt out of automatic registration.

The Desktop launcher uses a small starter workspace at `$XDG_DATA_HOME/hfx/workspace` for a fresh profile—not your entire home directory. Existing saved projects/chats are preserved. Launching from a terminal still uses its current directory for a fresh profile.

Other installer options:

```bash
./scripts/install-linux.sh --no-build                 # use the existing release build
./scripts/install-linux.sh --binary /path/to/hfx      # install a prebuilt binary
./scripts/uninstall-linux.sh                         # preserve chats, settings and login credentials
```

Rerun the installer to update. Binary replacement is atomic, even if the old instance is running; restart hfx to load the update. Quit hfx before uninstalling. Uninstall does not delete saved chats, credentials, or workspace files. Install/uninstall refuse to replace a binary not managed by these scripts; desktop registration refuses to overwrite unrelated user launchers.

### Portable Linux bundle

```bash
./scripts/package-linux.sh
# Creates dist/hfx-<version>-linux-<architecture>.tar.gz and its SHA-256 checksum.
```

Extract the bundle and run `./install.sh --desktop-shortcut`; Rust is not required to install that prebuilt bundle. It includes `hfx`, installer/uninstaller scripts, and the license. This is a native build for the build host’s architecture/glibc, **not** a universal static binary, AppImage, or `.deb`/`.rpm`. It still requires compatible desktop/OpenGL runtime libraries. Bubblewrap is only needed for the optional strict command sandbox.

### Completion notifications and sound

On Linux, successful work produces a small in-app **Work complete** toast, a desktop notification, and a quiet two-note chime. This works while focused, unfocused, or minimized. Each chat’s automatically running queued follow-ups count as one batch: alerts fire after the last follow-up, not after every tool call or steering boundary. Paused follow-ups do not prevent an alert for the completed run. Stop and errors stay silent; previews never issue native alerts.

**Settings → Appearance → Completion alerts** has independent, persisted switches for desktop notifications and the completion sound, enabled by default. Notifications contain no prompt/output excerpts. Desktop **Do Not Disturb** controls notification banners. The chime is independent: mute it with the sound checkbox or system audio settings. Desktop-service availability still applies.

Native alerts use `notify-send` and an audio helper (`paplay`, `pw-play`, or `canberra-gtk-play`, in that order). On Debian/Ubuntu, `libnotify-bin` supplies `notify-send` and `pulseaudio-utils` supplies `paplay`; PipeWire installations may already have `pw-play`. The installer reports missing helpers but does not install system packages. The packaged chime is 0.48 seconds at low amplitude/reduced playback volume; the last-resort Canberra fallback uses the desktop’s completion sound. All helpers run on a detached worker with a timeout, never on the window event thread. Missing services/helpers do not interrupt a run. Native completion alerts are currently Linux-only; the in-app toast works on other platforms.

## Connect a provider

Open **Settings → Providers** from the gear icon, workspace menu, or `Ctrl+,` (`Cmd+,` on macOS).

### Codex sign-in

1. Select **Codex**, then click **Sign in with OpenAI**.
2. Complete sign-in in the browser. hfx receives the callback at `http://localhost:1455/auth/callback`, validates OAuth state, and exchanges the authorization code with its PKCE verifier.
3. Choose a discovered model, then send a message. The default is `gpt-6.1-sol`; access and reasoning effort depend on your ChatGPT account.

This is built-in OpenAI authentication in Rust, based on the flow in [the supplied OAuth sample](https://github.com/7shi/codex-oauth/blob/main/codex_oauth.py). It uses authorization code + PKCE (S256), the public Codex client ID, and `openid profile email offline_access`. hfx refreshes tokens before expiry and retries once after HTTP 401.

Responses requests carry the access token and `ChatGPT-Account-Id` header. They use `store: false`, explicit input text content, reasoning summaries, and a stable chat cache key. Conversation/tool items and encrypted reasoning context are retained for later turns. The backend uses the current `https://chatgpt.com/backend-api/codex` route; the sample uses the older WHAM route. These backend details are not a versioned public API and can change.

Tool calls are assembled from streamed output items and function argument events, even when the terminal response omits its output array. Repeated terminal items are merged by identity so each tool runs once. Tools execute after successful stream completion; incomplete responses do not trigger execution. Codex's `response.done` terminal event is also accepted.

hfx manages its own login. OAuth credentials are stored separately from chats in `codex-auth.json` in hfx's application data directory. On Unix, its directory is mode 0700 and the file is mode 0600. This file contains bearer credentials in plaintext. **Sign out of hfx** removes that cache. Cancel closes the pending callback listener. No Codex CLI installation is required.

### OpenAI API

1. Select **OpenAI API**.
2. Enter an API key, or launch with `OPENAI_API_KEY` in the environment.
3. Set a Responses-compatible reasoning model ID available to your account. The default is `gpt-6.1-sol`, matching the model family shown in the reference. Codex model IDs can also be entered directly.
4. Click **Test connection** to discover models, then choose one.

Requests use `/v1/responses`, SSE streaming, configurable reasoning effort, and `reasoning.summary = "auto"` when reasoning is visible. OpenAI provides **reasoning summaries**; raw internal reasoning is not exposed. Summary availability depends on the model and account. API billing and model access are separate from the Codex desktop app's subscription.

The endpoint is configurable for compatible gateways. Include `/v1` in the base URL. Remote OpenAI endpoints must use HTTPS. Requests use `store: false`; encrypted reasoning items are preserved within tool rounds.

### OpenRouter

1. Select **OpenRouter**.
2. Enter your OpenRouter API key, or launch with `OPENROUTER_API_KEY` in the environment.
3. Use `https://openrouter.ai/api/v1` and a `provider/model` ID available to your account. **Test connection** discovers available models. The default is `openai/gpt-6-luna`.

Requests use Chat Completions with SSE streaming, configurable reasoning effort, tools, temperature, and output limits. hfx renders `delta.reasoning` and textual reasoning details. Assistant tool calls, tool results, encrypted reasoning details, and the final answer are saved and replayed on subsequent user turns. Earlier chats also recover their recorded action outputs as reference context. Reasoning and tool support vary by model and provider. Usage is billed through your OpenRouter account.

### llama.cpp

Start a current llama.cpp server with a model that supports chat and, if wanted, reasoning/tools:

```bash
llama-server -m /path/to/model.gguf --alias local-model \
  --host 127.0.0.1 --port 8080 --jinja --reasoning-format deepseek
```

Select **llama.cpp**, use `http://127.0.0.1:8080/v1`, and test the connection. Match the model ID to the server alias. An API key is optional; `LLAMA_API_KEY` is also supported.

Requests use `/v1/chat/completions` and stream `delta.content` separately from `delta.reasoning_content`. Reasoning effort, tool calls, and reasoning extraction depend on the chosen model's chat template and server version. Temperature and output limits are configurable.

## Queueing and steering

While a response is running, **Enter** queues the composer text and attachments as a follow-up instead of discarding or interrupting the current run. The queue appears above the composer with **Steer**, edit, and delete controls. Multiple follow-ups run in FIFO order after successful completion. **Shift+Enter** still inserts a newline.

Click **Steer** on a pending row, or press **Ctrl+Enter** (**Cmd+Enter** on macOS) in the composer, to guide the active run in that chat. Steering is applied at the next completed model-request/tool-round boundary—not halfway through streaming, a running command, an approval, or a tool call/result pair. It continues the same backend run, preserving its context, and records the guidance as a visible user message followed by a new assistant reply. A dispatched row shows **Steering…** until accepted; it cannot be edited or deleted after dispatch. If the run completes before accepting it, it becomes a normal queued follow-up instead of being lost. Demo mode supports queueing but does not perform model steering.

Queues belong to their original chats. Switching chats never redirects a steer; another chat's queue waits until the current run finishes. Deleting an inactive chat leaves the running reply, tools, and queues untouched; stop the active chat before deleting it. The active chat's pending follow-ups are preferred, then other unpaused chats. Queued messages are not model context until sent. Automatic follow-ups use the provider settings current when they start and that chat's workspace; steering uses the active run's existing provider/settings. **Pause/Resume** controls follow-ups without stopping the current run; an already dispatched steer may still be accepted.

**Stop**, generation errors, and application restart pause unsent work. Messages and attachments remain saved, and **Resume** restarts the queue. Adding to a paused queue does not silently unpause it. Queue edits leave the composer draft and existing attachments intact.

## Automatic context compaction

Automatic compaction is enabled by default for Codex, OpenAI, OpenRouter, and llama.cpp. Before the next model request—including continuations after completed tool rounds—hfx checks whether the context has reached **75% of the model's context window**. Provider input/output usage is used when available; otherwise a conservative estimate includes instructions, tool definitions, text, and an image allowance (not the image's base64 length). The circular meter immediately left of the composer’s reasoning selector fills with the selected chat/model’s context usage. Hover to see used / maximum tokens, the percentage consumed, and whether the maximum comes from provider metadata, a saved/manual override, or an unverified fallback. A `~` beside the token count means it is estimated. New chats or newly selected connections show an empty ring until usage is available.

Older context is summarized by the selected model without workspace tools. The latest user request and as much recent context as fits are retained; tool calls and results are never split. Oversized old transcripts are summarized in bounded chunks. Compaction creates a persisted checkpoint for future model input, **not a deletion of visible chat messages, attachments, edits, or tool history**. The UI shows “Compacting context…” while it runs. Summary requests use the same provider credentials and may incur additional inference cost. Stop cancels them too. If summarization fails, the original history remains intact and the turn stops with an error instead of silently discarding context.

**Settings → Providers → Context** lets you disable compaction or set the selected model's context window independently of its maximum output tokens. Model metadata is learned through **Test connection** (OpenRouter/compatible servers and llama.cpp's allocated context) or Codex model discovery. When metadata is unavailable, the fallback is **128,000 tokens**, not a verified model limit: set it to your model's actual window for an accurate threshold. Codex windows vary by model; for a model with a **272,000-token** window, compaction starts at **204,000 tokens**. Provider metadata refreshes separately from manual overrides; use **Use discovered limit / fallback** to remove an override. Limits are stored per provider, endpoint, and model. Output requests are capped at one quarter of the context window to leave room for input; compaction can run sooner if necessary to reserve output space. A single oversized latest request that cannot be safely compacted produces a clear error rather than being truncated.

## Workspace tools

Replies keep progress updates and tool activity in chronological order: normal conversation, a compact gray tool listing, the next progress update, then a fresh tool listing. Each group starts collapsed and expands independently to show its own commands/files, results, timings, images, and diffs. Tool-only rounds stay in the same group; a new visible progress update starts a new one. Context compaction notices appear inline where they happened. This ordering is saved with new replies; older chats without recorded event order keep their existing single-list layout.

Add projects with the **+** beside Projects, or **File → Add project**. Enter an existing folder path. Project chats have separate histories and drafts; right-click a chat to rename or delete it. Use the paperclip to choose files, paste copied files/screenshots with **Ctrl+V** (**Cmd+V** on macOS) while the chat input is focused, or drag files into the window. The **+** menu also offers paste and attachment by path. Each attachment has a removable preview; images show thumbnails. Attachments can be sent without a text prompt. Background loading stays with the chat where it started.

For **Codex**, **OpenAI API**, **OpenRouter**, and **llama.cpp**, enabling tools gives the assistant:

| Tool | Behavior |
| --- | --- |
| `ask_user` | Ask a multiple-choice question with exactly one recommended option. The question card shows a 30-second countdown. Send a selection or custom response, or Skip. Timeout automatically returns the recommended answer. |
| `list_files` | List up to 200 directory entries. |
| `read_file` | Read a UTF-8 file of any size, with optional byte `offset` / `max_bytes`. Large reads return a successful context-sized page and continuation offset; no file-size rejection. |
| `view_image` | Inspect a workspace image, send its pixels to the model, and show a thumbnail in the action history. |
| `send_image` | Return a workspace image visibly in the assistant's reply. |
| `write_file` | Write a full-file replacement up to 256 KiB and record its diff. |
| `run_command` | Run a shell command in the selected project using trusted host access by default, or the optional strict sandbox. Configurable timeout (30 minutes by default), beginning/error-tail capture and durable logs. |
| `web_search` | Search for up to five source URLs, titles and snippets using DuckDuckGo, Brave Search API, or a SearXNG server. |
| `web_fetch` | Read an HTTP/HTTPS page as text, with its final source URL and relevant links. Supports HTML, text, JSON and XML, including local docs servers. |

**Large source files are supported, not refused at 64 KiB.** `read_file` accepts `path`, `offset` (zero-based byte position; null starts at zero), and `max_bytes` (null uses the default). Omitted range fields from older calls still work. Ordinary complete reads return the text as before. Partial/explicit reads include `eof`, `file_bytes`, `bytes_returned`, and `next_offset`; use that offset to continue when needed. The agent must not mistake a partial page for the complete file. Pages preserve UTF-8 boundaries and seek directly rather than reading the whole file first, even for huge files or single long lines. Payloads are bounded by the available model context (shared across reads in a tool round), with a 1 MiB per-page memory ceiling, **not a maximum file size**. Invalid text and workspace escapes still produce errors.

File tools allow every file within the project, including dotfiles. They reject absolute paths, parent traversal, and symlink escapes. Writes use a temporary file and rename.

**Workspace actions run automatically by default.** Shell commands are enabled for new settings; an existing saved choice to disable them is preserved. **Settings → Tools → Review each tool action** enables optional approval for reads, writes, image tools, commands, and web requests. Declined actions are reported back to the model. Stop cancels the active task and pending questions or approvals; a file write already underway can finish.

### Command access and Git authentication

**Trusted host commands are now the default**, including when loading older settings without an explicit command mode. Existing choices to disable commands or enable review are preserved. Change this in **Settings → Tools → Command environment**. Active tasks keep the mode captured when started; a changed setting applies to the next run.

Trusted commands run as your normal user in the selected project's actual directory. They inherit the environment available to hfx: normal home and temporary storage, PATH/toolchains, Git configuration/identity, SSH agent/keys/known hosts, credential helpers, proxy settings and desktop/GPU access. `git fetch` and `git push` are available without a special shell permission gate. hfx does not change your primary author/committer or configure authentication on your behalf. If a key is locked, login is absent, or an agent is not available to the app, complete that setup normally; commands have non-interactive stdin. There is no built-in privilege elevation.

**Trusted mode is not a filesystem or secret-isolation boundary. Commands and repository build scripts can read credentials and change files outside the project. Use it only for projects/code you trust.** The known inference/search API-key environment variables (`OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `LLAMA_API_KEY`, `BRAVE_SEARCH_API_KEY`) are removed from command children, and keys entered in settings are session-only; this does not prevent trusted code from accessing other host secrets. File/image tools still enforce workspace-relative paths and reject symlink escapes.

Choose **Strict workspace sandbox** for untrusted projects. On Linux, bubblewrap confines commands to the writable project and private temporary storage. The working directory is `/workspace`; the original absolute project path is also available. OS and Rust runtime files are mounted read-only. Other projects, host temporary files, login caches and desktop session sockets are hidden. Commands receive a cleared environment with a private home. Cached Cargo crates are reused through project-owned writable overlays under `.hfx/cache`; nested runs reuse the already-mounted merged cache rather than mounting a busy overlay work directory again. Network access remains available. Missing sandbox support produces a tool error, **never an automatic fallback to trusted execution**. Strict command confinement remains Linux-only; trusted commands use `/bin/sh` on Unix and `cmd.exe` on Windows.

The timeout is configurable from 30 seconds to two hours (default 30 minutes). Both output pipes are drained, retaining up to 64 KiB per stream: the beginning and final diagnostics rather than only the beginning. Timeouts preserve partial output. Private command logs survive under `.hfx/command-logs/<run-id>/{stdout,stderr}.log`, capped at 16 MiB per stream; their folders contain Git ignore rules to avoid accidental staging. If durable logging is unavailable (for example a read-only project), the command still runs with bounded in-memory diagnostics and an explicit warning. Logs are retained until you remove them and can contain sensitive command output—do not publish them indiscriminately. On Unix the run folder is mode 0700 and log files are 0600. Stop/timeout cancels the active Unix command process group; deliberately detached jobs are outside that group. Windows currently cancels the direct command process.

### Web search and fetch

**Settings → Tools → Web research** enables native `web_search` and `web_fetch` for all four inference providers, independently of shell command enablement. Search defaults to **DuckDuckGo**, with no key required. Free automated search can encounter rate limits/challenges; these are reported explicitly rather than fabricated results or repeated workaround attempts. For a dedicated API use **Brave Search API** with a session-only key or `BRAVE_SEARCH_API_KEY`. Alternatively configure a **SearXNG** search endpoint whose server allows `format=json`.

Search queries are sent to the selected service. Fetch makes anonymous HTTP/HTTPS GET requests, including to local documentation servers, follows up to five redirects, supports gzip and declared text charsets, and returns readable text plus source links. It sends no model-provider credentials or browser cookies and does not execute JavaScript. Downloads are bounded to 2 MiB and returned content to 20,000 characters, with truncation indicated. Binary/PDF downloads and JavaScript-only browsing are not supported. Web content is untrusted reference data, never instructions; the assistant is told to verify relevant sources and cite their actual URLs. Expanded web actions include clickable source links. The web toggle controls these native tools, not a network firewall for shell commands.

### Tool responsiveness

Contiguous independent `read_file`, `list_files`, `web_search`, and `web_fetch` calls run up to four at a time when review is off. Their protocol results retain the model's original ordering. Commands, writes, questions, images, and all reviewed actions remain sequential barriers; a read after a write sees the completed write. No tool runs on a partial/unsuccessful model turn.

Chat-compatible adapters proceed after a successful finish reason rather than waiting indefinitely for a trailing `[DONE]` or connection close. They allow at most 100 ms for optional trailing usage, retaining normally delivered counters; late usage can be absent, so context estimates remain the fallback. Context estimation reads borrowed history instead of repeatedly cloning large tool outputs, and compaction only copies a candidate tail after choosing it. Streamed function arguments and reasoning-detail strings append in place instead of repeatedly copying their entire accumulated payload. Hover the completed reply's elapsed-time label for provider/model time versus summed tool runtime (parallel runtimes overlap); sub-second actions display milliseconds. Model generation, network latency, reasoning effort, long-running commands, and auto-compaction can still dominate a turn.

Provider turns continue until the assistant finishes, the user presses Stop, or a request fails. There is no fixed tool-round cutoff or required follow-up after 12 rounds. Completed tool rounds are retained even if a later request fails or is stopped. Expand the action history to see human-readable file reads, edits, commands, and questions, with elapsed time and approval/failure/cancellation status. Expand an individual entry for its arguments, full captured output, and edit diff. The badge above the composer opens all recorded file edits: it counts distinct files and cumulative added/removed lines from completed `write_file` calls in this chat. No-op and declined writes do not add changes; arbitrary edits made inside shell commands are not included in that badge. Demo mode performs no file tools or model requests.

To return a chart, screenshot, or artwork, the assistant can create it with `run_command`, inspect it with `view_image`, and attach it with `send_image`. These tools use the same workspace path rules and optional review mode. Image outputs are passed to Responses as multimodal function results, and to Chat Completions as labelled image context after all tool calls are answered. Command execution drains stdout and stderr while keeping bounded beginning/tail diagnostics and durable workspace logs.

Click any image thumbnail to open a resizable viewer with Fit, zoom, and **Save PNG**. Viewed images appear beneath their action; sent images appear in the reply and remain in saved chats. Each reply can return up to 8 images. hfx also displays image payloads returned directly by a compatible provider (completed Responses image items or Chat Completions image content). This does not configure a separate image generation API: availability depends on the selected model and endpoint. Returned base64 images and HTTPS image URLs use the same file and pixel limits as attachments; downloads are anonymous, bounded, and do not forward provider credentials.

`ask_user` requires 2–6 options with `label`, `description`, and exactly one `recommended: true`. The 30-second timer starts when the tool question is issued and continues if the user switches chats. Choosing an option without pressing Send does not override the recommendation at timeout. Stop cancels the active turn and question. This timer never approves file writes or commands.

Attachments support UTF-8 text/code (256 KiB each) and PNG, JPEG, WebP, or GIF images (10 MiB, at most 32 megapixels each), with up to 8 files per message. Images are normalized to PNG; GIF uses the first frame. Other binary document formats are rejected with a visible error. Text file contents are included as reference data; images use Responses `input_image` for OpenAI/Codex and Chat Completions `image_url` for OpenRouter/llama.cpp. Image inference requires a vision-capable model; llama.cpp also needs its multimodal projector configured. Explicit attachments may be outside the project. Clipboard images use arboard, Wayland file lists use data-control, and Windows file lists use CF_HDROP. Clipboard access happens only after a paste gesture. Platform clipboard/portal support varies; file selection and drag-and-drop remain available.

## Git co-author attribution

The built-in task instructions tell **Codex, OpenAI, OpenRouter, and llama.cpp** to add exactly one `hfx` co-author trailer to commits they create containing their contributions, and to inspect those commit messages before pushing. Older user/third-party/PR commits are left alone and do not become attribution failures. The user remains the primary author/committer; hfx does not change global Git identity, make commits/pushes just for credit, claim unrelated commits, or silently rewrite existing history to add attribution. A push alone cannot change a commit message.

The default is a deliberately unlinked, non-deliverable placeholder, not somebody else’s account:

```text
Co-authored-by: hfx <hfx@local.invalid>
```

Set **Settings → Tools → Git attribution → Co-author email** to the hfx hosting profile’s verified email or its exact GitHub/GitLab noreply address. The name always stays **hfx**, independent of the model/provider. This public commit email is saved with preferences. Existing profiles and custom assistant prompts receive the rule automatically without having their prompt text overwritten. Empty/malformed addresses fall back to the safe placeholder, with an inline warning; email validation checks syntax, not profile ownership.

### Showing the little hfx icon on GitHub/GitLab

Git stores a name/email trailer, **not an image**. Hosting sites show a co-author’s avatar when that email maps to a recognized profile. To link our name and icon:

1. Use a hosting account you control for hfx; set its display name to **hfx**.
2. Upload [`assets/hfx.png`](assets/hfx.png) as its profile avatar. The portable bundle also includes `hfx-icon.png` for upload; Linux installs have the same PNG under `$XDG_DATA_HOME/icons/hicolor/256x256/apps/hfx.png` (normally `~/.local/share/icons/hicolor/256x256/apps/hfx.png`).
3. Copy that account’s verified email or exact noreply address into the co-author email setting. On GitHub, find it in **Settings → Emails**; do not guess another account’s noreply address.
4. New hfx-created commits use that trailer, allowing the hosting site to associate the co-author with the profile and its avatar.

The placeholder cannot supply a linked profile/avatar. hfx does not create, verify, sign into, or upload an avatar to a hosting account automatically. This is an **instruction-level rule**, not a Git hook or a shell-command interceptor; model compliance and host identity mapping still apply. It does not modify pre-existing commits or the current checkout just by enabling it.

## Motion and persistence

- UTF-8-safe text reveal smooths bursts from every provider and accelerates to catch up with long responses.
- Messages fade in; the welcome screen fades out; panels slide; hover states and menus fade; the idle mark breathes gently.
- **Settings → Appearance** controls reveal speed, text size, reasoning visibility, and reduced motion.
- Chats, project paths, drafts, queued/sent attachments, action history, edit diffs, and preferences save every five seconds and on clean exit. Interrupted responses restore as stopped.
- Serialization and disk writes run on a background worker with buffered I/O and a single coalesced latest pending snapshot. Large tool outputs and image/context payloads are shared in save snapshots, and the UI renders conversations without copying their history each frame. Action labels are cached instead of repeatedly parsing full file-edit arguments while typing.
- API keys entered in settings remain in memory and are **never serialized**. Use environment variables to avoid re-entering them after relaunch.
- Conversations and file context are saved locally in plaintext; images are stored as base64 in the same local state. On Linux, chat state is normally in `~/.local/share/hfx/chats.json` (or under `XDG_DATA_HOME`); `app.ron` holds lightweight UI state. Legacy chats migrate automatically after the first successful background save. Chat files use mode 0600 on Unix. macOS and Windows use eframe's platform data directory.

### Linux background responsiveness

Generation, steering acknowledgements, queued follow-ups, question timers, and background chat autosaves run in eframe’s logic hook, including when it skips painting a minimized/occluded window. Unfocused/hidden windows use a slower background tick and skip animated text reveal; updates catch up without losing their recorded order. Incoming event bursts have a per-tick count limit and a soft time budget so they yield back to the window event loop.

Linux OpenGL rendering disables blocking VSync waits. glutin documents that, on Wayland, waiting for a buffer swap in a hidden window can block until it is visible again—preventing the event loop from answering the desktop’s responsiveness watchdog. hfx uses repaint timers for its streaming/animation pacing instead. This is not a suppression of the desktop’s “Application Not Responding” warning; a driver override or a different UI stall can still cause that warning. Click **Wait** rather than **Terminate** if you want to allow an in-progress run to recover; force termination can lose work since the last save.

### Shutdown and diagnostics

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

## Verify and preview

```bash
cargo fmt --all --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo run --locked -- --preview=welcome
cargo run --locked -- --preview=chat
cargo run --locked -- --preview=settings
cargo run --locked -- --preview=approval
cargo run --locked -- --preview=actions
cargo run --locked -- --preview=attachments
cargo run --locked -- --preview=question
cargo run --locked -- --preview=images
cargo run --locked -- --preview=image-viewer
python3 tests/linux_install.py target/release/hfx
```

Preview modes use isolated, scripted state and never replace saved chats. Other previews: `reasoning`, `appearance`, `menu`, `codex`, `openrouter`, `git-attribution`.

Tests cover fragmented Unicode/SSE, streamed and parallel tool calls with sparse terminal output, incomplete-response execution guards, automatic execution through fourteen tool rounds, replay across user turns for all four inference adapters, legacy action context, typing while large tool history saves in the background, image tool round trips and image-only replies, saved image context, bounded beginning/tail command output, durable partial logs and command-group cancellation, fixture Git pushes with preserved host identity/SSH setup, native web search/fetch and source replay across all providers, workspace and optional shell sandbox boundaries, credential serialization, PKCE against the RFC test vector, callback/state validation, code exchange, token rotation, 401 retry, cancellation, keyboard handling, project/rename dialog validation, 75% context thresholds, bounded summary chunks, tool-safe compaction and checkpoint replay across all four providers, summary failure safety, chronological progress/tool groups, independent live disclosures and persisted ordering, hidden-window logic and timeout handling, bounded stream-event draining, Linux nonblocking VSync configuration, launcher path escaping/ownership/localized Desktop support, installer updates and data-preserving uninstall, completion alert settings/batch timing/helper fallbacks/timeouts, coalesced final saves/flush fences/error reporting, bounded runtime cleanup, atomic PNG exports, FIFO queues, safe-boundary steering and attachment replay across all four providers, paused queue recovery, inline icon alignment across display scales/row heights, persisted/safe Git co-author emails and attribution instructions across all four providers/tool continuations, and layouts at two window sizes. Auth and provider tests use local Rust HTTP fixtures; they perform no real sign-in or model requests.

For a PNG capture in a desktop session:

```bash
cargo run --locked -- --preview=welcome --capture=/tmp/hfx.png
```

## Source map

- `src/app.rs`: native shell, panels, conversation, settings, approvals, lifecycle.
- `src/backend.rs`: async streaming adapters, SSE decoder, agent loop, connection probing.
- `src/codex.rs`: direct OpenAI OAuth, PKCE callback, private login cache, token refresh, and model discovery.
- `src/tools.rs`: workspace path checks, file tools, command execution and captured output.
- `src/sandbox.rs`: trusted host command setup and optional Linux confinement/private Cargo cache overlays.
- `src/commands.rs`: configurable command deadlines, process-group cancellation, beginning/tail output and durable private logs.
- `src/web.rs`: provider-independent search/fetch, readable HTML extraction, source URLs and bounded anonymous HTTP requests.
- `src/persistence.rs`: buffered/coalesced background chat serialization, durable final flush, and atomic storage.
- `src/lifecycle.rs`: bounded async/background runtime cleanup and optional shutdown timing diagnostics.
- `src/attachments.rs`: bounded file loading, clipboard/file URI handling, image normalization, and thumbnails.
- `src/motion.rs`: frame-paced text reveal and easing.
- `src/linux_desktop.rs`: Linux first-launch/CLI desktop registration, icons, and safe launcher removal.
- `src/notifications.rs`: nonblocking completion notifications and quiet audio playback.
- `scripts/`: per-user Linux installer/uninstaller and portable bundle builder.
- `src/state.rs`: persisted models, provider settings, interrupted-turn recovery.
- `src/theme.rs`: palette, vector icons, styled controls, lightweight Markdown rendering.

The Markdown view supports headings, bold spans, inline code, fenced code with copy controls, and selectable text. It is intentionally lightweight; full CommonMark tables and inline Markdown images are not yet implemented. Use `send_image` to return images as clickable thumbnails.

## Protocol references

- [OpenAI image inputs](https://developers.openai.com/api/docs/guides/images-vision)
- [OpenAI reasoning summaries](https://developers.openai.com/api/docs/guides/reasoning)
- [OpenAI streaming events](https://developers.openai.com/api/docs/guides/streaming-responses)
- [OpenAI function calling](https://developers.openai.com/api/docs/guides/function-calling)
- [OpenAI authentication](https://developers.openai.com/codex/auth)
- [Reference OAuth implementation](https://github.com/7shi/codex-oauth/blob/main/codex_oauth.py)
- [OpenRouter reasoning tokens](https://openrouter.ai/docs/guides/best-practices/reasoning-tokens)
- [OpenRouter tool calling](https://openrouter.ai/docs/guides/features/tool-calling)
- [bubblewrap](https://github.com/containers/bubblewrap/blob/main/README.md)
- [llama.cpp server and reasoning support](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md)
- [eframe](https://docs.rs/eframe/0.36.1/eframe/)

The vendored `egui-winit` 0.36.2 integration contains one paste-event fix: upstream consumes paste shortcuts without delivering an event when there is no clipboard text. hfx preserves an empty paste event so image/file pastes reach `raw_input_hook`. See `vendor/egui-winit/HFX-PATCH.md` when upgrading egui.

Headless visual QA (software-rendered egui geometry, without opening a desktop window):

```sh
cargo test --locked export_headless_previews -- --ignored
```

This writes agent-tools settings, web actions, welcome, attachment, question, returned-image, and image-viewer previews to `artifacts/headless-*.png`.
