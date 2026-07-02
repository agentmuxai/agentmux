# Spec: Modularize `blockcontroller/shell.rs`

**Date:** 2026-07-02
**File:** `agentmux-srv/src/backend/blockcontroller/shell.rs` (2,394 lines)
**Type:** Pure reorganization — zero logic changes, zero public API changes
**Tier:** Large

---

## Current state

- **2,394 lines:** ~1,620 impl + ~769 inline `#[cfg(test)] mod tests` (30+ tests)
- Contains **`impl Controller for ShellController`** — a large trait impl (~855 lines, lines 362–1217) which is the main complication vs the flat files split earlier
- **Platform-conditional code:** Unix-only signal delivery (`libc::kill`, SIGTERM→SIGKILL), Windows-only shell detection (`detect_local_shell_path_windows`), plus runtime `cfg!(windows)` branches for shell wrapper + PATH separator. We can only compile-verify Windows locally — CI covers ubuntu.

## Public API surface (must remain re-exported from `shell/mod.rs`)

Consumed by: `blockcontroller/mod.rs` (resync), `acp.rs`, `persistent.rs`, `subprocess.rs`, `watchdog.rs`, `agent_handlers/input.rs`, `blockfile.rs`, `app_api/mod.rs`.

- `ShellController` (struct) — instantiated in `resync_controller()`
- `handle_append_block_file()` — acp, persistent, subprocess, input
- `persist_to_blockfile_silent()` — persistent, subprocess
- `resolve_global_output_zone()` — acp, persistent, subprocess
- `rebuild_output_idx()`, `OUTPUT_IDX_HEADER_LEN` — blockfile, app_api
- `ConnFactory` (type alias) — test mocking
- `extract_agent_events()` — if referenced externally; else keep private

Keep private: `ShellControllerInner`.

## Proposed layout

```
blockcontroller/shell/
├── mod.rs             (re-exports + module decls)
├── controller.rs      (ShellController + ShellControllerInner struct defs; ConnFactory alias)
├── lifecycle.rs       (impl Controller: start/stop/get_runtime_status wiring + status mgmt + meta helpers)
├── pty.rs             (pty_size_from_rt_opts + platform shell detection w/ its #[cfg] guards)
├── spawn.rs           (command building, env/PATH setup — the ~200-line block from start())
├── io_tasks.rs        (the read / input / wait async task closures)
├── file_ops.rs        (handle_append_block_file, persist_to_blockfile_silent, mirror_append_to_global, resolve_global_output_zone)
├── indexing.rs        (rebuild_output_idx, OUTPUT_IDX_HEADER_LEN)
├── translation.rs     (extract_agent_events, accumulate_and_translate)
└── tests.rs           (the 769-line #[cfg(test)] mod tests)
```

## Execution notes — HIGHER RISK than the flat splits

- **The trait impl must stay one `impl Controller for ShellController` block.** Rust does not allow splitting one inherent/trait impl across files by simply moving methods. Two viable approaches:
  1. **Keep `impl Controller` whole in `lifecycle.rs`**, and move the big *free-function* helpers (spawn command-builder, io task bodies, file_ops, indexing, translation) into sibling modules that `lifecycle.rs` calls. The `start()` method body then calls `spawn::build_command(...)`, `io_tasks::spawn_read_loop(...)`, etc. This is the recommended approach — extract the helper bodies, keep the trait impl as the orchestrator.
  2. If any helper is currently an inherent method (`impl ShellController { fn ... }`), it can move to its own `impl ShellController` block in another file (multiple inherent impl blocks ARE allowed across files in the same crate).
- Preserve every `#[cfg(unix)]` / `#[cfg(windows)]` / `#[cfg(not(windows))]` guard exactly. Do not change which platform compiles which code.
- Each submodule gets its own `use` imports; no `#![allow(unused_imports)]`.
- Because a trait impl calls into moved free functions, those functions must be `pub(super)` or `pub(crate)` as needed.

## Verification gate

- `cargo check` + `cargo check --tests` clean on Windows, zero new warnings
- `cargo test shell` (or `cargo test blockcontroller::shell`) — all 30+ tests pass
- Manual re-read of every moved `#[cfg]`-guarded block to confirm guard integrity (CI ubuntu run covers Unix compile)
- reagent review

## Risk: **Medium.** Trait-impl boundary + platform cfg + hot-path PTY code. Do approach (1): keep `impl Controller` intact, extract helper bodies only. Verify tests pass before pushing.
