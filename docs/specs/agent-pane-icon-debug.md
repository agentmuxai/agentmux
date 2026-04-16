# Agent Pane Icon Buttons — Debug Log

**Date:** 2026-04-15  
**Version:** v0.33.188  
**Status:** Investigation in progress

---

## Observed Symptoms

### v0.33.187 (first attempt)
- Back-arrow button that existed previously: **disappeared**
- New ✏ ⚙ 👤 buttons: **never appeared**
- Icon `"settings"` confirmed invalid → changed to `"gear"`

### v0.33.188 (after fix attempt)
- **Still zero icons visible** — including the back-arrow that worked before v0.33.187
- All 4 buttons (`pencil`, `gear`, `person`, `arrow-left`) absent
- This is a **regression** — something broke `endIconButtons` entirely

---

## Changes Made in v0.33.187 That Could Cause Regression

In `frontend/app/view/agent/agent-model.ts`:

1. Added `import { createSignal } from "solid-js"` — first time this module imports from solid-js
2. Called `createSignal<OverlayTab | null>(null)` inside the `AgentViewModel` class constructor
3. Extended `endIconButtons` from 1 button to 4 buttons

---

## Primary Hypotheses (ranked by likelihood)

### H1 — `createSignal` called outside reactive owner breaks the model (HIGH)
SolidJS signals created outside a component tree have no owner and cannot be disposed.
More critically, if `AgentViewModel` is instantiated in a context where SolidJS's
internal tracking state is inconsistent (e.g., during module init), calling
`createSignal` there may corrupt the reactive graph or throw silently.
**Test:** Remove the `createSignal` call from the constructor (use a plain callback
ref pattern instead) — if icons return, this is the cause.

### H2 — Block frame has a max button count (MEDIUM)
The block frame header may render only the first N `endIconButtons` entries, and
adding 4 entries where 1 was expected causes the entire array to be skipped.
**Test:** Revert to 1 button (just back-arrow) and check if it reappears.

### H3 — `endIconButtons` type signature mismatch (MEDIUM)
The ViewModel interface declares `endIconButtons: () => IconButtonDecl[]`.
Adding `showOverlayTab` and `setOverlayTab` as additional class properties
might have caused a TS or runtime structural mismatch. (Less likely since
`tsc --noEmit` passes.)

### H4 — Icon names `"pencil"` / `"gear"` / `"person"` not valid FA solid icons (LOW)
Previous investigation confirmed `"gear"` and `"pencil"` are in use elsewhere.
But `"person"` may not be — FontAwesome solid uses `"user"` not `"person"`.
This would cause empty icon boxes, not missing buttons entirely.

---

## Action Plan

1. **Isolate H1:** Refactor — replace `createSignal` in constructor with a
   callback ref (`setOverlayTab` stored as a mutable class field, assigned
   by `AgentPresentationView` on mount) to eliminate solid-js dependency
   from the model constructor.

2. **Verify icon names:** Confirm `"pencil"`, `"gear"`, `"person"` (or `"user"`)
   against the FA icon set actually bundled in the frontend.

3. **Check block frame max buttons:** Read block frame header component to see
   if `endIconButtons` has a slot limit.

---

## Tilde Folder Bug

### Root Cause
`GH_CONFIG_DIR = "~/.agentmux/config/gh-${slug}"` is set as a string literal in
`agent-model.ts`. When the Rust backend receives this and calls `create_dir_all`
on it (to ensure the directory exists before passing it to the agent CLI), Rust
does NOT expand `~` — it treats it as a literal directory name. This creates a
`~` folder in the process's current working directory.

The same issue exists for:
- `cmd:cwd = "~/.agentmux/agents/${slug}"` — if the Rust backend doesn't tilde-expand
  this before calling `.current_dir()`, a `~` folder is created
- `AGENTMUX_AGENT_ID/GH_CONFIG_DIR` in `app_api.rs` line 157 which also calls
  `create_dir_all` on a tilde path

### What Was Fixed in v0.33.188
`GIT_CONFIG_GLOBAL` tilde path removed. But `GH_CONFIG_DIR` and `cmd:cwd` still
use tilde paths — these are the remaining sources.

### Fix Needed
Either:
- Expand `~` in the Rust backend before any `create_dir_all` or `current_dir()` call
- Or use the `shellexpand` crate (already used elsewhere?) to expand all tilde paths
  in `cmd:env` values and `cmd:cwd` before they reach the OS

---

## Resolution (v0.33.189)

### Bug 1 — Icons not rendering
**Root cause confirmed:** `createSignal` called in `AgentViewModel` constructor, which runs
inside a SolidJS reactive scope when the block mounts. This corrupted owner tracking for
`endIconButtons`, causing `EndIcons` to see a stale empty array.

**Fix:** Removed `createSignal` from the model entirely. Added a plain mutable callback
`_setOverlayTab: ((tab: OverlayTab | null) => void) | null` on the model. The signal
`createSignal<OverlayTab | null>(null)` now lives in `AgentPresentationView` (the
SolidJS component), wired to `model._setOverlayTab` in `onMount` / cleared in
`onCleanup`. Also corrected `"person"` → `"user"` (correct FontAwesome solid icon).

### Bug 2 — Tilde folder created
**Root cause confirmed:** `WriteAgentConfigCommand` handler in `websocket.rs` line 838
called `std::fs::create_dir_all` on `cmd.working_dir` raw, which contained `~/.agentmux/...`.
Rust does not shell-expand `~`.

**Fix:** Added `expand_home_dir_safe(&cmd.working_dir)` (from `backend::base`, already
used in subprocess.rs) before the `create_dir_all` call.

---

## Regression Retro

### What broke and why it wasn't caught

The back-arrow button worked in v0.33.186. In v0.33.187 it disappeared — along
with the 3 new buttons. The regression was **introduced, not noticed in code review,
and shipped in two consecutive builds**.

**Root cause of the regression (hypothesis):**  
Calling `createSignal` from `solid-js` inside `AgentViewModel`'s constructor —
a plain TypeScript class instantiated outside of any SolidJS reactive tree —
likely corrupts or short-circuits the reactive graph in a way that silently
breaks `endIconButtons`. SolidJS requires `createSignal` to be called in a
tracking scope or at least with no active owner assumptions.

**Why it wasn't caught:**  
- `tsc --noEmit` passes — this is a runtime SolidJS issue, not a type error  
- No automated test covers "does the back-arrow actually render"  
- The pre-build check is `bump verify` only — no functional smoke test  
- The regression was introduced in the same commit as the new feature, making
  it harder to bisect mentally

**What should change going forward:**  
1. **Never call SolidJS primitives (`createSignal`, `createMemo`, etc.) in class
   constructors** that live outside the component tree. Use plain mutable fields
   or callback refs instead. If reactive state is needed in the model, initialize
   it lazily inside a component via `onMount` or pass it in.
2. **Test the simplest existing behavior first.** Before verifying new buttons,
   verify the OLD back-arrow still renders after a change to `endIconButtons`.
3. **Cargo `check` + TypeScript `noEmit` are not enough** — a UI change needs
   a visual pass before calling it done.
