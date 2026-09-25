# Spec: Armory Personal Memory — Content View

**Status:** implemented — #3218 (current content shown in the Personal Memory history panel). §6's "no editing in the Armory" non-goal is REVERSED by `SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md` §2.4 (approved by the repo owner): the Personal Memory full view is editable, with the content moved below the history.
**Date:** 2026-09-11
**Verified against:** `e9d1d1b4d` (code, not spec prose)
**Follows:** the Global Memory markdown-preview/resizable fix (PR #3199, merged
`64e42dd8b`) — same underlying request, applied to the other Memory sub-tab.
**Related:** `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md` (scoped
the current Armory surface to history/diff/revert), PR #2678 (introduced that
surface), PR #3000 (file picker → tile grid), `SPEC_FIX_PERSONAL_MEMORY_EMPTY_
WORKDIR_2026_09_01.md` / `SPEC_MEMORY_RPC_HANDLERS_BLANK_WORKDIR_2026_09_02.md`
(prior, unrelated, already-fixed bugs in this same RPC family).

## 1. The request

"Armory → Memory → Global" now renders its content as real markdown in a
resizable panel (PR #3199). The same is wanted for "Armory → Memory →
Personal" — today, selecting a file there shows no content at all.

## 2. Current state, verified end-to-end

The component chain, found via the same Armory routing Global Memory uses
(`armory-view.tsx`'s `MEMORY_SUBNAV`):

`armory-view.tsx` (`"personal"` subsection) → `NativeMemoryManager`
(`frontend/app/view/native-memory/native-memory-manager.tsx`) → agent card
grid (`MemoryAgentCard.tsx`) → per-file tile grid (`MemoryFileCard.tsx`) → on
file select → `NativeMemoryHistoryPanel`
(`frontend/app/view/agent/components/NativeMemoryHistoryPanel.tsx`, shared
with the Stash modal), backed by `NativeMemoryHistoryModel`
(`frontend/app/view/agent/native-memory-history-model.ts`).

**`NativeMemoryHistoryPanel` renders exactly two things, and neither is a
file's current content:**

- A version list — each row shows only a source tag and a timestamp
  (`sourceLabel(v.source)` + `formatTimestamp(v.created_at)`), fed by
  `RpcApi.NativeMemoryHistoryCommand` (`agent:memory:history`), whose own
  response type is explicitly documented as metadata **"without its full
  content"** (`gotypes.d.ts` `NativeMemoryVersionMeta`).
- A diff (`<pre class="native-memory-history-diff-body">`, plain text, no
  markdown, no resize) — but only once exactly two versions are checkbox-
  selected.

Grepped exhaustively: `native-memory-manager.tsx`,
`NativeMemoryHistoryPanel.tsx`, and `native-memory-history-model.ts` contain
**zero** occurrences of a content signal, a `<textarea>`, a raw-content
`<pre>`, or `Markdown`. There is no code path in this surface that ever
calls `agent:memory:read_file` — the one RPC that returns a file's actual
current content — on the normal view path. (It's called once, deep inside
`revertTo()`, but only used if the caller supplies an `onContentReverted`
callback; `NativeMemoryManager`'s mount doesn't supply one, so even that path
is a dead end here.)

## 3. Root cause: not a bug, a missing surface

This is not a fetch-but-fail-to-render defect — there's no fetch to begin
with. It is scoped-out, not broken:

- The introducing commit (#2678) describes the design as Stash getting *"a
  'History' toggle next to the existing content view"* and Armory getting
  *"an agent picker in front of the **same** [history] panel"* — a content
  view was never part of this surface's scope.
- `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md` §4.3/G3 scopes
  the Armory surface to "view history, diff two versions, revert" — never
  "view current content."
- The most recent related commit (#3000) describes the intended drill-down
  as *"agents grid → file tiles → version history"* — content was never the
  destination.

So the reported symptom — "doesn't show any text" — reproduces for every
agent and every file, including ones with real, non-empty content, because
no path renders a file's plain content at all. It is not an empty-state
message misfiring; there is no content-showing state to misfire in the
first place.

**The backend and the RPC are already correct and already used
successfully elsewhere** — ruled out as the cause. `agent:memory:read_file`
already exists, already resolves the agent and merges live-FS with the
`db_agent_native_memory` mirror correctly (guarded against the earlier,
unrelated, already-fixed blank-`working_directory` bug), and is already
wired end-to-end and working in the **other** surface that reads native
memory: the Agent-pane Stash "Memory" tab
(`AgentNativeMemoryModal.tsx` + `AgentNativeMemoryModel`,
`frontend/app/view/agent/agent-native-memory-model.ts`). That model's
`contentAtom` (`:157-181`) is the reference implementation this spec reuses.

## 4. What Global Memory's fix reused, and what's different here

Global Memory's fix (PR #3199) swapped an existing `<pre>{text}</pre>` dump
for `<Markdown text={...} scrollable contentClass="..." />` wrapped in a
`resize: vertical` sized div, with a `.content.<name>-markdown-content {
overflow: auto }` compound-selector CSS override (the established pattern,
also used at `.content.editor-preview-markdown-content` in
`editor-view.scss`) — because `.markdown .content`'s base rule
(`markdown.scss`) is `overflow: scroll` (always-on scrollbars) and `height:
100%; overflow: hidden` on `.markdown`'s own root, which fights a
resize/height override applied directly to it.

That rendering mechanism (`Markdown` + the wrapper-div + compound-selector
override) is reusable here unchanged. What's different:

- **There is no content view to swap — this is new surface area**, not a
  rendering-technology swap.
- **The reference content-fetch implementation already exists**
  (`AgentNativeMemoryModel.selectFile()` → `RpcApi.NativeMemoryReadFileCommand`
  → `contentAtom`) and should be reused/mirrored, not reinvented.
- **Personal Memory is editable elsewhere** (Stash's `AgentNativeMemoryModal`
  already has a plain-`<textarea>` edit mode). Whether Armory's Personal
  Memory view should also become editable is a separate, bigger question
  this spec does NOT take on — see §6. Global Memory's own fix was scoped
  the same way: it touched only the already-read-only preview surfaces, not
  its editable section textareas (which were already resizable, untouched).
  This spec mirrors that scope exactly: add a **read-only**, markdown-
  rendered, resizable content view to Armory's Personal Memory surface;
  editing stays exactly where it already lives (Stash), unchanged.
- **Layout differs.** Global Memory's boxes sit inline in a long scrolling
  page of sections, where a fixed-height `resize: vertical` box (240px
  default, 120–600px drag range) reads naturally. Armory's Personal Memory
  file-detail view is a full-height flex pane (closer to Stash's modal
  layout). A fixed resizable box still works here (matches what was asked
  for literally, and gives the user direct control over how much of the
  pane the content view takes relative to the version list below it) — this
  spec keeps the same `resize: vertical` box pattern rather than a flex-fill,
  for consistency with Global Memory and because the version list below it
  still needs its own visible space regardless of content length.

## 5. Design

Add a new "Current content" section to `NativeMemoryHistoryPanel`, above the
existing version list, always visible (not behind a toggle — unlike Global
Memory's secondary "Combined preview," a file's current content is this
panel's primary reason to exist, not a convenience add-on).

**Data:** `NativeMemoryHistoryModel` gains a `contentAtom` + a
`loadContent()` call mirroring `AgentNativeMemoryModel`'s existing
implementation exactly (same RPC, `agent:memory:read_file`, same
agent_id/filename params already available in this model's own state) —
fetched once on mount/file-select, alongside the existing `loadHistory()`
call, not gated behind any user action.

**Render:**
```tsx
<div class="native-memory-content-section">
    <div class="native-memory-content-label">Current content</div>
    <Show when={model.contentAtom() !== null} fallback={<p class="native-memory-content-empty">Empty.</p>}>
        <div class="native-memory-content-preview">
            <Markdown
                text={model.contentAtom() ?? ""}
                scrollable={true}
                nativeScrollbar={true}
                contentClass="native-memory-content-markdown-content"
            />
        </div>
    </Show>
</div>
```

**CSS** (`NativeMemoryHistoryPanel`'s own stylesheet — find/confirm the exact
file at implementation time, likely co-located with the component):
```scss
.native-memory-content-preview {
    height: 240px;
    min-height: 120px;
    max-height: 600px;
    overflow: hidden;
    resize: vertical;
    padding: var(--space-2);
    background: var(--main-bg-color);
    border: 1px solid var(--border-color);
    font-size: 11px;
}
.content.native-memory-content-markdown-content {
    overflow: auto;
}
```
Identical shape to `global-bundle.scss`'s two new rules from PR #3199 — same
sizing, same override reasoning, same class-naming convention
(`<surface>-content-markdown-content`).

**Error/loading states:** mirror `NativeMemoryHistoryPanel`'s existing error
banner pattern (`:62-64`) for a content-fetch failure, distinct from the
existing history-fetch error so a user can tell which one failed. A `null`
initial `contentAtom` (loading) vs. an empty-string result (genuinely empty
file) must render differently — reuse `AgentNativeMemoryModel.contentAtom`'s
own `string | null` convention for this distinction, don't invent a new one.

## 6. Non-goals

- **Editing Personal Memory content from Armory.** Out of scope — mirrors
  Global Memory's own fix, which only touched read-only surfaces. Editing
  continues to happen exclusively via the Agent-pane Stash "Memory" tab.
  Revisit as its own spec if wanted later; it's a genuinely bigger design
  question (a source/preview toggle would be needed, since rendered markdown
  isn't directly editable — `editor-view.tsx`'s `editorMode()` "source" /
  "preview" / "split" pattern is the closest existing precedent in this
  codebase if that work is ever taken on).
- **Changing what `NativeMemoryHistoryPanel` shares with Stash.** This spec
  adds a section to the shared component; Stash's own modal already shows
  content via a separate implementation (`AgentNativeMemoryModal`'s own
  `<pre>`) and is unaffected either way — not consolidating the two
  reference implementations into one is a deliberate non-goal here, to keep
  this change small and low-risk.
- **Version-specific content view** (viewing an OLDER version's full content,
  not just its diff-against-another-version). The diff view already exists
  for comparing two versions; a "show me exactly what version N said in
  full" view is a different, separately-scoped feature.
