# SPEC — Browser pane: "Set as Start Page" in the bookmarks menu

**Date:** 2026-09-16
**Status:** implemented (2026-09-16) — `cargo test -p agentmux-srv`: 3550
passed; `tsc --noEmit`: clean; `vitest`: 2962 passed (29 pre-existing
Windows-only @solid-refresh/jsdom failures, none touching this feature's
files). Not yet manually verified in a running `task dev` instance. — #3288
**Scope:** `frontend/app/view/browser/` (nav bar, model),
`agentmux-srv/src/backend/wconfig/` (`FullConfigType`, `ConfigState`),
`agentmux-srv/src/backend/config_watcher_fs.rs`, `agentmux-srv/src/bootstrap.rs`.
**Revision note:** superseded design in §3 — the first pass proposed a
dedicated `shared_dir` file plus a `.get`/`.set` RPC pair, with the value
preloaded via a new sequential RPC call added to `frontend/app-init.ts`'s
boot sequence. On reconsideration (repo owner: "we want robust fast
performance, engineering cost is not important"), that design is replaced
below with one that adds **zero** boot-time latency and gets **true
live cross-window sync for free**, by riding the app's existing
`GetFullConfig`/`ConfigState`/file-watcher infrastructure instead of
building a parallel one. This costs more backend engineering (touching
`FullConfigType`, `ConfigState`, the fs-watcher, and `bootstrap.rs`) in
exchange for exactly the two properties asked for.

---

## 1. The ask

> "in the browser pane, we want an additional entry when expanding the
> bookmarks button, it is to set the start page. Make it the first entry."

A new item at the **top** of the bookmarks dropdown
(`frontend/app/view/browser/browser-nav-bar.tsx:285-329`,
`bookmarkMenuItems`) that saves the active tab's URL as the page a **new**
browser pane opens to, replacing the hardcoded fallback that exists today.

## 2. Current state

- **No start-page concept exists anywhere in the codebase today** — checked
  `frontend/app/view/browser/`, `agentmux-srv/src/config/` for
  `startPage`/`homePage`/`defaultUrl`/`newTabUrl`; nothing.
- A fresh browser pane's URL comes from `BrowserViewModel`'s constructor
  (`browser-model.ts:522-537`):

  ```ts
  const meta = this.blockAtom()?.meta;
  const initialUrl = ((meta?.["url"] as string | undefined) ?? "").trim() || DEFAULT_BROWSER_URL;
  ```

  `DEFAULT_BROWSER_URL` (`browser-model.ts:82`) is a hardcoded constant,
  `"https://agentmux.ai"`. This is the fallback this feature adds a
  user-configurable layer in front of.
- The bookmarks menu is built by `bookmarkMenuItems`
  (`browser-nav-bar.tsx:285-329`): a pinned "Bookmark This Page" / "Remove
  Bookmark" row first, then a divider, then every saved bookmark, or a
  muted "No bookmarks yet" row. Bookmarks persist via
  `bookmarks.list`/`bookmarks.set` RPC
  (`agentmux-srv/src/server/app_api/bookmarks.rs`) against
  `~/.agentmux/shared/browser-bookmarks.json`
  (`agentmux-srv/src/backend/bookmarks_store.rs`) — a dedicated
  `shared_dir` file, fetched lazily on menu open, with **no live sync
  across windows** (`docs/specs/SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md`'s
  own unhappy-paths table names this explicitly: "a menu opened before
  another window's edit lands shows a stale snapshot until closed and
  reopened"). That gap is fine for bookmarks' accepted scope; it is exactly
  the gap this spec's revised design closes for the start page, since
  robustness is the stated priority here.

## 3. Design

### 3.1 Where the value lives, and why not a dedicated file this time

The first draft of this spec proposed a sibling file mirroring
`bookmarks_store.rs` exactly. On reflection that mirrors bookmarks'
*persistence* pattern (correct: still `shared_dir`, still global across
channels, for the identical reasoning
`SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md` already gives) but
also mirrors its *delivery* pattern — lazy, on-demand, no live sync — which
is the one part of the bookmarks design explicitly **not** worth copying
when the goal is robustness over minimal engineering effort.

**The value still lives in its own `shared_dir` file**,
`~/.agentmux/shared/browser-start-page.json`:

```json
{ "url": "https://example.com" }
```

— global across channels, same reasoning as bookmarks: nobody wants their
start page reset by switching `dev-<branch>` channels, and `settings.json`
isolation is whole-file only with no per-key tiering
(`docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md` §6).

**What changes is how the frontend learns about it.** Instead of a
dedicated `.get` RPC fetched lazily (bookmarks' pattern) or a new preload
call added to app boot (this spec's own first draft), the value rides
along in the **existing** `FullConfigType`/`GetFullConfig` machinery that
already:

- loads once at srv startup, before any client connects
  (`bootstrap.rs:1048`, `wconfig::build_default_config()`),
- is already fetched by the frontend and awaited **before the app even
  renders** (`frontend/app-init.ts:1122`, `RpcApi.GetFullConfigCommand`),
  so this is genuinely free — not "one more parallel call," zero additional
  round trips of any kind,
- already has a live file-watch → in-memory-update → WebSocket-broadcast
  pipeline for exactly this "a config file changed on disk, tell every
  connected window" problem
  (`agentmux-srv/src/backend/config_watcher_fs.rs`, `WpsEvent.Config` on
  the frontend, `frontend/app/store/global.ts:242-246`:
  `setFullConfigAtom(fullConfig)` on every push).

This is the same infrastructure `settings.json` itself uses, and — unlike
`settings.json` — a field sourced from `shared_dir` inside that same
wire struct is **not** subject to per-channel isolation, because isolation
there is about which *file on disk* feeds the `settings` sub-object
(`SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md`), not about the RPC
struct that carries it over the wire. `FullConfigType` already has a
precedent for a sibling top-level field sourced independently of
`settings.json` — `bookmarks: HashMap<String, WebBookmark>`
(`agentmux-srv/src/backend/wconfig/types.rs:758`, a legacy/unused field
today, not the same one the shipped browser-bookmarks feature uses, but
proof the struct's shape already anticipates fields like this).

### 3.2 Backend changes

**`agentmux-srv/src/backend/wconfig/types.rs`** — one new field on
`FullConfigType`:

```rust
#[serde(rename = "browserStartPage", default)]
pub browser_start_page: Option<String>,
```

**New module `agentmux-srv/src/backend/browser_start_page.rs`** — the
`read`/`write` file helpers, structurally identical to
`bookmarks_store.rs` (same `DataPaths::from_env()` best-effort path
resolution, same temp-file-then-rename write for crash safety, same
"missing file is `None`, corrupt file is a loud `Err`" read contract) —
**plus**, unlike bookmarks, the load/watch/broadcast wiring that plugs it
into `ConfigState`:

- `load_start_page_from_disk(config_watcher: &ConfigState)` — mirrors
  `config_watcher_fs::load_settings_from_disk`, called once at boot.
- `spawn_start_page_watcher(fs_watch_pool, config_watcher, event_bus)` —
  mirrors `config_watcher_fs::spawn_settings_watcher` **exactly**, reusing
  the **same shared `FsWatchPool`** instance already passed to the
  settings watcher (no new watcher subsystem — one more path registered on
  infrastructure that already exists and is already proven). On a detected
  change: read the file, call a new `ConfigState::update_browser_start_page`,
  then broadcast — reusing a small shared `broadcast_full_config(config_watcher,
  event_bus)` helper factored out of `config_watcher_fs.rs`'s existing
  `reload_and_broadcast` (today duplicated once already, in the `setconfig`
  handler at `websocket.rs:1736-1752` — factoring it out fixes that
  duplication too rather than adding a third copy).

**`agentmux-srv/src/backend/wconfig/state.rs`** — one new method on
`ConfigState`, mirroring `update_settings` exactly:

```rust
pub fn update_browser_start_page(&self, url: Option<String>) {
    let mut current = self.config.write().unwrap();
    let mut new_config = (**current).clone();
    new_config.browser_start_page = url;
    *current = Arc::new(new_config);
}
```

**`agentmux-srv/src/bootstrap.rs:1048-1057`** — two lines added right next
to the existing settings load/watch calls, same `fs_watch_pool`/
`config_watcher`/`event_bus` already in scope there:

```rust
backend::browser_start_page::load_start_page_from_disk(&config_watcher);
// ... existing spawn_settings_watcher call ...
backend::browser_start_page::spawn_start_page_watcher(
    fs_watch_pool.clone(),
    config_watcher.clone(),
    event_bus.clone(),
);
```

**New RPC — write side only, no read side.** `.get` is unnecessary: the
value is always already present in `FullConfigType`.

| Constant | Command string | Request | Response |
|---|---|---|---|
| `COMMAND_BROWSER_START_PAGE_SET` | `browser_start_page.set` | `{ "url": string }` | `{ "url": string }` |

The handler mirrors `setconfig`'s own four-step immediate-update pattern
(`websocket.rs:1703-1752`) exactly, rather than writing to disk and waiting
on the fs watcher's debounce (~300ms, per that file's own comment) to
notice:

1. Write `url` to `browser-start-page.json` (fails loudly on error — same
   "don't tell the caller it saved when it didn't" posture bookmarks'
   `.set` already has).
2. `config_watcher.update_browser_start_page(Some(url))` — update in-memory
   state immediately.
3. Broadcast the updated `FullConfigType` **now**, via the same
   `broadcast_full_config` helper §3.2 above factors out — every connected
   window, not just the one that clicked, sees the new value within one
   broadcast, not one file-watcher debounce cycle.
4. The fs watcher (`spawn_start_page_watcher`) still exists and will also
   fire on this same write and re-broadcast — harmless and redundant by
   design, same as `setconfig`'s own comment: `"fs watcher will
   re-broadcast, harmlessly"`. It's not dead weight, either — it's what
   makes an **external** edit to `browser-start-page.json` (a different
   AgentMux instance, or a hand-edit) picked up and broadcast too, not just
   RPC-driven writes.

`redact_full_config_for_renderer` (`agentmux-srv/src/backend/wconfig/redact.rs:39-56`)
needs no change — it redacts a denylist of keys nested inside the
`settings` sub-object specifically; a new top-level, non-secret field
passes through untouched, confirmed by reading its implementation.

### 3.3 Frontend changes

**`frontend/app/store/config-signals.ts`** — one new derived signal,
identical shape to `settingsAtom` on the line right above it:

```ts
export const browserStartPageAtom = createMemo<string | null>(
    () => fullConfigAtom()?.browser_start_page ?? null,
);
```

No new subscription code anywhere: `fullConfigAtom` is already populated
before first render (`app-init.ts:1122`) and already re-set on every
`WpsEvent.Config` push (`global.ts:242-246`) — `browserStartPageAtom`
inherits both properties for free, exactly like every other config-derived
memo in that file already does.

**`browser-model.ts`**'s constructor (§2) becomes:

```ts
const initialUrl =
    ((meta?.["url"] as string | undefined) ?? "").trim()
    || browserStartPageAtom()
    || DEFAULT_BROWSER_URL;
```

`meta.url` (an explicit URL this specific pane was asked to open) still
wins; the configured start page is the new middle fallback; the hardcoded
constant becomes purely the before-anyone-has-ever-set-a-start-page
bootstrap value, never removed.

**`browser-nav-bar.tsx`**'s `bookmarkMenuItems` (`:285-329`) gains a new
pinned row **before** the existing bookmark-toggle row, same gating
(`model.urlAtom()` non-empty):

```ts
if (model.urlAtom()) {
    items.push({
        label: "Set as Start Page",
        icon: "house",
        checked: model.urlAtom() === browserStartPageAtom(),
        onClick: setCurrentAsStartPage,
    });
    items.push({
        label: currentBookmark() ? "Remove Bookmark" : "Bookmark This Page",
        icon: "star",
        onClick: toggleCurrentBookmark,
    });
}
```

Order: **Set as Start Page → Bookmark This Page/Remove Bookmark → divider
→ saved bookmarks (or "No bookmarks yet")** — first entry in the menu, as
asked, with the two pinned action rows sitting together above the divider
that already separates actions from the saved list.

`checked: true` reuses `MenuItem.checked` — a field the bookmarks spec's
own UI section named as available for exactly this ("a check/star state on
the pinned first row") but never used. `FlyoutMenu`'s existing renderer
already draws a check glyph for `checked: true`
(`frontend/types/custom.d.ts:463-477`) — no new rendering code.

`setCurrentAsStartPage` — an optimistic local update for instant feedback
in the clicking window (mirroring `toggleCurrentBookmark`/
`persistBookmarks`'s existing pattern at `browser-nav-bar.tsx:241-271`),
**backstopped by the live broadcast in §3.2** for every other window
rather than being the only mechanism either window has:

```ts
const [startPageError, setStartPageError] = createSignal<string | null>(null);

const setCurrentAsStartPage = async () => {
    const url = model.urlAtom();
    if (!url) return;
    try {
        await RpcApi.SetBrowserStartPageCommand(TabRpcClient, { url });
        // No local optimistic write needed beyond this: the RPC handler
        // broadcasts the updated FullConfigType before responding (§3.2
        // step 3), so fullConfigAtom() — and therefore
        // browserStartPageAtom() — is already current by the time this
        // await resolves, in THIS window and every other open one.
    } catch (e) {
        setStartPageError(`Failed to save start page: ${(e as Error).message ?? e}`);
    }
};
```

Failure surfaces via the same tooltip/tint slot bookmark-save failures
already use on the bookmarks button (`bookmarksError()`,
`browser-nav-bar.tsx:381,385`), extended to also reflect
`startPageError()`.

## 4. Unhappy paths

| Scenario | Behavior |
|---|---|
| **No start page ever set** | `browser_start_page` is absent/`null` in `FullConfigType`; `browserStartPageAtom()` is falsy; every new pane falls through to `DEFAULT_BROWSER_URL`, unchanged from today. |
| **Active tab has no URL yet** | Row omitted entirely, same handling as the existing bookmark row. |
| **`shared_dir` can't be resolved at boot** | `load_start_page_from_disk` leaves `browser_start_page: None` (best-effort, matching `bookmarks.list`'s empty-list fallback) rather than failing srv startup. |
| **`.set` write fails** (disk full, permissions) | RPC returns an error; no in-memory update, no broadcast — the caller's error surfaces via the tooltip, never told it saved when it didn't. |
| **`browser-start-page.json` exists but is corrupt** | `load_start_page_from_disk` logs and keeps `None` (same "keep previous config" posture `reload_and_broadcast` already has for a corrupt `settings.json` — startup must not fail over this) rather than crashing boot. |
| **Two windows set different start pages near-simultaneously** | Last write wins at the file level (no merge) — same accepted risk every single-JSON-file store in this app has. Unlike the first-draft design, **both windows converge on the same value almost immediately** via the broadcast in §3.2 step 3, rather than staying stale until reopened — this is the concrete robustness improvement over bookmarks' current accepted limitation. |
| **An external process/instance edits `browser-start-page.json` directly** | Picked up by `spawn_start_page_watcher` (the fs-watcher backstop) and broadcast to every connected window, same as an external `settings.json` edit already is today. |
| **User wants to go back to the hardcoded default after setting one** | Out of scope for v1, matching the "start smallest" discipline the bookmarks spec applied to per-row delete. A future "Reset to Default" row is a small addition given the plumbing above already exists (`SetBrowserStartPageCommand` with `url: null`/empty would need one extra branch, not new infrastructure). |

## 5. Non-goals for v1

- A visible "current start page" indicator anywhere outside this menu.
- Per-channel or per-window start pages — global via `shared_dir`, same
  scope decision as bookmarks.
- Clearing/resetting the start page (see table above).
- Multiple/named start pages or session-restore ("open N tabs on start").

## 6. Testing

- **Backend:** `browser_start_page.rs` unit tests mirroring
  `bookmarks_store.rs`'s existing suite (missing file → `None`, empty file
  → `None`, write-then-read round-trip, missing parent dirs created,
  corrupt file → `Err` on load, kept-previous-config on reload parse
  error). Plus: `ConfigState::update_browser_start_page` sets exactly that
  field and leaves the rest of the snapshot untouched (mirrors an existing
  `update_settings` test if one exists, same shape). Plus: an integration-
  style test that `spawn_start_page_watcher` detects a write and the
  resulting `get_full_config()` reflects it (mirrors the existing settings-
  watcher test at `fs_watch::pool::tests::a_change_event_is_observable_on_the_broadcast_stream`
  in spirit).
- **Frontend:** `bookmarkMenuItems` row order (start page first, then
  bookmark toggle, then divider) and `checked` state when
  `model.urlAtom() === browserStartPageAtom()`; `browser-model.ts`'s
  initial-URL fallback chain (`meta.url` > `browserStartPageAtom()` >
  `DEFAULT_BROWSER_URL`) as three explicit cases.
- **Manual/CDP verification** (per this repo's own precedent — the
  bookmarks feature's live-favicon follow-up was "live-verified via CDP
  against a running `task dev` instance," not assumed from unit tests
  alone): open two windows, set a start page in one, confirm the other's
  `checked` state updates without reopening the menu, and confirm a
  freshly-opened pane in either window honors it.

## 7. References

- `frontend/app/view/browser/browser-nav-bar.tsx` (menu construction,
  optimistic-write pattern)
- `frontend/app/view/browser/browser-model.ts` (`DEFAULT_BROWSER_URL`,
  constructor's initial-URL resolution)
- `agentmux-srv/src/backend/bookmarks_store.rs` (file read/write pattern
  mirrored by the new module)
- `agentmux-srv/src/backend/wconfig/{types,state}.rs`,
  `agentmux-srv/src/backend/config_watcher_fs.rs`,
  `agentmux-srv/src/server/websocket.rs:1703-1752` (`setconfig` handler —
  the immediate-update-then-broadcast pattern this spec's `.set` handler
  mirrors)
- `agentmux-srv/src/bootstrap.rs:1048-1057` (boot wiring, where the two new
  calls are added)
- `frontend/app/store/config-signals.ts`, `frontend/app/store/global.ts:242-246`
  (`fullConfigAtom`'s boot-time population and live `WpsEvent.Config`
  update — the mechanism this spec rides instead of building a parallel one)
- `docs/specs/SPEC_BROWSER_PANE_BOOKMARKS_AND_GO_ICON_2026_08_22.md` (design
  ancestor — persistence-location reasoning, `FlyoutMenu` reuse, and the
  accepted-staleness limitation this spec's revised design deliberately
  improves on)
- `docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md` (why
  channel isolation is per-file, not per-key — the reasoning that keeps a
  `shared_dir`-sourced field inside `FullConfigType` global despite
  `settings` itself being isolated)
