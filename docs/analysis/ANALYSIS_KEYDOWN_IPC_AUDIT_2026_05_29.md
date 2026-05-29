# ANALYSIS — Keydown-path synchronous-IPC audit

**Date:** 2026-05-29
**Author:** AgentX
**Scope:** Phase 0.2 of the input-first execution plan (follow-up to [discussion #1161](https://github.com/agentmuxai/agentmux/discussions/1161)), enforcing invariant **I2 — "No synchronous IPC on any input path, ever."**
**Builds on:** `docs/specs/SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md`

---

## Why this audit

The input-first review identified the renderer↔host IPC layer as the highest-severity, hardest-to-see typing-latency threat: a synchronous renderer→host round-trip reachable from a `keydown` handler adds latency with **no JS-layer workaround**. Unlike the layout-read regression (already CI-guarded), there was no guard ensuring the keystroke path never blocks on IPC. This audit establishes the baseline and ships a guard to keep it.

## What "synchronous IPC on the input path" means here

The host IPC transport is **async by construction** — `invokeCommand()` (`frontend/app/platform/ipc.ts:30`) issues `fetch("http://127.0.0.1:${port}/ipc", …)` and returns a Promise. So a keystroke handler that *dispatches* a command fire-and-forget is **compliant** with I2 — it returns immediately and the result settles later. The violations are the **blocking** shapes:

1. `await invokeCommand(...)` / `await fetch(...)` inside an input handler — stalls the handler on a round-trip before paint.
2. An `async` `keyDownHandler` (returns a Promise) — the central dispatch in `store/keymodel.ts` treats the return as a synchronous boolean; making it async breaks that contract and invites awaited IPC.
3. Synchronous XHR (`XMLHttpRequest` + `.open(…, false)`) — always blocking; unacceptable on any path.

## How the input path actually dispatches

Central dispatch is `appHandleKeyDown()` (`frontend/app/store/keymodel.ts:369`, entry via `:433`):

```
keydown → appHandleKeyDown(waveEvent): boolean
  ├─ active chord? → chord handler (sync, returns boolean)
  ├─ global keymap handler (sync, returns boolean — may fire-and-forget invokeCommand)
  └─ focused block's viewModel.keyDownHandler(waveEvent): boolean  (keymodel.ts:412-413)
```

Every handler returns a **synchronous boolean** (consumed or not). Block-level handlers: `termViewModel.ts:390`, `launcher.tsx:63` — both synchronous.

## Findings — baseline is CLEAN

| Check | Result |
|---|---|
| `await invokeCommand` / `await fetch` inside input-dispatch files | **None** |
| `async keyDownHandler` / `keyDownHandler(): Promise` | **None** — all return synchronous `boolean` |
| Synchronous XHR (`open(…, false)`) anywhere in `frontend/app` | **None** |
| Fire-and-forget `invokeCommand` from keymap handlers | Present and **compliant** (dispatch, not await) |

The keystroke path does not block on IPC today. This is the property to preserve.

## Guard shipped

- **`tools/lint/check-input-handler-sync-ipc.sh`** — grep+awk scan (mirrors `check-input-handler-layout-reads.sh`). Flags awaited-IPC and async key handlers within the input-dispatch scope (`keymodel.ts`, `termViewModel.ts`, `launcher.tsx`, `AgentFooter.tsx`) plus a repo-wide synchronous-XHR ban. Comment lines skipped; escape hatch `// perf:allow-input-ipc — <why>`.
- **`.github/workflows/input-handler-sync-ipc.yml`** — path-filtered PR + main check.

Verified locally: passes clean on current `main`; exits non-zero on an injected `await invokeCommand` in scope; escape-hatch and comment-skip honored.

## Cross-platform note

This audit is **platform-independent** — it concerns JS on the renderer thread, identical across Windows/macOS/Linux. (The separate Win32 `SetWindowRgn` airspace path — `agentmux-cef/src/browser_panes.rs` — is already off the keydown path and is handled by Phase 0.3's stateless region cache; macOS NSView embedding is officially unsupported by CEF per `creation_views.rs:9`, so native-pane region work is per-platform and out of scope here.)

## Next (per execution plan)

- Phase 0.1 — statistically-real bench harness (pinned low-end Windows + macOS runner; delta-vs-baseline; reporting-mode first).
- Phase 0.3 — browser-pane keydown handler (verify it's a real gap first), region skip-if-unchanged + HWND cache, `block.scss` blur audit.
