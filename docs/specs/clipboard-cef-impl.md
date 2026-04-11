# Clipboard for CEF — Implementation Spec

**Date:** 2026-04-02
**Status:** Spec
**Problem:** Copy/paste doesn't work in CEF host — Ctrl+C/V, context menu copy/paste all fail silently

## Root Cause

The frontend's `util/clipboard.ts` imports from `@tauri-apps/plugin-clipboard-manager`, which only works in the Tauri host. In CEF, this import fails silently — clipboard operations do nothing.

CEF's Chromium blocks `navigator.clipboard.readText()` without a Permissions-Policy header, so the browser API doesn't work either.

## Solution

Route clipboard through CEF host IPC. The backend code already exists (`clipboard.rs`) but is untracked and not wired up.

## Changes Required

### 1. Host side: Wire up clipboard module

**File: `agentmux-cef/src/commands/mod.rs`**
```rust
pub mod clipboard;  // Add this line
```

**File: `agentmux-cef/src/ipc.rs`** — Add routes:
```rust
"read_clipboard" => commands::clipboard::read_clipboard(),
"write_clipboard" => commands::clipboard::write_clipboard(args),
```

**File: `agentmux-cef/src/commands/clipboard.rs`** — Already exists (untracked), needs `git add`.

### 2. Frontend side: Route through CEF IPC when in CEF mode

**File: `frontend/util/clipboard.ts`**

Replace Tauri-only implementation with platform-aware routing:

```typescript
import { getApi } from "@/util/getapi";

// Detect CEF mode
function isCef(): boolean {
    return typeof (window as any).__AGENTMUX_IPC_PORT__ !== "undefined";
}

export async function readText(): Promise<string> {
    if (isCef()) {
        return getApi().invokeCommand("read_clipboard", {});
    }
    // Tauri fallback
    const { readText: tauriReadText } = await import("@tauri-apps/plugin-clipboard-manager");
    return (await tauriReadText()) ?? "";
}

export async function writeText(text: string): Promise<void> {
    if (isCef()) {
        await getApi().invokeCommand("write_clipboard", { text });
        return;
    }
    // Tauri fallback
    const { writeText: tauriWriteText } = await import("@tauri-apps/plugin-clipboard-manager");
    await tauriWriteText(text);
}
```

Key: Use dynamic `import()` for Tauri so it doesn't fail at load time in CEF.

### 3. CEF API shim: Add clipboard commands

**File: `frontend/util/cef-api.ts`** — Already has `invokeCommand` which handles IPC routing. No changes needed — `readText`/`writeText` in clipboard.ts call it directly.

## Existing Callers (no changes needed)

All these import from `@/util/clipboard` which we're updating:
- `app.tsx` — read + write
- `blockframe.tsx` — write
- `pane-actions.ts` — read + write
- `markdown.tsx` — write (code blocks copy button)
- `streamdown.tsx` — write
- `base-menus.ts` — write (context menu)
- `usenotification.tsx` — write
- `agent-view.tsx` — write
- `termViewModel.ts` — read + write (terminal paste)
- `termwrap.ts` — write (terminal selection copy)

One exception: `AgentDocumentView.tsx:115` uses `navigator.clipboard.writeText()` directly — should be updated to use `clipboardWriteText`.

## Files Changed

| File | Change |
|------|--------|
| `agentmux-cef/src/commands/mod.rs` | Add `pub mod clipboard;` |
| `agentmux-cef/src/ipc.rs` | Add 2 IPC routes |
| `agentmux-cef/src/commands/clipboard.rs` | `git add` (already exists) |
| `frontend/util/clipboard.ts` | CEF IPC routing + dynamic Tauri import |
| `frontend/app/view/agent/components/AgentDocumentView.tsx` | Replace `navigator.clipboard` with util |

## Test Plan

- [ ] Terminal: Ctrl+C copies selection, Ctrl+V pastes
- [ ] Context menu: right-click → Copy/Paste works
- [ ] Code blocks: copy button works
- [ ] Agent view: copy URL works
- [ ] Multi-window: clipboard shared (OS clipboard)
- [ ] Tauri host: still works (dynamic import fallback)

## Complexity

Low — 3 lines on host side (mod + 2 routes), ~20 lines on frontend side.
