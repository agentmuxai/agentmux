# v0.33.26 Release Report

**Date:** 2026-04-02
**Base:** v0.33.24 → v0.33.26 (2 version bumps, 2 PRs merged, 1 PR closed)
**Built:** Portable ZIP deployed to Desktop

---

## Changes from v0.33.24

### PR #277 — Clipboard Support for CEF Host
**Files:** `agentmux-cef/src/commands/clipboard.rs` (new), `commands/mod.rs`, `ipc.rs`, `frontend/util/clipboard.ts`

Native clipboard implementation replacing the deprecated Tauri clipboard API:

- **Windows:** Win32 `GlobalAlloc`/`GlobalLock`/`OpenClipboard`/`SetClipboardData` — direct system clipboard access
- **macOS:** Shells out to `pbcopy` (write) and `pbpaste` (read)
- **Linux:** Environment-aware fallback chain:
  - Wayland: `wl-copy`/`wl-paste` (only attempted when `WAYLAND_DISPLAY` is set)
  - X11 fallback: `xclip -selection clipboard`, then `xsel --clipboard`
- **Frontend:** `frontend/util/clipboard.ts` updated to route through CEF IPC (`read_clipboard`/`write_clipboard` commands) instead of Tauri API
- **Review fixes applied before merge:**
  - `GlobalFree(hmem)` on `GlobalLock` and `OpenClipboard` failure paths (memory leak fix)
  - Wayland detection via `WAYLAND_DISPLAY` env check — prevents silent `wl-copy` failure on X11 sessions

### PR #278 — Terminal Echo Delay Fix
**Files:** `frontend/app/view/term/termwrap.ts`

- Small PTY writes (typical of character echo) bypass `requestAnimationFrame` batching and write directly to xterm.js
- Eliminates visible keystroke delay in fast-typing scenarios
- Guard added against out-of-order writes when RAF bypass and queued RAF writes interleave

### PR #276 — Closed
- Duplicate echo delay fix submitted by AgentX, superseded by #278

---

## Files Changed (v0.33.24 → v0.33.26)

| File | Lines | Description |
|------|-------|-------------|
| `agentmux-cef/src/commands/clipboard.rs` | +177 | New — native clipboard for Win/Mac/Linux |
| `agentmux-cef/src/commands/mod.rs` | +1 | Register clipboard module |
| `agentmux-cef/src/ipc.rs` | +4 | Route `read_clipboard`/`write_clipboard` IPC |
| `agentmux-cef/src/sidecar.rs` | +1/-1 | Minor (unrelated) |
| `frontend/app/store/ws.ts` | +13/-34 | Simplify WebSocket reconnect |
| `frontend/app/view/agent/components/AgentDocumentView.tsx` | +1/-1 | Minor |
| `frontend/app/view/term/termwrap.ts` | +14/-4 | RAF bypass for echo delay |
| `frontend/util/clipboard.ts` | +6/-6 | Route clipboard to CEF IPC |
| Version files (×7) | +13/-13 | 0.33.24 → 0.33.26 |

**Total:** 15 files, +227 / -55

---

## Current State

- **Local main:** `08ff00b` (v0.33.26 bump — **not yet pushed to remote**)
- **Remote main:** `156bfa6` (v0.33.25, PR #277 merge commit)
- **Working tree:** Clean
- **Desktop build:** `agentmux-cef-0.33.26-x64-portable/` — running

## Pending Action

- `git push origin main` to sync the v0.33.26 bump to remote
