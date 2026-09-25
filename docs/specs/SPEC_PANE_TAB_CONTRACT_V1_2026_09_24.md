# SPEC: Pane Tab contract v1 — one general interface for every pane tab (and future user-loaded widgets), starting with the Help "ghost" fix

**Date:** 2026-09-24
**Status:** active — Phase 0 (§1.5, the Help ghost) implemented in PR #3723;
per-tab keep-alive (§5, decided) in PR #3725; Phases 1–6 not started.
**Author:** Camper
**Trigger:** repo owner, 2026-09-24: "the help pane tab, when going away, the
help content lingers and goes away like a ghost. sounds like it could be a bad
architectural issue with the help pane tab's interface. investigate how clean
our interface for pane tabs is — we likely eventually will have a widget
library where users can load custom pane tabs that appear as widgets, so we
want the interface general."
**Grounded in:** `main` at `f5233c0cc` (after #3714). Read from code; the Help
ghost's mechanism is verified in code but has not been reproduced with
DevTools. See §1.4.
**Related:** `SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md` (the pane-tab
redesign), `SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md` (keep-alive),
`SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md`,
`SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md`, and #3714
(per-tab ViewModel slots, `viewNameIsPlaceholder`, browser dormant-hide). That
PR fixed the last round of "one tab's state leaks into another" bugs, which
had the same root shape as §2.4.

---

## 0. Summary

- **The Help ghost is not a Help bug.** It comes from the generic way a
  pane hides an inactive tab. Once a pane has shown an agent or terminal
  tab, it keeps *every* tab mounted and hides the inactive ones with
  `visibility: hidden` alone. Content with a CSS transition on `visibility`
  (QuickTips uses `transition-all`) stays visible for the transition's
  duration. §1 gives the root cause and a two-line fix plus one general fix.
- **The interface a view implements to be a pane tab is implicit,
  scattered and partly hard-coded.** It's spread across the `ViewModel`
  grab-bag type, a static view registry, three hand-maintained view-name
  sets, the `PaneTabDescriptor` side-registry, `PaneChromeModel`, and
  view-name checks in shared code. Several real bugs follow directly from
  that (§2.4).
- **Proposal:** an explicit, versioned **Pane Tab contract v1** (§3): a
  manifest plus an instance with host-driven lifecycle. That's the
  interface a future widget library would load third-party pane tabs
  through. It lands in phases (§4) behind a legacy adapter, so existing
  views keep working unchanged while they migrate.

---

## 1. The Help "ghost"

### 1.1 What Help is
Plain DOM. `HelpViewModel` sets only `viewType`, `renderPaneChrome` and
`noHeader` (`frontend/app/view/helpview/helpview.tsx:23-42`). `HelpView` is a
scrollable div around `<QuickTips />` with CSS `zoom` (`:87-101`). No iframe,
no webview, no native surface, no browser-pane code.

### 1.2 How a pane switches tabs (two paths, `frontend/app/tab/pane-leaf-chrome.tsx`)
1. **Remount path**, the default: only the active tab's `<Block>` is mounted,
   keyed by block id, so a switch unmounts the old tab immediately. No ghost
   is possible here.
2. **Keep-alive path**: `keepAlive()` latches **for the whole pane**, for
   good, the first time the pane's *active* view is `term` or `agent`
   (`KEEP_ALIVE_TYPES`, `:101`; latch at `:144-150`). From then on
   **every** tab of the pane stays mounted, Help included. The slots are
   stacked with `position:absolute; inset:0`, and an inactive one is hidden
   only by `visibility: hidden; pointer-events: none` (`:404-405`).

### 1.3 Root cause
- QuickTips' four section cards use Tailwind `transition-all duration-300`
  (`frontend/app/element/quicktips.tsx:101, 143, 220, 253`). These are the
  only four `transition-all` uses in `frontend/app`.
- `visibility` is animatable. When it transitions from `visible` to
  `hidden`, the computed value stays `visible` until the transition ends,
  and a transition starts on an *inherited* change too. So when the slot
  flips to `visibility:hidden`, each card keeps `visibility:visible` for
  300 ms, and so do its children, which inherit from it. It's drawn over or
  under the newly active tab, then vanishes: the "ghost".
- **Why it looks like a Help problem:** Help is the one view whose content
  uses `transition-all`. Any view with `transition: all` inside a dormant
  keep-alive slot would do the same, e.g. `.wave-button` (`button.scss:26`,
  0.3 s) and several agent styles.
- **The same mechanism applies to window tabs.** Inactive window tabs are
  hidden the same way by default (`visibility:hidden`,
  `window-tab-visibility.ts:66-77`, the default since #3686), stacked
  absolutely (`workspace.tsx:228-229`). So the same ghost likely appears on
  window-tab switches (hypothesis).

### 1.4 How to confirm
- **Expected:** the ghost appears **only** in panes that have shown an
  agent or terminal tab (the keep-alive path). A pane holding only Help,
  Browser, Editor and so on (the remount path) should never show it.
- **DevTools check:** the Animations panel should show `visibility`
  transitions on the QuickTips cards during the switch.
- **If it's a smooth opacity fade rather than a lingering-then-cut:** look
  for a second, overlapping cause before closing this out.

### 1.5 Fix (Phase 0, small, ships first)
1. **The general fix.** A hidden tab must not be able to paint, whatever its
   content's CSS does. Add `opacity: 0` to the inactive keep-alive slot
   (`pane-leaf-chrome.tsx:398-406`: `opacity: id === activeBlockId() ? "1" : "0"`).
   Opacity on a container can't be escaped by descendants, not by a
   `visibility` transition and not by an explicit `visibility: visible`.
   The slot keeps its full size, so terminal fitting and ResizeObservers
   are unaffected. Apply the same to the hidden-but-laid-out window tab
   (`window-tab-visibility.ts:66-77`).
   **Don't** use `display:none` or `content-visibility:hidden`: both break
   the "keeps its real size" guarantee the keep-alive path depends on
   (`pane-leaf-chrome.tsx:364-373`).
2. **Hygiene.** `quicktips.tsx`: `transition-all` → `transition-colors`,
   which in Tailwind v4 also covers the gradient stops the hover animates.
   With fix 1 this is no longer needed for correctness, but `transition-all`
   also animates layout properties on every change.
3. **Tests.** The existing keep-alive slot test (`pane-leaf-chrome.test.tsx`)
   asserts the inactive slot has `opacity: 0` as well as
   `visibility: hidden`, and the active one `opacity: 1`.

---

## 2. How general is the pane-tab interface today?

### 2.1 What a view type must provide (the `ViewModel` type, `frontend/types/custom.d.ts`)

| Member | Declared? | Read by | Problem |
|---|---|---|---|
| constructor `(blockId, nodeModel)` | type only | `block.tsx` `makeViewModel` | — |
| `viewType`, `viewComponent` | yes | `block.tsx` | undocumented |
| `blockId` | **no** | `PaneChrome.tsx` (`viewModelIsFor`, #3714), `block-component-registry.ts` | relied on, not typed |
| `noHeader` | optional | `blockframe.tsx:1108` | **required in practice**; the same one line is copy-pasted into 12 ViewModels even though the host already knows the answer |
| `renderPaneChrome` | optional | `pane-leaf-chrome.tsx:458` (with a non-null `!`) | every view sets it to the same `renderPaneChromeShell`; a view in `HOISTS_OWN_CHROME` that forgets it throws |
| `paneChromeModel` | optional | `PaneChrome.tsx:76`, read **once** | only agent and term implement it (see §2.4 #2) |
| `viewName`, `viewNameIsPlaceholder`, `viewFaviconUrl` | optional | header and the tab-name memory | pills can read them only for the **active** tab (§2.4 #6) |
| `viewIcon` | optional | header only | **pills ignore it**, so agent duplicates its icon logic in a descriptor (`agent-pane-tab.ts` vs `agent-model.ts`) |
| `setViewName` | optional | header inline rename | a second rename path next to the descriptor's `renamer` |
| `dispose` | optional | `block.tsx` | undocumented; runs on **every** switch-away in the remount path, only on close in keep-alive |
| view-specific fields | optional | various | the shared type carries agent-only (`setProgressBarMount`), term-only (`isBasicTerm`, `voiceHandle` shown only for term), browser-only (`getBodyContextMenuItems(browserCtx)`) members |

### 2.2 Where a view type must be registered

| What | Where | Problem |
|---|---|---|
| View name → ViewModel class | `block-registry.ts:25-43`, a hard-coded map with static imports | no runtime registration; the comment mentions a `registerBlockView()` that doesn't exist |
| Legacy aliases | `block.tsx:69-72` | hard-coded |
| Default name and icon | `blockutil.tsx` (icons, `VIEW_LABELS`) | hard-coded, incomplete, duplicates widgets.json |
| Widget bar / "+" picker | `agentmux-srv/src/config/widgets.json` (`defwidget@<view>`) | generic, but can only point at built-in views |
| Tab label, icon and rename | `registerPaneTabDescriptor` (`pane-tab-model.tsx`), via side-effect imports (term, agent) | a second, parallel registry |
| Gets the shared chrome? | `HOISTS_OWN_CHROME` (`pane-leaf-chrome.tsx:59-72`) | hand-maintained; **launcher, memory, identity, toolchain and settings are missing**, and toolchain and settings are in the "+" picker |
| Stays mounted? | `KEEP_ALIVE_TYPES` (`:101`) | hand-maintained, and applies to the whole pane (§2.4 #1) |
| Backend defaults | `pane.rs` `build_pane_meta` allow-list | only used when no `meta` is passed; the "+" picker always passes `meta`, so that path is already generic |

### 2.3 Lifecycle: no general "active/inactive" signal
A view is told about visibility through three unrelated mechanisms, or not
at all:
1. **`isBlockDormant`**: keep-alive path only (`pane-leaf-chrome.tsx`,
   `block-component-registry.ts`).
2. **`useWindowTabHidden` / `TAB_VISIBILITY_CHANGED_EVENT`**: window tabs
   (`window-tab-visibility.ts`).
3. **Unmount**, in the remount path: nothing beyond `onCleanup`/`dispose`.

The browser, the only view with a native surface, combines all three by
hand (`use-pane-rect-sync.ts`). Focus-on-activate exists only for the
terminal (`term.tsx`); other views rely on `onMount`, which doesn't run
again for a kept-alive tab.

### 2.4 Where the implicit contract causes real bugs

1. **Keep-alive spreads to the whole pane.** One agent or term tab makes
   *every* tab in the pane permanent, whatever that tab expects. Result:
   the Help ghost (§1), the browser dormant-hide workaround (#3714), and,
   hypothetically, sysinfo or editor background work while hidden.
2. **The whole pane gets its chrome options from the first active tab,
   forever.** The chrome ViewModel is latched once
   (`pane-leaf-chrome.tsx:442-448`), and `paneChromeModel` is read once
   (`PaneChrome.tsx:70-76`). Examples:
   - An agent-first pane applies agent's `onActivate`, `onClose`,
     `extraTabs` and `contentClass` to every tab.
   - A Help-first pane with a terminal added loses the terminal's cwd
     inheritance for new tabs, its background image and its focus restore.
   - A terminal-first pane wraps Help in the terminal's background.
3. **Missing chrome or double headers (from code; not reproduced).**
   - A pane that *starts* as Toolchain, Settings, Launcher, Identity or
     Memory never gets the shared chrome: no pills and no "+".
   - Adding one of those views to a pane that already has the chrome gives
     it a second header, because it doesn't implement `noHeader`.
4. **`noHeader` boilerplate** in 12 ViewModels, when the host already knows
   the answer (`nodeModel.paneChromeHoisted`).
5. **View-name checks in shared code:**
   - `blockframe.tsx`: term mic, term name from env, browser logging,
     agent header class, term border
   - `PaneChrome.tsx`: agent default header color
   - `pane-actions.ts`: only term accepts paste; agent split blocklist
   - `zoom.ts`: editor
   - `keymodel-blockcreate.ts`: term

   Each one is a capability a third-party tab couldn't declare.
6. **Titles and icons of inactive tabs** only exist in a per-pane memory of
   whatever the tab last reported while active (`pane-tab-model.tsx`),
   because the live ViewModel exists only for the active tab. There's no
   pure "block meta → title" function. That's why #3714 needed
   `viewNameIsPlaceholder`.

### 2.5 What a third-party pane tab would need that doesn't exist
- **Registration and a manifest:** a runtime registration API, and a
  manifest (id, version, label, icon, default meta, capabilities).
- **Lifecycle hooks that fire on both paths:** activate, deactivate,
  dormant, window-hidden, focus.
- **Declared capabilities** instead of view-name checks: keep-alive, native
  surface, accepts input, connection, voice.
- **A small, versioned, stable type**, instead of today's grab-bag
  `ViewModel`.
- **A host context** instead of raw `nodeModel`, `RpcApi` and `MOS`.
- **Isolation:** at least an error boundary (`BlockErrorBoundary` exists),
  ideally a sandbox with a scoped RPC surface.

---

## 3. Proposal: Pane Tab contract v1

```ts
/** What a pane tab type declares once. Everything the host needs to know
 *  WITHOUT an instance: name, icon, title of an unmounted tab, capabilities. */
interface PaneTabManifest {
    apiVersion: 1;
    view: string;                        // "help"; plugins: "ext:<package>/<name>"
    aliases?: string[];                  // replaces block.tsx's hard-coded aliases
    label: string;                       // replaces VIEW_LABELS / widgets.json duplicates
    icon: string;
    capabilities?: {
        lifecycle?: "remount" | "keepAlive"; // per TAB, never per pane
        nativeSurface?: boolean;             // host drives it via ctx.visibility
        acceptsInput?: boolean;              // paste / multi-input (today: term by name)
        connection?: boolean;                // ConnectionButton (today: manageConnection)
        voice?: boolean;                     // mic (today: term by name)
    };
    defaultMeta?: Record<string, unknown>;   // replaces pane.rs build_pane_meta branches
    /** Pure — works for an UNMOUNTED tab, so pills never need a memory of a
     *  dead ViewModel's name. */
    tabTitle?(meta: MetaType, ordinal: number): string | undefined;
    tabIcon?(meta: MetaType): PaneTabIcon | undefined;
    rename?(blockId: string, meta: MetaType): ((title: string) => Promise<void>) | undefined;
    create(ctx: PaneTabHostContext): PaneTabInstance;
}

/** What the host gives an instance. The only way in: no raw nodeModel. */
interface PaneTabHostContext {
    blockId: string;
    meta: Accessor<MetaType>;
    setMeta(patch: Record<string, unknown>): Promise<void>;
    /** ONE signal for both switch paths and for window tabs. */
    visibility: Accessor<"active" | "dormant" | "windowHidden">;
    isFocused: Accessor<boolean>;
}

/** A live tab. The host decides the header, chrome and hiding. */
interface PaneTabInstance {
    component: ViewComponent;
    /** Live title; `placeholder: true` while it's a stand-in (see #3714). */
    liveTitle?: Accessor<{ text: string; placeholder?: boolean }>;
    liveFavicon?: Accessor<string>;
    headerActions?: Accessor<IconButtonDecl[]>;
    headerText?: Accessor<string | HeaderElem[]>;
    contextMenu?(ctx?: unknown): ContextMenuItem[];
    focus?(): boolean;
    onKeyDown?(e: MuxKeyboardEvent): boolean;
    onActivate?(): void;                 // fired by the host on BOTH paths
    onDeactivate?(): void;
    /** Pane-level contributions, read reactively from the ACTIVE tab. */
    chrome?: {
        belowHeader?(): JSX.Element;
        wrapContent?(content: JSX.Element): JSX.Element;
        newTabMeta?(view?: string): Record<string, unknown> | undefined;
        extraTabs?(): { blockId: string; label?: string }[];
        onActivateTab?(id: string): boolean | void;
        onCloseTab?(id: string): boolean | void;
    };
    dispose(): void;                     // REQUIRED
}

function registerPaneTab(manifest: PaneTabManifest): () => void; // returns unregister
```

**Rules the host enforces:**
1. **Every registered view gets the shared chrome.** `HOISTS_OWN_CHROME` and
   every `noHeader` go away (fixes §2.4 #3 and #4).
2. **Keep-alive is per tab**, decided by that tab's own manifest. A
   `remount` tab inside a pane that also has keep-alive tabs is still
   unmounted when inactive (fixes §2.4 #1).
3. **Hidden means hidden.** The host hides an inactive kept-alive tab with
   opacity *and* visibility (§1.5), so no content CSS can leak through.
4. **`chrome.*` is read from the active tab** through reactive accessors
   inside the stable, never-remounted chrome (fixes §2.4 #2). That keeps the
   anti-flicker latches' guarantee: chrome is never recreated, only what it
   reads changes.
5. **`ctx.visibility` is the one active/dormant/hidden signal** on both
   paths and for window tabs. A `nativeSurface` view must collapse its
   surface whenever it isn't `"active"`. The browser's hand-combined logic
   becomes the reference implementation.
6. **Capabilities replace view-name checks** in shared code (§2.4 #5).
7. **Titles of unmounted tabs** come from the pure `tabTitle(meta)` first,
   and only then from the last `liveTitle` (fixes §2.4 #6).

**For a future widget library:** a user widget is a trusted, locally
installed ES module (decided, §5) whose default export is a
`PaneTabManifest` with `view: "ext:…"`, listed in widgets.json. The host
loads it, checks `apiVersion`, wraps it in an error boundary, and gives it
`PaneTabHostContext`. No sandbox in v1.

---

## 4. Migration phases

Each phase is its own PR and is independently shippable. Existing views keep
working throughout through a legacy adapter.

0. **Help ghost** (§1.5): slot opacity (pane and window tabs), QuickTips
   `transition-colors`, and a test. Also: add `blockId` to the `ViewModel`
   type, and delete the stale `registerBlockView` comment. **Small; do
   first.**
1. **Host-derived chrome:** BlockFrame reads `nodeModel.paneChromeHoisted`
   directly. Remove the 12 `noHeader` lines. Hoist every registered view,
   which fixes Toolchain, Settings, Launcher, Identity and Memory. Make
   `renderPaneChrome` optional with a host default.
2. **Registry:** add `registerPaneTab`, plus
   `legacyAdapter(view, ViewModelClass, descriptor?)` so existing classes
   work unchanged. Derive `block-registry`, the blockutil maps,
   `KEEP_ALIVE_TYPES`, aliases and `PaneTabDescriptor` from it. Migrate in
   this order: help, sysinfo, swarm, drone, warden, armory, media, editor,
   then browser, then term and agent.
3. **Unified visibility:** `ctx.visibility` on both paths and for window
   tabs. Move the browser's rect sync, agent dormancy
   (`agent-dormancy.tsx`), `useWindowTabHidden` consumers and term's focus
   restore onto it. Make keep-alive per tab.
4. **Per-active-tab chrome:** `PaneChromeModel` becomes `instance.chrome`,
   read for the active tab.
5. **Capabilities** replace the view-name checks (§2.4 #5). On the backend,
   `defaultMeta` comes from the manifest instead of the `pane.rs` allow-list.
6. **Third-party loading:** trusted, locally installed ES-module widgets
   listed in widgets.json, a shared Solid runtime, `apiVersion` checks and
   error boundaries. No sandbox in v1 (decided, §5).

## 5. Risks and open questions

- **Flicker regressions.** The existing latches exist so that chrome never
  remounts (`pane-leaf-chrome.tsx:129-133, 417-441`). Phase 4 must keep
  per-active-tab contributions as *reactive reads* inside the stable chrome,
  never a second `renderPaneChrome` call.
- **Memory.** Per-tab keep-alive will likely *reduce* retained blocks (fewer
  views kept alive by association), but anything newly declared `keepAlive`
  adds to the registry (`block-component-registry.ts`).
- **Native surfaces.** The visibility signal must still close the
  async-create orphan race fixed in #3714 (`use-pane-rect-sync.ts`).
- **Plugin trust — decided (repo owner, 2026-09-24): v1 loads trusted,
  locally installed ES modules only. No sandbox.** A widget runs with full
  renderer privileges (RpcApi, the tab RPC client), the same as a built-in
  view. The iframe sandbox with scoped RPC in Phase 6 is out of scope for v1;
  a future marketplace or remote-loading story would revisit it.
- **Solid coupling.** Plugins must share the app's Solid instance; the
  `apiVersion` bump policy has to cover that.
- **Decided (repo owner, 2026-09-24): which views keep their state when you
  switch to another tab in the same pane.** Selecting another pane tab has to
  do one of two things with the tab you left:
  - **remount**: destroy it and rebuild it when you come back. It uses no
    memory or CPU while not shown, but its state resets. A browser page
    reloads (scroll position, form input and in-page state are lost), an
    editor loses cursor, scroll and undo unless the view saves them itself,
    and titles and favicons re-load.
  - **keepAlive**: keep it mounted but not shown. Switching is instant and
    nothing is lost, but the tab keeps using memory and possibly CPU (a
    browser page is a whole Chromium renderer, often 100–300 MB). The host
    must also make it *truly* invisible, which is §1 and, for native browser
    pages drawn above the app, #3714's dormant-hide.

  Today this is decided **per pane**, by accident (§2.4 #1). The contract
  makes it a per-tab capability, with these values:
  - `keepAlive`: **agent**, **term** (unchanged), plus **browser** (stop
    reloading the page on every switch; a count or memory cap is a possible
    follow-up) and **editor** (keeps cursor, scroll and undo).
  - `remount`: everything else (help, sysinfo, cpuplot, swarm, drone,
    warden, armory, media, …), *even in a pane that also holds keep-alive
    tabs*.

  Shipped ahead of the full contract in PR #3725: a per-tab lifecycle in
  `pane-leaf-chrome.tsx` (`KEEP_ALIVE_TYPES` + `mountedBlockIds`), driven by
  the view-type set that the manifest's `capabilities.lifecycle` later
  replaces.

## 6. Test plan (per phase)

- **Phase 0:** a unit test for the inactive slot's style (opacity plus
  visibility). Manually, in `task dev`: a pane with an agent tab plus Help —
  switching away from Help shows no ghost, and switching back is instant.
  Same for window tabs.
- **Phase 1:** every registered view renders exactly one header, with pills,
  when opened alone and when added as a second tab (unit test over the
  registry). Toolchain and Settings opened from "+" get pills.
- **Phases 2–5:**
  - legacy-adapter parity tests per migrated view (same label, icon and
    header actions before and after)
  - a `ctx.visibility` transition-table test for both paths and for window
    tabs
  - chrome contributions follow the active tab (agent `extraTabs` only while
    agent is active)
- **Phase 6:** a sample `ext:` widget in the repo exercising the full
  contract, loaded from widgets.json, including a version-mismatch rejection
  and an error-boundary crash test.
