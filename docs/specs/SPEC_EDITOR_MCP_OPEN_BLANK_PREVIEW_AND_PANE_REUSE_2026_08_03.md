# Plan: MCP-opened markdown blank-preview investigation + Editor-pane reuse

**Status:** Proposed
**Author:** AgentY
**Date:** 2026-08-03
**Related:** `docs/analysis/ANALYSIS_EDITOR_MD_BUGS_2026_06_20.md` (the original
"Bug 3" analysis this plan's Part 1 re-investigates), `agentmux-srv/src/server/app_api/mod.rs`
(`find_agent_block`, `resolve_tab_id` — the exact precedent Part 2's design
mirrors), `agentmux-srv/src/backend/editor_file_watcher.rs` (the WPS
push-event pattern Part 2 reuses), `docs/specs/SPEC_MEDIA_PANE_V4_MCP_OPEN_TOOL_2026_08_03.md`
(sibling MCP-tool spec from the same session, unrelated feature, same
general area of the codebase).

## Trigger

User report: opening a `.md` file via the `OpenEditor` MCP tool (agent-driven)
renders the Editor pane's markdown preview **blank** — clicking Source then
back to Preview is required before the real content appears. User noted "we
saw a similar issue before."

This plan has two independent parts, requested together: **Part 1** inspects
the blank-preview bug and lays out a remediation plan. **Part 2** is a new,
separately-requested feature — reuse an already-open Editor pane in the
agent's own tab for subsequent `OpenEditor` calls, instead of always opening
a new pane.

---

# Part 1 — Blank markdown preview on MCP-opened files

## What's already known (verified, not assumed)

An existing analysis, `docs/analysis/ANALYSIS_EDITOR_MD_BUGS_2026_06_20.md`
("Bug 3"), described this exact symptom on 2026-06-20 and was marked
"Analysis only — no code changed" at the time. A background investigation
for this plan found that its root cause **was in fact fixed**, in commit
`5c6bc11af` (2026-06-20, same day) — `containerRef` was converted from a
plain variable to `createSignal<HTMLDivElement>()` (`editor-view.tsx:85`),
and the re-seeding `createEffect`'s guard (`editor-view.tsx:463`,
`if (!activeId || loading || !containerRef()) return;`) now tracks that
signal directly. This makes the effect's eventual re-run **independent of
tick-ordering** against the `<Show>` that mounts the container: once the
effect has read `containerRef()` even via an early-return path, it's
subscribed to that signal, so it is guaranteed to re-fire when
`setContainerRef` is later called — regardless of whether that happens in
the same reactive flush or a later one.

`git log --since=2026-06-20 -- frontend/app/view/editor/editor-view.tsx`
shows no later commit touches these specific lines (only unrelated JSX/markup
changes). Tracing Solid's execution model by hand (render effects — the
`<Show>`'s reconciliation and ref callbacks — flush before deferred
`createEffect`s), this fix looks structurally correct for **both** the
original repro shape (switching files within an already-mounted, long-lived
Editor pane) **and** a brand-new pane mounting for the first time (the MCP
`OpenEditor` case) — `onMount`'s own `setLiveDoc("")` seed
(`editor-view.tsx:387-403`) was also checked and confirmed harmless: it
calls `setupEditor("", ...)`, which bails synchronously on the not-yet-
mounted container before any `await`, so there's no dangling async
resumption that could later stomp correctly-loaded content.

**Static analysis could not find a code-level cause for the reported
symptom.** This is stated plainly rather than papered over with a guessed
fix — the June 20 race looks closed, and no other code path reading
`liveDoc`/`containerRef`/`loadingAtom` was found to regress it.

### A confirmed, separate, and orthogonal finding: dead `editor:source_hidden` meta

`agentmux-srv/src/server/app_api/pane.rs:228-232`'s `build_pane_meta` still
writes `editor:source_hidden: true` to block meta for `.md` files opened via
`OpenEditor`. The frontend **used to** read this (`sourceHiddenAtom`, added
in PR `bd573ebe2`, 2026-06-23) but PR `3a2a5ebd0` (2026-07-01, "restore
preview/source toggle, remove split-screen default") **deleted that
meta-read**, replacing it with the in-memory-only `editorMode()`/`_tabModes`
map (`editor-model.ts:1059-1066`), whose fallback
(`activeTabAtom()?.language === "markdown" ? "preview" : "source"`) ignores
meta entirely. `editor-model.ts:1026` (added later still, PR `42fa0e04d`,
2026-07-12, "Open to the Side") writes the *same already-dead key* again —
nobody noticed it had gone dark 11 days earlier.

This is real dead code (backend writes a key nothing reads) but **confirmed
not the cause** of the blank-preview symptom, since `editorMode()`'s own
fallback already defaults `.md` to `"preview"` independent of any meta.
Worth cleaning up regardless — tracked as a small independent action item
below, not blocking Part 1's main investigation.

## Plan — live repro before any fix, not a guess

Given static analysis is inconclusive, the next step is an **instrumented
live repro**, not a speculative patch. Concretely:

1. **Add temporary diagnostic logging** (removed before merge) at the two
   sites the investigation flagged as the crux of the timing question:
   - `editor-view.tsx:396-403` (the `onMount` seed path) — log
     `contentAtom()`/`filePathAtom()`/`loadingAtom()` values at the moment
     this runs.
   - `editor-view.tsx:457-463` (the re-seeding `createEffect`) — log every
     entry, whether it early-returns and why (`!activeId` / `loading` /
     `!containerRef()`), and the values of `contentAtom()` whenever it
     proceeds to call `setLiveDoc`.
2. **Reproduce specifically via the MCP path**, not a normal file-tree click
   — call `OpenEditor` (or the not-yet-existing `OpenMedia`'s sibling
   pattern) against a `.md` file from an agent pane in a `task dev` build,
   with `muxlog fe` tailing, to capture the actual Solid flush ordering for
   a **freshly docked-split pane created via the backend `pane/open` →
   `CreateBlock` → layout-insert path** — this is the one variable that
   couldn't be verified by reading source alone: whether a docked-split
   pane's frontend component mount timing (relative to when its
   `RpcApi`/content-load subscription actually wires up) diverges from the
   already-mounted-pane tab-switch case the June 20 fix was reasoned about.
3. **Candidate hypotheses to check against the log output**, in the order
   they'd first show up in the trace:
   - **(a) Regression not caught by git-blame-on-two-lines.** Some other
     commit changed adjacent behavior (e.g. `loadingAtom`'s own definition,
     `_loadFileIntoTab`'s dispatch timing) in a way that still satisfies "no
     line in `editor-view.tsx:85` or `:457-463` changed" but shifts *when*
     `loadingAtom` flips relative to the container mounting. Check
     `editor-model.ts`'s `loadingAtom`/`contentAtom`/`_loadFileIntoTab`
     history the same way `editor-view.tsx` was checked.
   - **(b) Fresh-docked-pane-specific timing gap.** The MCP path goes
     through `open_pane`'s docked-block-creation branch
     (`agentmux-srv/src/server/app_api/mod.rs:129-173`) with `focus: true`
     — worth checking whether the frontend's block-creation event handling
     (reacting to the `BlockCreated` WS event, mounting a fresh `EditorView`
     component instance) has any gap between "block exists in the layout
     tree" and "this specific `EditorViewModel` instance's own RPC-driven
     content load has actually started" that a same-pane tab-switch never
     hits (since in that case the `EditorViewModel` instance and its content
     dispatch machinery already exist before the new tab is even added).
   - **(c) Reveal/staging jank, not a permanent blank state.** Ruled out as
     the primary cause on inspection — `docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md`
     describes a *related-sounding but distinct* general staged-paint-jank
     issue (Status: Proposed, not implemented, and explicitly scoped to
     transient multi-frame flicker during tab switches, not a
     `<Markdown>` component staying empty until manual interaction) — kept
     here only as a reference in case the live trace reveals something in
     that family, not because it's expected to be the cause.
4. **Fix scope, once the trace confirms a cause**: this plan intentionally
   does not pre-commit to a patch shape, since guessing one now (before the
   live trace) risks repeating the "Analysis only" outcome of the original
   June 20 doc — a plausible-looking fix that doesn't match the actual
   runtime behavior. Whatever the trace shows, the fix should follow the
   same shape as the June 20 fix: make the re-seed path depend on a real
   signal the effect is guaranteed to be subscribed to, not an imperative
   side effect that can race.

## Action item (independent, low-risk): remove dead `editor:source_hidden` meta

Not blocking the above, and safe to do regardless of what the live repro
finds:
- Remove the `"editor:source_hidden": true` insert from
  `agentmux-srv/src/server/app_api/pane.rs:228-232`'s markdown branch of
  `build_pane_meta`.
- Remove the matching dead write at `frontend/app/view/editor/editor-model.ts:1026`.
- Confirm no other reader was missed with one more repo-wide grep for
  `source_hidden`/`sourceHidden` immediately before removing, since this
  plan's own research is now ~a day old relative to a fast-moving repo.

---

# Part 2 — Reuse an already-open Editor pane instead of always opening a new one

## Motivation

Today, every `OpenEditor` MCP call creates a **brand-new Editor pane/block**,
split next to the calling agent's own pane
(`agentmux-mcp/src/main.rs`'s `"OpenEditor"` handler →
`agentmux-srv/src/server/app_api/mod.rs`'s `open_pane` → `CreateBlock` +
layout-insert). If an agent opens several files across a session (or several
agents share a tab), this litters the tab with one Editor pane per call
instead of one Editor pane with multiple file-tabs — which is exactly what
the Editor pane's own internal tab system (`tabsAtom`, `activeIdAtom`,
`openFile()`) already supports for user-driven file-tree clicks. Requested:
if an Editor pane is already open in the agent's own tab, add the file as a
new tab **inside that pane** instead of creating another pane.

## Research: the exact mechanism already exists in a sibling form

**Detecting "is there already a pane of this kind in this tab" is an
established pattern, not new plumbing.** `find_agent_block`
(`agentmux-srv/src/server/app_api/mod.rs:837-850`) does precisely this for
agent panes — iterate `tab.blockids`, fetch each `Block`, check a meta field
— and is used by the agent-open flow
(`agentmux-srv/src/server/app_api/agent_open.rs:84`) to decide "reuse this
existing block" vs. "create a new one." This plan's design is a direct
sibling of that function, checking `meta.view == "editor"` instead of
`meta.agentId == <id>`.

**Pushing "please open this file" into an already-mounted pane is also an
established pattern.** `EditorViewModel`'s constructor already runs a scoped
WPS subscription for live file-reload
(`frontend/app/view/editor/editor-model.ts:214-221`):

```ts
this._unsubFileChanged = waveEventSubscribe({
    eventType: WpsEvent.EditorFileChanged,
    scope: makeORef("block", blockId),
    handler: (event) => {
        const path = (event as any)?.data?.path as string | undefined;
        if (path) void this._handleExternalFileChanged(path);
    },
});
```

backed by a server-side `Broker::publish(WaveEvent{...})` call scoped
`block:<id>` (`agentmux-srv/src/backend/editor_file_watcher.rs:226-237`).
This is exactly the shape needed to tell an **already-running**
`EditorViewModel` instance "open this other file as a new tab" — reusing
100% of the existing, already-correct `openFile()` path (pin-if-existing,
language detection, RPC load) rather than inventing a second way to open a
file.

**A real, related gap found along the way**: `OpenEditor` never resolves the
*calling agent's own* tab_id today. `resolve_tab_id`
(`agentmux-srv/src/server/app_api/mod.rs:815-834`) only accepts an explicit
`tab_id` (which the MCP tool never sends) or falls back to "the first
workspace's active tab" — which only coincidentally matches the agent's own
tab in the common single-workspace case. This plan's design needs a real
"which tab is this block in" resolution to work correctly in multi-tab
setups, which doesn't exist yet as a helper (only the inverse — "which
blocks are in this tab," via `tab.blockids` — exists).

## Design

### 1. New backend helper: resolve a block's own tab_id

`agentmux-srv/src/server/app_api/mod.rs`, alongside `resolve_tab_id`/
`find_agent_block`:

```rust
/// Find which tab a given block currently belongs to, by scanning every
/// tab's `blockids`. Needed because `resolve_tab_id` only resolves an
/// explicit tab_id or "the active tab" — neither answers "which tab is
/// MY OWN block in," which callers passing split_reference_block_id
/// (the calling agent's own block id) actually need.
pub(super) fn resolve_tab_id_for_block(wstore: &Store, block_id: &str) -> Result<String, String> {
    let tabs: Vec<Tab> = wstore.get_all::<Tab>()
        .map_err(|e| format!("resolve_tab_id_for_block: list tabs: {e}"))?;
    for tab in tabs {
        if tab.blockids.iter().any(|b| b == block_id) {
            return Ok(tab.oid.clone()); // or whatever Tab's id field is named
        }
    }
    Err(format!("resolve_tab_id_for_block: block {block_id} not found in any tab"))
}
```

(`Tab.oid` confirmed as the correct field name — `agentmux-srv/src/backend/obj.rs:409`.)

### 2. New backend helper: find an existing editor block in a tab

Direct sibling of `find_agent_block`:

```rust
/// Find an existing Editor-view block in a tab, if any.
pub(super) fn find_editor_block(wstore: &Store, tab_id: &str) -> Result<Option<Block>, String> {
    let tab: Tab = wstore.must_get(tab_id)
        .map_err(|e| format!("TAB_NOT_FOUND: {e}"))?;
    for block_id in &tab.blockids {
        if let Ok(Some(block)) = wstore.get::<Block>(block_id) {
            if obj::meta_get_string(&block.meta, "view", "") == "editor" {
                return Ok(Some(block));
            }
        }
    }
    Ok(None)
}
```

### 3. New WPS event: request an already-mounted Editor pane to open a file

`agentmux-srv/src/backend/editor_file_watcher.rs` (or a new small module,
matching where `EVENT_EDITOR_FILE_CHANGED` lives) — a new const,
`EVENT_EDITOR_OPEN_FILE_REQUEST` (e.g. `"editor:open_file_request"`),
published the same way:

```rust
broker.publish(WaveEvent {
    event: EVENT_EDITOR_OPEN_FILE_REQUEST.to_string(),
    scopes: vec![format!("block:{existing_block_id}")],
    sender: String::new(),
    persist: 0,
    data: Some(json!({ "path": file_path })),
});
```

Frontend: `frontend/app/store/wps-events.ts` gets a matching
`EditorOpenFileRequest: "editor:open_file_request"` entry (alongside
`EditorFileChanged`/`MediaFileChanged`). `EditorViewModel`'s constructor
(`editor-model.ts`, right next to the existing `_unsubFileChanged`
subscription at line 214) gets a twin subscription whose handler calls
`this.openFile(path)` — the same public method the file-tree's click
handler already calls, so pin-if-existing/language-detection/RPC-load all
apply unchanged.

### 4. Wire it into `open_pane`'s `"editor"` branch

In `agentmux-srv/src/server/app_api/mod.rs`'s `open_pane`, before the
existing `CreateBlock` docked path: when `cmd.view == "editor"`,
`cmd.reuse_editor_pane == Some(true)` (§5 below — explicit opt-in, not
inferred), and `cmd.floating != Some(true)`:

1. `resolve_tab_id_for_block(&wstore, split_reference_block_id)` → the
   agent's real tab_id (replaces relying on `resolve_tab_id`'s
   active-tab fallback for this specific check).
2. `find_editor_block(&wstore, &that_tab_id)` → `Some(existing_block)` or
   `None`.
3. If `Some`: publish `EVENT_EDITOR_OPEN_FILE_REQUEST` scoped to
   `existing_block.oid` with the requested file path, and return a
   `PaneOpenResult` pointing at the **existing** block id (`created: false`)
   — skip `CreateBlock`/layout-insert entirely.
4. If `None`: fall through to today's unchanged behavior (create a new
   block, split next to the agent's pane).

### 5. Explicit `reuse_editor_pane` opt-in (not inferred)

**Corrected from this plan's original draft** — see "Corrections from
automated review" below for why an inferred trigger (e.g. "meta absent")
is unsafe. `CommandPaneOpenData`/`PaneOpenRequest` gain a new
`reuse_editor_pane: Option<bool>` field. Only `agentmux-mcp`'s `OpenEditor`
tool handler sets it (`Some(true)`); every other `pane.open` caller
(`EditorViewModel.openToTheSide`/`openInTerminal`, the widget-bar preset
path, etc.) leaves it `None`, so reuse never triggers for them.

## Open questions

1. ~~**Opt-out.**~~ **Resolved as opt-IN, not opt-out** — see §5. The
   original framing ("should a caller be able to opt out of reuse") had it
   backwards: reuse needs to be something only the intended caller
   (`OpenEditor`) asks for, not something every `pane.open` caller gets by
   default and has to escape.
2. **Focus/visibility of the reused pane.** `OpenEditor`'s existing
   `focus: true` behavior is only defined for the newly-created-block path
   (a layout-insert action with a "focused" flag). No equivalent "bring this
   existing, already-docked pane into view and make its new tab active"
   primitive was confirmed to exist — if the reused pane is already visible
   in the currently active tab (the common case — same tab the agent lives
   in), this may be a non-issue in practice; worth confirming rather than
   assuming, but not blocking for v1.
3. **Multiple editor blocks in one tab.** If a tab somehow has more than one
   Editor-view block (e.g. from the pre-existing `openInNewTab`
   "open to the side" flow, `editor-model.ts:1011-1034`, which deliberately
   creates a second Editor pane in a *different* tab, not this one — but a
   user could still end up with two Editor panes in the *same* tab via
   drag/split), `find_editor_block` as sketched returns the first match by
   `blockids` order. Fine for v1; not spec'd further here.

## Corrections from automated review (reagent + codex, pre-merge)

All four confirmed real, against the actual code, before fixing — not
assumed correct from the finding alone:

1. **Trigger was inferred, not explicit — collided with existing callers.**
   The original draft above triggered reuse whenever `cmd.meta.is_none()`
   (paired with `view == "editor"` and a `split_reference_block_id`). Both
   `EditorViewModel.openToTheSide` and `openInTerminal`
   (`frontend/app/view/editor/editor-model.ts:958-984`) call the same
   generic `pane.open` RPC with `split_reference_block_id: this.blockId` —
   their OWN block, used purely for split placement — and no `meta`. The
   original condition would have matched that shape too, silently
   redirecting "Open to the Side" into the calling pane itself instead of
   creating the requested second pane. Fixed with the explicit
   `reuse_editor_pane` opt-in (§5), set only by `OpenEditor`.
2. **Floating requests could be swallowed into a reused docked pane.** The
   reuse check originally ran before the `floating` branch, so
   `OpenEditor(file, floating: true)` with an existing Editor pane already
   in the tab would silently open the file in the *docked* pane instead of
   creating the requested floating window. Fixed by excluding
   `cmd.floating == Some(true)` from the reuse condition (§4).
3. **`persist: 0` lost requests under a real race.** Two back-to-back
   `OpenEditor` calls: the first creates the Editor block, but its frontend
   `EditorViewModel` may not have mounted (and subscribed to
   `EVENT_EDITOR_OPEN_FILE_REQUEST`) by the time the second call reuses that
   block — publishing with `persist: 0` to zero subscribers silently drops
   the second file. Fixed using the broker's existing replay-on-subscribe
   mechanism (`Broker::subscribe`, `agentmux-srv/src/backend/wps.rs:206-239`)
   — `persist: 20` instead of `0`, so a late-subscribing `EditorViewModel`
   still receives it. Not a new mechanism; reuses the same pattern already
   proven for live-log streaming's identical race.
4. **Wrong-tab fallback wasn't just theoretical — reproduced by a test.**
   When reuse doesn't apply (no existing pane, or a floating request), the
   code fell through to `resolve_tab_id`'s "first workspace's active tab"
   fallback, discarding the tab already correctly resolved via
   `split_reference_block_id` for the reuse check. A new regression test
   for the floating case concretely failed against this before the fix
   ("tab not found in reducer state"). Fixed by deriving the tab from
   `split_reference_block_id` generally (§4's step 1, now applied to the
   non-reuse fallback too) whenever no explicit `tab_id` is given — this
   benefits every `pane.open` caller that specifies a relative block, not
   just the editor-reuse path.

## Files (anticipated — this plan does not implement)

| File | Relevance |
|------|-----------|
| `agentmux-srv/src/server/app_api/mod.rs` | New `resolve_tab_id_for_block`, new `find_editor_block`, both mirroring `find_agent_block`/`resolve_tab_id`'s existing shape; `open_pane`'s tab-resolution and `"editor"` branch gain the reuse-or-create decision |
| `agentmux-srv/src/server/app_api/pane.rs` | New `maybe_reuse_editor_pane` + `EVENT_EDITOR_OPEN_FILE_REQUEST` const, published with `persist: 20` (§5's correction) |
| `agentmux-srv/src/backend/rpc_types/block.rs`, `agentmux-common/src/api_types.rs` | New `reuse_editor_pane: Option<bool>` field on `CommandPaneOpenData`/`PaneOpenRequest` (§5) |
| `agentmux-mcp/src/main.rs` | `OpenEditor` handler sets `reuse_editor_pane: Some(true)` — the only caller that does |
| `frontend/app/store/wps-events.ts` | New `EditorOpenFileRequest` entry |
| `frontend/app/view/editor/editor-model.ts:214-221` | New twin WPS subscription calling `this.openFile(path)`, alongside the existing `EditorFileChanged` one |
| `agentmux-srv/src/server/app_api/agent_open.rs:84` | Precedent this design's reuse-detection directly mirrors |
| `docs/analysis/ANALYSIS_EDITOR_MD_BUGS_2026_06_20.md` | Part 1's starting point — re-investigated, its Bug 3 fix confirmed already shipped (commit `5c6bc11af`), current symptom's cause still open pending live repro |
