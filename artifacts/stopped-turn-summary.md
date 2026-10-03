# Stopped-turn harness bug

## Reproduced trigger

Delete an inactive chat from the sidebar while a different chat has a running turn. The old deletion handler called `Saved::restore()`, which is restart recovery, not merely selection repair. It marked all streaming replies and running/waiting activities cancelled, paused remaining queues, and cleared dispatched-steering markers. It did not abort the active backend task. This reproduces a stopped reply footer alongside the composer's running Stop button.

The locally saved reply matching the reported screenshot was cancelled with no recorded provider error. There is no recorded UI-event history proving which action triggered that particular occurrence, so the confirmed deletion trigger is a reproduction matching the symptom, not proof of the original interaction.

## Fix

- Extract `Saved::repair_chat_selection()` to repair project/chat references and selection without changing valid live messages or queues.
- Keep interruption recovery in `Saved::restore()`, called only at startup in production.
- Use selection repair instead of restart recovery after sidebar deletion.
- Document inactive-chat deletion behavior in README.md.

## Verification

- The new real-UI deletion regression failed before the fix (`Cancelled` instead of `Streaming`): stopped-turn-repro.txt.
- It passes after the fix for both keeping the active chat selected and deleting the selected inactive chat. It checks task identity, reply status, tool statuses/results, dispatched steering (including no duplicate dispatch), continued output and terminal completion: stopped-turn-regression.txt.
- State tests cover live-message/queue preservation, orphan-reference repair, and empty-chat/project fallbacks.
- `cargo fmt --all --check`: passed.
- `cargo test --locked`: 149 passed, 0 failed, 6 ignored: stopped-turn-tests.txt.
- `cargo build --release --locked`: passed; updated target/release/hfx: stopped-turn-release-build.txt.
- Strict Clippy is blocked by pre-existing collapsible_if, len_zero, and map_or_identity findings in unchanged code: stopped-turn-clippy.txt. With only those three categories excluded, remaining strict checks passed: stopped-turn-clippy-scoped.txt.

Only source/docs and workspace artifacts were changed. The running app, installed executable, persisted chats and credentials were not modified. To update the installed app, run `./scripts/install-linux.sh --no-build` and restart hfx when ready.
