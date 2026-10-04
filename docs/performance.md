# Performance findings

[Development and checks](development.md) · [Usage](usage.md)

The first pass measured synthetic workloads before making two targeted changes.
Its baseline was commit `39946bc` with benchmark instrumentation only; the optimized
working tree used the same fixtures, machine, and build profile. A follow-up compared
legacy versus borrowed request construction, direct versus batched stream events,
and existing tool I/O. Those comparative variants run in the same release binary.
No saved user conversations, real authentication, or model endpoints were used.

## Measured improvements

Release builds, Rust 1.98.1, Linux x86_64, Intel Core i5-1145G7. Comparisons below
are medians of three consecutive runs. These are diagnostic microbenchmarks, not
whole-application FPS, peak-memory, network-traffic, or inference-speed claims.

| Workload | Before | After |
| --- | ---: | ---: |
| Render a completed reply with 8.8 MiB of hidden reasoning, 300 frames | 146.59 ms | 2.27 ms |
| Publish context updates for 64 tool rounds, about 67 KiB text per round | 27.02 ms | 1.26 ms |
| Cumulative text published in those 64 tool updates | 136.47 MiB | 4.20 MiB |
| Publish context updates for 128 tool rounds | 85.22 ms | 1.92 ms |
| Cumulative text published in those 128 tool updates | 541.67 MiB | 8.40 MiB |

### Borrow reply text during rendering

The conversation renderer previously copied both the visible answer and reasoning
into new strings on every frame, even when reasoning was hidden or collapsed.
Rendering now copies only the small reveal cursors and borrows UTF-8-safe slices
of the saved message. The lifetime of these slices is tied to the message, not the
mutable widget cache. Markdown, links, selection, and reveal behavior are unchanged.

The large hidden-reasoning fixture deliberately isolates an avoidable copy;
ordinary small replies will see a smaller absolute benefit.

### Append tool-context updates

Every completed tool round previously cloned and published the whole accumulated
wire suffix. Publishing repeated prefixes grows quadratically with the number of
rounds. Tool rounds now publish only their newly appended items, and the UI extends
its copy-on-write saved context. If an autosave retains an earlier snapshot, that
snapshot remains immutable; otherwise, appending does not copy the earlier prefix.

Final replies, steering seals, and resets still replace the full suffix, preventing
duplicate replay. Steering transfers ownership of the sealed suffix rather than
cloning and immediately clearing it. Model requests still contain their required
conversation history: this optimization changes internal UI updates, not provider
input or token accounting.

Regression tests cover all four adapters, successful and failed provider replies,
original-chat routing after a switch, compaction resets, final replacement,
pending deltas drained by Stop, and immutable saved snapshots.

## Bounded Markdown parsing cache

A later UI pass reused unchanged inline Markdown parsing, while leaving egui's
native label layout, glyph handling, and interactions in place. Three-run release
medians for the same 1180×820 conversation fixture (eight repeated prose/Unicode/link
lines per completed reply) were:

| Completed replies | Before, CPU ms/frame | After, CPU ms/frame |
| --- | ---: | ---: |
| 16 | 0.289 | 0.171 |
| 64 | 1.143 | 0.631 |
| 256 | 4.527 | 2.558 |

A separate same-binary comparison used individually distinct lines and link URLs,
comparing the original owned parser/label path with the cached path. Both variants
warm up for ten frames, then measure 30 frames:

| Distinct Markdown lines | Legacy, CPU ms/frame | Cached, CPU ms/frame |
| --- | ---: | ---: |
| 128 | 0.237 | 0.109 |
| 512 | 0.842 | 0.375 |
| 2,048 | 3.416 | 2.017 |
| 8,192 (cache pressure) | 16.702 | 13.318 |

These diagnostics measure CPU UI construction/layout and shape collection, not GPU
rendering or whole-application FPS. Cache benefits are smaller for changing text,
short plain strings, fenced-code bodies, and transcripts beyond its capacity.

The per-egui-context cache reuses immutable parsed text/formatting/link metadata.
It is limited to **4,096 entries and 8 MiB of accounted retained storage**, including
source/formatted text, section/link capacities, strings, and entry metadata. The
hash table and allocator have additional overhead bounded by the entry limit; this
is not a peak-process-memory measurement. Entries unused for a UI pass are evicted.
Overflow bypasses the cache instead of churning still-used entries, and uncached
oversized lines retain the original owned drawing path without a text clone.

Keys include exact source, font size, color, and strong style. Full source/style
checks also prevent hash collisions from reusing the wrong formatting or link URL.
Raw hashes avoid retaining streaming prefixes in egui's global debug Id registry.
This parsing cache stores no glyphs, screen positions, or message heights: width,
zoom/DPI, font definition changes, reasoning/tool disclosures, and text reveal
continue through native layout. The conversation viewport separately retains exact
message-height measurements as described below. Tests cover warm-frame geometry/formatting, resize/zoom/font changes,
Unicode streaming prefixes, unsafe URLs, selection/copy, keyboard and pointer link
activation, and cache entry/byte bounds. The cache is not part of saved chat data.

## Message-level viewport virtualization

The conversation now reserves the exact measured height of unchanged off-screen
messages instead of laying out their labels on every frame. Visible messages and
one viewport of overscan above/below use the original native message renderer.
The renderer still scans lightweight message metadata; it avoids the expensive
Markdown/widget construction for stable off-screen messages.

The long-transcript diagnostic compares the full-layout fallback and virtualized
path in the same release binary, with reduced-motion styling installed explicitly.
The fixture has eight repeated prose/Unicode/link lines per reply at 1180×820,
warms up for ten frames, then measures 30 frames. Three-run medians on the same
Linux/Rust 1.98.1 machine used above were:

| Completed replies | Full layout, CPU ms/frame | Virtualized, CPU ms/frame |
| --- | ---: | ---: |
| 16 | 0.171 | 0.067 |
| 64 | 0.637 | 0.068 |
| 256 | 2.652 | 0.077 |
| 1,024 | 10.850 | 0.099 |

The final warm frame laid out **five messages** in each virtualized fixture, versus
all messages in the full-layout variant. These are CPU UI-construction/layout
measurements, not GPU rendering, whole-app FPS, or model latency claims. Logs are
under `.hfx/performance-review/viewport-benchmarks.log` (Git-ignored).

### Correctness and invalidation

- Heights are measured, never guessed. Cold rows, reordered/replaced messages, and
  invalidated rows go through normal native layout even when off-screen.
- Measurements are kept only for the selected conversation, outside saved state.
  Chat switches and recovery reloads cannot reuse another conversation's heights.
- Width, style, font size, font definitions/family order, pixel scale, reasoning
  preferences, and approval-label changes invalidate the environment. Font data
  identities are compared without scanning font-file bytes on each frame.
- Backend events and Stop invalidate their original reply. Cheap structural keys
  also track visible reveal lengths and message metadata without copying or
  hashing large answer/reasoning bodies.
- Each skipped message reserves its original auto-ID slot as well as its height.
  Links, selection, code-copy buttons, and disclosure IDs therefore remain stable.
- Native scrolling still follows the bottom only while sticky; appending below a
  reader who scrolled up does not force them down. When remeasurement changes the
  clamped/sticky offset, an egui corrective pass avoids painting a blank viewport.
- Disclosures continue to be measured until their native animations settle, even
  if the row leaves the viewport. Text selection/dragging and Tab traversal use the
  full native path; a focused row remains pinned on subsequent idle frames.

Viewport regressions cover warm-frame visible geometry against full layout at
multiple scroll positions, resize/zoom/fonts/preferences, append/delete/reorder,
chat switches, off-screen events, Unicode reveal, sticky/non-sticky scrolling,
disclosure state/animation, focused composer and keyboard traversal, links,
selection/copy, code copying, image viewing, and Markdown list/table blocks.
Existing chronological-tool and recovery tests also continue to run.

### Remaining limits

Cold layout and global layout changes still measure the entire transcript once;
this pass optimizes steady-state rendering, not initial layout. A single enormous
visible reply still lays out its whole body. The standalone distinct-Markdown-line
benchmark does not exercise message virtualization and is unchanged. Block-level
virtualization within individual replies is a separate future target.

## Tool calls, text streaming, and request bodies

The follow-up used the same machine/toolchain and reports three-run release
medians. Its request fixture serializes 50 requests per variant, each 4.76 MiB on
the wire. Its streaming fixture decodes/parses/enqueues 50,000 Unicode deltas in
16 KiB network chunks; timings exclude UI rendering and model inference.

| Follow-up workload | Before | After |
| --- | ---: | ---: |
| Codex request construction + serialization, 50 requests | 162.74 ms | 150.90 ms |
| OpenAI request construction + serialization, 50 requests | 161.99 ms | 144.27 ms |
| OpenRouter request construction + serialization, 50 requests | 221.15 ms | 148.71 ms |
| llama.cpp request construction + serialization, 50 requests | 234.86 ms | 149.39 ms |
| Codex-format streaming decode/parse/enqueue | 23.42 ms | 17.52 ms |
| UI text events produced by that stream | 50,000 | 206 |
| llama.cpp-format streaming decode/parse/enqueue | 43.30 ms | 37.70 ms |
| UI text events produced by that stream | 50,000 | 240 |

### Borrow request history during serialization

Main requests now serialize borrowed history/tool definitions directly rather than
first constructing owned JSON copies. Chat-format requests serialize the system
message followed by borrowed history, eliminating the intermediate `with_system`
copy on ordinary requests and tool continuations. Summary request construction is
unchanged. Payload size and provider-visible fields remain the same; the improvement
is local request preparation, not a reduction in model input tokens or inference.

Parity tests cover all four adapters, reasoning/tool toggles, output caps including
zero settings, Unicode, images, opaque reasoning, temperature representation, and
Codex cache/tool options. The f32 temperature is promoted to f64 intentionally to
preserve the old JSON value exactly. Authentication, headers, retry behavior, and
endpoint validation remain separate and unchanged.

### Coalesce adjacent streamed UI deltas without a timer

Text and reasoning deltas are now combined only within each already-received network
chunk. The first nonempty answer and reasoning deltas are emitted immediately.
Type changes and control events flush pending text in order; merged events are capped
at 16 KiB, without splitting Unicode. A larger original provider delta is preserved
rather than truncated. Each synchronous batch ends before another network await,
and its drop guard flushes partial output on parsing errors. There is no added
cross-chunk debounce, no wait for the full response, and no early tool execution.

Tests verify direct/batched equivalence for all adapters with byte-fragmented UTF-8,
reasoning/encrypted state, usage, and fragmented tool arguments. Successful terminal
status requirements, trailing-output guards, and the existing 100 ms optional chat
usage-trailer grace period are unchanged. First-token assertions check event delivery
inside the batch; these diagnostics do not claim lower server time-to-first-token.

### Existing tool execution is already efficient

The controlled I/O fixture measured a 96 kB local file read at **0.173 ms/call**,
including tool events. Four independent reads with 80 ms simulated HTTP service
latency took **323.34 ms serially** and **80.87 ms with four-way concurrency**.
That concurrency is already used for contiguous read-only native calls. Writes,
commands, questions, reviewed calls, and unknown MCP tools remain ordered barriers;
this pass does not trade correctness or approval guarantees for extra parallelism.

Possible later investigations include scoped HTTP connection reuse across user turns
and avoiding redundant terminal-response payload copies. Neither is claimed as a
measured improvement here; cached clients/sessions must preserve configuration,
credential, project-trust, and shutdown boundaries.

## Already-efficient paths

The initial release diagnostics found no reason to rewrite these existing paths:

- SSE decoding: 100,000 coalesced short events in 9.61 ms; a fragmented 256 KiB
  line in 0.38 ms.
- Borrowed context estimation: 0.66 ms for 500 passes over 4.1 MiB of tool history.
  String lengths are inspected without cloning or scanning payload bytes.
- Compaction planning: 4.58 ms for the existing 3,001-message fixture.
- Buffered persistence: approximately 4.49 ms and 67 writes for a 4.3 MB synthetic
  snapshot, retaining file and directory durability synchronization.

These figures come from individual diagnostic runs, not the three-run comparisons
above. Storage timings depend on the filesystem and its current cache state.

## Reproduce

Run the current opt-in diagnostics from the repository root:

```bash
cargo test --release --locked profile_hidden_reasoning_frames -- --ignored --nocapture
cargo test --release --locked profile_tool_context_publishing -- --ignored --nocapture
cargo test --release --locked profile_long_transcript_frames -- --ignored --nocapture
cargo test --release --locked profile_distinct_markdown_lines -- --ignored --nocapture
cargo test --release --locked profile_request_serialization -- --ignored --nocapture
cargo test --release --locked profile_stream_event_batching -- --ignored --nocapture
cargo test --release --locked profile_independent_tool_io -- --ignored --nocapture
cargo test --release --locked profile_sse_decoding -- --ignored --nocapture
cargo test --release --locked profile_compaction_planning -- --ignored --nocapture
cargo test --release --locked profile_large_history_budgeting -- --ignored --nocapture
cargo test --release --locked profile_shutdown_save_io -- --ignored --nocapture
```

Use the same fixture, machine, and build profile for comparisons. Normal regression
tests assert correctness and preservation of borrowed/shared storage, not timing
thresholds. Local before/after logs are under `.hfx/optimization/` (Git-ignored).
