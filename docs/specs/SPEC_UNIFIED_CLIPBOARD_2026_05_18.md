# SPEC: Unified Copy/Paste + Export

**Status:** Draft
**Date:** 2026-05-18
**Author:** AgentA
**Related:** [`SPEC_AGENT_INSTALL_STAGE_2026_05_17.md`](./SPEC_AGENT_INSTALL_STAGE_2026_05_17.md) (the install modal that surfaced this gap), `frontend/util/clipboard.ts` (existing CEF clipboard bridge).

---

## 0. TL;DR

The user reports they can't copy text from the install modal's xterm.js terminal. They want **copy/paste to work from anywhere in the app**, plus two adjacent actions:

- **Save selection / pane content to a file** (e.g. `~/.agentmux/clips/install-2026-05-18.log`).
- **Open in editor** (VS Code, or the user's configured external editor).

Today: clipboard works only in *some* surfaces (regular terminal pane, code blocks, context menu actions), and even then via four different paths. The install modal terminal has **zero** clipboard wiring — the audit found no `onSelectionChange`, no `term:copyonselect` read, no paste handler, no keyboard binding. xterm.js's `getSelection()` is never queried, and the container has `user-select: none` from the shared `xterm.css`, so browser-native selection-copy is blocked too.

Fix: introduce a **selection-provider** abstraction at the pane level + **global keyboard / context-menu / action-bar** plumbing on top. Every selectable surface registers a provider; global commands (Copy / Copy All / Save As… / Open in Editor) query the focused provider. Single source of truth for clipboard writes (the existing `frontend/util/clipboard.ts` CEF wrapper); single source of truth for write-to-file and open-in-editor (new `RpcApi.WriteFile` + `RpcApi.OpenExternal` wiring).

---

## 1. Problem

### 1.1 What the user reports (2026-05-18)

> "I tried copying the npm log in the install modal and it was unsuccessful."

Open install modal → install → try to select the log → either no selection visible (xterm.js's selection didn't register) or selection visible but Ctrl+C does nothing.

### 1.2 Audit findings (2026-05-18)

| Surface | Copy on selection | Ctrl+Shift+C | Paste | Context menu Copy | Notes |
|---|---|---|---|---|---|
| Regular terminal pane (`term.tsx`) | ✅ via `term:copyonselect` setting | ✅ xterm.js default | ✅ wired | ✅ | Reference implementation |
| **Install modal terminal** | ❌ | ❌ | ❌ | ❌ | Zero clipboard wiring |
| Code blocks (`markdown.tsx`, `streamdown.tsx`, `table-block.tsx`) | n/a | ✅ button | n/a | ❌ | Dedicated copy button |
| Agent pane (chat/doc view) | n/a | partial | ❌ | ❌ | `window.getSelection()` only |
| Browser pane | n/a | depends on CEF | depends on CEF | ❌ | Out of scope (uses CEF native) |
| PreLaunchAuth OAuth URL copy | n/a | ✅ button | n/a | n/a | Uses `navigator.clipboard.writeText` *directly*, bypassing the wrapper |

### 1.3 Root cause: no unifying contract

Four different clipboard paths exist:

1. `frontend/util/clipboard.ts` — CEF IPC wrapper (`read_clipboard` / `write_clipboard`). The "right" path.
2. xterm.js's built-in selection model (used by the terminal pane but the API isn't exposed to anyone else).
3. `window.getSelection()` (used by agent doc view).
4. `navigator.clipboard.writeText` directly (used in `PreLaunchAuthPanel.tsx:366`).

Path 4 only works by accident: Chromium's permission policy normally blocks `navigator.clipboard.readText()`. The write side works without permission today, but this is fragile.

Each path has its own context menu wiring (or none). Adding a new pane means re-implementing the four actions (Copy, Copy All, Save As…, Open in Editor) from scratch — which is why the install modal launched with none.

---

## 2. Best practices research

### 2.1 VS Code

- Single global Copy/Paste command. Every editor/output view registers a "text contribution" so the command knows what to grab.
- "Reveal in Editor" pattern: any output buffer (terminal, problems panel, output channel) has a single click that opens its contents in a real editor tab.
- "Save Selection As…" and "Save Output As…" use a single OS file dialog; the resolved path goes to either `fs.writeFile` or a "tail this file" pseudo-watch.

### 2.2 Slack / Discord / Linear

- Hover-revealed action bar on selectable content blocks (messages, log lines): Copy, Copy Link, Save, Share. Lower discoverability than a context menu but no right-click required.
- Slack additionally exposes a "copy link to this message" — selection-aware, not just text.

### 2.3 Chrome DevTools

- Right-click anywhere → context menu with "Copy", "Copy all as cURL" (action-specific), "Save as…". The console terminal supports both `window.getSelection()` (native) AND a custom selection model.
- Lesson: the right unit is "this view's current selection" — let each view decide what counts.

### 2.4 Common rules across all three

1. **One global command per action** (Copy, Save, Open in Editor). No per-pane reinvention.
2. **Each view tells the global command what's selected** via a thin provider interface — never asks each view to handle the command itself.
3. **Keyboard shortcut + context menu + (optional) action bar** are three views of the same command. Pick a command; surface it three ways.
4. **External writes use the OS's save dialog or a known directory**. Never paste data into a random path without user awareness.

---

## 3. Proposed architecture

### 3.1 The contract: `SelectionProvider`

A pane that has selectable content implements:

```ts
interface SelectionProvider {
    /** Best-effort current selection. Returns the empty string when nothing
     *  is selected (so callers can branch on truthiness). */
    getSelection(): string;
    /** Full content of the view, regardless of selection. Used by
     *  "Copy All", "Save As…", "Open in Editor". For long-running
     *  panes (terminal scrollback), bounded by the existing scrollback
     *  cap. */
    getAll(): string;
    /** Optional label for export filenames — e.g. `install-${agent}` so
     *  the suggested file is named meaningfully. Defaults to the pane's
     *  view type if omitted. */
    exportLabel?(): string;
}
```

Providers register themselves with the focused-pane registry on mount, unregister on unmount. The global commands query the registry for the focused pane's provider and act on what they get back.

```ts
// frontend/util/selection-registry.ts (new)
let active: SelectionProvider | null = null;

export function registerSelectionProvider(p: SelectionProvider): () => void;
export function getActiveSelectionProvider(): SelectionProvider | null;
```

Activation follows DOM focus. The pane's container wires `onFocusIn` / `onFocusOut` to `register` / `unregister`. Multiple panes can be mounted; only the focused one is active.

### 3.2 The four global commands

Wired once at app root (`frontend/app/app.tsx`), keyed off the active provider:

| Command | Key | Action |
|---|---|---|
| Copy | `Ctrl+C` (or `Ctrl+Shift+C` in terminal context) | `clipboardWriteText(provider.getSelection() || provider.getAll())` |
| Copy All | `Ctrl+Shift+A` | `clipboardWriteText(provider.getAll())` |
| Save Selection As… | `Ctrl+S` (when focus is in a non-editor pane) | `RpcApi.WriteFile({ path: dialogPick, content: provider.getSelection() || provider.getAll() })` |
| Open in Editor | `Ctrl+Shift+E` | Write to a temp file in `~/.agentmux/clips/`, then `RpcApi.OpenExternal(tempPath)` which routes to the user's configured editor (default: VS Code's `code <path>` if on PATH; fallback: OS default for `.txt`). |

All four also appear in the right-click context menu (`buildPaneContextMenu` extension) and as buttons in a hover-revealed action bar (Phase β).

### 3.3 Clipboard write path

All clipboard writes go through `frontend/util/clipboard.ts` → CEF IPC `write_clipboard` → `agentmux-cef/src/commands/clipboard.rs`. This is the existing path; the gap is enforcing it. Concretely:

- Audit + replace the one `navigator.clipboard.writeText` direct call in `PreLaunchAuthPanel.tsx:366`.
- Add an ESLint rule (or comment-noted code review checkpoint) forbidding direct `navigator.clipboard` use outside `frontend/util/clipboard.ts`.

### 3.4 xterm.js providers

The two xterm consumers (`termwrap.ts`, `AgentInstallModal.tsx`) wrap a tiny adapter:

```ts
function xtermSelectionProvider(term: Terminal, scrollback: () => string): SelectionProvider {
    return {
        getSelection: () => term.getSelection() || "",
        getAll: scrollback,
        exportLabel: () => "terminal",
    };
}
```

For the install modal specifically: `scrollback` is the full buffer (xterm.js's `serializeAddon` if loaded, else the live `buffer.active.getLine(i)` walk).

The xterm's existing `onSelectionChange` hook also gets wired in the install modal (it's already wired in `termwrap.ts:200-214`), respecting the global `term:copyonselect` setting. With this one change, **selecting text in the install modal copies it to the clipboard immediately** — covering the user's reported regression in two lines.

### 3.5 Non-xterm providers

| Pane | `getSelection` | `getAll` |
|---|---|---|
| Agent doc view | `window.getSelection().toString()` | Full markdown content of the view |
| Code blocks (markdown.tsx) | `window.getSelection().toString()` if inside block, else "" | The block's source text |
| Error banner / log viewers | `window.getSelection().toString()` | Concatenated lines |
| Forms (LaunchModal, etc.) | input/textarea native selection | n/a (not a logical "all" — skip Copy All) |

### 3.6 Write-to-file

Backend already has `RpcApi.FileAppendCommand`. Add a sibling `RpcApi.WriteFile` (overwrite) or piggyback on AgentMuxError-style error wrapping. For "Save As…" specifically:

```rust
// agentmux-srv/src/server/file_handlers.rs (new or extend existing)
#[derive(Deserialize)]
struct WriteFileReq {
    path: String,        // absolute, must be under user's home unless `unrestricted`
    content: String,
    overwrite: bool,
}
```

Path resolution: relative paths are resolved against `~/.agentmux/clips/`. Absolute paths under the user's home are allowed; outside the home requires `unrestricted: true` (future).

For the OS file picker, use `getApi().showSaveDialog({ defaultPath, filters })` — CEF exposes the native Chromium save dialog. If we don't have that bridge yet, the v1 implementation can drop content into `~/.agentmux/clips/<timestamp>-<label>.txt` and surface a toast with the path (good enough for "open in editor" follow-up).

### 3.7 Open in Editor

Two-step:

1. Write content to `~/.agentmux/clips/<timestamp>-<label>.txt` via the same path as 3.6.
2. Resolve the user's editor: setting `editor:external` (default: `"code"` on Windows/Linux, `"open -a 'Visual Studio Code'"` on macOS, fallback to OS default `open` / `xdg-open` / `start`).
3. Spawn editor via `RpcApi.OpenExternalCommand(path)` — already exists at `agentmux-cef/src/commands/platform.rs::open_external`.

The cleanup story: clips older than 7 days get pruned on startup. Capped at 1 MB / file to keep disk usage bounded.

---

## 4. Implementation phases

### Phase α — Install modal copy works (this PR)

Smallest viable fix the user can feel immediately:

1. Wire `onSelectionChange` in `AgentInstallModal.tsx` to call `clipboardWriteText(terminal.getSelection())` when `term:copyonselect` is true.
2. Wire `terminal.attachCustomKeyEventHandler` to intercept Ctrl+Shift+C → manual copy of current selection.
3. Add a right-click context menu (single item: "Copy") on the install modal terminal.
4. Replace the one `navigator.clipboard.writeText` in `PreLaunchAuthPanel.tsx:366` with `clipboardWriteText` from the wrapper.

That's ~30 lines. Spec + Phase α implementation ship as one PR. Internals doc page added.

### Phase β — Selection-provider registry + global commands

1. New file `frontend/util/selection-registry.ts` per §3.1.
2. App-root keyboard listener registers Ctrl+C / Ctrl+Shift+A / Ctrl+S / Ctrl+Shift+E → dispatches to active provider.
3. Both xterm consumers + agent doc view + markdown blocks register providers.
4. Extend `buildPaneContextMenu` with Copy / Copy All / Save As… / Open in Editor entries.

Separate PR after Phase α stabilizes.

### Phase γ — Write to file + Open in editor

1. New `RpcApi.WriteFile` backend handler.
2. `~/.agentmux/clips/` directory + 7-day pruning.
3. `editor:external` setting wired through to `open_external`.
4. Toast on save with click-to-open.

Separate PR.

### Phase δ — Action bar (optional, low priority)

Hover-revealed "[ Copy | Save | Open ]" bar on the install-modal terminal and other long-output surfaces. Nice-to-have; defer until the keyboard + context-menu surfaces have stabilized.

---

## 5. Acceptance criteria

### Phase α
1. Select text in install modal terminal → clipboard receives it (if `term:copyonselect` is on, or via Ctrl+Shift+C / right-click → Copy).
2. `grep -nE 'navigator\.clipboard' frontend/` returns 0 results.

### Phase β
1. `Ctrl+C` in any focused pane copies the pane's selection (or its full content, if nothing is selected).
2. New pane types add clipboard support by implementing `SelectionProvider` + calling `registerSelectionProvider` in `onMount`.

### Phase γ
1. `Ctrl+S` in any focused pane opens a save dialog (or writes to `~/.agentmux/clips/` with a toast).
2. `Ctrl+Shift+E` opens the saved file in the user's configured editor.

### Phase δ
1. Hovering an exportable output region shows the [Copy | Save | Open] bar.

---

## 6. Out of scope

- **Rich content copy** (HTML, images, MIME types). The wrapper is plain-text only; lifting this is a separate spec.
- **Browser pane** clipboard. CEF's browser child handles its own clipboard via Chromium internals; the host app shouldn't intercept.
- **Cross-app drag-and-drop** of files / panes. Different problem.
- **History / clipboard manager**. The user's OS handles this.

---

## 7. Internals doc

A new page at `agentmux-docs/src/content/docs/internals/clipboard.md` documents the contract, the wrapper, the keyboard/context surfaces, and the provider registration pattern. Linked from the Architecture sidebar entry. Lands alongside the Phase α PR.
