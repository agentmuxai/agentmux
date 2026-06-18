# DnD File Drop: Retrospective & Re-Architecture

**Date:** 2026-03-10
**Status:** Rethinking after circular debugging loop

---

## Retrospective: What We Tried and Why It Failed

### Round 1 — `cmd:cwd` not set (spawn-time seeding)

**Observation:** Drop overlay showed "No working directory detected."
**Hypothesis:** Backend doesn't seed `cmd:cwd` in block meta when the shell spawns.
**Fix:** After shell spawn, write `cmd:cwd` to SQLite via `update_object_meta`.
**Failure:** Store was written but frontend Jotai atom never updated.
**Root cause missed:** `update_object_meta` only writes to SQLite. It does NOT push a `waveobj:update` WebSocket event. The frontend is purely event-driven for block meta — no polling.

---

### Round 2 — Broadcast missing, added event_bus broadcast

**Fix:** After writing to store, also broadcast `waveobj:update` via `event_bus.broadcast_event`.
**Failure:** Overlay now showed a path (`C:\Users\area54\...`) but it was the **backend process's** `std::env::current_dir()`, not the terminal's actual working directory. After `cd C:\NoPoe`, the label didn't update.
**Root cause missed:**
1. `std::env::current_dir()` is the agentmuxsrv-rs process's CWD — irrelevant to the shell's CWD.
2. OSC 7 (which correctly tracks the shell's CWD) fires via `termosc.ts → UpdateObjectMeta (HTTP)`. That HTTP handler returns `success_empty()` — **no update is returned to the frontend**. So even when OSC 7 fires correctly, the Jotai atom doesn't update.

---

### Round 3 — File.path is undefined in WebView2

**Observation:** The overlay showed a path but no file was copied.
**Root cause:** The original code read `(file as any).path` from the Web `File` API. In WebView2 (unlike Electron), `File` objects don't have a `.path` property. The source path was `undefined` for every file, so the copy loop silently skipped everything.

---

### Round 4 — Switched to Tauri `onDragDropEvent`

**Fix:** Rewrote `useFileDrop` to use `getCurrentWebview().onDragDropEvent()` which provides real filesystem paths.
**Failure:** `dragDropEnabled: false` was set in `tauri.conf.json`. Tauri's native file-drop events never fire when disabled.
**Root cause missed:** We assumed Tauri's drop events were active without checking the config first.

---

### Round 5 — Enabled `dragDropEnabled: true`, more fixes in flight

**Status at time of retro:** Stuck in another iteration with an unverified set of changes (dragDropEnabled enabled, UpdateObjectMeta now returns updated block, useFileDrop rewritten again) — none tested end-to-end yet.

---

## Pattern of Failure

```
Unknown assumption → implement fix → discover broken dependency →
fix that → discover another broken dependency → repeat
```

Each fix was locally correct but built on an untested assumption one level deeper.
We never did a full end-to-end trace **before** writing code.

---

## The Two Actual Problems (Separated)

### Problem A: Getting file system paths on drop

The Web File API (`e.dataTransfer.files`) does not expose OS file paths in a Tauri/WebView2 context. Tauri provides an escape hatch via `onDragDropEvent` but it requires `dragDropEnabled: true` in the window config.

### Problem B: Knowing the terminal's current working directory

`cmd:cwd` needs to be correct and reactive. There are two update paths:
1. **Spawn-time seed** (our fix) — sets an initial value using the *backend's* cwd as fallback. Wrong value, but better than nothing.
2. **OSC 7 updates** — fires whenever the shell changes directory. Goes through `termosc.ts → HTTP UpdateObjectMeta → returns success_empty() → frontend atom never updates`.

These are independent problems that were entangled during debugging.

---

## Root Cause Tree

```
DnD not working
│
├── A. No file paths available
│   └── File.path undefined in WebView2
│       └── Need Tauri's onDragDropEvent (requires dragDropEnabled: true)
│
└── B. cmd:cwd incorrect or stale
    ├── B1. Spawn-time seed gives wrong value (backend's cwd, not shell's cwd)
    │   └── std::env::current_dir() is agentmuxsrv-rs's cwd, not the spawned shell's
    │
    └── B2. OSC 7 updates don't reach frontend
        └── HTTP UpdateObjectMeta returns success_empty()
            └── callBackendService processes respData.updates — if it's empty, atom stays stale
```

---

## Architectural Issues Exposed

### Issue 1: Two update channels with inconsistent broadcast behavior

| Path | Writes DB | Broadcasts WS | Frontend updates |
|------|-----------|---------------|-----------------|
| WebSocket SetMeta command | ✓ | ✓ | ✓ |
| HTTP UpdateObjectMeta | ✓ | ✗ | Only if response includes updates |
| Shell spawn seed (our fix) | ✓ | ✓ (added) | ✓ |

HTTP `UpdateObjectMeta` was clearly an oversight — it just needed to return the updated object. We fixed this in Round 5. But this same bug likely affects other features beyond DnD.

### Issue 2: `dragDropEnabled: false` was set without documentation

No comment in the config explaining why it was disabled. It was probably disabled early in development to prevent conflicts with the tile drag system, then forgotten. With `dragDropEnabled: true`, the OS-level file drop is intercepted by Tauri and doesn't interfere with internal HTML5 drags.

### Issue 3: No end-to-end test for the feature

If there had been an automated test that dropped a file and verified it appeared in the target dir, we'd have caught all these failures immediately instead of through manual iteration.

---

## Clean Architecture Proposal

### For file paths (Problem A)

**Keep `dragDropEnabled: true`** and use Tauri's `onDragDropEvent`.

The hook architecture should be:
- Subscribe to Tauri events **once** (useEffect with empty deps, callback via ref)
- Use `getBoundingClientRect` vs event position to determine which pane is the drop target
- HTML5 events (`onDragEnter`, `onDragLeave`) still handle the visual overlay — they fire for OS drags even with `dragDropEnabled: true` on some platforms; use as a fallback
- `onDrop` (HTML5) calls `e.preventDefault()` only — no file handling there

```
OS file drag
  │
  ├─► Tauri onDragDropEvent (type='enter'/'over'/'drop'/'leave')
  │     └─► position check against element.getBoundingClientRect()
  │         ├─► type='enter'/'over' + in bounds → setIsDragOver(true)
  │         ├─► type='drop' + in bounds → onFilesDropped(paths), setIsDragOver(false)
  │         └─► type='leave' → setIsDragOver(false)
  │
  └─► HTML5 onDragEnter/onDragLeave (fallback for visual state only)
```

### For CWD (Problem B)

**Two-part fix:**

**B1 — Fix HTTP UpdateObjectMeta to return updated block:**
Already done (Round 5). This fixes OSC 7 updates propagating to the frontend atom.

**B2 — Fix spawn-time seed to use shell's actual launch directory:**
Instead of `std::env::current_dir()` (backend's cwd), read the `cmd:cwd` from block meta if set, otherwise use the user's home directory as a sensible default.

```rust
let effective_cwd = if !cwd.is_empty() {
    cwd.clone()  // explicit cwd from block meta (set by user or pane split)
} else {
    dirs::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
};
```

**B3 — Consider: query shell CWD at drop time (future improvement):**
At the moment the user drops a file, we could query the actual shell process's CWD directly via OS API (Windows: `GetCurrentDirectory` on the child process via `NtQueryInformationProcess`; Unix: read `/proc/PID/cwd`). This would be correct even without shell integration and without OSC 7. But it's complex; OSC 7 is the right long-term solution once B1 is fixed.

---

## Implementation Plan (Clean, Ordered)

### Step 1: Verify OSC 7 → UpdateObjectMeta → frontend atom chain ✅ (done in Round 5)
- `UpdateObjectMeta` now returns the updated block
- Atom updates immediately after `termosc.ts` fires

### Step 2: Fix spawn-time seed to use home dir as fallback (not backend's cwd)
- 5 min Rust change in `shell.rs`
- Gives sensible initial value before OSC 7 fires

### Step 3: Confirm `dragDropEnabled: true` doesn't break tile drag ✅ (done in Round 5)
- Internal HTML5 drags (tile layout) are not OS file drops — Tauri doesn't intercept them

### Step 4: Verify `onDragDropEvent` fires and paths are real
- Add a minimal test: console.log the paths on every drop event
- Confirm before writing any copy logic

### Step 5: Wire paths to `copy_file_to_dir`
- Only after step 4 is verified

### Step 6: Add tests
- Unit test `copy_file_to_dir` and `normalize_path_for_platform` in Rust (`#[cfg(test)]`)
- Integration test: create a temp file, simulate drop event, verify file appears in target dir
- Frontend: test `useFileDrop` hook in isolation using mock Tauri events

---

## What We Should Have Done First

1. **Trace the full data flow before writing any code** — from drop event → path extraction → copy → result visible in terminal
2. **Check all config/prerequisites first** — `dragDropEnabled`, `File.path` availability, WS vs HTTP update semantics
3. **Write the test first** — an automated test that drops a file and checks the result would have caught every failure in seconds instead of hours
4. **One hypothesis at a time** — we were fixing multiple layers simultaneously, making it impossible to know which fix worked and which introduced new bugs
