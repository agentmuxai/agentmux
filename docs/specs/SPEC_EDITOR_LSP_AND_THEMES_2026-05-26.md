# Spec: Editor Pane — LSP integration + VS Code themes

**Branch:** `agenty/editor-lsp-themes-spec`
**Status:** Draft — design
**Date:** 2026-05-26
**Author:** AgentY
**Builds on:** [`SPEC_EDITOR_FILE_TREE_2026-05-26.md`](./SPEC_EDITOR_FILE_TREE_2026-05-26.md) (Phase 1 — already shipped via #1064 / #1067)

---

## TL;DR

Add **Language Server Protocol (LSP)** integration and **VS Code theme** support to the editor pane. Together these deliver ~90% of the smart-editor wins users expect from VS Code (autocomplete, hover, diagnostics, go-to-definition, themed UI) **without** building a full VS Code-style extension host.

Tier 1 (LSP) is ~3–4 weeks of work for an MVP (1 language, diagnostics + completion + hover); adding more languages is config-only after that. Tier 2 (themes) is ~1 week.

Explicitly **not** in scope: full VS Code extension API, contribution points, webviews, extension marketplace. Those are a multi-quarter project and the wrong battle — see § Why not full extension-host.

---

## Current state

The editor pane is **CodeMirror 6** + a file-tree explorer (shipped via #1064 + #1067 + #1070). Capabilities today:

- 7 lazy-loaded language packs (syntax highlighting only — no LSP)
- `oneDark` theme, hardcoded
- File-tree explorer rooted at `$HOME` with drives/mounts as sibling roots
- Read / write file with a 10 MB size cap
- Ctrl+S save, dirty tracking, read-only support

What's missing — and is the user-visible gap users hit immediately:

- **No autocomplete** (just whatever CM6's basicSetup provides for syntax-level identifiers)
- **No diagnostics** (no red squiggles for syntax errors, type errors, etc.)
- **No hover docs / go-to-definition / find references**
- **One theme** — no dark variants, no light, no community themes

The two tiers in this spec close those gaps for a couple weeks of work.

---

## Goals

1. **Smart code intelligence per language** — completion, hover, diagnostics, go-to-definition for at least TypeScript, Rust, Python, Go
2. **Themes parity** — let users pick from a curated set; import VS Code theme JSONs
3. **Graceful degradation** — if an LSP server isn't installed or crashes, the editor still works (highlighting + manual editing); a small banner explains
4. **Zero-config for common setups** — auto-detect language servers on `PATH`; require config only for non-standard locations
5. **Bounded resource use** — one LSP server per (workspace, language); shut down when the last file from that workspace closes

## Non-goals (explicit)

1. **VS Code extension API compatibility.** The `vscode` namespace is enormous. See § Why not full extension-host.
2. **Webview-hosted extensions.** No iframe sandbox for extension UIs.
3. **VS Code marketplace integration.** Themes via file drop; servers via OS binary discovery — no in-app marketplace browsing or signing.
4. **Snippet packs.** Could come later as Tier 3; not part of this spec.
5. **Multi-file refactors / rename across project.** LSP supports it; we'd add it after the basics land.
6. **Debug-adapter (DAP) integration.** Separate spec.

---

## Why not full extension-host

Three reasons, in order of weight:

1. **Cost.** A credible VS Code-API shim is 6–12 months of work — `vscode` namespace alone has ~50 contribution points, ~hundreds of API surfaces, with edge cases that determine whether real-world extensions actually work. Cursor and Windsurf solved this by *forking VS Code itself*, not reimplementing it. Zed deliberately did not attempt compat.
2. **Wrong differentiator.** AgentMux's edge is agent panes + jekt + the warden — none of that is unique to "how you edit code." Spending quarters on extension-host parity dilutes that focus without adding to it.
3. **80/20.** The capabilities users actually miss from VS Code — autocomplete, hover docs, diagnostics, themes — come from **language servers** (off-the-shelf binaries, standard protocol) and **theme JSONs** (a file format we can parse). We can deliver them in ~5 weeks of work, sized for one engineer.

If a user critically needs a specific VS Code extension, they have VS Code right there. AgentMux + VS Code is a viable workflow.

---

## Tier 1 — LSP integration

### Architecture

```
┌─ Frontend (CEF renderer) ────────────────────────────────┐
│                                                          │
│  CodeMirror editor                                       │
│    ↕                                                     │
│  lsp-client.ts                                           │
│   (sends/receives LSP JSON-RPC over the existing WS)     │
└──────────────────┬───────────────────────────────────────┘
                   │
                   │ WS — RPC messages: lspstart / lsprequest / lsp:* events
                   │
┌──────────────────▼───────────────────────────────────────┐
│ Backend (agentmux-srv)                                   │
│                                                          │
│  LspSupervisor                                           │
│   ├─ HashMap<(workspace_root, language), Child>          │
│   ├─ spawn server process (rust-analyzer, tsserver, …)   │
│   ├─ pipe stdio, JSON-RPC framing                        │
│   ├─ forward client requests → server                    │
│   ├─ forward server notifications → WS events            │
│   └─ kill server when last referencing file closes       │
└──────────────────┬───────────────────────────────────────┘
                   │ stdio (JSON-RPC framed)
                   │
┌──────────────────▼───────────────────────────────────────┐
│ LSP server process (off-the-shelf)                       │
│   rust-analyzer | typescript-language-server |           │
│   pyright | gopls | clangd | …                           │
└──────────────────────────────────────────────────────────┘
```

The backend is a **process supervisor + proxy** — it doesn't understand LSP semantics, it just frames messages and routes them.

### Backend supervisor

New module: **`agentmux-srv/src/backend/lsp/`**

| File | Role |
|---|---|
| `mod.rs` | Module facade + types |
| `supervisor.rs` | `LspSupervisor` — owns the `HashMap<ServerKey, ServerHandle>`; spawn/kill semantics |
| `client.rs` | Per-server frame parsing (Content-Length headers + JSON body) |
| `workspace.rs` | Workspace-root detection (git root → file dir → home) |
| `discovery.rs` | Server binary discovery (`which` lookup + settings overrides) |

```rust
struct ServerKey {
    workspace_root: PathBuf,
    language: String,    // "rust" | "typescript" | …
}

struct ServerHandle {
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout_task: tokio::task::JoinHandle<()>,
    refcount: usize,     // open files referencing this server
    next_request_id: AtomicI64,
    pending: Mutex<HashMap<i64, oneshot::Sender<Response>>>,
}
```

### Wire protocol — staying close to LSP

We **don't invent a new RPC for each LSP method**. Three RPCs are enough:

| RPC | Purpose | Direction |
|---|---|---|
| `lspstart({ language, file_path })` → `{ server_id, workspace_root, capabilities }` | Open a connection to (start if needed) the right server for this file | client → backend |
| `lspsend({ server_id, message })` | Forward an arbitrary LSP message to the server | client → backend |
| `lspstop({ server_id, file_path })` | Decrement refcount; backend kills the server when last file closes | client → backend |

Plus one WS event:

| Event | Payload |
|---|---|
| `lsp:message` | `{ server_id, message }` — any server-pushed notification (diagnostics, progress, etc.) |

This keeps the backend dumb (just framing/routing) and the frontend's `lsp-client.ts` thin (LSP semantics live there).

### Frontend

New module: **`frontend/app/view/editor/lsp/`**

| File | Role |
|---|---|
| `lsp-client.ts` | `LspClient` class — manages the WS request/response cycle, surfaces typed methods (completion, hover, definition, etc.) |
| `lsp-extensions.ts` | CM6 extension factories — `lspLint(client)`, `lspCompletion(client)`, `lspHover(client)`, `lspGoToDefinition(client)` |
| `lsp-types.ts` | LSP type definitions (subset) |

`editor-view.tsx` changes:
- After file load, identify language → instantiate `LspClient` for that language → push LSP extensions into the CodeMirror state via `Compartment.reconfigure`
- On file change (open a different file): `client.didChange()` → `client.didClose()` on the previous file → `client.didOpen()` on the new one
- On pane close: `client.shutdown()`

Reference implementation we'll borrow from heavily: [`codemirror-languageserver`](https://github.com/FurqanSoftware/codemirror-languageserver). It expects a WebSocket transport that speaks LSP directly — we'll plug our backend proxy into that contract.

### Languages — initial set + binary lookup

Default discovery: try `which <binary>` (Rust's `which` crate). User overrides via settings.

| Language | Server binary | Discovery hint | Status |
|---|---|---|---|
| Rust | `rust-analyzer` | `rustup component add rust-analyzer` | Tier-1 MVP target |
| TypeScript / JavaScript | `typescript-language-server` | `npm install -g typescript-language-server typescript` | Tier-1 MVP target |
| Python | `pyright` (LSP variant) | `npm install -g pyright` or `pipx install pyright` | Tier-1 MVP target |
| Go | `gopls` | `go install golang.org/x/tools/gopls@latest` | Tier-1 MVP target |
| C / C++ | `clangd` | Distro-packaged | Phase 3 |
| Java | `jdtls` | Heavyweight; later | Phase 4 |
| Lua | `lua-language-server` | Later | Phase 4 |

### Configuration

New settings keys (per [`settings.md`](https://docs.agentmux.ai/settings/) conventions):

```jsonc
{
  // Master switch — if false, no LSP at all
  "editor:lsp.enabled": true,

  // Per-language overrides — null means auto-detect from PATH
  "editor:lsp.rust.command": null,            // override: e.g. "/opt/homebrew/bin/rust-analyzer"
  "editor:lsp.rust.args": [],
  "editor:lsp.typescript.command": null,
  "editor:lsp.typescript.args": [],
  "editor:lsp.python.command": null,
  "editor:lsp.python.args": [],
  "editor:lsp.go.command": null,

  // Workspace-root strategy when opening a file
  "editor:lsp.workspace_root_strategy": "git", // "git" | "file" | "home"

  // Per-capability enables (default: all on)
  "editor:lsp.diagnostics": true,
  "editor:lsp.completion": true,
  "editor:lsp.hover": true,
  "editor:lsp.definition": true
}
```

### Lifecycle

1. **Open file** → frontend detects language → calls `lspstart({ language, file_path })`
2. **Backend** computes workspace_root (git root walk; fall back to file dir; fall back to home) → looks up server binary → if no live server for `(workspace_root, language)`, spawns one → registers the file → returns server_id
3. **Frontend** opens an `LspClient`, sends LSP `initialize` → `initialized` → `textDocument/didOpen`
4. **CM6 extension** attaches diagnostics / completion / hover providers
5. **User edits** → debounced `didChange` (incremental, using LSP's TextDocumentContentChangeEvent)
6. **Server pushes** diagnostics → backend forwards as `lsp:message` event → frontend's LspClient routes to the `lspLint` extension
7. **User opens a different file** in the same pane → `didClose` previous, `didOpen` new
8. **Pane closes / file closes** → `lspstop({ server_id, file_path })` → backend decrements refcount; if zero, after a 60s idle grace shuts down the server (avoids thrashing when the user closes and re-opens a file quickly)
9. **Server crashes** → supervisor detects exit, broadcasts `lsp:status` with kind=`crashed`; frontend retries with exponential backoff (3 attempts, then stops and surfaces an error)
10. **App quits** → supervisor SIGKILLs every child

### Workspace-root detection

Walk up from the file's directory:
1. If any ancestor has `.git/`, that's the workspace root
2. Else if any ancestor has `package.json` / `Cargo.toml` / `go.mod` / `pyproject.toml`, that's the root
3. Else the file's containing directory
4. Override via setting: `editor:lsp.workspace_root_strategy = "file" | "home" | "git"`

### Language server installation — VS Code-style notice, no pre-install

**Decision: AgentMux does NOT bundle language servers.** Users install them via their existing package managers, exactly like VS Code does today. The editor surfaces a friendly notice when a server's missing.

**Why not pre-install?**

| Option | Cost / risk | Verdict |
|---|---|---|
| Pre-install everything | +200–500 MB to the installer (rust-analyzer ~50 MB on its own, × 5 languages × 3 OSs × 2 arches). License variance. Version drift — we'd ship server v2024.5 while the user's project uses v2026.2 features. Some servers (tsserver) prefer project-local installs. | Reject |
| Pre-install one or two (e.g. TS) | Half the bundle pain of "all" but most of the UX inconsistency ("why isn't Rust just there too?"). | Reject |
| VS Code-style notice + manual install | Small install, current versions, respects project-local installs. One manual step on first use per language. | **Accepted** |
| Auto-install in-app (click to install) | One-click UX, but needs package-manager detection, permission prompts, network handling, and a fail-mode UX for each. Worth doing as a polish phase, not the default. | Future |

The first-time-per-language friction is acceptable: VS Code does the same and users are conditioned to it. The notice copy is designed to be copy-paste actionable.

#### The notice

When `lspstart` reports `server_binary_not_found`, the editor pane shows an inline banner above the CodeMirror surface:

```
┌─ /Users/asaf/src/foo.rs ─────────────────────────────────────┐
│ ┌───────────────────────────────────────────────────────────┐ │
│ │ ⓘ Rust language server (rust-analyzer) not installed.    │ │
│ │   Install:  rustup component add rust-analyzer    [Copy] │ │
│ │   [Open install docs ↗]    [Disable for this session]    │ │
│ └───────────────────────────────────────────────────────────┘ │
│  fn main() {                                                  │
│      println!("hello");                                       │
│  }                                                            │
└───────────────────────────────────────────────────────────────┘
```

The banner is dismissible (closes for the session); it returns on the next launch if the binary is still missing. The editor remains fully functional with syntax-highlight-only mode while dismissed. Status chip in the footer stays `dimmed` ("rust: not installed") so the dismissed banner is still discoverable.

#### Install hints per language

A small static table backs the banner's "Install:" line. Source of truth lives in `frontend/app/view/editor/lsp/install-hints.ts`:

| Language | Server binary | Install command (cross-platform default) | Docs link |
|---|---|---|---|
| Rust | `rust-analyzer` | `rustup component add rust-analyzer` | [rust-analyzer.github.io](https://rust-analyzer.github.io/manual.html#installation) |
| TypeScript / JavaScript | `typescript-language-server` | `npm install -g typescript-language-server typescript` | [github.com/typescript-language-server/typescript-language-server](https://github.com/typescript-language-server/typescript-language-server) |
| Python | `pyright` | `npm install -g pyright`  *(or `pipx install pyright`)* | [microsoft.github.io/pyright](https://microsoft.github.io/pyright/) |
| Go | `gopls` | `go install golang.org/x/tools/gopls@latest` | [pkg.go.dev/golang.org/x/tools/gopls](https://pkg.go.dev/golang.org/x/tools/gopls) |
| C / C++ | `clangd` | (distro-packaged: `brew install llvm` / `apt install clangd` / windows: LLVM installer) | [clangd.llvm.org/installation](https://clangd.llvm.org/installation) |

Bonus: many of these have known alternative install paths (`pipx`, `cargo install`, etc.); the table holds the one default + a `details` link rather than a maze of conditional commands. Easier to maintain.

### Auto-install (deferred to a polish phase, not v1)

VS Code itself doesn't auto-install language servers via shell commands; its extensions handle binary download internally. We could match that experience for Tier 1.5 by:

- Adding a backend `lspinstall(language)` RPC that runs the appropriate command (`cargo install`, `npm install`, `go install`) with the user's existing environment
- One-click "Install" button on the missing-server banner
- Progress UI; surface stderr; sane timeout (5 min); error states

The complexity is in the failure UX — what if the user has no `cargo`? What if their `npm` is locked to a corporate proxy? What if they need `sudo`? Better to ship Tier 1 with copy-paste and let community feedback drive whether auto-install pays for its complexity.

### Error handling (other cases)

- **Server crashes:** small status chip in the pane footer; auto-retry 3× with backoff (1s, 5s, 30s) then stop
- **Slow startup (cold rust-analyzer can take 30s indexing):** spinner in the footer "Indexing…"
- **Server unresponsive:** 10s response timeout → drop the in-flight request, log; the next request restarts the supervisor's request cycle
- **Wrong server version (LSP `initialize` reports unsupported protocol):** banner: "rust-analyzer v0.3 is too old. Update: `rustup component add rust-analyzer` again." Same copy-paste pattern.

### Status surfaces

A small **LSP status chip** in the editor pane footer (matches the spec's existing "active file" indicator):

```
┌─ /Users/asaf/repo/src/main.ts * ─── 🟢 ts (rust-analyzer ready) ─┐
│  ... code ...                                                    │
└──────────────────────────────────────────────────────────────────┘
```

Chip states: `gray` not-started · `yellow` starting · `green` ready · `red` crashed · `dimmed` not-installed (clickable for install hint).

---

## Tier 2 — Integrate with AgentMux's existing theme system

### What's already in place

AgentMux ships **9 themes** via SCSS files under `frontend/app/themes/`: `default`, `midnight`, `high-contrast`, `monokai`, `nord`, `dracula`, `catppuccin`, `tokyo-night`, `gruvbox`. Each defines a `[data-theme="<name>"]` block of CSS custom properties — `--main-bg-color`, `--accent-color`, `--border-color`, etc.

Selection lives in the **hamburger menu (≡) → Theme submenu** (defined in `frontend/app/menu/base-menus.ts:THEME_OPTIONS`), persists as `window:theme` in `settings.json`, and applies via `document.documentElement.setAttribute("data-theme", …)` (see `frontend/app/app.tsx:185`). The enum is locked in the schema at `schema/settings.json`.

**The editor currently ignores this.** CodeMirror uses a hardcoded `oneDark` theme, so switching AgentMux's theme leaves the editor stuck. That's the integration gap this section closes.

### Design: editor follows app theme via CSS variables — no separate picker

The right answer is **not** a second theme picker. The hamburger Theme submenu becomes the **single control** for theme; the editor reskins automatically.

Two pieces of work:

1. **Add editor-syntax CSS variables** to each existing theme SCSS. Variables cover the ~15 syntax-token roles that matter: keyword, comment, string, number, function, type, variable, operator, punctuation, tag, attribute, property, regex, builtin, link.
2. **Make CodeMirror read those variables at runtime** and reapply on theme change.

```scss
// frontend/app/themes/dracula.scss — add at the bottom of the [data-theme="dracula"] block
--editor-bg:               #282a36;      // matches --main-bg-color
--editor-fg:               #f8f8f2;
--editor-gutter-bg:        #21222c;
--editor-gutter-fg:        #6272a4;
--editor-line-active-bg:   rgba(255, 255, 255, 0.04);
--editor-selection-bg:     rgba(189, 147, 249, 0.3);
--editor-cursor:           #f8f8f2;

--editor-syntax-keyword:    #ff79c6;
--editor-syntax-comment:    #6272a4;
--editor-syntax-string:     #f1fa8c;
--editor-syntax-number:     #bd93f9;
--editor-syntax-function:   #50fa7b;
--editor-syntax-type:       #8be9fd;
--editor-syntax-variable:   #f8f8f2;
--editor-syntax-operator:   #ff79c6;
--editor-syntax-tag:        #ff79c6;
--editor-syntax-attribute:  #50fa7b;
--editor-syntax-property:   #8be9fd;
--editor-syntax-regex:      #ffb86c;
--editor-syntax-builtin:    #8be9fd;
--editor-syntax-link:       #8be9fd;
```

Defaults in `frontend/app/themes/index.scss` cover any theme that doesn't override (falls back to the same colors `oneDark` ships with today — no regression for incomplete coverage).

### Runtime wiring

New module: **`frontend/app/view/editor/theme/cm-theme-from-css.ts`**

```ts
import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags } from "@lezer/highlight";

function readVar(name: string): string {
    return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function buildEditorTheme() {
    const view = EditorView.theme(
        {
            "&": { backgroundColor: readVar("--editor-bg"), color: readVar("--editor-fg") },
            ".cm-gutters": {
                backgroundColor: readVar("--editor-gutter-bg"),
                color: readVar("--editor-gutter-fg"),
            },
            ".cm-activeLine": { backgroundColor: readVar("--editor-line-active-bg") },
            ".cm-selectionBackground, ::selection": {
                backgroundColor: readVar("--editor-selection-bg"),
            },
            ".cm-cursor": { borderLeftColor: readVar("--editor-cursor") },
        },
        { dark: true },
    );
    const highlights = HighlightStyle.define([
        { tag: tags.keyword,       color: readVar("--editor-syntax-keyword")    },
        { tag: tags.comment,       color: readVar("--editor-syntax-comment"),   fontStyle: "italic" },
        { tag: [tags.string, tags.special(tags.string)], color: readVar("--editor-syntax-string") },
        { tag: tags.number,        color: readVar("--editor-syntax-number")     },
        { tag: tags.function(tags.variableName), color: readVar("--editor-syntax-function") },
        { tag: [tags.typeName, tags.className], color: readVar("--editor-syntax-type") },
        { tag: tags.variableName,  color: readVar("--editor-syntax-variable")   },
        { tag: tags.operator,      color: readVar("--editor-syntax-operator")   },
        { tag: tags.tagName,       color: readVar("--editor-syntax-tag")        },
        { tag: tags.attributeName, color: readVar("--editor-syntax-attribute")  },
        { tag: tags.propertyName,  color: readVar("--editor-syntax-property")   },
        { tag: tags.regexp,        color: readVar("--editor-syntax-regex")      },
        { tag: tags.standard(tags.variableName), color: readVar("--editor-syntax-builtin") },
        { tag: [tags.link, tags.url], color: readVar("--editor-syntax-link"), textDecoration: "underline" },
    ]);
    return [view, syntaxHighlighting(highlights)];
}
```

In `editor-view.tsx`:

```ts
const themeCompartment = new Compartment();
// in extensions array:
themeCompartment.of(buildEditorTheme()),

// on mount, listen for theme changes:
const observer = new MutationObserver(() => {
    cmView?.dispatch({
        effects: themeCompartment.reconfigure(buildEditorTheme()),
    });
});
observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme"],
});
onCleanup(() => observer.disconnect());
```

Switching the hamburger Theme to Dracula fires the MutationObserver → `buildEditorTheme()` reads the new CSS variable values → CodeMirror reconfigures → editor reskins in ~50ms. No flash, no restart.

### What about the tree column?

The tree's SCSS already uses CSS custom properties (`--main-text-color`, `--border-color`, etc.). It'll just work once the editor itself follows the theme — no additional changes there.

### VS Code theme import (still useful, reframed)

The third-party theme story changes from "editor-only picker" to "extend the AgentMux theme system."

**Backend RPC: `importvscodetheme({ path })` → `{ theme_id }`**

The backend:
1. Parses the VS Code theme JSON
2. Maps `colors` (UI tokens) + `tokenColors` (TextMate scopes via `scope-to-tag.ts`) to AgentMux's `--main-*` and `--editor-syntax-*` variables
3. Writes a generated SCSS file to `~/.agentmux/channels/<channel>/themes/<id>.scss` *or* — more lightweight — stores a JSON record and lets the frontend inject `<style>` tags at load
4. Appends to a `themes.json` registry in the channel dir
5. Returns the new theme's id

**Frontend behavior:**

1. On startup, frontend reads the channel themes registry and merges them into `THEME_OPTIONS` (bundled + imported)
2. Hamburger menu's Theme submenu lists everything — imported entries get a `↗` chip
3. Selecting an imported theme writes `window:theme = "imported-<id>"`; the runtime injects the corresponding `<style>` block

**Mapping coverage:**

| VS Code field | AgentMux variable | Notes |
|---|---|---|
| `colors["editor.background"]` | `--editor-bg`, `--main-bg-color` (fallback) | Most important |
| `colors["editor.foreground"]` | `--editor-fg`, `--main-text-color` (fallback) | |
| `colors["sideBar.background"]` | `--secondary-bg-color`, file-tree column bg | |
| `colors["editorLineNumber.foreground"]` | `--editor-gutter-fg` | |
| `colors["editor.selectionBackground"]` | `--editor-selection-bg` | |
| `colors["focusBorder"]` | `--accent-color` | |
| `tokenColors[].scope = "comment"` | `--editor-syntax-comment` | |
| `tokenColors[].scope = "keyword"` | `--editor-syntax-keyword` | |
| …~15 syntax scopes | …matching `--editor-syntax-*` vars | See `scope-to-tag.ts` |

Lossy parts (acceptable for v1, documented):
- VS Code differentiates `string.quoted.double` vs `string.quoted.regexp.javascript`; we collapse to `--editor-syntax-string` + `--editor-syntax-regex`
- Tab colors, panel headers, terminal palette — out of scope for v1 (the existing AgentMux themes don't expose these via CSS vars yet; future work to expand AgentMux's theme tokens)
- Italic/bold `fontStyle` flags on token colors — we honor `italic` only (matches CM6's typical extension surface)

### What changes vs. the original plan

| Original (parallel editor theme system) | Revised (integrate with AgentMux themes) |
|---|---|
| New `editor:theme` setting | Reuses existing `window:theme` |
| New toolbar paint-roller button | Hamburger Theme submenu (already present) |
| 9 NEW bundled CodeMirror themes | Add ~15 `--editor-syntax-*` vars to existing 9 theme SCSS files |
| `editor-themes/` import dir | `channels/<channel>/themes/` (single themes dir for everything) |
| ~30 KB frontend bundle for bundled themes | ~5 KB total (translator + runtime CSS-var reader) |
| Toolbar reskin needed | Tree column already uses CSS vars — no change |
| User confused by two theme pickers | One control — hamburger only |

---

## Costs & performance

Numbers below are field-realistic ranges from running the same servers under VS Code / Helix / Zed on typical developer hardware. Edge cases (huge monorepos, NFS-mounted projects) push the upper bounds further — those are flagged.

### Memory (resident, warm)

| LSP server | Small project | Medium project | Large / monorepo | Notes |
|---|---|---|---|---|
| `typescript-language-server` | 150–250 MB | 300–600 MB | 800 MB – 1.5 GB | Scales with `tsconfig` reach + node_modules size |
| `rust-analyzer` | 250–500 MB | 600 MB – 1.2 GB | **1.5 – 3 GB** | Worst offender; indexes the whole Cargo workspace + transitive deps |
| `pyright` | 150–300 MB | 300–700 MB | 800 MB – 1.2 GB | Type-checking weight ≈ project scale |
| `gopls` | 100–200 MB | 250–500 MB | 600 MB – 1 GB | |
| `clangd` | 80–200 MB | 200–400 MB | 500 MB – 1 GB | Needs `compile_commands.json` to be useful |

**Per-instance bound:** one server per `(workspace_root, language)` pair, regardless of how many panes are open from that workspace. Two panes editing the same Rust crate share one `rust-analyzer`.

**Mitigation:** the 60s idle-grace shutdown (see § Lifecycle) means a momentarily-closed pane doesn't kill the server, but a project the user actually walked away from releases its memory within a minute. A "Stop all LSP servers" action is listed in §Open questions.

### CPU

| Phase | Cost | Duration |
|---|---|---|
| Idle (no edits, no requests) | < 1% one core | Steady |
| Initial indexing | 50–100% one core | 1–5s typical, **up to 60s for big Rust crates** |
| Reindex on file change | 5–30% one core, briefly | A few hundred ms |
| Active request (completion / hover) | spike to 20–60% one core | 5–50 ms |

Servers run on their own threads (separate process), so they don't block the editor renderer. The renderer's perceived cost is just JSON-RPC marshalling — sub-millisecond per request.

### Latency (warm, after server is "ready")

| Operation | Typical | P95 |
|---|---|---|
| Completion list | 5–30 ms | 80 ms |
| Hover docs | 5–30 ms | 70 ms |
| Go-to-definition (same file) | 10–40 ms | 100 ms |
| Go-to-definition (cross-file load) | 50–200 ms | 500 ms (depends on file size) |
| Diagnostics after edit (server-pushed) | 100–500 ms | 1–2 s |

All numbers add the **WS round-trip** through the backend proxy: empirically ~1 ms on loopback. Negligible vs. server processing time. No reason to consider a direct frontend-to-server transport for v1.

### Startup costs

The cold-path when a user opens the first file in a workspace:

| Step | Time | Notes |
|---|---|---|
| Backend `lspstart` RPC | < 5 ms | Lookup binary, compute workspace root |
| `spawn(child_binary)` | 10–50 ms | OS process creation |
| LSP `initialize` request | 200 ms – 2 s | Server-dependent; rust-analyzer is the slow one |
| First indexing pass | **1–60 s** | Depends entirely on project size; user sees progress in the status chip |
| First diagnostic batch | + 100–500 ms after indexing completes | |
| **Total to "first squiggle"** | **~1.5 s small / 5–10 s medium / 30s+ huge Rust** | |

**Subsequent files in the same workspace:** server already running → `didOpen` takes ~20–100ms → diagnostics appear within ~500ms. Per-file cost is well within UI responsiveness budgets.

**Mitigation:** a status chip in the pane footer surfaces the state (`yellow` starting · `green` ready · `red` crashed · `dimmed` not-installed). For long indexing operations, rust-analyzer publishes `$/progress` notifications which we render as "Indexing crates… 12/50".

### Frontend bundle size

All sizes are minified + gzipped (matches Vite production output).

| Item | Δ size | Loading strategy |
|---|---|---|
| `LspClient` + WS bridge | ~15 KB | Main bundle (always loaded) |
| `lsp-extensions.ts` factories | ~10 KB | Main bundle |
| `@codemirror/lint` | ~8 KB | **Lazy** — loaded on first LSP activation |
| `@codemirror/autocomplete` (already in `basicSetup` for some installs) | 0–12 KB | Already included or already deferred |
| LSP type definitions | type-only, ~0 KB runtime | — |
| **Tier 1 net add to main bundle** | **~25 KB** | + ~20 KB lazy on first LSP file |
| `cm-theme-from-css.ts` (CSS-var reader + CM6 reconfigure) | ~3 KB | Main bundle |
| ~15 new `--editor-syntax-*` CSS vars per theme file | ~1 KB per theme uncompressed; <0.5 KB gzipped | SCSS — already loaded for chrome theming |
| MutationObserver on `data-theme` | ~0.5 KB | Main bundle |
| `vscode-theme-loader.ts` (translator, used by import RPC) | ~5 KB | **Lazy** — loaded only when user imports |
| **Tier 2 net add to main bundle** | **~4 KB** | Plus ~1 KB per existing theme file (negligible vs current theme weight) |

**Pre-LSP / pre-theme baseline:** current editor bundle is dominated by CodeMirror + language packs (already lazy). Tier 1 adds ~25 KB to the always-loaded path — small compared to the existing editor weight.

### Backend binary size

| Item | Δ size |
|---|---|
| `LspSupervisor` + `client.rs` + `workspace.rs` + `discovery.rs` | ~500–1000 lines of Rust |
| Binary delta (post-LTO) | ~50–150 KB |
| New crate dependencies (`tokio`-process already present, `which` for binary discovery) | +1 small crate (~10 KB binary delta) |

Negligible against the agentmux-srv binary's existing ~20 MB footprint.

### Disk

| Item | Size |
|---|---|
| LSP server binaries | Not bundled — user installs separately. `rust-analyzer` ~50 MB, `typescript-language-server` is a 50 MB npm package install |
| Bundled themes (raw JSON) | ~150 KB checked in (uncompressed); ~30 KB gzipped after build |
| Per-server cache directories (`.rust-analyzer/`, etc.) | Lives in the user's project, AgentMux doesn't manage it |

### What "the cost" actually feels like

Putting the numbers together for typical use:

| Scenario | Effective cost |
|---|---|
| **Open a small TS file in a fresh tsserver workspace** | ~1s to first squiggle, ~250 MB RAM, ~15ms hover latency |
| **Open a Rust file in a small Cargo project** | ~3s to first diagnostic, ~500 MB RAM, ~20ms hover latency |
| **Open a Rust file in a large monorepo** | 15–60s to first diagnostic (with progress indicator), ~2 GB RAM, ~30ms hover latency |
| **No language server installed** | Banner appears within 50ms, editor remains responsive with highlighting-only |
| **Three editor panes on three languages** | 3 LSP servers running, summed RAM ~1 GB on a typical mix, ~10ms per request |
| **Theme switch (existing hamburger menu)** | < 50 ms — MutationObserver fires, `getComputedStyle` reads new vars, CM6 reconfigures via Compartment; no flash, no double-paint |

### Bounding the cost — recommended defaults

The spec already enforces several limits; documented here for context:

- **Per-workspace single instance**: one server per `(workspace_root, language)` — already in §Lifecycle
- **Idle grace 60s** before server shutdown — avoids thrash during pane-cycle
- **Per-capability disable**: `editor:lsp.completion = false` etc. — if a server's completion is too noisy, disable it without losing diagnostics
- **Master kill switch**: `editor:lsp.enabled = false` shuts down everything; editor returns to highlight-only mode
- **Per-language opt-in default in Phase 1**: only TypeScript starts on first launch; Rust/Python/Go opt-in via settings (until Phase 3 hardens). Avoids surprising users with a 2 GB rust-analyzer process they didn't ask for.

### Comparable footprints

For reference, opening the same file in:

| Editor | RAM | Bundle / install |
|---|---|---|
| **VS Code** | 200–500 MB renderer + LSP servers (so same as us above) | ~350 MB install |
| **Cursor** | Similar (forks VS Code) | ~400 MB |
| **Zed** | 100–300 MB editor + LSP servers (built native) | ~40 MB |
| **Helix** | 40–80 MB editor + LSP servers | ~10 MB binary |
| **AgentMux (post-Tier 1+2)** | CEF renderer (~150 MB baseline) + LSP servers | Net +30 KB frontend, +100 KB backend |

The LSP-server cost dominates everywhere; AgentMux's additional overhead from these tiers is small.

---

## Implementation phases

### Phase 1 — LSP scaffolding + TypeScript MVP (2 weeks)

- Backend `LspSupervisor` + `lspstart` / `lspsend` / `lspstop` RPCs + `lsp:message` event
- Frontend `LspClient` (WS bridge)
- One language: TypeScript (most universally available; tsserver-language-server is one `npm install -g` away)
- One capability: **diagnostics** (publishDiagnostics → `@codemirror/lint` markers)
- Server discovery (`which typescript-language-server`)
- Status chip (gray/yellow/green/red)
- Acceptance: open a `.ts` file with a type error → red squiggle appears within 2s

### Phase 2 — Capability rollout (1 week)

- Completion (`textDocument/completion` → `@codemirror/autocomplete` provider)
- Hover (`textDocument/hover` → CM6 hover tooltip)
- Go-to-definition (`textDocument/definition` → open target file in same pane, scroll to range)
- Settings: per-capability enables

### Phase 3 — More languages (1 week)

- Add rust-analyzer, pyright, gopls
- Per-language config (overrides + args)
- Workspace-root detection: git → manifest → file dir
- Acceptance: opening a `.rs` file in a Cargo workspace gets diagnostics from rust-analyzer

### Phase 4 — Editor follows AgentMux theme system (3–5 days)

- Add ~15 `--editor-syntax-*` CSS variables to each of the 9 existing theme files (`frontend/app/themes/*.scss`); add baseline defaults in `index.scss`
- `cm-theme-from-css.ts` — reads vars via `getComputedStyle`, builds CM6 `EditorView.theme` + `HighlightStyle`, returns extensions
- `editor-view.tsx` mounts theme via a `Compartment`, subscribes to `data-theme` attribute changes via `MutationObserver`, reconfigures on change
- Tree column already uses chrome CSS vars — no change needed
- Result: hamburger Theme submenu controls everything (chrome + editor); editor reskins live; no editor-specific picker added

### Phase 5 — VS Code theme import (3–5 days)

- `vscode-theme-loader.ts` — parses VS Code JSON, maps scopes/colors to AgentMux's `--main-*` and `--editor-syntax-*` variables
- `importvscodetheme({ path })` RPC — stores the imported theme under `channels/<channel>/themes/`, adds to a `themes.json` registry
- Frontend startup merges imported themes into `THEME_OPTIONS` so they appear in the hamburger Theme submenu (with a `↗` chip distinguishing them)
- Lossy mapping documented (regex sub-scopes, italic fonts, terminal palette)

### Phase 6 — Polish & long-tail (open-ended)

- Find references, document formatting, code actions, signature help
- Refresh themes from `~/.agentmux/channels/<channel>/editor-themes/`
- Outline view (LSP `textDocument/documentSymbol` → sibling tab to file tree)
- LSP-aware sidebar status (per-language: ready, indexing, count of open files)
- C/C++ via clangd

---

## Open questions

1. **Idle grace before LSP server shutdown.** 60s feels right for thrash protection; user-configurable? Default 60s, setting if anyone asks.
2. **Workspace root for files outside any project** (browsing system folders). Fall back to file's parent dir — but some servers (rust-analyzer) refuse to operate without `Cargo.toml`. Recommend: don't start the server for those files, mark the chip "no project root".
3. **Multi-instance LSP servers** (e.g. two AgentMux instances open the same project). Each instance spawns its own server (simple). Could share via a single supervisor per channel later; not worth complexity for v1.
4. **Memory pressure.** rust-analyzer alone can hit 2GB on big crates. Recommend documenting the cost; the per-workspace lifecycle (shut down on last file close + 60s idle) helps. A "shut down all LSP servers" command might be useful.
5. **Scope coverage for VS Code theme translation.** ~50 TextMate scopes covers most themes; edge cases (regex sub-scopes, language-specific overrides) collapse silently. Acceptable for v1; document the limitation; build a "scope coverage" debug tool later if needed.

**Decided** (locked into the spec body):

- ✅ **No pre-installed language servers.** Users install via their package manager. The editor surfaces a copy-paste banner when a server's missing.
- ✅ **Theme picker placement: hamburger Theme submenu.** No editor-specific picker; the single hamburger menu controls both chrome and editor.
- ✅ **Auto-install is deferred** to a future polish phase, not v1.

---

## Acceptance criteria

### Phase 1 (LSP — TypeScript diagnostics MVP)

- [ ] Open a `.ts` file with a known type error → red squiggle visible within 2s
- [ ] Edit the file to fix the error → squiggle clears
- [ ] Close the pane → `typescript-language-server` process exits within 60s
- [ ] Open the same file in two panes → only one server instance
- [ ] Settings: `editor:lsp.enabled = false` disables it entirely
- [ ] Banner shown when `typescript-language-server` not in PATH: includes install command, Copy button, docs link, "Disable for this session" button
- [ ] Banner is dismissible per-session; reappears on next launch if server still missing
- [ ] Editor remains fully functional with syntax-highlight-only mode while banner dismissed
- [ ] Status chip reflects state (gray → yellow → green → red)

### Phase 4 (editor follows AgentMux theme)

- [ ] Switching theme from the hamburger menu reskins the editor (background, foreground, gutter, line numbers, syntax colors) within ~50 ms
- [ ] No editor-specific theme picker is added — the hamburger menu remains the single control
- [ ] Each of the 9 existing themes defines all `--editor-syntax-*` vars (no fallback to default colors for keyword/comment/string/etc.)
- [ ] Tree column visually inherits the active theme (already does — confirm no regression)

### Phase 5 (VS Code theme import)

- [ ] `importvscodetheme` RPC reads a VS Code theme JSON path → stores under `channels/<channel>/themes/` → returns id
- [ ] Imported themes appear in the hamburger Theme submenu with a `↗` indicator
- [ ] Selecting an imported theme writes `window:theme = "imported-<id>"` and applies live
- [ ] Removing a `themes/*.scss` file → theme disappears from menu on next restart

---

## References

- [Language Server Protocol spec (3.17)](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/)
- [`codemirror-languageserver`](https://github.com/FurqanSoftware/codemirror-languageserver) — reference implementation of CM6 ↔ LSP
- [`@codemirror/lint`](https://codemirror.net/docs/ref/#lint) — diagnostic markers
- [`@codemirror/autocomplete`](https://codemirror.net/docs/ref/#autocomplete) — completion infrastructure
- [VS Code Theme Color Reference](https://code.visualstudio.com/api/references/theme-color)
- [VS Code Syntax Highlighting Guide](https://code.visualstudio.com/api/language-extensions/syntax-highlight-guide) — scope details
- [`@lezer/highlight`](https://lezer.codemirror.net/docs/ref/#highlight) — CM6's tag-based highlighting
- Existing code: `frontend/app/view/editor/{editor.tsx, editor-model.ts, editor-view.tsx}`, `agentmux-srv/src/server/websocket.rs` (RPC handler patterns)
- Prior art: [Zed's editor architecture](https://zed.dev/blog/zed-decoded-rope-sumtree) (different stack, similar design choices on LSP integration)
- Prior art: Cursor / Windsurf — both fork VS Code rather than reimplement; reinforces the recommendation to not chase full extension-host compat
