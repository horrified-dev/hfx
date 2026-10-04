# Workspace tools and security

[README](../README.md) · [Providers](providers.md) · [Usage](usage.md) · [Tools & security](tools.md) · [Linux](linux.md) · [Development](development.md)

All four inference providers share the same tools. **Commands use trusted host access by default and are not confined to the project.** Enable review or choose the strict Linux sandbox before working with untrusted code. File and image tools always enforce workspace-relative paths.

## Workspace tools

Replies keep progress updates and tool activity in chronological order: normal conversation, a compact gray tool listing, the next progress update, then a fresh tool listing. Each group starts collapsed and expands independently to show its own commands/files, results, timings, images, and diffs. Tool-only rounds stay in the same group; a new visible progress update starts a new one. Context compaction notices appear inline where they happened. This ordering is saved with new replies; older chats without recorded event order keep their existing single-list layout.

For **Codex**, **OpenAI API**, **OpenRouter**, and **llama.cpp**, enabling tools gives the assistant:

| Tool | Behavior |
| --- | --- |
| `ask_user` | Ask a multiple-choice question with exactly one recommended option. The question card shows a 30-second countdown. Send a selection or custom response, or Skip. Timeout automatically returns the recommended answer. |
| `list_files` | List up to 200 directory entries. |
| `read_file` | Read a UTF-8 file of any size, with optional byte `offset` / `max_bytes`. Large reads return a successful context-sized page and continuation offset; no file-size rejection. |
| `view_image` | Inspect a workspace image, send its pixels to the model, and show a thumbnail in the action history. |
| `send_image` | Return a workspace image visibly in the assistant's reply. |
| `write_file` | Write a full-file replacement up to 256 KiB and record its diff. Prefer precise edits for existing files. |
| `edit_file` | Replace one unique exact text fragment in an existing UTF-8 file up to 16 MiB, with optional whole-file SHA-256 verification, concurrent-change checks, and a recorded diff. |
| `run_command` | Run a shell command in the selected project using trusted host access by default, or the optional strict sandbox. Configurable timeout (30 minutes by default), beginning/error-tail capture and durable logs. |
| `web_search` | Search for up to five source URLs, titles and snippets using DuckDuckGo, Brave Search API, or a SearXNG server. |
| `web_fetch` | Read an HTTP/HTTPS page as text, with its final source URL and relevant links. Supports HTML, text, JSON and XML, including local docs servers. |

**Paged file reads.** `read_file` accepts `path`, `offset` (zero-based byte position; null starts at zero), and `max_bytes` (null uses the default). Omitted range fields from older calls still work. All reads include `eof`, `file_bytes`, `bytes_returned`, `next_offset`, and `sha256`; use that offset to continue when needed. `sha256` is a full-file digest only when the entire file was returned from offset zero; it is null for partial reads, including a final page with a non-zero offset. hfx never scans unseen file contents just to compute a digest. The agent must not mistake a partial page for the complete file. Pages preserve UTF-8 boundaries and seek directly rather than reading the whole file first, even for huge files or single long lines. Payloads are bounded by the available model context (shared across reads in a tool round), with a 1 MiB per-page memory ceiling, **not a maximum file size**. Invalid text and workspace escapes still produce errors.

### Precise edits

Use `edit_file` for existing files: pass `path`, non-empty `old_text`, `new_text`, and `expected_sha256` (a digest from a complete read, or null when unavailable). The old fragment must occur exactly once, including overlapping occurrences. Include surrounding context for insertions; an empty new fragment deletes the match. Each fragment is limited to 256 KiB; the resulting file may be up to 16 MiB. Missing or ambiguous matches, invalid/stale digests, and detected changes while preparing the replacement fail without writing. Re-read before retrying. Unrelated current file contents are retained, permissions are preserved, and the replacement is atomic. These are optimistic concurrency checks, not a cross-process lock; an external editor can still race between the final check and rename.

Complete edit results include the new full-file SHA-256. Linux replacements retain ownership, permission bits, POSIX ACLs and extended attributes; concurrent ownership, inode or xattr changes are rejected. macOS uses native ACL/xattr copying and Windows uses metadata-preserving `ReplaceFile` for existing files. If access controls cannot be preserved, the operation fails rather than weakening them. Linux metadata snapshots are bounded to 1 MiB. No-op edits produce no change badge. Review mode shows both old and new text, and approvals never disable stale-content checks. `write_file` remains available for creation and deliberate small full-file replacement. Both tools feed the same saved diff history. Shell edits remain outside that badge.

File tools allow every file within the project, including dotfiles. They reject absolute paths, parent traversal, and symlink escapes. Writes use a temporary file and rename.

**Workspace file actions run automatically by default; enabled host commands and MCP require explicit project trust first.** Shell commands are enabled for new settings; an existing saved choice to disable them is preserved. **Settings → Tools → Review each tool action** enables optional approval for reads, writes, image tools, commands, and web requests. Declined actions are reported back to the model. Stop cancels the active task and pending questions or approvals; a file write already underway can finish.

### Command access and Git authentication

**Trusted host commands are the default**, including for older settings without an explicit command mode. Existing choices to disable commands or enable review are preserved. Change this in **Settings → Tools → Command environment**. Before enabled host tools may run, each project must explicitly acknowledge host access. New and migrated projects start untrusted, even with review mode enabled (review does not cover MCP server startup). The trust dialog pauses the original chat queue without consuming its message and never auto-accepts. Choose **Trust project and continue**, **Continue without shell commands or MCP**, or, on Linux, **Use strict sandbox (disable MCP)**. Alternatives change the global tool preferences for subsequent runs. Trust is stored against the project canonical directory; moving its path or retargeting a symlink does not grant trust to another directory. Tasks capture that canonical directory. **Settings → Tools → Project host access** can grant or revoke trust. Revoking stops the active run for that project and cancels MCP discovery; previously launched background services must be stopped separately. Active tasks keep the mode captured when started; a changed setting applies to the next run.

Trusted commands run as your normal user in the selected project's actual directory. They inherit the environment available to hfx: normal home and temporary storage, PATH/toolchains, Cargo/Rustup homes, language/package-manager configuration and caches, Git configuration/identity, SSH agent/keys/known hosts, credential helpers, API-key environment variables, proxy/TLS settings, network/localhost and desktop/GPU access. Ordinary builds, tests, runs, debugging and package-manager commands use this environment without a sandbox-specific setup step. `git fetch` and `git push` are available without a special shell permission gate. hfx does not change your primary author/committer or configure authentication on your behalf. If a key is locked, login is absent, or an agent is not available to the app, complete that setup normally; commands have non-interactive stdin. Installed tools and ordinary OS permissions still apply. There is no built-in privilege elevation.

**Trusted mode is not a filesystem or secret-isolation boundary. Commands and repository build scripts can read credentials and change files outside the project. Use it only for projects/code you trust.** Host environment variables, including `OPENAI_API_KEY`, `OPENROUTER_API_KEY`, `LLAMA_API_KEY` and `BRAVE_SEARCH_API_KEY`, are preserved so your own CLIs/tests can use them. Keys entered only in hfx settings remain session-only and are not automatically exported to command children. File/image tools still enforce workspace-relative paths and reject symlink escapes; that boundary does **not** apply to shell paths. Native anonymous web tools likewise do not limit authenticated CLI networking.

Every provider receives explicit, mode-specific guidance to run tools normally, preserve the supplied environment, and avoid invented sandbox workarounds. Models are told not to redirect Cargo/other host caches into `.hfx`, replace home/temp directories, add `--offline`, or create substitute environments merely because they assume confinement. Existing project configuration, explicit user requests and verified task-specific needs still take precedence. They are also told to prefer dedicated file tools and standard project commands over unnecessary Python wrappers for routine edits, backups, builds and tests; Python remains appropriate when requested, when the project uses it, or when it materially simplifies the task.

Choose **Strict workspace sandbox** for untrusted projects. On Linux, **bubblewrap 0.12 or later**, user namespaces, and overlay filesystem support are required. Bubblewrap confines commands to the writable project and private temporary storage. The working directory is `/workspace`; the original absolute project path is also available. OS and Rust runtime files are mounted read-only. Other projects, host temporary files, login caches and desktop session sockets are hidden. Commands receive a cleared environment with a private home. Cargo cache setup is automatic: run Cargo normally without setting `CARGO_HOME` yourself. Cached Cargo crates are reused through project-owned writable overlays under `.hfx/cache`; nested runs reuse the already-mounted merged cache rather than mounting a busy overlay work directory again. Network access remains available; sandbox mode does not require `--offline`. Missing sandbox support produces a tool error, **never an automatic fallback to trusted execution**. Strict command confinement remains Linux-only; trusted commands use `/bin/sh` on Unix and `cmd.exe` on Windows.

The timeout is configurable from 30 seconds to two hours (default 30 minutes). Both output pipes are drained, retaining up to 64 KiB per stream: the beginning and final diagnostics rather than only the beginning. Timeouts preserve partial output. Private command logs survive under `.hfx/command-logs/<run-id>/{stdout,stderr}.log`, capped at 16 MiB per stream; their folders contain Git ignore rules to avoid accidental staging. If durable logging is unavailable (for example a read-only project), the command still runs with bounded in-memory diagnostics and an explicit warning. Logs are retained until you remove them and can contain sensitive command output—do not publish them indiscriminately. On Unix the run folder is mode 0700 and log files are 0600. Stop/timeout cancels the active Unix command process group; deliberately detached jobs are outside that group. Windows currently cancels the direct command process.

### Development servers and other background commands

In trusted host mode, start a long-running service in the background, for example
`bun run dev &`. Foreground commands still wait for the shell to exit; hfx does
not guess that a slow build is a server or rewrite shell syntax.

After a successful shell exit, hfx gives stdout/stderr up to 150 ms to finish
draining. If either pipe remains open, the tool returns without waiting for the
background service to exit. A runtime-owned reader keeps both pipes alive, avoids
broken-pipe errors, and flushes subsequent output to the same private command
logs. The usual capture and 16 MiB log limits still apply. If logs cannot be
created, the result explicitly says so and output is drained without durable
storage.

A successful launch is **not** a readiness check. The assistant should poll the
service with a bounded request before taking screenshots or using it, inspect
its logs if startup fails, and stop it when finished. On Unix, the result reports
the background process-group ID. Stop/timeout still cancels an active foreground
command; Stop during a later tool does not retroactively stop an earlier
background launch. Attached background groups are stopped when hfx's runtime
shuts down. Fully redirected/deliberately detached jobs are not managed by those
pipe readers. A failed shell with open pipes has its remaining group stopped
instead of leaving a service running unnoticed.

Strict sandbox lifetimes remain scoped to their invocation; do not assume a
background server survives sandbox exit. There is no fallback to trusted mode.

### Web search and fetch

**Settings → Tools → Web research** enables native `web_search` and `web_fetch` for all four inference providers, independently of shell command enablement. Search defaults to **DuckDuckGo**, with no key required. Free automated search can encounter rate limits/challenges; these are reported explicitly rather than fabricated results or repeated workaround attempts. For a dedicated API use **Brave Search API** with a session-only key or `BRAVE_SEARCH_API_KEY`. Alternatively configure a **SearXNG** search endpoint whose server allows `format=json`.

Search queries are sent to the selected service. Fetch makes anonymous HTTP/HTTPS GET requests, including to local documentation servers, follows up to five redirects, supports gzip and declared text charsets, and returns readable text plus source links. It sends no model-provider credentials or browser cookies and does not execute JavaScript. Downloads are bounded to 2 MiB and returned content to 20,000 characters, with truncation indicated. Binary/PDF downloads and JavaScript-only browsing are not supported. Web content is untrusted reference data, never instructions; the assistant is told to verify relevant sources and cite their actual URLs. Expanded web actions include clickable source links. The web toggle controls these native tools, not a network firewall for shell commands.

### MCP servers

Optional [MCP support](mcp.md) connects external stdio and Streamable HTTP servers for all inference providers. Configure and test servers in **Settings → Tools → MCP servers**; MCP is off by default and no project configuration is auto-loaded. Discovered tools appear in the action history and every call requires approval in review mode, regardless of server annotations. External servers are not constrained by native file-tool paths; local servers inherit host access, and remote servers receive tool arguments. MCP is blocked in strict sandbox mode. See the [MCP guide](mcp.md) for configuration, secret references, limits, and unsupported features.

### Tool responsiveness

Contiguous independent `read_file`, `list_files`, `web_search`, and `web_fetch` calls run up to four at a time when review is off. Their protocol results retain the model's original ordering. commands, writes, precise edits, questions, images, and all reviewed actions remain sequential barriers; a read after a write sees the completed write. No tool runs on a partial/unsuccessful model turn.

Chat-compatible adapters proceed after a successful finish reason rather than waiting indefinitely for a trailing `[DONE]` or connection close. They allow at most 100 ms for optional trailing usage, retaining normally delivered counters; late usage can be absent, so context estimates remain the fallback. Context estimation reads borrowed history instead of repeatedly cloning large tool outputs, and compaction only copies a candidate tail after choosing it. Streamed function arguments and reasoning-detail strings append in place instead of repeatedly copying their entire accumulated payload. Hover the completed reply's elapsed-time label for provider/model time versus summed tool runtime (parallel runtimes overlap); sub-second actions display milliseconds. Model generation, network latency, reasoning effort, long-running commands, and auto-compaction can still dominate a turn.

Provider turns continue until the assistant finishes, the user presses Stop, or a request fails. There is no fixed tool-round cutoff or required follow-up after 12 rounds. Completed tool rounds are retained even if a later request fails or is stopped. Expand the action history to see human-readable file reads, edits, commands, and questions, with elapsed time and approval/failure/cancellation status. Expand an individual entry for its arguments, full captured output, and edit diff. The badge above the composer opens all recorded file edits: it counts distinct files and cumulative added/removed lines from completed `write_file` and `edit_file` calls in this chat. No-op and declined writes do not add changes; arbitrary edits made inside shell commands are not included in that badge. Demo mode performs no file tools or model requests.

To return a chart, screenshot, or artwork, the assistant can create it with `run_command`, inspect it with `view_image`, and attach it with `send_image`. These tools use the same workspace path rules and optional review mode. Image outputs are passed to Responses as multimodal function results, and to Chat Completions as labelled image context after all tool calls are answered. Command execution drains stdout and stderr while keeping bounded beginning/tail diagnostics and durable workspace logs.

Click any image thumbnail to open a resizable viewer with Fit, zoom, and **Save PNG**. Viewed images appear beneath their action; sent images appear in the reply and remain in saved chats. Each reply can return up to 8 images. hfx also displays image payloads returned directly by a compatible provider (completed Responses image items or Chat Completions image content). This does not configure a separate image generation API: availability depends on the selected model and endpoint. Returned base64 images and HTTPS image URLs use the same file and pixel limits as attachments; downloads are anonymous, bounded, and do not forward provider credentials.

`ask_user` requires 2–6 options with `label`, `description`, and exactly one `recommended: true`. The 30-second timer starts when the tool question is issued and continues if the user switches chats. Choosing an option without pressing Send does not override the recommendation at timeout. Stop cancels the active turn and question. This timer never approves file writes or commands.

See [Projects and attachments](usage.md#projects-and-attachments) for input-file limits and clipboard behavior.

## Git co-author attribution

The built-in task instructions tell **Codex, OpenAI, OpenRouter, and llama.cpp** to add exactly one `hfx` co-author trailer to commits they create containing their contributions, and to inspect those commit messages before pushing. Older user/third-party/PR commits are left alone and do not become attribution failures. The user remains the primary author/committer; hfx does not change global Git identity, make commits/pushes just for credit, claim unrelated commits, or silently rewrite existing history to add attribution. A push alone cannot change a commit message.

The default is a deliberately unlinked, non-deliverable placeholder, not somebody else’s account:

```text
Co-authored-by: hfx <hfx@local.invalid>
```

Set **Settings → Tools → Git attribution → Co-author email** to the hfx hosting profile’s verified email or its exact GitHub/GitLab noreply address. The name always stays **hfx**, independent of the model/provider. This public commit email is saved with preferences. Existing profiles and custom assistant prompts receive the rule automatically without having their prompt text overwritten. Empty/malformed addresses fall back to the safe placeholder, with an inline warning; email validation checks syntax, not profile ownership.

### Hosting-site avatars

Git stores a name/email trailer, **not an image**. Hosting sites show a co-author’s avatar when that email maps to a recognized profile. To link the hfx name and icon:

1. Use a hosting account you control for hfx; set its display name to **hfx**.
2. Upload [`assets/hfx.png`](../assets/hfx.png) as its profile avatar. The portable bundle also includes `hfx-icon.png` for upload; Linux installs have the same PNG under `$XDG_DATA_HOME/icons/hicolor/256x256/apps/hfx.png` (normally `~/.local/share/icons/hicolor/256x256/apps/hfx.png`).
3. Copy that account’s verified email or exact noreply address into the co-author email setting. On GitHub, find it in **Settings → Emails**; do not guess another account’s noreply address.
4. New hfx-created commits use that trailer, allowing the hosting site to associate the co-author with the profile and its avatar.

The placeholder cannot supply a linked profile/avatar. hfx does not create, verify, sign into, or upload an avatar to a hosting account automatically. This is an **instruction-level rule**, not a Git hook or a shell-command interceptor; model compliance and host identity mapping still apply. It does not modify pre-existing commits or the current checkout just by enabling it.
