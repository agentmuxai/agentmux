# Spec: recycle a window's renderer when it's idle and over budget

**Date:** 2026-10-13, revised 2026-10-14 against `main` @ `ab6a9c400`
**Status:** active. Shipped: §2's reload without a history entry (#4940); composer drafts, sent-message history and attachment drafts across a reload (#4942); pane overlay changes saved, and `flushPaneSnapshots()` (#4943); browser panes keep their page (#4944); secondary windows re-attach to their own row (#4945); rows get ids from their transcript line (#4952). Left: the rest of Phase A (§2: scroll position), Phase B (§3), Phase C (§4).
**Plan item:** #5 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.
**Evidence:** `docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md` §4.3.
**Related:** `SPEC_WINDOW_CHURN_RENDERER_LEAK_2026_10_13.md` (#4858: a navigation that adds a history entry keeps the old page in the back/forward cache).

## 1. The problem

Blink's garbage-collected heap doesn't compact, and on Windows it keeps its empty pages. A renderer stays at its high-water mark:

| Lab state (after a forced GC) | Blink heap reserved | Live objects | Empty |
|---|---:|---:|---:|
| 12 agent panes | 544 MB | 61 MB | 89% |
| 4 agent panes | 200 MB | 17 MB | 92% |

Reloading the page in the same renderer returned it: **1,009 → 338 MB** (12 panes) and **407 → 198 MB** (4 panes). VS Code measured the same in its agents window (microsoft/vscode#337444: 93% empty, about 520 MiB back on recreating the renderer). Microsoft's WebView2 guidance recommends periodic refresh for long-running pages.

#4751 and #4756 lower the peaks, so there's less to strand, but every burst still raises the high-water mark for good. A long session ends at its worst moment.

The fix has three parts, shipped in order:
- **Phase A**: a reload loses nothing (§2). Useful by itself: the reloads that already happen (version skew, startup auto-recover, a WebGL context loss) stop losing drafts.
- **Phase B**: a "Reload window (free memory)" command (§3), the same recycle on demand. It lets us measure real sessions before anything runs automatically.
- **Phase C**: the governor (§4), which recycles an idle, oversized window by itself.

## 2. Phase A: a reload loses nothing

Today a reload of the main window restores the workspace, layout, agent transcripts (from srv; running turns aren't interrupted, and the transcript cursor fills any gap), terminals (serialized cache + PTY replay) and waiting-for-you state. What it loses, and the fix for each:

| Lost today | Where | Fix |
|---|---|---|
| Composer drafts | in-memory `Map` (`AgentFooter.tsx:87`) | Persist per block in `sessionStorage` on change (debounced 300 ms), restore on mount. Survives a reload, not an app restart, as today. |
| Up-arrow history | a `let` in the component body (`AgentFooter.tsx:740-743`), lost on every unmount, including a pane-tab switch (`:75-83`) | Move it to a module-level `Map` per block (fixes the tab switch), persisted like drafts. |
| Attachment drafts | in-memory `Map` (`attachments/attachment-draft.ts:495`) | Persist the ready attachments' ids and names, and bring them back with the existing `restoreRefs`, which shows a file that's gone as an error tile. The files are in srv's attachment store (`:86-95`), kept 30 days after their last use (`crates/srv/src/backend/attachments/mod.rs:36`), so a reload is no risk to them. An upload in progress (an XHR, `:129`) can't survive: it blocks a recycle (§4.3). |
| Messages queued while the agent is busy | `heldQueue` (`useAgentCommands.ts:301-321`, pushed at `:735-741` and `:779`) | Never recycle while one is non-empty; the predicate exists (`hasHeldMessages`, `:1103`, exposed at `agent-view.tsx:1105`). |
| Pane state (collapsed, pinned, filter, details) | the 30 s snapshot (`useSnapshotPersistence.ts:43`) is marked dirty only when the document's nodes change (`:128-134`), so a change to this state alone may never be written; Solid's cleanup doesn't run on unload | Mark the snapshot dirty on these changes too, and add `flushBeforeRecycle()`, which writes every pane's snapshot now (§3). |
| Which row is which | text, message and user rows got counter ids (`node_N`, `msg_N`, `user_N`, `stream-parser.ts` `nextIdOf`) counted from wherever a parse began, which moves as the conversation grows (the restore reads only the last turns) and restarts on every older page | Build a row's id from its transcript line and generation (`node_<gen8>_L<line>_<k>`; the generation keeps a new transcript's line 0, after an archive, apart from the old one's): history reads pass their first line, the live loop each event's line. Known limit: a read that starts partway through a text run (not at a turn boundary; the default tail-of-turns restore always is) anchors that one row on its first line read. Saved collapsed rows and a scroll anchor then find their row after a reload, and an older page's rows no longer collide with the window's (they were dropped: loading older history showed only its tool calls). |
| Scroll position | `scrollPosition` is only ever 0 (`state.ts:110`); it is copied into the snapshot (`useSnapshotPersistence.ts:101`) and read on restore (`useHistoryPagination.ts:438`), but never written | Write it from the virtual list's scroll anchor (first visible row id + offset), restore it after history loads. |
| Unsaved editor text | not persisted; no prompt | Never recycle a window with a dirty editor (§4.3). |
| Open modals and menus | in-memory | Never recycle with one open (§4.3). |
| Browser panes' pages | the pane stays Live across the reload (nothing closes it on unload), so on remount `browserPanes.create` hits `AlreadyLive` and the host calls `load_url(url)` (`crates/cef/src/browser_panes/mod.rs:222-253`), reloading every page | When the pane is Live with the same URL, re-attach without navigating. |

**Secondary windows don't re-attach.** `initHostNewWindow` calls `WindowService.CreateWindow` on every load (`app-init.ts:317-331`). A `workspaceId` in the URL (`:312`) only decides which workspace the new window row shows, and `previousWindowId()` (`app/init/reload-window-id.ts:21-36`) only copies the theme and opacity (`crates/srv/src/server/service/window_create.rs:398-405`). So each reload of a secondary window creates a new window row, and leaves the old one open: nothing closes a row on a reload. A window promoted from the pool for New Window has no `workspaceId` in its URL (`app/init/pool.ts:108-110`), so its reload also gets a new, empty workspace. Fix (frontend only): when the remembered row is still open (in `Client.windowids`), still this window's (its `host:label` crumb) and its workspace exists, the window re-attaches to it (`reattachableWindow`, `app/init/reload-window-id.ts`); otherwise it creates a row as before. Using the row's own workspace makes a `workspaceId` in the URL unnecessary. The main window keeps re-attaching as today (`initHostMux`, `windowids[0]`, `app-init.ts:169`).

**The reload must not add a history entry.** `reloadKeepingHostCredentials` (`app/init/error-display.ts:114-118`, wired as `AppApi.reloadWindow`, `util/cef-api.ts:134-135`) navigates with `location.assign`. That adds an entry, and the back/forward cache keeps the old page, the whole app with its heap (`crates/cef/src/commands/replace_page.rs:5-14`): the reload frees nothing. Use `location.replace(credUrl)`. A plain `location.reload()` adds no entry either, and keeps working without the credentialed URL, because the host re-injects the IPC credentials on every main-frame load of its frontend (`crates/cef/src/client/navigation.rs:698-739`) and the frontend waits up to 2.5 s for them (`cef-init.ts:73-77`). The credentialed URL only removes that wait (`error-display.ts:19-30`). The plain reloads in `bootstrap.ts:80,99` should use the same helper.

## 3. Phase B: "Reload window (free memory)"

A Window command in the palette (`frontend/app/store/command-registry.ts`, Window category from `:232`; reachable from the host's `run_command`, `crates/cef/src/commands/palette.rs:20-50`). It runs the recycle below on the window it's invoked in, skipping the governor's conditions (§4.2, §4.3) but not the flush.

### 3.1 The recycle

1. **Prepare.** The host emits `recycle:prepare` to the window. The frontend checks the blockers (§4.3; for the command, only those that would lose data: a held queue, an upload in progress, a dirty editor), runs `flushBeforeRecycle()`, and answers with the `recycle_ready` IPC: `{ ok }` or `{ ok: false, reason }`. No answer within 2 s means don't recycle this time.
2. **Cover.** If the window is visible, the host captures its viewport (the capture tear-offs use, `browser_api/routes.rs:295-309`) and keeps it for this label. The new document asks for it while booting (a `take_reload_snapshot` IPC) and shows it in the startup splash, as a promoted tear-off shows its snapshot (`showTearOffSnapshot`, `app/init/startup-splash.ts:56-80`). Today nothing carries a picture across a reload of the same window. Minimized or occluded windows skip this.
3. **Reload** with `location.replace(credUrl)` (§2). The splash and each pane's reveal cover hide the rebuild; the snapshot fades out as the splash lifts.
4. **Log** `[recycle] {label} {reason} {before_mb} MB`, then, 60 s after the load settles, `[recycle] {label} settled at {after_mb} MB`, which becomes the window's new baseline (§4.1).

A recycle is a navigation, not a crash: it doesn't touch the crash budget (`crates/cef/src/client/mod.rs:24-50`).

## 4. Phase C: the governor

The host decides when to recycle each top-level app window's renderer: the main window and secondary windows. Not floating panes, browser panes or unpromoted pool windows.

### 4.1 Measure

- Every 60 s, on the memory heartbeat's thread (`crates/cef/src/memory_heartbeat.rs`, 20 s tick, `:129`; every third tick), read the private working set of each top-level window's renderer. The PIDs come from the host's `renderer_map` (`crates/cef/src/renderer_map.rs:116-154`); the reading from `agentmux_procstats` (`crates/procstats/src/lib.rs:135`, private working set on Windows, `:62-69`), which `crates/cef` doesn't depend on yet. Windows first: on Linux the map holds the sandbox-namespace PID, which only srv translates today (`tower_agentmux.rs:189-198`).
- **A renderer serving more than one window** (Chromium can do this; the map lists every label per PID, `renderer_map.rs:152`) is skipped and logged once. Reloading one of its windows frees only that page's share, and the measure can't tell whose share it is.
- Record each window's **baseline**: its renderer's size 60 s after its last load settled.

### 4.2 Over budget

A window qualifies when its renderer is above both:
- 2× its baseline;
- 600 MB.

The second condition keeps a small window from being recycled for a doubling that costs little.

### 4.3 Idle and safe

All of these must hold:
- **Not in use for 10 minutes:** not the foreground window for 10 minutes, or minimized. The host's foreground hook (`crates/cef/src/wrr/win_event.rs:112,982-987`) today only forwards to the launcher (`last_foreground_at_ms`, `crates/launcher/src/state.rs:126`, which records when a window became foreground); the host keeps its own per-label "foreground since / until" instead. **And** no input in the window for 10 minutes: the frontend keeps a last-input time (capture-phase `pointerdown`, `keydown` and `wheel` listeners; the Event Timing observer, `frontend/perf/observers.ts:57-92`, skips short events and isn't a clock) and returns it in step 1's answer. There is no heartbeat to carry it, and the prepare round trip makes one unnecessary.
- No agent pane in it has held messages (`hasHeldMessages`).
- No attachment upload in progress.
- No dirty editor, no open modal or menu, no drag in progress (`isAnyUnderway`, `app/drag/drag-session.ts:100`), no native file dialog (`host_dialog_open()`, private today in `crates/cef/src/floating_pane.rs:241-243`).
- No agent pane in it is waiting for the user (`term:awaiting_user`, `crates/srv/src/backend/agent_state.rs:25`; `view/agent/hooks/awaiting-user.ts`). A question panel's timer and focus would be reset.

The host checks what it knows (foreground, minimized, file dialog) before asking; the frontend checks the rest in step 1.

An agent mid-turn doesn't block a recycle: its turn continues in srv and the pane catches up from the transcript. It does lower the priority (prefer a window with no turn running, if several qualify).

### 4.4 Memory pressure

Under `Warn` commit pressure (`crates/cef/src/memory_pressure.rs:77-84`), the idle time drops to 2 minutes. Under `Critical`, the 2× baseline condition is dropped (600 MB still applies). The other conditions in §4.3 always hold.

### 4.5 Limits

At most once per window per hour. Never two windows within 60 s of each other.

### 4.6 Kill switch

`window:autorecycle` (default `true`) turns the governor off; the command stays. It follows the existing `window:` keys: `crates/srv/src/backend/wconfig/types.rs:241-286`, `schema/settings.json`, `settings-template.jsonc`, `frontend/types/srv-types.d.ts`. The host reads it from `settings.json` as it reads `window:transparent` (`crates/cef/src/app/window_settings.rs:257`).

## 5. Tests

**Phase A:**
- a composer draft, the up-arrow history and an attachment draft survive `location.reload()`;
- the up-arrow history survives a pane-tab switch;
- collapsing a row alone marks the snapshot dirty;
- `flushBeforeRecycle` writes every pane's snapshot, including the scroll anchor, and panes restore to it;
- a Live browser pane isn't navigated on re-attach;
- a secondary window's reload re-attaches to its window row (no new row), and a pool window promoted to a new window has `workspaceId` in its URL;
- the reload helper uses `location.replace` (no history entry).

**Phase B:**
- the command reloads the window, and the log shows before and after sizes;
- a held queue, an upload in progress or a dirty editor refuses it with the reason;
- no answer within 2 s leaves the window alone;
- the snapshot covers the reload of a visible window.

**Phase C** (a pure decision function over a snapshot of window states):
- over budget and idle → recycle;
- not over 600 MB → no;
- foreground, or input in the last 10 minutes → no;
- held messages, an upload, a dirty editor, an open modal, a drag, a waiting-for-you pane, a file dialog → no;
- recycled within the hour, or another window within 60 s → no;
- a renderer shared by two windows → no;
- `Warn` shortens the idle time; `Critical` drops the 2× condition;
- `window:autorecycle: false` → no.

**Lab:**
- the 12-agent run, then 10 minutes idle;
- confirm the recycle fires, the renderer falls to about its post-load size (expect about 340 MB) with the old document gone (one document left, no back/forward-cache entry), every pane is restored with its draft and scroll position, and nothing visible flashes;
- also with a streaming agent and a browser pane playing video;
- the command on a secondary window: same window row before and after.
