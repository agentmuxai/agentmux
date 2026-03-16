# Plan: Agent Pane Fixes (v0.32.11)

## Analysis: CLI Install Failure (v0.32.10)

**Log timeline:**
```
17:23:14.603  ResolveCli provider=claude agentmux_version="0.32.10"
17:23:14.604  installing CLI via official installer (irm https://claude.ai/install.ps1 | iex)
17:23:19.608  [rpc-perf] command=resolvecli handler=5005.00ms
             (no further logs — handler likely hit RPC timeout or error swallowed)
```

**Result:** `~/.agentmux/0.32.10/cli/claude/bin/` exists but is empty. The installer
ran for ~5s (likely a no-op since claude.exe already exists at ~/.local/bin/) but the
handler timed out before the copy step. `~/.local/bin/claude.exe` exists (240MB).

**Root causes:**
1. PowerShell startup + `irm | iex` is slow (~5s) even when nothing needs downloading
2. The RPC engine has a default timeout — handler exceeded it
3. No "official installer completed" log appeared → the install_output await may have
   hit the timeout boundary, and the error was returned as an RPC error that the
   frontend displayed but we didn't log

**Fix:** The official Claude installer is designed for interactive system-wide install.
For AgentMux's use case (isolated per-version copy), we should:
- **Skip the official installer if claude.exe already exists at ~/.local/bin/**
- Just copy `~/.local/bin/claude.exe` → versioned dir directly
- Only run the official installer if the binary doesn't exist anywhere
- Increase RPC timeout for resolvecli (or make the install async with progress)

---

## Bug Fixes & Features

### 1. Fix CLI Install (Critical)

**File:** `agentmuxsrv-rs/src/server/websocket.rs` — ResolveCliCommand handler

Changes:
- Before running official installer, check known install locations first:
  - `~/.local/bin/<cmd>[.exe]` (official installer default)
  - PATH via `where`/`which` (as a source to copy from, NOT to use directly)
- If found at a known location, **copy** to versioned dir immediately (fast, no network)
- Only run official installer as last resort (when binary doesn't exist anywhere)
- Add timeout handling: wrap the powershell install in a tokio::time::timeout (60s)
- Log the install output (stdout/stderr) for debugging

### 2. Selectable Text + Right-Click Copy

**File:** `frontend/app/view/agent/agent-view.tsx` (or agent presentation component)

The agent pane renders markdown content. Currently text may not be selectable.

Changes:
- Ensure `user-select: text` on the agent content area (CSS)
- Add `onContextMenu` handler that shows a native context menu with "Copy" option
- Use `window.getSelection()` to get selected text
- Call `navigator.clipboard.writeText()` or `getApi().writeClipboard()` for copy

**File:** `frontend/app/view/agent/agent.scss` (or equivalent CSS)

```css
.agent-content {
    user-select: text;
    cursor: text;
}
```

### 3. Thin Borders (Remove Thick Borders)

**File:** `frontend/app/view/agent/agent.scss`

Current borders are too thick. Reduce to 1px or match terminal pane style.

Changes:
- Audit `.agent-*` classes for border styles
- Remove or reduce `border-width` to 1px
- Match the terminal pane's border treatment (thin, subtle)

### 4. Per-Pane Zoom (Same as Terminal)

The terminal already has per-pane zoom (Ctrl+/-, Ctrl+Scroll). Apply the same
to the agent pane.

**Files:**
- `frontend/app/view/agent/agent-view.tsx` — add zoom signal + keyboard handlers
- `frontend/app/view/agent/agent.scss` — apply font-size from zoom level

Approach:
- Read `term:zoom` from block meta (same atom as terminal: `termZoomAtom`)
- Or create `agent:zoom` meta key if we want independent zoom
- Apply zoom as CSS `font-size` multiplier on `.agent-content`
- Register Ctrl+/- and Ctrl+Scroll handlers (same pattern as terminal)
- Store zoom in block meta so it persists

---

## Implementation Order

1. Fix CLI install (unblocks testing everything else)
2. Selectable text + copy (quick CSS + handler)
3. Thin borders (CSS only)
4. Zoom (needs signal wiring)

## Build & Test

- `cargo check` after Rust changes
- `tsc --noEmit` after TS changes
- `bump patch -m "..." --commit && task package:portable`
- Test: open agent pane → should install/resolve claude → send message → get response
- Test: select text, right-click copy
- Test: Ctrl+/- zoom in agent pane
