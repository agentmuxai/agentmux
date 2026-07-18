# Spec: live-reload for editor/preview panes on external file changes

**Status:** Draft — design only, no implementation. Written after confirming
the gap empirically (a live "TEST EDIT" write to an open preview pane's file
was not reflected without closing/reopening the tab) and at the code level.
**Author:** Agent3
**Date:** 2026-07-18
**Related:** `docs/specs/SPEC_EDITOR_FILE_ENCODINGS_2026_06_17.md`,
`docs/specs/SPEC_EDITOR_FILE_TREE_2026-05-26.md`, `agentmux-srv/src/backend/config_watcher_fs.rs`
(the existing single-file watcher this design generalizes from).

## Problem

Editor/preview panes read a file's content exactly once, at open time.
`readeditorfile` (`agentmux-srv/src/server/editor_handlers.rs:198-236`) is a
plain one-shot `std::fs::read` — no watcher, no subscription, no push
mechanism. The frontend (`frontend/app/view/editor/editor-model.ts`) caches
that read in a view-local `Map` (`_contentByTab`) and has an explicit
early-return guard (`_openFileWithMode`, lines 401-405) that skips
re-fetching entirely once a tab is marked `contentLoaded`.

Confirmed empirically: editing
`docs/specs/REPORT_ARMORY_BUNDLE_STANDARD_RESEARCH_2026_07_16.md` on disk
(via this agent's own `Edit` tool, which writes the file directly — not
through AgentMux's `writeeditorfile` RPC) while it was open in a preview pane
left the pane showing stale content. This is a real, everyday friction: any
file an agent is actively editing that a human also has open for review
(exactly this session's workflow) silently desyncs, and the only recovery is
close/reopen the tab — with no indication anything is stale in the first
place.

## Non-goals

- **Not** building general OS-level file-sharing/collaborative-editing
  (OT/CRDT merge of concurrent edits). Scope is strictly: detect an external
  change, and either refresh automatically or prompt — never silently merge
  content.
- **Not** watching arbitrary directories preemptively. Only files that are
  actually open in at least one editor tab, anywhere in the app, get watched
  — added when the first tab opens them, removed when the last tab closes.
- **Not** solving the general "two AgentMux windows/panes editing the same
  file" case beyond what this also incidentally fixes (both panes are just
  separate watch-subscribers to the same backend event).

## Design

### Backend: per-path watch registry, refcounted, event-scoped per block

New module `agentmux-srv/src/backend/editor_file_watcher.rs`, modeled on
`config_watcher_fs.rs`'s `notify`-crate usage but generalized from "one
static file" to "a dynamic, refcounted set of paths":

```rust
pub struct EditorFileWatcher {
    watcher: RecommendedWatcher,
    // path -> set of block_ids with a tab open on that path (refcount via set size)
    watched: Mutex<HashMap<PathBuf, HashSet<String>>>,
}

impl EditorFileWatcher {
    pub fn watch_path(&self, path: &Path, block_id: &str) { /* add, watcher.watch() if newly-added */ }
    pub fn unwatch_path(&self, path: &Path, block_id: &str) { /* remove; watcher.unwatch() if now-empty */ }
}
```

- One process-wide `notify::RecommendedWatcher` (same crate already a
  dependency via `config_watcher_fs.rs`), watching individual files in
  `RecursiveMode::NonRecursive` mode (matches the existing pattern —
  `notify` on most platforms needs the *parent directory* watched even for
  single-file mode; reuse `config_watcher_fs.rs`'s approach of watching the
  containing dir and filtering events by exact path match).
- `watch_path`/`unwatch_path` are called from the `openeditorfile`/tab-close
  RPC handlers (new hook points, see below) — not from `readeditorfile`
  itself, since a file can be *read* (e.g. by the file-tree preview-on-hover,
  if that exists) without a tab actually being open on it.
- On a debounced `Modify`/`Create` event for a watched path (same 300ms
  debounce as `config_watcher_fs.rs:139` — batches an editor's
  multi-write-syscall saves into one event), publish a new WPS event scoped
  to **every block that has a tab on that path**, not a global broadcast:

```rust
pub fn publish_editor_file_changed(broker: &Broker, path: &Path, block_ids: &[String]) {
    let event = WaveEvent {
        event: EVENT_EDITOR_FILE_CHANGED.to_string(),
        scopes: block_ids.iter().map(|id| format!("block:{id}")).collect(),
        sender: String::new(),
        data: Some(json!({ "path": path.to_string_lossy() })),
    };
    broker.publish(event);
}
```

  This mirrors `publish_controller_status`'s `scopes: vec![format!("block:{}", ...)]`
  pattern exactly (`blockcontroller/mod.rs:486-500`) — the existing
  per-block WPS scoping mechanism, not a new one.

- **Deliberately does not send content in the event** — same "wake signal,
  not payload" pattern muxbus's `InjectAvailable` uses (zero-metadata
  broadcast; the receiver re-fetches). Here: the frontend receives "path X
  changed," and re-calls `readeditorfile` itself. Keeps the watcher's hot
  path cheap (no need to read+hash the file inside the notify callback) and
  reuses the existing read RPC instead of adding a second content-delivery
  channel.

- Lifecycle: `Arc<EditorFileWatcher>` constructed once in `main.rs` alongside
  the existing `spawn_settings_watcher` call, held in `AppState`. `AppState`
  is already threaded into RPC handlers, so `openeditorfile`/tab-close can
  reach it the same way `writeeditorfile` reaches other shared state today.

### Frontend: subscribe per open path, reconcile against dirty state

`editor-model.ts` needs three additions:

1. **Subscribe on tab open, unsubscribe on tab close** (mirrors the existing
   `_globalHandler`/`installGlobalSinkOnce` fan-out pattern already in the
   file, lines 178-198) — a `waveEventSubscribe({ eventType: EVENT_EDITOR_FILE_CHANGED, scope: makeORef("block", blockId) })`
   per pane, dispatched into a new reducer command.

2. **New reducer command `TabExternallyChanged { tabId, source: "system" }`**
   in `editor-pane-state-store.ts`, alongside the existing `TabContentLoaded`/
   `TabSaved` cases (~line 583-613):
   - If `!tab.dirty`: safe to auto-refresh. Emit a `TabNeedsReload` event;
     `editor-model.ts`'s handler for it re-runs the same `ReadEditorFileCommand`
     → `_contentByTab.set` → `TabContentLoaded` path already used by
     `_openFileWithMode` (lines 409-455) — **reuse that code path**, don't
     duplicate it. A brief flash/highlight (reuse the pattern from the swarm
     view's `summaryFlash` in `swarm-view.tsx`, or the tab-bar's existing
     save-indicator styling) signals "this just changed," so the user
     notices even a silent auto-reload happened.
   - If `tab.dirty`: **never auto-clobber.** Set a new `externalChangeDetected:
     boolean` flag on the tab (parallel to the existing `dirty`/`loadError`
     fields) and surface a banner in `editor-view.tsx` — "This file changed on
     disk. [Reload (discard my changes)] [Keep my changes] [Diff]" — reusing
     the existing dirty-confirm-modal groundwork already flagged as a
     follow-up in `closeTab`'s comment (`editor-model.ts:474-479`: "the
     dirty-confirm modal lands in a follow-up commit"). This spec's banner
     and that modal should share one confirm-UI component once both exist,
     not duplicate it.
   - **Preview-mode special case:** a tab in `editorMode() === "preview"`
     with no local edits is *by definition* never dirty (preview is
     read-only rendering) — those tabs always take the silent-refresh path.
     This directly fixes the motivating case (a report file open in preview
     while an agent edits it).

3. **Hash-based no-op guard:** compare the newly-fetched content's SHA256
   (already computed via `sha256Hex`, `editor-model.ts:448`) against the
   tab's stored `contentHash` before dispatching `TabContentLoaded` — if
   unchanged (e.g. the watcher fired on a metadata-only touch, or a save
   round-trip from AgentMux's own `writeeditorfile` also triggered the
   watcher), skip the reload/flash entirely. This also naturally suppresses
   the self-triggered case: saving *from* the editor pane writes the file,
   which the watcher will also see and re-notify — without the hash guard,
   every save would cause a redundant self-reload flash.

### Open questions

1. Does `notify` reliably fire on Windows for saves from every common editor
   (VS Code, Notepad, this agent's own `Edit`/`Write` tool via Rust
   `std::fs::write`)? `config_watcher_fs.rs` already proves the basic
   mechanism works for `settings.json` on this platform — worth confirming
   editors that use atomic rename-based saves (write to temp file, rename
   over original) still fire `Modify`/`Create` on the *original* path, since
   `notify`'s behavior on rename-over-existing can differ from a direct
   in-place write depending on platform backend.
2. Should the write-your-own-save self-notification (point 3 above) be
   suppressed *before* it reaches the frontend at all — e.g. the backend
   remembers "this path was just written by `writeeditorfile` in the last
   500ms, don't re-publish" — rather than relying solely on the frontend's
   hash guard? Cheaper (avoids a wasted RPC round-trip + WS event) but adds
   backend-side state; the hash-guard alone is correct, just slightly
   chattier. Lean toward hash-guard-only for v1 (simpler, already-available
   primitive) and revisit if the self-notification round-trip proves
   measurably wasteful.
3. Multi-window: does the WPS broker's `scopes` filtering already fan out
   correctly to a block open in a *different* OS window (not just a
   different tab in the same window)? This should already work if
   `publish_controller_status`'s existing scoping does (used today across
   windows for agent status) — worth a smoke test, not a design change.
4. Large-file guard: `readeditorfile`/`writeeditorfile` already cap at 10MB
   (`editor_handlers.rs:212-214,261-263`); should the watcher additionally
   skip re-publishing for files near that cap to avoid a debounce-storm on a
   frequently-rewritten large log-like file opened in an editor tab? Probably
   yes, but low priority — same size guard, reused, not a new decision.

## Implementation phases

**Phase 1 — backend watcher + event, no frontend changes.** Ship
`EditorFileWatcher`, wire `watch_path`/`unwatch_path` into the
open/close-tab RPC handlers, publish `EVENT_EDITOR_FILE_CHANGED`. Verifiable
independently via `wscat`/a manual WS listener before touching the frontend.

**Phase 2 — frontend silent-refresh path for non-dirty tabs.** Subscribe,
dispatch `TabExternallyChanged`, reuse the existing content-load code path,
add the hash guard. This alone fixes the motivating preview-pane case and is
safe to ship even before the dirty-conflict UI exists (dirty tabs simply
don't reload yet — same behavior as today, just scoped down from "all tabs
never reload" to "only clean tabs reload").

**Phase 3 — dirty-conflict banner.** Requires (or should be built alongside)
the dirty-confirm-modal component already flagged as pending in
`closeTab`'s comment — natural to land together since both need the same
"discard vs. keep local edits" UI primitive.

## Files

| File | Relevance |
|------|-----------|
| `agentmux-srv/src/server/editor_handlers.rs:198-236` | `readeditorfile` — the one-shot read this design keeps reusing for the actual content fetch |
| `agentmux-srv/src/backend/config_watcher_fs.rs` | The single-file watcher pattern this design generalizes (debounce, `notify` crate usage, WPS broadcast) |
| `agentmux-srv/src/backend/blockcontroller/mod.rs:486-500` | `publish_controller_status` — the exact per-block WPS scoping pattern to reuse for the new event |
| `frontend/app/view/editor/editor-model.ts:100-249,351-472` | Tab content cache, `_openFileWithMode`'s load path (to be reused, not duplicated), the existing event fan-out pattern to extend |
| `frontend/app/store/editor-pane-state-store.ts:583-613` | `TabContentLoaded`/`TabSaved` reducer cases — where the new `TabExternallyChanged` case is added |
| `frontend/app/view/swarm/swarm-view.tsx` (`summaryFlash`) | Existing "flash on change" UI pattern to reuse for the silent-refresh visual cue |
