# Find: in-editor & app-wide

**Date:** 2026-06-17
**Status:** Draft
**Author:** naki
**Components:** `frontend/app/view/editor/`, `frontend/app/store/keymodel.ts`, `frontend/app/element/search.tsx`, `agentmux-srv` (new search endpoint)

---

## 1. Why Ctrl+F doesn't work in the editor today

It's not a missing engine — it's a routing collision.

- **AgentMux has a universal block-search.** `keymodel.ts:626` binds `Cmd:f` app-wide:
  ```ts
  globalKeyMap.set("Cmd:f", activateSearch);
  ```
  `activateSearch` (keymodel.ts:604) opens the focused block's `viewModel.searchAtoms` overlay, rendered by `frontend/app/element/search.tsx`. The **terminal** block implements this (`termViewModel.ts`), so Ctrl+F works there.
- **The editor never adopted it.** `EditorViewModel` exposes `viewType = "editor"` but **no `searchAtoms`** (and no `keyDownHandler`), so `activateSearch` returns `false` for an editor pane → nothing opens.
- **CodeMirror's own search is present but unreached.** `editor-view.tsx` adds `search()` from `@codemirror/search`, and `basicSetup` bundles `searchKeymap` (Mod-f → `openSearchPanel`). But the document-level `Cmd:f` binding claims the keystroke for the universal path, so CM's panel doesn't open for the editor.

**Net:** Ctrl+F is captured by the app's block-search, which the editor hasn't wired up. The fix is to give the editor *a* find path (§3).

---

## 2. What VS Code uses (for reference)

| Scope | VS Code engine |
|---|---|
| **In-editor find** (Ctrl+F) | **Monaco**'s custom find widget — `TextModel.findMatches`, JS `RegExp`, with case / whole-word / regex / in-selection toggles. Not portable (Monaco ≠ CodeMirror). |
| **Workspace find** (Ctrl+Shift+F) | **ripgrep** (`@vscode/ripgrep`) behind a search service; results stream into the Search view tree. |

Takeaway: VS Code does **not** use one engine for both. In-editor find is editor-native; workspace find is ripgrep. We should mirror that split.

**We don't need Monaco.** CodeMirror's `@codemirror/search` already ships the in-editor engine we'd want: `SearchCursor` + `RegExpCursor`, with case-sensitive, whole-word, and regex support, plus find/replace and "find in selection". It's already a dependency and already in the editor's extension set.

---

## 3. In-editor Find (Ctrl+F) — options

### Option A — CodeMirror's native search panel (recommended, low effort)

Route Ctrl+F to CM's `openSearchPanel(view)` for editor panes. Two sub-approaches:

- **A1 (cleanest):** give `EditorViewModel` a `keyDownHandler` (the universal dispatch already calls `viewModel.keyDownHandler` — keymodel.ts:412) that, on Cmd/Ctrl+F, calls `openSearchPanel` on the active `EditorView` and returns `true`. No change to the global binding; the editor just claims the key when focused.
- **A2:** make `activateSearch` fall through (return `false` without consuming) for `viewType === "editor"` so CM's bundled `searchKeymap` fires.

Gives find/replace, regex, case, whole-word out of the box. **Effort: ~½ day.** Downside: the panel's look differs from the app's `search.tsx` overlay (visual inconsistency with terminal find).

### Option B — Adopt the universal `searchAtoms` overlay (consistent UI, more work)

`EditorViewModel` exposes `searchAtoms` like `termViewModel`; `search.tsx` becomes the editor's find bar too, driving CM under the hood via `setSearchQuery` / `findNext` / `findPrevious` / match highlighting. One consistent find UI across terminal + editor (+ future blocks). **Effort: ~2 days** (wire overlay state ↔ CM search commands, match count, current-match scroll).

### Recommendation

Ship **A1** now (makes Ctrl+F work with a full feature set immediately), then optionally converge on **B** later if we want one unified find bar across all block types. A1 doesn't block B — B can replace the panel wiring later.

---

## 4. App-wide Find — "Find Anything" in the title bar

A single, prominent search that spans **everything currently open in the app**, surfaced in the **title bar** (centered, VS Code-style — the same place VS Code puts its command/search box). Distinct from the editor's in-file Ctrl+F.

### Scope (per the product direction)

Not a filesystem grep first — a search over the app's **live surfaces**, extensible by provider:

1. **Open editor items** — the text of every open editor tab/buffer (in-memory, *including unsaved edits*).
2. **Agent panes** — the conversation content of agent panes (user/assistant messages, tool output).
3. **More over time** — terminals/scrollback, the file tree / workspace files (ripgrep — see below), settings, command palette, etc.

### Architecture: a provider registry

A small frontend contract any surface can implement:
```ts
interface FindProvider {
  id: string;              // "editors", "agents", …
  label: string;           // group heading in results
  search(query: string, opts): Promise<FindHit[]>;
}
interface FindHit {
  providerId: string;
  title: string;           // e.g. file name / agent name
  snippet: string;         // matched line with highlight offsets
  navigate(): void;        // focus + scroll to the hit
}
```
- The title-bar Find calls every registered provider (debounced), merges + ranks, and renders results **grouped by provider**. Keyboard-navigable; Enter / click runs `hit.navigate()`.
- **Initial providers are frontend-only — no backend needed.** Open-editor text already lives in the editor model (`_contentByTab`, including unsaved edits); agent-pane content lives in agent-pane state. Both can be searched in memory.
- **Navigation closes the loop per surface:** an editor hit opens/focuses that tab and jumps to the line (reuse CM `setSearchQuery` + scroll); an agent-pane hit focuses that pane and scrolls to the message.

### Future provider: workspace files (ripgrep)

When we want to search files *not* open, add a "workspace files" provider backed by ripgrep's Rust crates **in `agentmux-srv`** — `grep-searcher`/`grep-regex` + `ignore` (gitignore-aware walk). AgentMux does **not** bundle the `rg` binary (verified — only `agentmux-bashwrap`/`agentmux-mcp` are in `runtime/tools/bin`), so in-process crates avoid shipping/locating an external binary. New auth-gated endpoint, streamed over the WPS bus, with `maxResults` caps. This is just another `FindProvider` behind the same title-bar UI — additive, not a separate feature.

### Surface & keybinding

- **Title-bar search affordance** (centered), opened by **Ctrl/Cmd+Shift+F** (mirror VS Code) or click. A dropdown/overlay renders grouped results beneath it.
- Filters (case / whole-word / regex) as toggles; provider-scoping chips ("Editors", "Agents", …) so you can narrow.
- Reuse `frontend/app/element/search.tsx` chrome where it fits.

**Effort: ~2–3 days** for the title-bar UI + provider registry + the two in-memory providers (editors, agents). The ripgrep file provider is a later, additive ~2–3 days.

---

## 5. How app-wide find and editor find interplay

Two **complementary scopes**, like VS Code's Find vs Find-in-Files — not competitors:

| | Shortcut | Scope | Engine |
|---|---|---|---|
| Editor find | Ctrl+F | current file | CodeMirror `@codemirror/search` |
| Find Anything | Ctrl+Shift+F | all open surfaces (editors, agents, …) via providers | in-memory providers + (future) ripgrep |

**Connections:**
1. **Hit → navigate + seed in-file find.** An editor hit from Find Anything opens/focuses that tab and seeds the editor's in-file find with the same query (CM `setSearchQuery` + scroll), so "find across everything → jump in → keep finding in this file" flows continuously.
2. **Shared query state.** The last query/flags carry between the title-bar Find and the editor's Ctrl+F.
3. **The editor is just one provider.** The in-file Ctrl+F (§3) and the "open editors" provider both read the same editor buffers — one is scoped to the focused file, the other spans all open files. No duplicated engine: both lean on CodeMirror's text/`SearchCursor`.

---

## 6. Recommended sequence

1. **Fix Ctrl+F (Option A1)** — editor `keyDownHandler` → `openSearchPanel`. Small, immediate, full-featured in-file find. *(½ day)* — **shipped.**
2. **Find Anything shell** — title-bar search affordance + overlay + `FindProvider` registry + Ctrl+Shift+F. *(1–1.5 days)*
3. **In-memory providers** — open-editor buffers + agent-pane content, with navigate-to-hit. *(1–1.5 days)*
4. **Interplay** — editor hit seeds the in-file find; shared query state. *(½ day)*
5. **(Future) Workspace-files provider** — srv ripgrep endpoint (`ignore`+`grep`), streamed, behind the same UI. *(2–3 days)*
6. **(Optional) Unify in-editor find** — adopt `searchAtoms` for the editor (Option B) so the in-file bar matches the rest of the app. *(2 days)*

Phase 1 resolves the reported Ctrl+F bug; phases 2–4 deliver the title-bar "Find Anything" across open editors + agent panes, built on a provider model so future sources (files, terminals, …) drop in without reworking the UI.

---

## 7. Key references

| What | Location |
|---|---|
| Global `Cmd:f` → universal search | `frontend/app/store/keymodel.ts:626`, `activateSearch` @604 |
| Universal search overlay | `frontend/app/element/search.tsx`; impl example `frontend/app/view/term/termViewModel.ts` |
| Per-view key dispatch hook | `frontend/app/store/keymodel.ts:412` (`viewModel.keyDownHandler`) |
| Editor CM setup (`search()`, `basicSetup`) | `frontend/app/view/editor/editor-view.tsx:315-317` |
| Editor view model (no `searchAtoms` today) | `frontend/app/view/editor/editor-model.ts` |
| ripgrep crates | `ignore`, `grep-searcher`, `grep-regex` (crates.io) |
