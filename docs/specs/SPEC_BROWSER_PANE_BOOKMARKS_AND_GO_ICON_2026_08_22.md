# SPEC — Browser pane: bookmarks (design exploration) + Go-button icon (quick tweak)

**Date:** 2026-08-22
**Type:** Two asks bundled in one doc — (1) a small presentational fix, ready to
implement; (2) a design exploration for a bigger feature, **not yet approved
for implementation**.
**Status:** Draft
**Scope:** `frontend/app/view/browser/` (nav bar, view model), `frontend/app/store/browser-pane-state/`, potentially `settings.json`/`agentmux-srv` config plumbing for part 2.

---

## Part 1 — Replace the "Go" text button with an icon (ready to implement)

### Current state

`BrowserNavBar` (`frontend/app/view/browser/browser-nav-bar.tsx:117-193`) renders
four controls: Back (`←`), Forward (`→`), Reload (`↻`) — all single-glyph,
icon-only, with a `title=` tooltip for accessibility — and then a text-labeled
`Go` button (line 190):

```tsx
<button class="browser-nav-btn browser-go-btn" onClick={handleNavigate}>Go</button>
```

This is already the outlier in its own row: three icon buttons and one text
button. `browser-view.scss:54-58` gives it bespoke sizing (`width: auto; padding: 0 10px`)
that the other three buttons don't need (fixed 28×28, `browser-view.scss:31-52`).

The app's general icon convention is FontAwesome via a shared helper
(`clsx(\`fa fa-solid fa-${icon}\`, ...)`, `frontend/util/util.ts:60`), used
throughout menus, the command palette, file tree, etc. Back/Forward/Reload
predate that convention and use raw Unicode glyphs instead — not a pattern
worth extending, just a pre-existing inconsistency. FontAwesome's CSS is
already bundled (`public/fontawesome/css/fontawesome.min.css`), so using it
here costs nothing new.

### What icon

The address bar in this app is an omnibox, not a pure search box:
`handleNavigate` (`browser-nav-bar.tsx:55-80`) and `BrowserViewModel.navigate`
(`browser-model.ts:540-568`) both treat the input as a URL when it looks like
one and fall back to a Google search only otherwise. Icon research (NN/g and
general UI-icon guidance) draws a consistent line between the two available
metaphors: a magnifying glass signals "search," an arrow signals "navigate /
submit to a destination." Since navigation is the primary action here even
in the search-fallback case, an arrow reads correctly in both cases and a
magnifying glass would mis-set expectations for the common URL case.

**Recommendation:** `fa-solid fa-arrow-right` (a plain right-pointing arrow).
Keep `title="Go"` (or `"Navigate"`) on the button — icon-only affordances
need a discoverable label for users who haven't memorized the icon yet, and
this repo's own Back/Forward/Reload already establish `title=` as the
convention for exactly that.

### Change

- `browser-nav-bar.tsx:190`: swap the text child for an `<i class="fa fa-solid fa-arrow-right" />`, add `title="Go"`.
- `browser-view.scss:54-58` (`.browser-go-btn`): drop the `width: auto; padding: 0 10px; font-size: 12px` overrides so it falls back to the shared 28×28 `.browser-nav-btn` sizing, matching the other three buttons.

### Risk

Presentational only — no model/state/IPC changes. Safe as a standalone, tiny
PR, independent of Part 2 below.

---

## Part 2 — Bookmarks (exploration only — not a commitment to build)

### Why this section is "explore," not "build"

This exact repo already tried a bookmark feature once and removed it —
`docs/specs/SPEC_REMOVE_BOOKMARKS_2026_06_11.md` (2026-06-11), a "bookmark
this message" feature in the agent-pane feed. It was deleted for two
reasons: (1) it hijacked a right-click surface users expected to show the
standard pane context menu (split/float/close), silently hiding those
actions; (2) actual usage never justified the added surface area (a panel,
a hook, a keyboard shortcut, a per-row visual indicator, all for one
feature). That was a different kind of bookmark (annotating a chat message,
not saving a URL) so the lesson isn't "don't build bookmarks" — it's
**don't take over an existing interaction surface, and start at the
smallest useful surface area, not the fullest one.** Both constraints shape
the proposal below.

### Relevant current architecture

- **The nav bar is real DOM**; the page content area is not. Per
  `docs/specs/SPEC_NATIVE_BROWSER_PANE_2026_04_17.md`, a browser pane's
  content is a native `CefBrowserView` overlay, not a DOM subtree — that's
  why right-clicking *inside a page* still shows Chromium's own native menu
  today (`docs/specs/SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md`
  is a still-unimplemented Draft proposing to fix that). A bookmark toggle
  belongs in the nav bar, which is ordinary DOM (`browser-nav-bar.tsx`) —
  no native-overlay complications, unlike anything scoped to page content.
- **`showControlsAtom`** (`browser-model.ts:355`) hides the entire nav bar
  for widget-style panes — `agentmux-srv/src/config/widgets.json` sets
  `"browser:show_controls": false` for the Discord/Slack/Telegram/WhatsApp
  widgets (lines ~139-188). A bookmark button must respect the same flag:
  no bookmark UI on those panes, consistent with hiding back/forward/reload
  there today.
- **Multi-tab state already exists in the reducer** but not yet in the UI.
  `frontend/app/store/browser-pane-state/types.ts` defines a full
  `BrowserTab` (`url`, `title`, `faviconUrl`, per-tab loading/history state)
  and the reducer already handles `OpenTab`/`CloseTab`/`SwitchTab`/etc. —
  but `browser-view.tsx` renders no tab strip yet (per comments in
  `browser-model.ts`, e.g. "Phase 1A: diagnostic-only... Phase 1B's tab
  strip lands..."). Practically: today one pane = one URL, so a bookmark
  button can read `model.urlAtom()` / `model.titleAtom()` /
  `model.faviconUrlAtom()` directly. Building it against the model's
  existing accessors (rather than something pane-global) means it keeps
  working unchanged once a tab strip ships and "current URL" becomes
  per-active-tab.
- **`recentlyClosed`** (`browser-pane-state/types.ts:88-93`,
  `{url, title, closedAt}`, capped at 10) is the closest existing shape to a
  bookmark record, but it's per-pane, in-memory reducer state — it disappears
  on pane close and was never meant to be shared globally. Confirms bookmarks
  need their own, separate, global store — nothing existing already covers it.

### Where would bookmarks live? (persistence — the key open decision)

**`settings.json` is the wrong home, on reflection.** It was the first
instinct (see the now-superseded settings-blob option below) because it's
the lightest-weight precedent for "an ordered list of small records," via
`"widget:pinned"` (`frontend/app/window/action-widgets-config.ts:37,109`,
written through the generic `RpcApi.SetConfigCommand`). But `settings.json`
isolation is **whole-file, not per-key** — per
`docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md` §6, per-key
tiering ("some settings global, some isolated") was explicitly considered
and rejected: *"the whole-file isolation the auth precedent uses is
simpler, has no 'did we remember to list this key' failure mode."* There is
no supported way to make one key in `settings.json` behave differently from
the rest of the file. Since we specifically want bookmarks to stay global
(including on `dev-<branch>`/`local-*` channels, for a usable dev
experience — losing your bookmark list every time you `task dev` a new
branch defeats the point of the feature), putting them in `settings.json`
means fighting the file's own design principle, not using it.

**Better fit: `shared_dir` — the mechanism this codebase already uses for
"must always be global, regardless of channel."** `DataPaths.shared_dir`
(`agentmux-common/src/data_paths.rs:122,215`, resolved once as
`root.join("shared")`, exported as `AGENTMUX_SHARED_DIR`) sits as a
**sibling** of `channels/`/`dev/`/`versions/` — no channel switch or
isolation flag touches it, because it isn't reached through
`instance_dir`/`config_dir` at all. This is exactly what already backs the
things in this app that are deliberately global no matter what channel or
isolation setting is active:

- Agent registry — `~/.agentmux/shared/agents/registry/<uuid>.json` (`docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`)
- Conversation transcripts (same doc)
- Provider auth — `provider_auth_dir()` (`data_paths.rs:445`, `shared_dir.join("providers").join(...)`) — CLAUDE.md notes this "always resolves auth... regardless of isolation," the same guarantee bookmarks want

**Recommendation:** store bookmarks as their own small file under
`shared_dir` — e.g. `~/.agentmux/shared/browser-bookmarks.json`, a flat JSON
array of `{id, title, url, faviconUrl}` — resolved independently of
`resolve_settings_dir()`/channel logic entirely, not a `settings.json` key.
Concretely this needs: a tiny new read/write helper (mirrors
`provider_auth_dir()`'s shape, not a full sqlite migration), and either (a)
a couple of new RPC commands (`bookmarks.list`/`bookmarks.set`, following
the same thin-wrapper shape as `bundle.rs`'s registration but pointed at
this file instead of `db_bundles`), or (b) — if the generic config RPC can
be taught to accept a path override — reusing `SetConfigCommand`'s wire
shape but not its storage target. Either way it's still much lighter than
the ABF/`db_bundles` route (no migration, no versioned table, no
import/export machinery) while actually satisfying "global including
during dev," which the settings-blob route cannot.

The ABF/`db_bundles` route (full sqlite table + migration + `Store` methods
+ dedicated RPC file, e.g. `agentmux-srv/src/backend/storage/migrations.rs:1290`,
`memory_bundles.rs`, `app_api/bundle.rs`) remains the fallback if bookmarks
later grow folders/nesting, per-record versioning, or import/export — not
needed for v1.

One thing to flag explicitly: `shared_dir`-backed bookmarks would be global
on **every** channel, including `stable` — there's no "global in dev, still
somehow scoped in prod" middle ground in this mechanism, any more than
provider auth has one. That matches what a personal bookmark list should
probably do anyway (nobody wants a separate bookmark set per release
channel), but it's worth confirming that's the intended behavior, not an
accidental side effect of picking this mechanism.

### Proposed UI (v1 scope, if approved)

**One button, one menu — not a separate star + dropdown.** A single
bookmarks button sits in `browser-nav-bar.tsx` between Reload and the
address bar (`browser-nav-bar.tsx:132-136`, right before the `<input
class="browser-address-bar">` at line 137). Clicking it opens a menu
containing:

1. A pinned first row — "★ Bookmark this page" / "★ Remove bookmark"
   (label/state flips based on whether the active tab's URL is already
   saved) — acting on `model.urlAtom()`/`titleAtom()`/`faviconUrlAtom()`.
2. A separator, then every saved bookmark as a menu item (favicon → icon,
   title → label, click → `model.navigate(url)` + close the menu).

**Reuse `FlyoutMenu` directly — do not build a new menu component.**
`frontend/app/element/flyoutmenu.tsx` is the same primitive already behind
the hamburger menu, the widget bar's "More" dropdown, and right-click
context menus, so using it here is a DRY win, not just a style match:

- Its `MenuItem` shape (`frontend/types/custom.d.ts:463-477`) already
  supports `icon`, `label`, `onClick`, `divider` (for the separator above),
  and `checked` (could show a check/star state on the pinned first row) —
  no new prop shape to invent.
- **"Expands to the bottom of the window, then scrolls" is already built
  in, not something to implement.** `FlyoutMenu`'s positioning goes through
  `computeMenuPosition` (`frontend/app/util/menu-position.ts:228-296`),
  whose `size()` middleware measures `availableHeight` down to the window's
  paintable-area boundary and returns it as `maxHeight`
  (`menu-position.ts:279`); `styleToString`
  (`frontend/app/element/flyoutmenu.tsx:38-44`) applies that as
  `max-height:${maxHeight}px; overflow-y:auto` on the menu's own root. Every
  existing `FlyoutMenu` consumer already gets "grow until it hits the
  window edge, then scroll internally" for free — the bookmarks menu needs
  zero new positioning/scrolling code, only a long-enough item list to
  actually exercise it.
- Horizontal overflow (menu opened near the window's right edge) and
  avoiding native browser-pane overlays are likewise already handled by the
  same `flip()`/`shift()`/`avoidNativePanes` machinery every other
  `FlyoutMenu` menu gets (`menu-position.ts`, `flyoutmenu.tsx:100-111`) —
  nothing bookmarks-specific to add there either.

**No folders, tags, or import/export in v1.** A flat, most-recent-first (or
manually reorderable) list. This deliberately avoids the classic
"bookmarks bar overflows after ~10 items" trap current browser-UX critiques
call out (a flat scrollable dropdown — see above — scales considerably
further before folders become necessary) and avoids re-adding the
surface-area-without-proven-value problem the removed feature ran into.
`MenuItem.subItems` (nested submenus) stays available on the type if
folders are ever added later, but nothing in v1 uses it.

**Respect `showControlsAtom`** — no bookmark button at all on widget-style
panes (Discord/Slack/Telegram/WhatsApp), matching existing nav-bar
visibility for back/forward/reload/address-bar.

### Unhappy paths

Explicitly worked through per-scenario, since "reuse the existing menu"
answers the layout/scrolling questions but not the data/state ones:

| Scenario | Behavior |
|---|---|
| **Zero bookmarks saved** | Menu opens with just the pinned "Bookmark this page" row and no separator/list below it (an empty list under a separator would read as a rendering glitch, not "you have none yet") — show a muted "No bookmarks yet" row instead of an empty gap. |
| **Hundreds of bookmarks** | `FlyoutMenu`'s internal scroll (above) keeps the menu usable, but a plain scroll through hundreds of unfiltered rows is a real UX cliff — out of scope for v1 (flat list is the explicit v1 boundary), but worth a documented threshold: if usage data shows this happening, the next increment is a filter/search input pinned above the scrollable list, not folders. |
| **Bookmarking the same URL twice** | Toggle, not append-only: the pinned row's click handler checks whether the active URL already has an entry (by exact URL match) and removes it instead of inserting a duplicate. Prevents an ever-growing list of identical entries from repeated toggling. |
| **Very long title or URL** | Menu items must truncate with an ellipsis (existing `.menu-item .label` CSS likely already does this for other long menu labels — verify, don't assume, during implementation) rather than wrapping and blowing out the fixed-width menu. |
| **Favicon fails to load / bookmark predates a favicon** | Fall back to the existing globe icon convention already used elsewhere in the browser pane (`viewIcon = () => "globe"`, `browser-model.ts:89`) — never a broken-image glyph in the menu. |
| **Two windows/panes editing bookmarks concurrently** | v1 has no live cross-window sync (see persistence section — no wave-event broadcast, matching `MemoryViewModel`'s and `GlobalBrainViewModel`'s existing "does not subscribe... the manager is the only writer in practice" precedent). A menu opened before another window's edit lands shows a stale snapshot until closed and reopened. Acceptable for personal-bookmark scope; flagged here rather than silently assumed away. |
| **Two windows write at the exact same instant** | Last write wins (whole-file overwrite, no merge) — the earlier window's edit can be silently lost. Same accepted risk class as any other single-JSON-file store in this app; not solved by this feature, just inherited. |
| **`shared_dir` can't be resolved** (unusual/CI environment, per `providers_handlers.rs`'s own `DataPaths::from_env()` fallback precedent) | `bookmarks.list` returns an empty list rather than erroring the whole nav bar; the bookmark-toggle action should visibly fail (e.g. a toast/error, not a silent no-op) so the user isn't left thinking their bookmark saved when it didn't. |
| **Bookmarks file exists but is corrupt/unparseable JSON** | Fail loud on `bookmarks.list` (surface the parse error) rather than silently discarding the user's saved list — matches this codebase's general preference for warning over silent data loss (e.g. the ABF import path's warning-budget conventions) over quietly resetting to empty, which would look like the data vanished. |
| **Active tab has no URL yet** (blank/fresh pane) | Originally planned as "disabled, not hidden" — reverted during implementation once it turned out `MenuItem` has no `disabled` field at all. The row is omitted entirely in this case instead (falls back to "No bookmarks yet" if the list is also empty); rare and brief in practice since panes navigate to `DEFAULT_BROWSER_URL` almost immediately. |
| **Menu open, then the pane itself closes/navigates away underneath it** | `FlyoutMenu`'s existing outside-click-to-close (`flyoutmenu.tsx:126-133`) and Solid's unmount-on-dispose already tear the menu down; no bookmarks-specific handling needed, but worth a manual check during implementation since this menu is anchored inside a per-pane component, unlike the window-global hamburger menu. |
| **Keyboard-only user** | `FlyoutMenu` itself has no `Escape`-to-close handler today (only outside-click) — confirmed by grepping for `Escape` handling in `frontend/app/element/`, which turns up `modal.tsx` and `popover-menu.tsx` but not `flyoutmenu.tsx`. This is a pre-existing gap shared by every current `FlyoutMenu` consumer (hamburger menu, widget bar, context menus) — not something to silently fix as a side effect of adding bookmarks, but worth flagging explicitly since a keyboard user has no way to dismiss this menu without a mouse click today. Candidate follow-up, not a v1 blocker.

### Non-goals for v1 (explicitly out of scope)

- Folders/tags — defer until a flat list is demonstrably insufficient.
- Import/export (e.g. Chrome bookmarks HTML).
- Cross-device sync via muxbus — no existing channel carries user data like
  this today; out of scope until one does.
- Bookmarking inside the widget-style chat panes (Discord/Slack/etc.) — nav
  chrome is hidden there by design.

### Sequencing

**Status: both parts implemented (2026-08-22).** `cargo test -p agentmux-srv`
(2647 tests) and the frontend `vitest` suite (2855+ tests, plus a clean
`tsc --noEmit`) pass. Not yet manually verified in a running app (`task
dev` + clicking through the feature) — typecheck/unit-test coverage only.

1. ~~Ship Part 1 (Go-button icon).~~ Done —
   `frontend/app/view/browser/browser-nav-bar.tsx`,
   `frontend/app/view/browser/browser-view.scss`.
2. ~~Backend: `~/.agentmux/shared/browser-bookmarks.json` read/write helper
   + `bookmarks.list`/`bookmarks.set` RPC commands.~~ Done —
   `agentmux-srv/src/backend/bookmarks_store.rs`,
   `agentmux-srv/src/server/app_api/bookmarks.rs`, constants in
   `rpc_types/commands.rs`, registered in `app_api/mod.rs`.
3. ~~Frontend: RPC wrapper, bookmarks button + `FlyoutMenu` wiring.~~ Done —
   `frontend/app/store/rpc-api/bookmarks.ts`, spliced into `RpcApi`
   (`rpc-api/index.ts`); toggle/dedupe logic extracted to the pure, unit-
   tested `frontend/app/view/browser/browser-bookmarks-logic.ts`
   (`toggleBookmark`/`findBookmark`); wired into `browser-nav-bar.tsx`.

**Deviations from the plan above, decided during implementation:**

- **Live favicon thumbnails, added as a same-day follow-up (PR after
  #2730).** Originally deferred — `FlyoutMenu`'s default item renderer only
  supports a FontAwesome icon-name string, and `item.icon`'s `JSX.Element`
  branch (`custom.d.ts:465`) had no existing consumer to confirm the
  pattern. Revisited on request: `browser-nav-bar.tsx` now passes a
  `renderMenuItem` override on this menu's `<FlyoutMenu>` — it renders
  `item.icon` as-is when it's a `JSX.Element` (a small `<BookmarkFavicon>`
  component wrapping the real `<img src={favicon_url}>`, falling back to a
  plain globe icon on a missing URL or an `onError` load failure) and falls
  back to the original `fa-solid fa-fw fa-${icon}` rendering for string
  icons (the pinned toggle row, loading/empty-state rows). Scoped
  deliberately to only this menu's icon slot — `checked`/`shortcut`/
  `subItems` aren't replicated since nothing here uses them yet. Live-
  verified via CDP against a running `task dev` instance: a real bookmark
  row renders `<img src="https://agentmux.ai/favicon.ico">`.
- **No per-row delete for a bookmark other than the current page.**
  `MenuItem` has no secondary-action/trailing-button slot, and nesting a
  submenu (`subItems: [Open, Remove]`) per row would replace "click to
  navigate" with "hover to reveal, then click" as the primary interaction —
  a worse tradeoff for the common case. v1 supports removing a bookmark
  only by revisiting that URL and toggling it off again. A real, accepted
  limitation for this pass — same "start smallest" discipline as the
  no-folders decision above, not an oversight.
- **No native "disabled" state for the pinned toggle row.** `MenuItem` has
  no `disabled` field, so the originally-planned "Bookmark this page" row
  is disabled-with-reason when the active tab has no URL yet — instead,
  the row is omitted entirely in that case (falls back to "No bookmarks
  yet" if the list is also empty). In practice this state is rare and
  brief (every pane navigates to `DEFAULT_BROWSER_URL` almost immediately
  after construction).
- **Bookmark-save failures surface via the button itself** (a red icon
  tint + the failure text in the button's `title` tooltip), not the page
  error banner in `browser-view.tsx` — that banner is `model.errorAtom()`,
  scoped to page-load failures and cleared on the next navigation; a
  bookmark-save failure is a different lifecycle and would look wrong
  riding along on it.

---

## References

- `frontend/app/view/browser/browser-nav-bar.tsx`
- `frontend/app/view/browser/browser-model.ts`
- `frontend/app/view/browser/browser-view.scss`
- `frontend/app/store/browser-pane-state/types.ts`
- `frontend/app/window/action-widgets-config.ts` (`widget:pinned` precedent — superseded for this feature, see persistence section)
- `frontend/app/element/flyoutmenu.tsx`, `frontend/app/element/popover-menu.tsx`
- `frontend/app/util/menu-position.ts` (`computeMenuPosition` — the grow-to-window-edge-then-scroll primitive `FlyoutMenu` already uses)
- `frontend/types/custom.d.ts` (`MenuItem` shape)
- `frontend/util/util.ts` (FontAwesome icon-class helper)
- `agentmux-srv/src/server/providers_handlers.rs` (`DataPaths::from_env()` best-effort fallback precedent)
- `agentmux-srv/src/config/widgets.json`
- `agentmux-common/src/data_paths.rs` (`shared_dir`/`AGENTMUX_SHARED_DIR`, `provider_auth_dir()` — the recommended persistence precedent)
- `agentmux-srv/src/backend/storage/migrations.rs`, `memory_bundles.rs`,
  `agentmux-srv/src/server/app_api/bundle.rs` (ABF/`db_bundles` — fallback route only)
- `docs/specs/SPEC_REMOVE_BOOKMARKS_2026_06_11.md` (prior-art / lesson learned)
- `docs/specs/SPEC_NATIVE_BROWSER_PANE_2026_04_17.md`
- `docs/specs/SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md`
- `docs/specs/SPEC_SETTINGS_ISOLATED_BY_CHANNEL_2026_08_19.md` (why settings.json doesn't fit — whole-file-only isolation, no per-key tiering)
- `docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` (`shared_dir` precedent for agent registry/transcripts)
- Web research: NN/g on magnifying-glass-vs-label search affordances
  (nngroup.com/articles/magnifying-glass-icon); browser bookmarks-bar UX
  critique (bookmarker.cc/blog/browser-bookmark-bar-alternative)
