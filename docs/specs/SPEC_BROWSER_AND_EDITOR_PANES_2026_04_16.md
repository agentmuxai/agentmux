# SPEC: Browser and Editor Panes

**Date:** 2026-04-16
**Status:** active — Editor pane shipped in #415 (CodeMirror 6, matching this spec's recommendation over Monaco/Ace); Browser pane shipped in #422/#423 (native CefBrowserView, matching Option A over the iframe fallback — an earlier iframe-based version (commit 03e0730ec) was superseded); LSP (this spec's optional Phase 4) landed as "LSP Phase 1" in #1074 (TypeScript diagnostics). Verified 2026-08-23 against `frontend/app/view/browser/`, `frontend/app/view/editor/` (including an `lsp/` subdirectory), `agentmux-cef/src/browser_pane/`. **What remains**: this spec's Phase 3 "Agent Integration" required both `OpenEditorPaneCommand`/`mcp__agentmux__open_editor` AND `OpenBrowserPaneCommand`/`mcp__agentmux__open_browser` (lines 257-261) — only the editor half shipped (`OpenEditor` exists in `agentmux-mcp`); no `OpenBrowser` equivalent exists. Kept as `active`, not `implemented`, until that's closed (Codex catch on this PR's first pass, which had wrongly called it `implemented` with the gap only as a footnote).
**Priority:** Medium — enables agent workflows that need web access and file editing

---

## Problem

Agents frequently need to:
- Browse documentation, review PRs in GitHub, check dashboards
- Edit files with syntax highlighting, find/replace, go-to-line
- Preview HTML/Markdown output

Currently they can only do this via terminal tools (`curl`, `gh`, vim in a
terminal pane) or by asking the user to open an external window. There's no
in-pane browser or editor. This breaks the agent's flow — context switches
to external apps lose focus and break the single-window workflow.

---

## Goal

Add two new pane types to AgentMux:

1. **Browser pane** — embedded web browser for documentation, dashboards,
   PR reviews, OAuth flows
2. **Editor pane** — code/text editor with syntax highlighting, LSP support,
   file operations

Both render inside the existing pane layout system (split, resize, drag)
alongside terminal and agent panes.

---

## Browser Pane

### Approach: CEF Sub-Browser

AgentMux already runs inside CEF (Chromium). A browser pane creates a
**second CefBrowser instance** inside the same window, rendered into the
pane's DOM region.

**Two viable implementations:**

#### Option A: Native CefBrowserView (recommended)

Use CEF's Views framework to create a `CefBrowserView` as an overlay or
child view positioned over the pane's DOM rect.

```
AgentMux Window (CefBrowser #1 — main UI)
  └─ Pane layout
       ├─ Terminal pane (xterm.js in main browser)
       ├─ Agent pane (SolidJS in main browser)
       └─ Browser pane → CefBrowser #2 (separate process)
                          └─ navigates to any URL
```

**Pros:**
- Full Chromium rendering (not a sandboxed iframe)
- Separate process — crashes don't affect main UI
- Full DevTools per browser instance
- Native address bar, back/forward, reload

**Cons:**
- Higher memory (each CefBrowser ≈ 50-100MB)
- Position sync: must track pane DOM rect and reposition the native view
- Focus management between main browser and sub-browser

**Implementation in Rust (`agentmux-cef`):**
- `BrowserPaneView` struct wrapping `CefBrowserView`
- IPC command: `create_browser_pane(block_id, url)` → creates CefBrowser,
  positions over the block's DOM rect
- Resize observer in frontend sends rect updates → Rust repositions the view
- Navigation events (URL changes, title) piped back to frontend via IPC

#### Option B: iframe (simpler, limited)

Render an `<iframe>` inside the pane's DOM.

**Pros:**
- Pure frontend, no Rust changes
- Works immediately with existing pane system

**Cons:**
- Same-origin restrictions block many sites (X-Frame-Options)
- No address bar (must build in frontend)
- Shared process with main UI
- Cookie/storage conflicts

**Recommendation:** Start with **Option A (CefBrowserView)** for full
capability. Fall back to iframe for internal pages (help docs, settings).

### Browser Pane Features

| Feature | Priority | Notes |
|---------|----------|-------|
| URL navigation (address bar) | P0 | Type URL, Enter to navigate |
| Back / Forward / Reload | P0 | Standard browser controls |
| Tab title from page title | P0 | Pane header shows page title |
| Open link from agent | P0 | Agent can open URLs in browser pane |
| DevTools for browser pane | P1 | Right-click → Inspect |
| Bookmarks | P2 | Quick access to common pages |
| Cookie isolation | P1 | Separate cookie jar per pane |
| Print / Save as PDF | P3 | |

### Widget Definition

```json
{
    "defwidget@browser": {
        "display:order": 3,
        "display:pinned": true,
        "icon": "globe",
        "label": "browser",
        "description": "Embedded web browser",
        "blockdef": {
            "meta": {
                "view": "browser",
                "url": "https://github.com"
            }
        }
    }
}
```

### Agent Integration

Agents can open URLs in browser panes via MCP or RPC:

```typescript
// From agent pane, open a URL in a split browser pane
RpcApi.OpenBrowserPaneCommand(TabRpcClient, {
    url: "https://github.com/agentmuxai/agentmux/pull/413",
    position: "right",  // split right of current pane
});
```

---

## Editor Pane

### Approach: CodeMirror 6

CodeMirror 6 is the best fit for an embedded editor in a CEF app:

| | CodeMirror 6 | Monaco | Ace |
|---|---|---|---|
| Bundle size | ~124KB gzipped | ~15MB full | ~1MB |
| Modularity | Excellent (plugins) | Monolithic | Moderate |
| Mobile support | Yes | No | Partial |
| LSP support | Via plugin | Built-in | Via plugin |
| Actively maintained | Yes | Yes (Microsoft) | Legacy |
| License | MIT | MIT | BSD |

Monaco is what VS Code uses, but it's 100x larger and designed for a full
IDE — overkill for a pane-embedded editor. CodeMirror 6 gives us syntax
highlighting, search, and extensibility at a fraction of the cost.

### Editor Pane Features

| Feature | Priority | Notes |
|---------|----------|-------|
| Open file by path | P0 | Agent or user specifies file |
| Syntax highlighting | P0 | Language auto-detected from extension |
| Line numbers | P0 | |
| Find / Replace | P0 | Ctrl+F / Ctrl+H |
| Go to line | P0 | Ctrl+G |
| Save (write back to disk) | P0 | Ctrl+S → backend writes file |
| Read-only mode | P1 | For viewing without edit risk |
| Multiple cursors | P1 | |
| Minimap | P2 | |
| LSP diagnostics | P2 | Errors/warnings inline |
| LSP autocomplete | P2 | |
| Diff view | P2 | Side-by-side or inline |
| Git gutter (changed lines) | P3 | |

### Architecture

```
Frontend (SolidJS)
  └─ EditorView component
       └─ CodeMirror 6 instance
            ├─ Language extensions (auto-loaded)
            ├─ Theme (matches AgentMux dark theme)
            └─ File sync via RPC

Backend (Rust sidecar)
  └─ FileEditorService
       ├─ read_file(path) → content + metadata
       ├─ write_file(path, content) → ok/error
       ├─ watch_file(path) → change notifications
       └─ Optional: LSP proxy (spawn language server, bridge JSON-RPC)
```

### Widget Definition

```json
{
    "defwidget@editor": {
        "display:order": 4,
        "icon": "file-code",
        "label": "editor",
        "description": "Code editor with syntax highlighting",
        "blockdef": {
            "meta": {
                "view": "editor",
                "file": ""
            }
        }
    }
}
```

### Agent Integration

Agents can open files in editor panes:

```typescript
// Open a file in a split editor pane
RpcApi.OpenEditorPaneCommand(TabRpcClient, {
    file: "/path/to/file.ts",
    line: 42,           // optional: scroll to line
    position: "right",  // split position
    readOnly: false,
});
```

This replaces the pattern where agents say "open this file in your editor"
— instead they open it directly in a pane next to the conversation.

---

## Implementation Plan

### Phase 1: Editor Pane (frontend-only)

1. Install CodeMirror 6 + language packages
2. Create `EditorViewModel` + `EditorView` component
3. Add `editor` to `BlockRegistry` and `widgets.json`
4. Backend: `ReadFileCommand` / `WriteFileCommand` RPC handlers
5. File open via pane header or agent command

**Effort:** 3-4 days. No Rust CEF changes needed.

### Phase 2: Browser Pane (CEF changes required)

1. `BrowserPaneManager` in `agentmux-cef` — creates/destroys CefBrowserViews
2. IPC commands: `create_browser_pane`, `navigate`, `resize`, `close`
3. Frontend: `BrowserViewModel` + position sync via ResizeObserver
4. Add `browser` to `BlockRegistry` and `widgets.json`

**Effort:** 5-7 days. Requires CEF Views API work in Rust.

### Phase 3: Agent Integration

1. `OpenEditorPaneCommand` / `OpenBrowserPaneCommand` RPC handlers
2. Wire into agent tool results (e.g., Read tool → "open in editor" button)
3. MCP tool: `mcp__agentmux__open_browser`, `mcp__agentmux__open_editor`

**Effort:** 1-2 days.

### Phase 4: LSP Integration (optional)

1. LSP proxy in Rust sidecar — spawn language servers, bridge JSON-RPC
2. CodeMirror LSP client extension
3. Diagnostics, autocomplete, hover info

**Effort:** 3-5 days.

---

## Security Considerations

### Browser Pane
- Each CefBrowser runs in a sandboxed renderer process
- Cookie jar should be isolated per pane (prevent cross-site leakage)
- Consider blocking navigation to `file://` URLs
- JavaScript execution is sandboxed — cannot access host filesystem

### Editor Pane
- `WriteFileCommand` must validate paths — no writing outside the agent's
  working directory
- Read-only mode should be enforced server-side, not just client-side
- Large files (>10MB) should be rejected or streamed

---

## Non-Goals

- **Full IDE.** The editor pane is for quick edits and file viewing, not a
  replacement for VS Code. Complex refactoring still happens in the agent's
  terminal or via agent tool calls.
- **Tab management in browser pane.** Each browser pane is one page. Multiple
  pages = multiple panes (use the existing split/tab system).
- **Browser extensions.** CEF sub-browsers don't support Chrome extensions.
