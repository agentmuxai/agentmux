# SPEC: Pane Tab contract v1 — one general interface for every pane tab (and future user-loaded widgets), starting with the Help "ghost" fix

**Date:** 2026-09-24
**Status:** active — Phase 0 (§1.5, the Help ghost) implemented in PR #3723;
per-tab keep-alive (§5, decided) in PR #3725; Phase 1 (host-derived chrome) in
PR #3752; host rules 8–10 (§3, instance lifetime) in PRs #3754 and the
split-browser fix (#3755); Phase 2a (the registry, §4) in #3757; Phase 2b (the native `create(ctx)` path,
Help as pilot) in #3759; Phase 3a (one visibility signal) in #3760; Phase 3b (host-fired
activation, focus hand-off) in #3761; Phase 4 (per-active-tab chrome) in #3764;
Phases 5a–5b (capabilities replace view-name checks) in #3765; 5c dropped
(§4); Phase 6 (third-party widgets from the user's widgets.json) in the PR
after it; Phase 2c (migrating the remaining built-in views to `create`) not
started.
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
8. **The host owns an instance's reactive lifetime.** `create(ctx)` runs in a
   root the host creates for that instance — untracked, so nothing the
   instance reads while building subscribes the host's own effects — and the
   host disposes that root together with `dispose()`. Before this, the legacy
   path built ViewModels inside `Block`'s effect: the first meta change
   re-ran the effect and killed every memo of the still-cached instance
   (Sysinfo's plot type changed once, then froze; 13 view models exposed).
   Implemented for legacy ViewModels in `block.tsx`'s `makeViewModel`
   (PR #3754).
9. **A preview never creates an instance.** A drag-preview thumbnail (mounted
   for every tile, always) renders from the manifest (`label`, `icon`,
   `tabTitle`) plus the live instance's `liveTitle` when there is one. An
   instance's `create` may have global side effects keyed by block id — the
   browser registered in its block-id-keyed store and unregistered it on
   dispose, so a preview instance left a split browser pane black. Implemented
   for legacy ViewModels in `block.tsx`'s `makePreviewViewModel`.
10. **A block may be mounted more than once in quick succession** (split,
    layout rebuild, hot reload). Any host-side resource keyed by block id — a
    `nativeSurface` view's native page — belongs to the latest mount that
    requested it; an older mount releases it only if it still owns it
    (`use-pane-rect-sync.ts` `nativePaneOwners`).
    REPORT_SYSINFO_PLOT_TYPE_AND_BROWSER_PREVIEW_VM_2026_09_25.md has the
    evidence for 8–10.

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
   - **2a (implemented):** `frontend/app/block/pane-tab-registry.ts` (pure,
     imports no view) holds `PaneTabManifest`, `registerPaneTab` (returns
     unregister; refuses a taken view or alias) and `legacyAdapter`.
     `block-registry.ts` registers all 17 built-ins through `legacyAdapter`;
     `getBlockViewClass`, `resolveEffectiveViewType`, `blockViewToIcon`/
     `blockViewToName`, the keep-alive set (`isKeepAliveView`) and
     `describePaneTab`'s descriptors all read the registry. The parallel
     `registerPaneTabDescriptor` is gone — a descriptor is the manifest's
     `tab`. `block-registry.test.ts` pins parity with every table replaced.
     One deliberate difference: an alias now resolves for label and icon too
     (a still-live `forge` block reads "Agent", not "forge").
   - **2b (implemented):** the host's native path. A manifest has exactly
     one of `create(ctx)` and `viewModelClass`. `makeViewModel` calls
     `create` in the instance's own root (rule 8) with a
     `PaneTabHostContext` — `blockId`, reactive `meta`, `setMeta`,
     `isFocused`; no raw nodeModel, MOS or RpcApi — and
     `pane-tab-host.tsx`'s `adaptPaneTabInstance` presents the
     `PaneTabInstance` as the ViewModel the rest of the host consumes today.
     Help is the pilot: `helpPaneTab` in `helpview.tsx`, its zoom read and
     written through `ctx.meta`/`ctx.setMeta`.
   - **2c:** the remaining views move to `create` as Phases 3–5 give them
     what they reach around the contract for today (`ctx.visibility`,
     per-active-tab chrome, capabilities) — migrating them first would only
     re-home their `nodeModel` reach-ins. Order as above.
     - **Sysinfo (implemented):** `sysinfoPaneTab("sysinfo" | "cpuplot")`
       (sysinfo.tsx). Its model is built from `ctx` — every meta read is
       `ctx.meta`, both plot-type writes are `ctx.setMeta` — and titles the
       pane with the plot type (`liveTitle`). The contract grew two things it
       needed: `PaneTabInstance.settingsMenu` (the header's Plot Type menu)
       and the `connection` capability (the header's connection button).
       `cpuplot`, the same view under an older name, now labels and icons
       itself like sysinfo in its tab pill too (its header always did).
     - **Swarm (implemented):** `swarmPaneTab` (swarm.tsx). Its model is
       built from the block id (it never used its `nodeModel`); the view reads
       and writes its own `term:zoom` through `ctx`; the instance forwards
       `dispose` (the model's subscriptions and timers). The contract grew the
       `noPadding` capability (full-bleed content).
     - **Drone (implemented):** `dronePaneTab` (drone.tsx), keeping the
       `workflows` alias. Its only own-block read, `frame:title`, comes from
       `ctx.meta` and titles the pane (`liveTitle`); `dispose` is forwarded.
3. **Unified visibility:** `ctx.visibility` on both paths and for window
   tabs. Move the browser's rect sync, agent dormancy
   (`agent-dormancy.tsx`), `useWindowTabHidden` consumers and term's focus
   restore onto it. Make keep-alive per tab.
   - **3a (implemented):** `usePaneTabVisibility(blockId)`
     (`frontend/app/block/pane-tab-visibility.ts`) →
     `"active" | "dormant" | "windowHidden"`, from pane-stack dormancy and a
     new per-window-tab `useWindowTabDisplayed` (`workspace.tsx` provides
     it; true in BOTH window-tab modes, and outside any window tab). A native
     instance gets it as `ctx.visibility`. The browser's rect sync collapses
     on anything but `"active"` — replacing its DOM walk for hidden-tab
     markers (`isInsideHiddenTabContent`), the `data-tab-hidden-laid-out`
     marker and the `agentmux:tab-visibility-changed` event. The agent pane's
     and history view's render pausing read it instead of `isBlockDormant` +
     `useWindowTabHidden` (removed). With inactive tabs kept laid out (the
     default) this is the same behavior; with them not laid out, a
     window-hidden agent pane now also pauses rendering, which
     `content-visibility` already skipped. Keep-alive is per tab since #3725.
   - **3b (implemented):** `Block` (block.tsx, real mounts only) fires a
     tab's `onActivate`/`onDeactivate` from the visibility signal on both
     paths — a remount tab activates by mounting, a kept-alive one by leaving
     dormancy or a hidden window tab; unmounting while visible deactivates —
     and a tab that becomes visible in a focused pane gets `giveFocus()`.
     That hand-off used to be the terminal's pane-chrome `onActivate`, so a
     pane only had it if its FIRST tab was a terminal (§2.4 #2), and then for
     every tab; it's gone, and every pane has it. `ViewModel` and
     `PaneTabInstance` gain `onActivate`/`onDeactivate`. The agent's question
     auto-timeout and failure auto-retry now pause whenever the tab isn't
     visible (their stated intent: never fire invisibly), not only while it's
     a dormant pane-stack member.
4. **Per-active-tab chrome:** `PaneChromeModel` becomes `instance.chrome`,
   read for the active tab.
   - **Implemented** as a manifest field rather than an instance one:
     `chrome?: (anchorBlockId, nodeModel) => PaneChromeModel`. A chrome model
     is pane-level and creates signals of its own, so the chrome
     (`PaneChrome.tsx`) builds each view type's model ONCE per pane, in its
     own reactive scope, the first time one of its tabs is active, and reads
     the model of the ACTIVE tab's view type reactively. Terminal and agent
     register `buildTermPaneChromeModel` / `buildAgentPaneChromeModel` on
     their manifests; `ViewModel.paneChromeModel` and the terminal's
     late-binding `setTermPaneChromeModel` are gone. Fixes §2.4 #2: a
     terminal-first pane no longer gives an agent tab the terminal's
     connection button, background or new-tab cwd, and a Help-first pane
     with a terminal tab gets them while the terminal is active.
   - `wrapContent` is replaced by `bodyClass` + `renderBehindContent`: the
     chrome always renders a body box (`.pane-stack-body`, the same flex
     column the content region used to sit in) around a stable content
     region, so a switch between view types changes classes and leading
     overlays but never moves the kept-alive content in the DOM (re-wrapping
     would, and a moved subtree can pause media or reset renderers).
5. **Capabilities** replace the view-name checks (§2.4 #5). On the backend,
   `defaultMeta` comes from the manifest instead of the `pane.rs` allow-list.
   - **5a (implemented): header and frame.** `PaneTabCapabilities` gains
     `nativeSurface` (browser), `header: "surface"` (agent: an uncolored
     header keeps the theme's block surface instead of the fixed default
     color), `headerMic: { title }` (term: the header mic and its tooltip —
     agent takes voice beside its composer, so it doesn't declare it),
     `statsBadgeSetting` (term: `term:showstatsbadge`) and `hueBorder` (term:
     `frame:hue` colors the active border). `paneTabCapability(view, key)`
     reads one; an alias carries its view's. `blockframe.tsx`'s and
     `PaneChrome.tsx`'s seven view-name checks for these read the
     capabilities; `block-registry.test.ts` pins that exactly the view types
     the old checks named declare each one.
   - **5b (implemented): input, zoom, new blocks.** `paneZoom: {
     baseFontSize? }` replaces `zoom.ts`'s allowlist of views using
     `term:zoom` (term, agent, swarm, editor, armory, warden — warden was once
     missing from it by accident) and editor's hard-coded base size (13);
     `acceptsInput` (term) is the pane menu's Paste rule
     (`pane-actions.ts`); `shellKeys` (term) makes Ctrl+F search stand down
     (`keymodel.ts`); `sharesCwd` (term) gives a new block the focused
     block's `cmd:cwd` (`keymodel-blockcreate.ts`). The basic-terminal count
     needs no capability — only the terminal implements `isBasicTerm`. The
     terminal's env-derived header name moved into the terminal itself
     (`termViewName`, termutil.ts, used by `TermViewModel.viewName`).
     `splitDropsMeta` (agent: `AGENT_SPLIT_DROPPED_META`) lists the meta a
     split must not copy — the split rule applied that blocklist only when
     `view === "agent"`. No view-name check remains in shared header, frame, zoom, key or
     pane-menu code; `command-registry.ts`/`keymodel-blockcreate.ts` still
     *create* terminals by name, which is a choice of default, not a check.
   - **5c (dropped, 2026-09-25):** backend `defaultMeta` from the manifest.
     On inspection `pane.rs`'s `build_pane_meta` is not a defaults table but
     validation of the agent-facing `pane.open`'s typed arguments (an editor
     needs `file`, a browser `url`, a terminal takes `cwd`), and it only runs
     when no `meta` is passed. With `meta`, `pane.open` is already generic —
     that is how an agent opens any widget, including an `ext:` one — and the
     backend can't see frontend manifests without a new sync channel nothing
     else needs.
6. **Third-party loading:** trusted, locally installed ES-module widgets
   listed in widgets.json, a shared Solid runtime, `apiVersion` checks and
   error boundaries. No sandbox in v1 (decided, §5).
   - **Design (2026-09-25):**
     - **Where a widget lives.** A `widgets.json` entry gains `"module"`: a
       path to an ES module, relative to `~/.agentmux/widgets/` or absolute.
       Its `blockdef.meta.view` must be `ext:<name>`. `WidgetConfigType`
       (`wconfig/types.rs`) gets the field — serde otherwise drops it.
     - **The user's widgets.json.** Widgets came only from the `widgets.json`
       embedded at build time, so there was nowhere to add one. The user's
       own `widgets.json` beside `settings.json` (same per-channel directory,
       `resolve_settings_dir`) is merged over the built-ins — a new key adds a
       widget, a built-in's key replaces it — loaded at startup and reloaded
       on change like `settings.json` (`backend/user_widgets.rs`). A parse
       error keeps the previous widgets.
     - **Loading** (`frontend/app/block/widget-loader.ts`). For each such
       entry the loader reads the module's text through the existing
       `readeditorfile` RPC (a trusted local file, like any file the editor
       opens), rewrites its bare `solid-js`, `solid-js/web` and
       `solid-js/store` imports to small blob modules that re-export the
       APP's own instances — one Solid runtime, as §5 requires — and imports
       it from a blob URL. No new server endpoint.
     - **Validation.** The default export must be a `PaneTabManifest` with
       `apiVersion: 1`, `view` equal to the entry's `ext:` view, and `create`
       (native only — a widget has no ViewModel class). Anything else is
       rejected with a logged reason and never registered; so is a module
       that fails to read, parse or import. The next reload of the config
       (a `widgets.json` change) registers widgets that appeared and retries
       ones that failed; a loaded widget is left alone.
     - **Crash containment.** A widget that throws while rendering is
       already contained by `Block`'s per-pane `BlockErrorBoundary`. What it
       didn't cover is `create(ctx)`, which runs in `makeViewModel` inside
       `Block`'s effect: the host now catches a throwing `create` and gives
       the pane an instance that shows the error. Either way only that pane
       is affected.
     - **Startup order.** app-init waits (up to 2 s) for the loader's first
       pass before the first render: a persisted `ext:` pane that mounted
       before its widget registered would build the default view model and
       never rebuild, since `Block` only does on a view change.
     - **Sample** (`docs/examples/widgets/hello/`): a dependency-free widget
       using only `solid-js` primitives and DOM nodes, exercising `ctx.meta`,
       `ctx.setMeta`, `ctx.visibility`, `liveTitle` and `onActivate`. Tests
       cover the import rewrite, a version-mismatch and a view-mismatch
       rejection, and crash containment; the loader's import step is
       injectable, since a test runner can't import a blob URL.

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
