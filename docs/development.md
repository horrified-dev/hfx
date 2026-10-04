# Development and visual QA

[README](../README.md) · [Providers](providers.md) · [Usage](usage.md) · [Tools & security](tools.md) · [Linux](linux.md) · [Development](development.md)

hfx is a Rust 2024 application built with egui/eframe. Rust 1.95 or later is required. The commands below use the locked dependency graph.

## Checks

Run from the repository root:

```bash
cargo fmt --all --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

The tests cover:

- **Providers and protocols:** fragmented Unicode/SSE, streamed and parallel tool calls, sparse terminal output, incomplete-response execution guards, fourteen tool rounds, replay across all four adapters, and legacy action context.
- **Authentication:** credential serialization, PKCE against the RFC test vector, callback/state validation, code exchange, refresh-token rotation, HTTP 401 retry, and cancellation.
- **Tools and security:** paged file reads, workspace and optional sandbox boundaries, bounded command output, durable partial logs, process-group cancellation, Git pushes with preserved host identity/SSH setup, and web search/fetch with source replay.
- **MCP:** explicit opt-in, configuration validation, collision-safe names, paginated discovery, stdio lifecycle, Streamable HTTP JSON/SSE, environment-based auth, timeouts without retries, images/structured results, and approvals with replay through every provider.
- **Conversation state:** 75% context thresholds, bounded summaries, tool-safe compaction and checkpoint replay, summary failure safety, FIFO queues, safe-boundary steering, attachment replay, paused queue recovery, and interrupted-turn recovery.
- **Images and UI:** Markdown link rendering, wrapped/Unicode link hit regions, click/keyboard activation, text-selection safety, hfx monogram reuse, image tool round trips, image-only replies, saved image context, keyboard handling, project/rename validation, chronological tool groups, independent disclosures, icon alignment, and layouts at two window sizes.
- **Lifecycle and desktop integration:** background saves with large histories, hidden-window logic, question timeouts, bounded event draining, Linux nonblocking VSync, localized launcher paths and ownership, installer updates, data-preserving uninstall, completion-alert batching and helper timeouts, final-save fences, bounded cleanup, atomic PNG export, and Git attribution settings/instructions.

[Rust quality CI](../.github/workflows/checks.yml) enforces these checks on Linux for both Rust 1.95 (the minimum supported version) and stable. It provisions bubblewrap 0.12 for the actual sandbox regression tests and runs Linux/macOS installer filesystem fixtures. The separate native macOS workflow still validates the macOS release bundle. No quality checks are skipped because of a missing sandbox runner.

Auth and provider tests use local Rust HTTP fixtures; they perform no real sign-in or model requests.

Linux installer integration checks require a release build:

```bash
cargo build --release --locked
python3 tests/linux_install.py target/release/hfx
```

### Streaming and context microbenchmarks

Two ignored tests exercise the streaming decoder and compaction planner with
synthetic inputs; they do not contact providers or read saved conversations:

```bash
cargo test --locked profile_sse_decoding -- --ignored --nocapture
cargo test --locked profile_compaction_planning -- --ignored --nocapture
```

The decoder benchmark covers fragmented long lines, many events delivered in one
chunk, and multiline events. The planner benchmark uses 3,001 messages. Compare
runs on the same machine and build profile; add `--release` to both commands for
optimized-build measurements. These are diagnostic microbenchmarks, not
end-to-end latency claims or timing assertions in the normal test suite.

### macOS installer checks

All installer fixtures keep install paths and application data in a temporary directory under `target`; they do not install into your actual profile.

```bash
python3 tests/macos_install.py
# On macOS, additionally exercise the built native binary and plutil:
python3 tests/macos_install.py target/release/hfx
```

The macOS filesystem tests also run on Linux with a fixture `uname`; that does **not** validate Finder/Launch Services or a native macOS build.

[Native macOS installer CI](../.github/workflows/installers.yml) builds on macOS and validates the installed bundle, icon, and executable. Local Linux fixture runs do not replace these native checks.

## Scripted previews

Preview modes use isolated state and never replace saved chats. They do not make real model requests or send native completion alerts.

```bash
cargo run --locked -- --preview=welcome
cargo run --locked -- --preview=chat
cargo run --locked -- --preview=settings
cargo run --locked -- --preview=approval
cargo run --locked -- --preview=actions
cargo run --locked -- --preview=attachments
cargo run --locked -- --preview=question
cargo run --locked -- --preview=images
cargo run --locked -- --preview=image-viewer
```

Other previews: `trust`, `recovery`, `edit-approval`, `markdown`, `reasoning`, `appearance`, `tools`, `menu`, `codex`, `openrouter`, and `git-attribution`. The [Markdown preview](../artifacts/markdown-preview.png) demonstrates commit links, formatted labels, and literal code.

### Refresh README screenshots

The README images are native-window captures of the scripted `welcome`, `actions`, and `codex` previews. Window size may vary with desktop scaling and window-manager placement. To regenerate them in a desktop session:

```bash
mkdir -p assets/screenshots
for preview in welcome actions codex; do
  cargo run --locked -- --preview="$preview" --capture="assets/screenshots/$preview.png"
done
```

The window closes after a successful capture. Use preview mode rather than a real chat to keep screenshots free of private history and login credentials.

### Headless visual QA

Software-rendered egui geometry can be exported without opening a desktop window:

```bash
cargo test --locked export_headless_previews -- --ignored
```

This writes previews to `artifacts/headless-*.png`, covering tool settings, web actions, welcome, attachments, questions, returned images, image viewer, queues, context meters, and dialogs. To regenerate only one group:

```bash
HFX_PREVIEW_FILTER=welcome cargo test --locked export_headless_previews -- --ignored
HFX_PREVIEW_FILTER=settings cargo test --locked export_headless_previews -- --ignored
HFX_PREVIEW_FILTER=appearance cargo test --locked export_headless_previews -- --ignored
HFX_PREVIEW_FILTER=tools cargo test --locked export_headless_previews -- --ignored
```

Settings exports cover the Providers, Appearance, and Tools tabs at 1180×820
and 720×540, including scrolled `-details` views and advanced tool settings.
Layout tests check card widths, keyboard/pointer switches, provider selection,
disabled preferences, and the fixed footer while scrolling.

### Shutdown diagnostics

See [Shutdown and diagnostics](usage.md#shutdown-and-diagnostics) for tracing and a synthetic-history I/O benchmark that does not read user history.

## Blender MCP live demonstration

The opt-in [Blender demonstration](blender-mcp.md) exercises the actual Rust MCP client against a running local Blender GUI, including discovery, modeling, viewport image decoding, rendering, and scene inspection. It is ignored in normal test runs; it deliberately modifies only a new or previously generated demo scene and writes ignored artifacts. After following that guide:

```bash
cargo test --locked demo_blender_mcp -- --ignored --nocapture
```

## Source map

- [`src/app.rs`](../src/app.rs): native shell, shared harness state, project navigation, and lifecycle hooks.
- [`src/app/turns.rs`](../src/app/turns.rs): queue/steering coordination and bounded event polling.
- [`src/app/conversation.rs`](../src/app/conversation.rs): conversation, action history and diff rendering.
- [`src/app/composer.rs`](../src/app/composer.rs): composer and queued-message controls.
- [`src/app/attachments_ui.rs`](../src/app/attachments_ui.rs): attachment input, thumbnails and image viewer.
- [`src/app/settings.rs`](../src/app/settings.rs): provider, tool, appearance and auth settings.
- [`src/app/dialogs.rs`](../src/app/dialogs.rs): project, rename, queue-edit and tool-approval dialogs.
- [`src/app/safety.rs`](../src/app/safety.rs): explicit recovery and project-trust UI.
- [`src/app/previews.rs`](../src/app/previews.rs), [`src/app/tests.rs`](../src/app/tests.rs): isolated scripted previews and headless GUI regression tests.
- [`src/backend.rs`](../src/backend.rs): async streaming adapters, SSE decoder, agent loop, connection probing.
- [`src/codex.rs`](../src/codex.rs): direct OpenAI OAuth, PKCE callback, private login cache, token refresh, and model discovery.
- [`src/context.rs`](../src/context.rs): token estimates, model context limits, compaction planning, and checkpoints.
- [`src/file_edit.rs`](../src/file_edit.rs): bounded precise text editing, full-file digests, stale-content checks and atomic replacement.
- [`src/recovery.rs`](../src/recovery.rs): startup load handling and explicit private history backups.
- [`src/file_read.rs`](../src/file_read.rs): context-budgeted UTF-8 file paging and continuation metadata.
- [`src/tools.rs`](../src/tools.rs): workspace path checks, file tools, command execution and captured output.
- [`src/sandbox.rs`](../src/sandbox.rs): trusted host command setup and optional Linux confinement/private Cargo cache overlays.
- [`src/commands.rs`](../src/commands.rs): configurable command deadlines, process-group cancellation, beginning/tail output and durable private logs.
- [`src/web.rs`](../src/web.rs): provider-independent search/fetch, readable HTML extraction, source URLs and bounded anonymous HTTP requests.
- [`src/mcp.rs`](../src/mcp.rs): opt-in MCP connections, stdio process ownership, Streamable HTTP, schema mapping, and tool results.
- [`src/mcp_ui.rs`](../src/mcp_ui.rs): native JSON configuration editor and cancellable discovery tests.
- [`src/persistence.rs`](../src/persistence.rs): buffered/coalesced background chat serialization, durable final flush, and atomic storage.
- [`src/lifecycle.rs`](../src/lifecycle.rs): bounded async/background runtime cleanup and optional shutdown timing diagnostics.
- [`src/attachments.rs`](../src/attachments.rs): bounded file loading, clipboard/file URI handling, image normalization, and thumbnails.
- [`src/motion.rs`](../src/motion.rs): frame-paced text reveal and easing.
- [`src/linux_desktop.rs`](../src/linux_desktop.rs): Linux first-launch/CLI desktop registration, icons, and safe launcher removal.
- [`src/notifications.rs`](../src/notifications.rs): nonblocking completion notifications and quiet audio playback.
- [`scripts/`](../scripts/): per-user Linux/macOS installers and uninstallers, shared ownership helpers, and the Linux portable bundle builder.
- [`src/state.rs`](../src/state.rs): persisted models, provider settings, interrupted-turn recovery.
- [`src/settings_ui.rs`](../src/settings_ui.rs): layered settings cards, provider tiles, switches, and shared form controls.
- [`src/theme.rs`](../src/theme.rs): palette, hfx monogram, vector icons, and styled controls.
- [`src/markdown.rs`](../src/markdown.rs): selectable Markdown, inline link parsing, wrapped link hit regions, and code-block copy controls.

The Markdown view supports headings, bold spans, inline code, clickable HTTP/HTTPS links (including formatted link labels), fenced code with copy controls, and selectable text. Links open only after a click or keyboard activation; unsupported URL schemes and credential-bearing links are not activated. It is intentionally lightweight; full CommonMark tables and inline Markdown images are not yet implemented. Use `send_image` to return images as clickable thumbnails.

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

The vendored `egui-winit` 0.36.2 integration contains one paste-event fix: upstream consumes paste shortcuts without delivering an event when there is no clipboard text. hfx preserves an empty paste event so image/file pastes reach `raw_input_hook`. See [`vendor/egui-winit/HFX-PATCH.md`](../vendor/egui-winit/HFX-PATCH.md) when upgrading egui.
