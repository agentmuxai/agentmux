# Report: A screenshot for every UX experience in the docs — coverage plan

**Date:** 2026-10-10
**Status:** proposed — a plan, not started. Nothing here is implemented yet.
**Context:** AgentMux is a UI product, but its docs site (agentmuxai/agentmux-docs)
is almost all prose. The screenshot tooling in `scripts/ui-screenshots/`
(spec: `docs/specs/SPEC_UI_MANUAL_SCREENSHOT_TOOLING_2026_09_19.md`) now captures
every widget at fixed sizes from an isolated instance, and the first images
landed in agentmux-docs#163 (the widget gallery). This report surveys every
user-facing docs page, lists each UX experience on it (anything the user sees or
operates, as opposed to pure concept) with the auxiliary screens, dialogs and
panels that go with it, and lays out how to get all of them captured, in parts.
Tracking issue on the docs side: agentmux-docs#162.

Surveyed at agentmux `ec5e8b5d2` and agentmux-docs `876d2b0` (both latest `main`
on 2026-10-10).

---

## 0. Summary

- **Today:** one docs page, the widget gallery, has screenshots: 14 widgets × 2
  sizes = 28 PNGs, all in the fresh, empty state. None of the other ~30
  user-facing pages has a single image of the UI. The only other images on the
  site are 8 architecture SVG diagrams.
- **The tooling is a good base** and should be extended, not replaced. It already
  does DOM-exact crops, named sizes, an isolated data folder, a made-up demo
  project, retries, cleanup and a `containsWorkspaceData` review flag.
- **What it can't do yet**, in order of how many shots each one blocks:
  1. **Content.** Every image is the empty state. Most experiences only make
     sense with content in them: an agent mid-conversation, a Swarm with
     subagents, populated Memory and Connectors, a Drone flow.
  2. **Transient and error states.** Failure rows, the install dialog's steps,
     sign-in rows, the shutdown log, the quit countdown, approval prompts. These
     need state injected, not just clicks.
  3. **Deep navigation safely.** `prep` clicks have no post-click check (`verify`,
     spec §7), so a wrong click silently captures the wrong thing.
  4. **A second capture target.** Approval windows (SSH, saved credential,
     memory adoption) and torn-off panes are separate windows with their own
     page targets; the runner only ever picks the main window.
  5. **A few things genuinely outside the page:** Browser-pane web content, OS
     file dialogs, OS notifications, the tray menu, the macOS menu bar, window
     translucency.
  6. **Motion.** No GIF or video recorder.
  7. **One-command refresh.** Each run is a manual 4-step procedure.
- **Good news from the survey:** every in-app menu (pane right-click, the pane `+`
  widget picker, Swarm's copy menus, Accounts' Bind to Agent, tab and title-bar
  menus) is drawn in the page (`app/host/js-context-menu.ts`, installed for every
  host by `util/cef-api.ts`), so a page screenshot **can** capture them. The
  spec's §8.2 note that the pane `+` and right-click menus are native is out of
  date and should be corrected.
- **Inventory:** **190 new shots** across 21 page groups (Appendix A), plus the 9
  existing widget images reused and about 8 short recordings. **65** can be taken
  with today's tool plus `verify`, **120** need content seeding or state fixtures,
  and **5** are outside the page.
- **Plan:** 8 parts (§6). Part 1 (put the existing widget images on their feature
  pages) needs no new tooling and can start immediately. Part 2 (chrome, menus,
  Settings: 37 shots) needs only `verify` and a hostname override. Part 3 (the
  agent pane and onboarding: 55 shots) builds the seeding layer that Parts 4–6
  then reuse, so it's the critical path. Parts 7–8 are variants, motion and
  keeping it all fresh.
- **Docs drift found along the way** (§7): Settings lists 9 themes of 13, the
  Stash lists 6 tabs of 7, the main menu page misses the Layouts submenu, the
  Tower widget has no docs page at all, Hangar (pinned by default) has no
  section beyond its gallery image, and Identity & Accounts still describes a
  Launch Agent modal that nothing opens any more.

## 1. What exists today

### 1.1 The tool (`scripts/ui-screenshots/`)

| File | Role |
|---|---|
| `capture.mjs` | Runner. Connects to an instance's CDP port, picks the app's main page target, runs a suite's shots in order: `prep` → settle → measure selector → `Page.captureScreenshot` with a clip → write PNG → append to `manifest.json`. Flags: `--port`, `--suite manual\|widgets`, `--only`, `--sizes`, `--out`. No npm dependencies. |
| `shots.mjs` | The `manual` suite: 10 hand-written chrome shots (top bar, hamburger menu, agent picker, pane header, Swarm, Sysinfo, Memory → Bundles, Settings → Appearance, fresh terminal, whole window). From the first run on 2026-09-19. |
| `widget-shots.mjs` | The `widgets` suite, generated from `crates/srv/src/config/widgets.json`: per widget, open a new window tab, open the widget (pinned icon → More list → hamburger → command palette), find its pane, maximize it, capture at each size, close the tab. `EXCLUDE` skips Messengers and Browser. |
| `sizes.mjs` | `small` 800×600, `medium` 1280×800, `large` 1920×1080, each with a `scale` (device pixel ratio). Re-layout via `Emulation.setDeviceMetricsOverride`, not a scaled copy. |
| `demo-env.mjs` | Writes the made-up `acme-web` project and a user `widgets.json` pointing Hangar, Terminal and Editor at it (and Sysinfo at CPU + Mem + Net). Refuses a demo path containing the user name or under the home folder. |
| `lib/bench-page.js`, `lib/one-way-soak-page.js` | Not screenshot code, but relevant: they inject synthetic history into an agent pane's document store and stream scripted Claude stream-json (tool calls, diffs, parallel tools, long reads) through the pane's real output pipeline. No agent, no tokens. Built for performance benches; reusable as the seeding layer for conversation shots (§5.2). |

The session API a shot's `prep` can use: `evaluate`, `clickSelector`,
`clickText`, `clickAt`, `wait`, `pressKey` (with modifiers), `typeText`,
`setViewport`/`clearViewport`, `parkMouse`. Shots can set `sizes`, `retries`,
`settleMs`, `cleanup` and `containsWorkspaceData`.

**How a run works today** (spec §8.4): build a portable (`task package`), start
it with `AGENTMUX_HOME_OVERRIDE=<empty folder>` and `AGENTMUX_CDP_PORT=<port>`,
run `demo-env.mjs`, run `capture.mjs --suite widgets`, review every image by hand,
copy the keepers into agentmux-docs `src/assets/screenshots/<suite>/`.

**Lessons the spec records** (§8.2), which every new shot must respect:
hidden window tabs' panes stay laid out in the page (hit-test, don't trust a
size); the top bar renders measuring copies of its icons; "Close tab?" blocks
every later click until answered; the Browser pane's web content is a native
view; park the mouse before capturing. (It also says the pane `+` and
right-click menus are native. That's no longer true: they're page overlays, see
§5.5.)

### 1.2 The docs (`agentmuxai/agentmux-docs`)

- Astro + Starlight, static. Images go in `src/assets/` (Astro optimizes them),
  embedded with a relative Markdown path, e.g.
  `![The Agent widget when it first opens](../../assets/screenshots/widgets/widget-agent-small.png)`.
- `src/assets/screenshots/widgets/`: 28 PNGs, `manifest.json`, a README saying
  how they were made and that they show the fresh state. Captured from 0.59.14 on
  2026-10-08.
- `widget-gallery.md` shows all 14; `pane-types.md` links to it. No other page
  uses them yet.
- Every PR needs a changeset (`scripts/changeset.sh`), and merging to `main`
  deploys to production.

### 1.3 Direction already agreed in agentmux-docs#162

From the issue body and AgentA's follow-up comment (2026-10-08):

- **Sizes:** small and medium are the realistic ones; large shots of an empty
  widget look wrong. The docs set uses small and medium only.
- **Fresh state is labelled** as such wherever it's used.
- **Next: realistic mock data** — a Swarm with many agents and subagents, an
  agent mid-conversation with tool calls, a terminal with history, a wired Drone
  flow, populated Memory and Connectors.
- **Next: GIFs** of short tasks with a highlighted cursor (needs a CDP screencast
  recorder, a drawn cursor and an encoder).
- Still open on the checklist: use widget images on their pages; more suites
  (settings tabs, menus, the agent pane's states, onboarding); light-theme and 2×
  variants; Browser-pane compositing; landing-site picks; one-command refresh.

This plan is consistent with all of that and fills in the per-page detail.

## 2. Scope

**In:** every experience a user sees or operates: a widget, a pane state, a
dialog, a menu, a panel, a banner, a form, a prompt, a status-bar item, a
keyboard-driven surface like the command palette.

**Out:** pure concept with nothing on screen: architecture, the trust model,
data sovereignty, the ABF on-disk format, the reducer stack, IPC catalog, env
var contract, glossary, API references. Where a concept page does describe a
visible surface (Widget security's approval prompt, LAN discovery's enable
toggle and pairing QR, Update model's update prompt), that surface is in.

**Auxiliary screens are first-class.** For each experience the inventory lists
the dialogs, sub-panels, menus and alternate states that make up the whole flow
(for example, creating an agent is the picker, the install dialog and its
failure state, the prerequisites dialog, the Create form with and without
Docker, and the sign-in rows), not just its main screen.

## 3. Capture classes

Every shot in Appendix A carries one of these, which is what decides when it can
be done:

| Class | Meaning | Today? |
|---|---|---|
| **A — fresh** | The surface in its empty, first-open state, reachable by a few clicks. | Yes. The widget suite already does all widgets. |
| **B — navigate** | A deeper surface reached by clicks or keys: a Settings tab, a submenu, a modal, a form, the command palette. No content needed. | Mostly. Works with `prep`, but needs `verify` (§5.1) so a wrong click fails instead of capturing the wrong thing. |
| **C — seeded content** | Needs believable data: agents in My Agents, an agent conversation, Swarm activity, Memory entries, skills, bundles, accounts, MCP servers, a Drone flow, terminal history. | No. Needs the seeding layer (§5.2–5.3). |
| **D — injected state** | A transient, error or permission state that clicking alone can't reach reliably: failure rows, install progress/failure, sign-in rows, disconnected banner, shutdown log, quit countdown, ask-question panel, approval prompts. | No. Needs state fixtures (§5.4). |
| **E — outside the page** | Drawn outside the app's DOM: Browser-pane web content, OS file pickers, OS notifications, the system tray, installers, the macOS menu bar, window translucency, OS window snapping. (In-app menus are *not* in this class; they're page overlays, Appendix B.) | No. Needs OS-level capture or compositing, or stays text (§5.5). |
| **F — motion** | A short recording of a task. | No. Needs a recorder (§5.8). |

## 4. Coverage by page — overview

Appendix A has the full per-page list (experience, auxiliary screens, class,
suggested shot ids). This table is the roll-up. **New** counts shots not yet
taken; **today** = classes A/B (capturable with the current tool plus `verify`);
**seed** = classes C/D (need content or state fixtures); **other** = E/F. The 9
existing widget images are reused, not counted.

| Docs page(s) | New | Today | Seed | Other | Main part |
|---|---:|---:|---:|---:|---|
| Getting Started | 2 | 2 | 0 | 0 | 2 |
| Installation | 3 | 1 | 0 | 2 | 2, 6 |
| Quickstart | 0 (reuses 11) | — | — | — | 3 |
| First Agent Setup | 26 | 4 | 22 | 0 | 3 |
| Pane Types | 45 | 14 | 30 | 1 | 3, 5, 6 |
| Hangar (no section yet) | 6 | 1 | 5 | 0 | 6 |
| Widget gallery | 2 | 2 | 0 | 0 | 5, 8 |
| Your own widgets, Build a widget | 7 | 0 | 7 | 0 | 4 |
| Browser pane | 9 | 4 | 4 | 1 | 6 |
| Connectors | 14 | 6 | 8 | 0 | 4 |
| Memory | 16 | 4 | 12 | 0 | 4 |
| Bundles, ABF, Identity, Auth flows (the Stash) | 8 | 0 | 8 | 0 | 3 |
| Swarm | 9 | 0 | 9 | 0 | 5 |
| Multiple instances, LAN discovery, Warden | 9 | 3 | 6 | 0 | 2, 5 |
| Window appearance | 3 | 2 | 0 | 1 | 2 |
| Settings reference | 12 | 11 | 1 | 0 | 2 |
| Main menu & command palette | 6 | 6 | 0 | 0 | 2 |
| System Metrics | 4 | 2 | 2 | 0 | 2, 5 |
| Tower (no page yet) | 3 | 1 | 2 | 0 | 5 |
| Toolchain | 2 | 1 | 1 | 0 | 2, 3 |
| Help, notices, global dialogs | 4 | 1 | 3 | 0 | 2 |
| **Total** | **190** | **65** | **120** | **5** | |

Plus about 8 recordings (Part 7). A few rows are "one or two shots", so the
real total lands around 185–195.

**Where the value is:** First Agent Setup, Pane Types, Memory, Connectors, Swarm
and the Stash hold 89 of the 120 seeded shots. They're the product's core, and none
of them can be taken until Part 3's seeding layer exists. That's why Part 3 is
the critical path.

## 5. Tooling work

Each item names the part (§6) that first needs it.

### 5.1 `verify` on every navigated shot (Part 2)

Spec §7's open follow-up. Add a `verify` field to a shot: a selector (or
`evaluate` expression) that must be true after `prep`, checked before capture.
Fail the shot if it isn't. Every class-B shot sets one (e.g. Settings →
Appearance verifies the Appearance section heading is visible). This turns a
wrong `clickText` match from a silently wrong image into a loud failure, which
matters more as suites grow from 10 shots to 150.

### 5.2 Seeding agent conversations (Part 3)

The conversation and agent-pane states are the most valuable shots in the docs
and none can be taken today. `lib/bench-page.js` and `lib/one-way-soak-page.js`
already drive the real rendering path with scripted provider output. Two
constraints:

- They import live Vite source modules (`/frontend/app/store/...`), so they work
  only against a `task dev` build, not the packaged portable the widget suite
  uses.
- They need an agent pane that already shows a composer (`textarea.agent-input`),
  i.e. an agent pane past the picker.

Options, cheapest first:

1. **Run the content suites against an isolated `task dev` instance** started
   with `AGENTMUX_HOME_OVERRIDE` (honored by `crates/common/src/data_paths.rs`,
   which every binary uses; confirm it isolates a dev instance end to end before
   relying on it) and a CDP port. Reuse the bench helpers as-is. Fastest path to
   a first conversation shot.
2. **A production-safe demo hook in the app**, gated on an env var such as
   `AGENTMUX_DEMO=1`: a small in-page API (`window.__amDemo`) that exposes
   "inject this transcript into pane X" and "show this UI state", with no Vite
   dependency. Lets every suite run against one packaged build.
3. **A replay harness at the provider level**: a fake provider CLI that plays a
   scripted stream-json transcript, so srv sees a real agent session. This is the
   only option where srv-derived state (agent status, token usage in the status
   bar, Swarm's fleet stats, History, the subagent watcher) is genuinely
   populated rather than faked in the page. Highest fidelity, most work.

**Recommendation:** start with (1) for Part 3's first shots. Decide between (2)
and (3) once it's clear which srv-derived surfaces matter. Swarm is the test
case: if its tree and stats come from srv state rather than pane events, only
(3) shows it truthfully. Settle that in Part 5's spike.

The scripted conversations should be written once as fixtures
(`scripts/ui-screenshots/fixtures/conversations/*.json`) about the `acme-web`
demo project: a short feature request with a Read, an Edit with a diff, a Bash
run with output, a todo list, a subagent dispatch, an ask-question. Using the
same project everywhere keeps the docs coherent.

### 5.3 Seeding app data (Parts 3–5)

My Agents, Memory (Global, Personal, Skills, Bundles), Connectors (accounts, MCP
servers), Drone flows, Warden decisions and widgets all live in the instance's
data and can mostly be created through the app's own RPCs. Add a
`demo-seed.mjs` that, over the same CDP connection, calls the page's `RpcApi`
(already authenticated in the page) to create:

- 3–5 agents with neutral names (`web-frontend`, `api-tests`, `docs-writer`)
  across 2–3 harnesses, so the picker, tabs and Swarm show variety;
- Global and Personal memory entries, 2–3 skills, 2 bundles (one with
  per-provider overrides);
- MCP servers with demo names;
- one Drone with wired blocks;
- a sample widget, installed but not approved, for the approval prompt.

Accounts are the hard case: a real account carries a real email and a
credential. Use made-up accounts (`dev@acme.example`) written as data only,
never a real login, and mark any shot showing them `containsWorkspaceData:
"review"`. Never seed from the capturing machine's own accounts.

### 5.4 UI state fixtures (Parts 3–4)

Class-D states are driven by srv events (failure classification, install
progress, auth checks, pending shutdown) or by modal requests. Two routes:

- Where a state is a modal (`frontend/app/element/modal-layer.ts`'s request
  kinds), open it directly through the modal layer from `evaluate` with a
  canned request. Cheap and exact.
- Where it's an srv-driven row or banner, inject the event the pane already
  renders (through the demo hook of §5.2 option 2, or the dev-build store as the
  benches do).

Each fixture is a named function in a shared module so both screenshots and
future UI tests can use it.

### 5.5 Menus, other windows, and what's truly outside the page (Parts 2, 4, 6)

- **In-app menus are page content.** `ContextMenuModel.showContextMenu` (used by
  the pane `+` picker, pane and tab right-click menus, Swarm's copy menus and
  Accounts' Bind to Agent) calls `getApi().showContextMenu`, which every host
  implements with `jsContextMenuApi()` from `app/host/js-context-menu.ts`: a
  positioned `.menu` overlay in the page. Some code comments and the spec (§8.2)
  still call these "native"; that wording predates the move and should be
  corrected. They're class B: open with a right-click or click, then crop to
  `.menu`. Menus stay as text tables in the docs too, since text is searchable.
- **A second capture target** (Part 4). The SSH approval, saved-credential
  approval and memory-adoption approval windows are separate top-level windows,
  each its own page target (`app/app.tsx`, `?initialView=`). A torn-off floating
  pane is too. Add a shot-level `target` (a URL substring such as
  `initialView=ssh-approval`) so `capture.mjs` can attach to the window a shot
  opens, alongside the main one.
- **Genuinely outside the page** (class E):
  - **Browser-pane web content**: a native child window (Windows) or CEF overlay
    view (macOS/Linux). Already on #162: composite the native view's own capture
    into the page shot.
  - **OS file dialogs** (`rfd`): Media "open file", Settings → Widgets install,
    Bundle import step 1, Save/Open layout. Start the flow one step later through
    a fixture instead (e.g. open Import step 2 with a fixture `.abf`).
  - **OS notifications, the tray menu, the macOS menu bar, window translucency,
    the native pre-splash, DevTools.** Platform-specific and not worth automating;
    keep as text, with at most one OS-level shot of the tray menu on
    Installation.
  - **Native `<select>` popups** are probably drawn outside the page too
    (inferred, not verified). Crop shots with the select closed.

### 5.6 Variants (Part 7)

- **Light theme.** The app has four light themes (Light, Catppuccin Latte,
  Solarized Light, Gruvbox Light; `frontend/app/menu/base-menus.ts`), and the
  docs site has light and dark modes. Capture one dark (Default) and one light
  (Light) set of the key shots and, if wanted, a small docs component that shows
  the one matching the site's mode. Not every shot needs both: start with the
  main window, Agent, Swarm and Settings.
- **2× scale** for the landing site only (`scale: 2` in `sizes.mjs`). The docs
  don't need it and it quadruples file size.

### 5.7 Annotation (optional, Part 7)

Spec §7 left callouts out of scope. Most shots are better cropped tightly than
annotated. The exceptions are dense screens (the agent pane's composer strip,
Settings, Swarm's toolbar) where numbered markers tied to a list beat a paragraph.
If added, draw them as a post-processing step from manifest coordinates
(`getBoundingClientRect` of each labelled element, recorded at capture), so they
stay correct across re-captures.

### 5.8 Recording (Part 7)

`Page.startScreencast` frames plus a drawn cursor overlay and a WebP/GIF encoder,
driven by the same `prep` steps as a shot. Candidates: create a first agent,
split and drag panes, Swarm following a running agent, Hangar drag and drop,
importing a bundle (.abf), approving a widget, the command palette.

### 5.9 One-command refresh and publishing (Part 8)

- `refresh.mjs`: start the isolated build with an empty data folder and a CDP
  port, wait until ready, apply `demo-env` and `demo-seed`, run the requested
  suites, stop the instance, print a review checklist from `manifest.json`.
- `publish.mjs` (or a documented step): copy approved images into
  agentmux-docs `src/assets/screenshots/<suite>/` under their stable names,
  refusing any shot whose `containsWorkspaceData` is `true` and any not marked
  reviewed.
- Stamp each manifest with the AgentMux version and date. A small docs-side
  check can then list images older than N releases.
- Re-capture per release at most, not per PR. The UI is alpha and changes fast;
  images that churn every week cost review time.

## 6. The plan, in parts

Each part is one tooling PR in agentmux (where needed) plus one content PR per
docs page group in agentmux-docs. Counts are approximate. Parts 4, 5 and 6 don't
depend on each other and can run in parallel once Part 3's seeding layer exists.

### Part 1 — Put the existing widget images on their pages

- **Tooling:** none.
- **Shots:** none new; reuse the 28 in `src/assets/screenshots/widgets/`.
- **Pages:** each widget's section in Pane Types (Terminal, Editor, Media, Agent,
  Swarm, Drone, Sysinfo); Swarm (`subagent-watcher`); Memory; Connectors; Warden;
  System Metrics; Settings; the agent picker on First Agent Setup.
- **Also:** fix the stale theme list on Settings reference (§7).
- **Done when:** every widget with a docs page shows its image near the top, with
  the "first opens / fresh state" wording from the gallery.

### Part 2 — Window chrome, menus, navigation and Settings (37 shots)

- **Tooling:** `verify` (§5.1). Mask or override the hostname in the status bar
  for capture runs (it identifies the capturing machine and appears in every
  full-window shot; see §7). A capture-time setting or a demo-mode display name is
  enough.
- **Shots:** the whole window in the starter layout (now possible from the
  isolated instance; skipped in the first run for privacy); an empty window tab;
  the in-page startup screen; the top bar with the More dropdown open; the
  hamburger menu and its Theme, Opacity and Layouts submenus (with the layout
  preview dialog); the title-bar and tab menus; the command palette, empty and
  filtered; the pane `+` widget picker and the pane right-click menu (both DOM
  menus, §5.5); a split layout and a maximized pane; the status bar strip and its
  popovers (Instance Panel, host popover with LAN discovery off, CPU cores, disk
  volumes, backend); every Settings tab plus search and the config-errors banner;
  the error toast, config-error and user-input dialogs; close confirmations.
  Toolchain is listed here but is ⛔ from an ordinary machine (Appendix A.21).
- **Pages:** Getting Started, Quickstart, Main menu & command palette, Settings
  reference, Configuration guide, Window appearance, Running multiple instances,
  LAN discovery, System Metrics (status bar), Report Issues (Instance Panel's
  version rows), Keybindings (resizing).

### Part 3 — Onboarding, the agent pane and the Stash (55 shots)

The highest-value part: the agent pane is AgentMux's main surface and has no
images at all today.

- **Tooling:** confirm `AGENTMUX_HOME_OVERRIDE` isolates a `task dev` instance;
  conversation fixtures driven by the bench helpers (§5.2 option 1); `demo-seed`
  for agents (§5.3); modal and state fixtures (§5.4).
- **Shots:** the picker with seeded My Agents, a row's menu and the "already
  open" choice; the install dialog in progress, done and failed with Details
  open; the prerequisites dialog; Create new agent (host; container; container
  greyed as "Docker not detected"; Claude with Model and Vendor); sign-in rows
  (Not signed in, No account linked, Log in with the paste box, Bind account);
  an agent mid-turn (streaming text, tool calls, an Edit diff, Working… with its
  timer); the composer strip; the activity dock; the ask-question panel; the shell
  drawer and the details drawer; three representative failure rows (rate limited
  with its retry countdown, usage limit, context window exceeded); the
  Disconnected banner; the shutdown log with Try again / Keep open; the quit
  countdown banner; Agent History; the Stash drawer, one shot per tab;
  attachments in the composer; a side question (`/btw`); the permission prompt;
  compaction; the session stats popover; session notices; a jekt bubble; the
  composer's account switch menu.
- **Skip:** the Launch Agent modal and its pre-launch OAuth, Add Account and New
  Bundle dialogs. They're unreachable dead UI (Appendix A.12); worth raising for
  removal rather than documenting.
- **Pages:** Quickstart, First Agent Setup, Pane Types (Agent and its
  subsections), Auth flows, Identity & Accounts.

### Part 4 — Memory, Connectors, bundles and widgets (37 shots)

- **Tooling:** `demo-seed` data (memory entries, skills, bundles, MCP servers,
  made-up accounts, one sample widget) (§5.3); fixtures for the approval prompt
  and the Settings → Widgets states (§5.4). A shot-level `target` for the
  separate approval windows (SSH approval, memory adoption) (§5.5).
- **Shots:** Memory → Global with tiles and the combined preview; the add-memory
  form; Personal (agent → file → history, diff, revert); the adopt and release
  panels and their confirmation window; Skills and the new-skill form; Bundles,
  a bundle's detail and the new-bundle form with per-provider overrides; Import
  Bundle steps 2 and 3 (preview with a skill collision, confirm), opened from a
  fixture because step 1 is an OS file dialog; Connectors → Accounts: the
  chooser, the AgentMux and Anthropic connect panels, made-up connected accounts
  with one open, the add-account form, the OAuth device-code panel, the delete
  confirmation and the Bind to Agent menu; MCP servers, a server's detail, its
  form and the catalog picker; Remotes populated (demo hosts only) and its add
  form; the SSH approval window; the widget approval prompt (sandboxed with
  permissions, trusted with its warning, agent-built naming the agent); Settings
  → Widgets showing each state; a sample sandboxed widget in a pane; the missing-
  widget placeholder; the result of Build a widget's first widget.
- **Pages:** Memory, Connectors, Bundles, Bundle Format (ABF) (import/export only),
  Identity & Accounts, Auth flows, Your own widgets, Build a widget, Widget
  security.

### Part 5 — Swarm, Drone, Warden, Sysinfo and Tower in use (23 shots)

- **Tooling:** a spike to settle where Swarm's tree and stats come from, then the
  matching seeding option (§5.2); a Drone fixture; Warden data (LAN peers, audit
  rows, supervisor decisions), most likely injected.
- **Shots:** Swarm with several agents, subagents, todos, running tools, a shell,
  a cron and background work; the fleet toolbar; the Stats panel; a copy menu;
  Clear completed; a Drone flow in the editor, the block palette and a running
  drone; each of Warden's five sections; each Sysinfo plot type; the per-pane CPU
  and memory badges; Tower's Agents and Processes tabs (from a quiet, clean
  machine only: they list real processes) and its fresh widget image.
- **Pages:** Swarm (`subagent-watcher`), Pane Types (Swarm, Drone, Sysinfo),
  Warden widget, System Metrics, Widget gallery (Tower), and a new Tower page.

### Part 6 — Panes in depth, Hangar and pane management (33 shots)

- **Tooling:** drag capture mid-gesture (CDP `Input.dispatchDragEvent` or a held
  mouse) for drop zones; Browser-pane compositing (#162); a demo image and video
  added to `demo-env`; a way to keep Hangar's Places and the terminal connection
  picker from listing the OS user's own folders and hosts (§7).
- **Shots:** Terminal with history in the demo project, its find bar, the
  connection picker and the drop-files overlay; Editor's file tree and tabs,
  Markdown split preview, find and replace, language-server status, open-from-
  remote and the tree menu; Media with a demo image and a video; Hangar's list,
  Places, thumbnail grid, preview, row menu and conflict dialog; Browser's empty
  state, a composited page, bookmarks and profile flyouts, HTTP sign-in prompt,
  load-failure banner, agent-driven bars, permission prompt and page menu; voice
  input; a pane with several tabs; drag-and-drop drop zones; a floating pane; the
  tray menu (one OS-level shot).
- **Pages:** Pane Types (Terminal, Editor, Media, Pane Management, Voice input),
  a new Hangar section, Browser pane, Installation (tray), Running multiple
  instances (tear-off, as a recording in Part 7).

### Part 7 — Variants and motion

- **Light theme:** re-run about 15 key shots under the Light theme (main window,
  Agent picker and a conversation, Swarm, Memory, Connectors, Settings), plus the
  optional docs component that matches the site's mode (§5.6).
- **2×** for the landing site's picks.
- **Recordings** (§5.8), 6–8: create a first agent, split and drag panes, Swarm
  following a running agent, Hangar drag and drop, import a bundle, approve a
  widget, the command palette.

### Part 8 — Refresh, publish and keep fresh

- `refresh.mjs` and the publish step (§5.9); the version stamp and a staleness
  check in agentmux-docs; a documented per-release re-capture.
- **Done when:** one command produces a reviewed candidate set from a clean
  instance, and the docs show which AgentMux version each image is from.

## 7. Risks and open questions

- **Privacy is the main risk.** Views that look empty still show workspace data
  (spec §6a: agent names in My Agents, bundle names). Remotes reads
  `~/.ssh/config`; Toolchain shows install paths; accounts show emails.
  Mitigations: always an isolated data folder, demo names only, the
  `containsWorkspaceData` gate, and a human review of every image. Two specific
  cases found in this survey:
  - **The status bar shows the machine's hostname**, so every full-window shot
    and every status-bar crop identifies the capturing machine, even from an
    isolated data folder. Part 2 adds a capture-time override.
  - **LAN discovery's Show QR code popover encodes the instance's full auth key**
    (docs `lan-discovery.md`). Never capture it from a real instance. If the docs
    want it, take it from a throwaway instance whose key is discarded right
    after, and consider a placeholder graphic instead.
  - **The empty-tab screen shows `user@host`, the version and the git hash**
    (`tab/tabcontent.tsx`), and the native pre-splash shows `user@host` too.
  - **`AGENTMUX_HOME_OVERRIDE` only moves AgentMux's own data.** Several surfaces
    read the OS user's machine directly and stay identifying from an isolated
    instance: Hangar's Places (home folders, drives, WSL distros), Remotes and the
    terminal connection picker (the OS user's `~/.ssh/config`), Toolchain (PATH and
    install paths), Tower (the real process list), Sysinfo's per-core view and the
    disk-volumes popover. The robust fix is to capture those shots on a clean VM
    (or a dedicated OS user) with neutral paths and no real hosts. The cheaper fix
    is small capture-mode overrides in the app (a Places root, an SSH config path,
    a display hostname). The plan marks each such shot ⚠ or ⛔ in Appendix A.
- **Swarm's data source** decides which seeding option it needs (§5.2). Unknown
  until Part 5's spike.
- **Separate windows** (approval windows, torn-off panes) need the runner to
  attach to a second page target (§5.5); until then those shots wait.
- **Packaged vs dev build.** Seeding through Vite modules ties content suites to
  a dev build until the demo hook exists.
- **Image churn.** The UI changes weekly. Prefer tight crops of stable surfaces,
  re-capture per release, and keep the number of near-duplicate shots down.
- **Repo size.** Source PNGs live in git even though Astro optimizes the output.
  Compress before committing (or commit WebP), and capture at scale 1 for docs.
- **Capture machine.** The first run couldn't start a third CEF instance on a
  GPU-saturated machine (report `REPORT_SCREENSHOT_TOOLING_ISOLATED_INSTANCE_SETUP_BLOCKERS_2026_09_19.md`).
  Capture on a quiet machine, ideally with nothing else running.
- **Docs drift found during the survey**, worth fixing in the same page passes
  (screenshots tend to expose drift anyway):
  - Settings reference lists 9 themes; the app has 13 (Light, Catppuccin Latte,
    Solarized Light and Gruvbox Light are missing; `menu/base-menus.ts`).
  - Memory and Connectors list the Stash's tabs as six; there are seven
    (**Devices** is missing; `agent/components/AgentStashModal.tsx`).
  - Main menu & command palette doesn't list the hamburger's **Layouts** submenu
    (Save layout… / Open layout…; `window/hamburger-menu.tsx`).
  - **Tower** (#4498, a read-only task manager widget) has no docs page and isn't
    in the widget gallery.
  - **Hangar** is pinned by default but has no section beyond its gallery image.
  - Identity & Accounts' **Launch flow** section describes the Launch Agent modal
    (and its Pre-Launch Auth panel) as how accounts are bound at launch. Nothing
    opens that modal any more (`openLaunchModal` in `AgentPicker.tsx` has no
    caller); Auth flows already says so, Identity doesn't.
  - Bundles draws the Create new agent form as an ASCII diagram, the clearest
    single place a screenshot would replace something worse.
  - The spec's §8.2 says the pane `+` and right-click menus are native; they're
    DOM overlays (§5.5). Some code comments say the same (`pane-tab-picker.ts`,
    `action-widgets-menu.ts`).

## Appendix A — Experiences and shots by page

In docs sidebar order. **Shot id** is a suggested stable id for the manifest
(prefix = the suite it would live in). **Class** is from §3. **Part** is §6. A
shot listed once is reused on other pages by linking the same image; each page
below lists only the shots that belong to it first. Crop selectors come from the
code survey (paths under `frontend/app/`); they're starting points, verified when
the shot is written.

Privacy marks: **⚠** = always shows machine or workspace data, so it must come
from the isolated, seeded instance and be reviewed; **⛔** = must never be
captured from a real instance at all.

### A.1 Getting Started (`getting-started`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| The main window, starter layout (Agent picker, Sysinfo, Swarm), labelled | `chrome-main-window` | A | 2 | Medium size. ⚠ hostname in the status bar (§7); the tooling override comes first. |
| An empty window tab | `chrome-empty-tab` | A | 2 | ⚠ shows `user@host`, version and git hash (`tab/tabcontent.tsx`). |

### A.2 Installation (`installation`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| The in-page startup screen | `install-startup-screen` | A | 2 | `#startup-loading` in `index.html`. The native pre-splash before it is E and shows `user@host`; skip it. |
| System tray icon and its menu | `install-tray-menu` | E | 6 | OS-level capture, one platform only. |
| Installers, Store page, AppArmor error | — | E | — | Not AgentMux UI. Keep as text. |

### A.3 Quickstart (`quickstart`)

All shots are first listed elsewhere; this page reuses `chrome-main-window`,
`chrome-top-bar`, `chrome-status-bar`, `agent-picker-fresh`,
`onboard-install-progress`, `onboard-create-host`, `onboard-signin-not-signed-in`,
`agent-turn-midstream`, `chrome-split-layout`, `chrome-palette` and
`memory-bundle-new`. It's the page most worth a recording (`rec-first-agent`,
Part 7).

### A.4 First Agent Setup (`first-agent`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Agent picker, fresh (New Agent cards only) | `agent-picker-fresh` | A | 1 | Already captured (`widget-agent-*`). |
| Agent picker with My Agents, filter bar and sort | `agent-picker-populated` | C | 3 | ⚠ agent names → demo agents only. `.agent-picker`. |
| My Agents row menu (Rename, Duplicate, View History, Delete) | `agent-picker-row-menu` | C | 3 | Menus are DOM (`.menu`), so capturable. |
| "Already open" choice (Open new session / Switch to existing) | `agent-picker-already-open` | C | 3 | |
| Install dialog: steps in progress | `onboard-install-progress` | D | 3 | `element/install/InstallDialog.tsx`, modal kind `install-agent`. |
| Install dialog: done (Continue to Launch) | `onboard-install-done` | D | 3 | |
| Install dialog: failed, Details open, Retry | `onboard-install-failed` | D | 3 | |
| Prerequisites dialog (missing tool, one-click install, Refresh, Launch anyway) | `onboard-prereqs` | D | 3 | Modal kind `agent-prereqs`. |
| Create new agent: host runtime | `onboard-create-host` | B | 3 | Modal kind `create-from-template`. |
| Create new agent: container runtime selected | `onboard-create-container` | B | 3 | Needs Docker on the capture machine, or a fixture. |
| Create new agent: container greyed, "Docker not detected" | `onboard-create-no-docker` | B/D | 3 | |
| Create new agent: Claude with Model and Model Vendor fields, two bundles added | `onboard-create-claude-full` | C | 3 | Shows the Bundles list editor. |
| Sign-in row: Not signed in (Log in, Login via terminal, Connectors → Accounts) | `onboard-signin-not-signed-in` | D | 3 | `failure/failure-accessory.ts`. |
| Sign-in row: No account linked, with Bind account | `onboard-signin-no-account` | D | 3 | |
| In-pane login: link shown and the authorization-code box | `onboard-signin-in-app` | D | 3 | `components/InAppLoginPanel.tsx`. ⚠ auth URL → fixture with a fake URL. |
| The agent pane mid-turn: streaming text, tool calls, an Edit diff, Working… timer | `agent-turn-midstream` | C | 3 | The page's hero image. Scripted conversation about `acme-web`. |
| Disconnected from stream banner with Reconnect | `agent-disconnected` | D | 3 | `AgentDisconnectedBanner.tsx`. |
| Shutting down… log | `agent-shutdown-log` | D | 3 | `shutdown/ShutdownOverlay.tsx`. |
| Shutdown stuck: Try again / Keep open | `agent-shutdown-stuck` | D | 3 | |
| Quit countdown banner ("Closing in 15 s", Keep running) | `agent-quit-countdown` | D | 3 | `shutdown/ShutdownPendingBanner.tsx`. |
| Shell drawer with a `!git status` result | `agent-shell-drawer` | C | 3 | `AgentShellDrawer.tsx`. |
| "The agent is asking" panel with its countdown | `agent-question-panel` | D | 3 | `AgentQuestionPanel.tsx`. |
| Failure row: rate limited, retrying with countdown, Retry now | `agent-failure-rate-limited` | D | 3 | One shot per row type is enough for these three. |
| Failure row: usage limit, Accounts (switch / upgrade) | `agent-failure-usage-limit` | D | 3 | |
| Failure row: context window exceeded, New session | `agent-failure-context` | D | 3 | |
| Failure row: running in another instance, Take over | `agent-failure-takeover` | D | 3 | |

### A.5 Pane Types (`pane-types`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| **Terminal:** fresh | `widget-terminal-*` | A | 1 | Exists. |
| Terminal with a few commands run in the demo project | `term-history` | C | 6 | ⚠ prompt shows path/host → demo project, neutral prompt. |
| Terminal find bar | `term-find` | B | 6 | `.search-container`. |
| Change Connection typeahead | `term-connection-picker` | B | 6 | ⚠ lists SSH hosts and WSL distros. Needs an empty `~/.ssh/config` view (§7). |
| Drop-files indicator ("Copy N files to …") | `term-file-drop` | D | 6 | Mid-drag capture (§5, Part 6). |
| **Editor:** empty | `widget-editor-*` | A | 1 | Exists. |
| Editor with the file tree, document tabs and a file open | `editor-tree-tabs` | C | 6 | Demo project. |
| Markdown preview, Split mode | `editor-md-split` | C | 6 | `.editor-mode-toolbar`, `.editor-preview-pane`. |
| Find / replace | `editor-find-replace` | B | 6 | CodeMirror `.cm-search`. |
| Language server: status chip and an install banner, or diagnostics in the gutter | `editor-lsp` | C | 6 | |
| Open from remote typeahead | `editor-open-remote` | B | 6 | ⚠ hosts. |
| File tree right-click menu | `editor-tree-menu` | B | 6 | DOM `.ctx-menu`. |
| **Media:** empty | `widget-media-*` | A | 1 | Exists. |
| Media with a demo image (zoom badge) | `media-image` | C | 6 | Add a demo image and clip to `demo-env`. |
| Media playing a demo video | `media-video` | C | 6 | |
| **Agent:** the pane tab's activity indicator and colours | `agent-pane-tabs` | C | 3 | |
| Activity dock | `agent-activity-dock` | C | 3 | `ActivityDock.tsx`. |
| Agent History view (day dividers) | `agent-history` | C | 3 | `history/AgentHistoryView.tsx`. |
| Pending messages / send-now queue | `agent-send-queue` | C/D | 3 | `PendingMessagesPanel.tsx`. |
| Persistent shell block in the transcript | `agent-persistent-shell` | C | 3 | |
| Next-prompt suggestion (ghost text) | `agent-ghost-text` | D | 3 | |
| Composer strip, labelled | `agent-composer-strip` | C | 3 | Candidate for numbered callouts (§5.7). |
| Attachments in the composer | `agent-attachments` | C | 3 | |
| Side question overlay (`/btw`) | `agent-btw` | C/D | 3 | `BtwOverlay.tsx`. |
| Details drawer with the embedded shell | `agent-details-drawer` | C | 3 | `ResizableDetailsDrawer.tsx`. |
| Slash command picker and autocomplete | `agent-slash-picker` | B | 3 | |
| Runtime picker (model / runtime drop-up) | `agent-runtime-dropup` | B | 3 | `AgentRuntimeDropup.tsx`. |
| In-transcript search (Ctrl+F) | `agent-search` | C | 3 | `AgentSearchBar.tsx`. |
| Subagent dispatch card ("View in Swarm →") | `agent-dispatch-card` | C | 3 | `tool-renderers/DispatchCard.tsx`. |
| Permission prompt (Tool, Target, Scope, Allow / Deny) | `agent-permission-prompt` | D | 3 | `AgentDecisionPanel.tsx`. Shown in Default and Plan permission modes. ⚠ target paths → demo project. |
| Compaction: "Compacting conversation…" and the compacted boundary | `agent-compaction` | C/D | 3 | `.agent-compaction-started`, `.agent-context-compacted`. |
| Session stats popover (tokens, cost, Archive / Export) and the auto-compact countdown | `agent-session-stats` | C | 3 | `AgentSessionStats.tsx`. |
| Session notices (large session → Archive; archived → Restore / Export) | `agent-session-notices` | D | 3 | `AgentSessionNotices.tsx`. |
| An inter-agent message (jekt) bubble | `agent-jekt-bubble` | C | 3 | `JektBubble.tsx`. ⚠ peer agent names → demo. |
| Composer auth tag and its account switch menu | `agent-account-switch` | C | 3 | ⚠ emails → made-up accounts. |
| **Drone:** empty | `widget-drone-*` | A | 1 | Exists. |
| Drone with a wired flow (Variables → Agent → Condition → Response) | `drone-flow` | C | 5 | `.drone-canvas`. |
| Drone run in progress, runs footer | `drone-run` | C/D | 5 | `.drone-runpanel`. |
| **Sysinfo:** CPU + Mem + Net (demo default) | `widget-sysinfo-*` | A | 1 | Exists. |
| Sysinfo: All CPU (per core) and Disk I/O | `sysinfo-all-cpu`, `sysinfo-disk` | A | 5 | Per-core count reveals the machine's CPU; acceptable, but note it. |
| Sysinfo plot-type menu | `sysinfo-plot-menu` | B | 5 | DOM menu. |
| **Voice input:** recording state in the composer | `voice-recording` | D | 6 | |
| **Pane management:** a pane with several tabs | `pane-tabs-multi` | C | 6 | `.pane-tab-strip`. |
| The pane `+` widget picker | `pane-add-menu` | B | 2 | DOM `.menu` (not native, see §5.5). |
| Pane right-click menu (Split, Magnify, Close…) | `pane-context-menu` | B | 2 | DOM `.menu`. |
| Split layout | `chrome-split-layout` | B | 2 | |
| A maximized (magnified) pane | `chrome-maximized` | B | 2 | |
| Drop zones while dragging a pane | `pane-drag-zones` | D | 6 | Mid-drag capture. |
| A floating pane | `pane-floating` | B | 6 | |
| Tab tear-off into a new window | `rec-tear-off` | F | 7 | Recording; spans two windows. |

### A.5b Hangar (pinned by default, but no docs section yet; only the gallery image)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Details list in the demo project, with git letters and an agent-touch dot | `hangar-list` | C | 6 | `.files-view`. ⚠ status line shows the git branch → demo repo. |
| Places sidebar (Places, Drives, WSL, Remote, Agents) | `hangar-places` | C | 6 | ⛔ from an ordinary machine: lists the OS user's folders, drives and WSL distros, which the data-folder override doesn't cover. Needs a clean VM or a Places override. |
| Thumbnail grid | `hangar-grid` | C | 6 | |
| Preview panel (Markdown, code, image) | `hangar-preview` | C | 6 | `.files-preview`. |
| Row right-click menu (Open in Editor, Attach to <agent>…) | `hangar-row-menu` | B | 6 | DOM `.ctx-menu`. ⚠ agent names → demo. |
| Paste conflict dialog (Keep both / Skip / Replace) | `hangar-conflict` | D | 6 | `.files-conflict`. |

### A.6 Widget gallery (`widget-gallery`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Tower (new since the gallery was captured) | `widget-tower-*` | A/C | 5 | ⚠ fresh state still lists the machine's processes; needs review or a quiet machine. |
| Re-capture of all 14 at the current version | `widget-*` | A | 8 | Captured from 0.59.14; part of the per-release refresh. |

### A.7 Your own widgets (`widgets`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Approval prompt: sandboxed widget with its permissions | `widget-approve-sandboxed` | D | 4 | Install a demo widget from the capture run. |
| Approval prompt: trusted widget, full-access warning | `widget-approve-trusted` | D | 4 | |
| Approval prompt from an agent ("<agent> wants to install…", Don't install) | `widget-approve-agent` | D | 4 | |
| Settings → Widgets with one widget in each state (Approved, Needs approval, Changed, Off, Can't be loaded) | `settings-widgets-states` | C/D | 4 | `.widgets-list`. |
| A sandboxed widget running in a pane | `widget-user-running` | C | 4 | |
| Missing-widget placeholder (Open Settings → Widgets) | `widget-missing` | D | 4 | `block/missing-widget.tsx`. |

### A.8 Build a widget (`build-a-widget`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| The page's first widget, running in a pane | `widget-build-first` | C | 4 | Build the sample from the page during the capture run. |

### A.9 Browser pane (`browser-pane`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Empty state ("Enter a URL above to browse") with the nav bar | `browser-empty` | A | 6 | DOM; capturable today. |
| A page loaded, with nav bar | `browser-page` | E | 6 | Needs compositing (#162). Use a neutral page (e.g. the docs site). |
| Bookmarks flyout | `browser-bookmarks` | B | 6 | DOM `.menu`. |
| Profile flyout and New profile dialog | `browser-profiles` | B | 6 | |
| HTTP sign-in prompt | `browser-auth-prompt` | D | 6 | Modal kind `browser-auth`. |
| Load-failure banner | `browser-load-failure` | D | 6 | `.browser-error`. |
| Agent attention banner and "Driven by <agent>" bar | `browser-agent-driven` | D | 6 | ⚠ agent name → demo. |
| Camera / microphone permission prompt | `browser-permission` | D | 6 | |
| Page right-click menu | `browser-page-menu` | B | 6 | DOM `.menu`. |

### A.10 Connectors (`connectors`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Accounts: service tile gallery, nothing connected | `widget-connectors-*` | A | 1 | Exists. |
| Auth-mode chooser (OAuth / API key) | `conn-chooser` | B | 4 | `.accounts-chooser`. |
| Connect AgentMux panel | `conn-agentmux` | B | 4 | Not-connected state only; connected shows an email. |
| Anthropic in-app login steps | `conn-anthropic-login` | D | 4 | ⚠ auth URL → fixture. |
| Connected accounts list with one account's detail open | `conn-accounts-detail` | C | 4 | ⚠ made-up accounts only (§5.3). |
| Add account form | `conn-add-account` | B | 4 | `.identity-form`. |
| OAuth device-code panel | `conn-oauth-device` | D | 4 | Fake code. |
| Delete account confirmation | `conn-delete-account` | C | 4 | |
| Bind to Agent menu | `conn-bind-menu` | C | 4 | DOM `.menu`. ⚠ agent names → demo. |
| MCP servers: list and a server's detail with its status pill | `conn-mcp-detail` | C | 4 | |
| MCP server form | `conn-mcp-form` | B | 4 | |
| MCP catalog picker | `conn-mcp-catalog` | B | 4 | `.mcp-picker`. |
| Remotes: empty | `widget-remotes-*` | A | 1 | Exists, but ⚠ reads the OS user's `~/.ssh/config` (§7). |
| Remotes: populated, a row expanded | `conn-remotes-detail` | C | 4 | ⛔ unless every host is a demo host; needs an SSH-config override. |
| Add remote form | `conn-remotes-add` | B | 4 | |
| SSH approval window (consent variant) | `conn-ssh-approval` | D | 4 | Separate window: needs a second capture target. ⚠ demo host and agent. |

### A.11 Memory (`memory`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Global: tiles, CLAUDE.md tile, Combined preview tile | `memory-global` | C | 4 | `.global-bundle`. |
| Global: an entry open, with History | `memory-global-entry` | C | 4 | `.memory-pinned-layout`. |
| Global: add / edit memory | `memory-global-edit` | B | 4 | |
| Global: Combined preview | `memory-global-preview` | C | 4 | |
| Global: isolated-channel import banner | `memory-global-import` | D | 4 | ⚠ channel names. |
| Personal: agent grid with filter bar | `memory-personal-agents` | C | 4 | ⚠ agent names → demo. |
| Personal: a file with history and a diff | `memory-personal-diff` | C | 4 | |
| Personal: earlier-memory (adopt) panel and claimed-folders panel | `memory-personal-adopt` | D | 4 | ⚠ paths and account ids → fixture. |
| Adoption approval window | `memory-adopt-approval` | D | 4 | Separate window (second capture target). |
| Skills: list and a skill's detail | `memory-skills` | C | 4 | |
| Skills: new skill form | `memory-skill-new` | B | 4 | |
| Bundles: rail and a bundle's detail | `memory-bundles` | C | 4 | `.bundle-view-rail`. |
| Bundles: new bundle form | `memory-bundle-new` | B | 4 | `.bundle-view-form`. |
| Bundles: per-provider overrides and the validation report | `memory-bundle-overrides` | B | 4 | |
| Import Bundle step 2: Preview & select, with a skill collision | `memory-import-preview` | C/D | 4 | Step 1 opens an OS file dialog (E): open step 2 directly through the modal layer with a fixture `.abf`. |
| Import Bundle step 3: Confirm, then "Bundle imported." | `memory-import-confirm` | C/D | 4 | |

### A.12 Bundles (`bundles`), Bundle Format (`abf`), Identity & Accounts (`identity`), Auth flows (`auth`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Stash drawer: Accounts | `stash-accounts` | C | 3 | `AgentStashModal.tsx` (7 tabs). ⚠ demo account. |
| Stash: Personal Memory | `stash-memory` | C | 3 | |
| Stash: MCP Servers | `stash-mcp` | C | 3 | |
| Stash: Skills | `stash-skills` | C | 3 | |
| Stash: Bundles | `stash-bundles` | C | 3 | Used on Bundles. |
| Stash: Registration | `stash-registration` | C | 3 | |
| Stash: Devices | `stash-devices` | C | 3 | Missing from the docs' tab list (§7). |
| Identity pane: linked accounts | `identity-pane` | C | 3 | `.identity-pane`. |
| The Create new agent form, replacing Bundles' ASCII diagram | `onboard-create-claude-full` | C | 3 | Listed in A.4; noted here because the page currently draws it in ASCII. |

ABF reuses `memory-import-*`; Auth flows reuses the sign-in and Connectors shots.

**Not captured, on purpose:** the Launch Agent modal, its pre-launch OAuth panel,
and the Add Account / New Bundle dialogs it opens. `AgentPicker.tsx` defines
`openLaunchModal` but nothing calls it, so no user can reach them. The modal kinds
`agent-identity` and `agent-memory` have no opener either. Identity & Accounts'
"Launch flow" section still describes the Launch Agent modal as live (§7).

### A.13 Swarm (`subagent-watcher`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Empty ("No active agent panes") | `widget-swarm-*` | A | 1 | Exists. |
| Populated tree: agents with status chips, todos, a subagent, running tools | `swarm-tree` | C | 5 | The page's hero image. ⚠ demo agents. |
| A subagent's activity feed expanded | `swarm-subagent-feed` | C | 5 | |
| Workflow, Shell, Cron, Running and Background buckets | `swarm-buckets` | C | 5 | One or two shots. |
| Fleet toolbar with a selection, and the inline Broadcast composer | `swarm-fleet-broadcast` | C | 5 | |
| Stop N agents? with staged rollout | `swarm-fleet-stop` | C | 5 | |
| Fleet result panel | `swarm-fleet-result` | C | 5 | |
| Stats panel | `swarm-stats` | C | 5 | |
| A copy menu | `swarm-copy-menu` | C | 5 | DOM `.menu`. |
| Other instances section | `swarm-remote` | C | 5 | ⛔ unless every instance shown is a demo instance. |

### A.14 Running multiple instances, LAN discovery, Warden

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Instance Panel (version chip), with the per-window opacity slider | `chrome-instance-panel` | A | 2 | Also used by Window appearance and Report Issues. ⚠ instance details. |
| Host popover, LAN discovery off | `chrome-host-popover` | A | 2 | ⚠ hostname (§7). |
| Host popover, LAN discovery on with peers | `chrome-host-popover-lan` | C/D | 2 | Fixture peers. |
| Show QR code (mobile pairing) | `chrome-pair-qr` | D | — | ⛔ encodes the instance's full auth key. Placeholder graphic recommended. |
| Warden: Host | `warden-host` | C | 5 | ⚠ agent and block ids → demo. |
| Warden: LAN | `warden-lan` | C/D | 5 | Fixture peers. |
| Warden: Internet (stub) | `warden-internet` | A | 5 | |
| Warden: Audit feed | `warden-audit` | C/D | 5 | |
| Warden: Supervisor with recent decisions | `warden-supervisor` | C/D | 5 | |

### A.15 Window appearance (`window-appearance`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Hamburger → Opacity submenu | `chrome-menu-opacity` | B | 2 | |
| Instance Panel opacity slider | `chrome-instance-panel` | A | 2 | Shared with A.14. |
| A translucent window over the desktop | `window-translucent` | E | 6 | Translucency is applied by the OS window, not the page. Optional. |

### A.16 Settings reference (`settings`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Appearance | `settings-appearance` | B | 2 | Existing manual-suite shot; re-verify. |
| Window & Panes | `settings-window-panes` | B | 2 | |
| Browser | `settings-browser` | B | 2 | Profile names → none, or demo. |
| Terminal | `settings-terminal` | B | 2 | |
| Sounds | `settings-sounds` | B | 2 | |
| Notifications & Tray | `settings-notifications` | B | 2 | |
| Recording | `settings-recording` | B | 2 | ⚠ microphone device names, local paths. |
| Paired devices | `settings-devices` | B | 2 | ⚠ device names, MuxBus email → empty state only. |
| Widgets | `settings-widgets` | B | 2 | Empty state here; populated states in A.7. |
| Advanced | `settings-advanced` | B | 2 | ⚠ global environment variables editor → keep empty. |
| Search with results | `settings-search` | B | 2 | `.settings-search-results`. |
| Config errors banner | `settings-config-errors` | D | 2 | ⚠ config path. |

### A.17 Main menu & command palette (`main-menu`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Hamburger menu | `chrome-hamburger` | B | 2 | Existing shot. |
| Theme submenu | `chrome-menu-theme` | B | 2 | |
| Layouts submenu and the layout preview dialog | `chrome-menu-layouts` | B | 2 | Not in the docs page yet (§7). |
| Command palette, empty | `chrome-palette` | B | 2 | |
| Command palette, filtered | `chrome-palette-filtered` | B | 2 | |
| Title-bar right-click menu, tab colour/rename panel, More dropdown | `chrome-tab-menus` | B | 2 | Two or three shots. |

### A.18 Keybindings (`keybindings`)

Reuses the Help pane (`widget-help-*`). Pane-border and window-edge resizing are
better as a recording (`rec-resize`); Windows snapping is E (OS behaviour).

### A.19 System Metrics (`system-metrics`)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Status bar strip | `chrome-status-bar` | A | 2 | ⚠ hostname. |
| CPU cores popover, Disk volumes popover, Backend popover | `chrome-status-popovers` | B | 2 | ⚠ disk volume names. |
| Token Usage popover | `chrome-token-usage` | C | 3 | ⚠ agent names → demo. |
| Per-pane CPU / memory badges | `pane-stats-badges` | C | 5 | `.block-stats-badge`. |

### A.20 Tower (no docs page yet)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Agents tab with an agent's process tree | `tower-agents` | C | 5 | ⚠⚠ process names, PIDs, commands. Quiet, clean capture machine only. |
| Processes tab | `tower-processes` | C | 5 | Same. |
| Machine select and pair form | `tower-pair` | B | 5 | ⚠ pairing link → fixture. |

### A.21 Toolchain (hamburger → Toolchain; referenced from First Agent Setup)

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Toolchain pane | `toolchain` | A | 2 | ⛔ from an ordinary machine (PATH, install paths). Only from a clean VM or container whose paths are neutral. |
| Inline one-click system install confirmation | `toolchain-install-inline` | D | 3 | |

### A.22 Help, notices and global dialogs

| Experience / state | Shot id | Class | Part | Notes |
|---|---|---|---|---|
| Help pane | `widget-help-*` | A | 1 | Exists. |
| Error toast | `chrome-toast-error` | D | 2 | `.flash-error-container`. Notification bubbles only render in dev builds. |
| Config error message dialog (from the status bar) | `chrome-config-error` | D | 2 | |
| User input dialog (backend prompt) | `chrome-user-input` | D | 2 | |
| Close tab? / Close this pane? confirmations | `chrome-confirm-close` | B | 2 | |

### A.23 Pages with no UX experience (out of scope)

Widget API reference, the ABF format sections, Configuration guide (its "Open raw
settings.json" opens an external editor), Report Issues (reuses the Instance
Panel), Glossary, all of Security & posture except the shots reused above, and
all of Internals.

## Appendix B — What a page screenshot can and can't capture

`Page.captureScreenshot` captures the app page's DOM. From the code survey:

**Capturable (page content), even though some look or are named "native":**

| Surface | Where | Note |
|---|---|---|
| Right-click menus, the pane `+` widget picker, Swarm copy menus, Bind to Agent | `host/js-context-menu.ts` (`#cef-context-menu-overlay .menu`), installed by `util/cef-api.ts` | Code comments calling these "native" are out of date. |
| Hamburger menu and submenus, More dropdown, title-bar menu, tab colour/rename panel | `window/hamburger-menu.tsx`, `titlebar-context-menu.tsx`, `tab/tab.tsx` | |
| Page menus in Hangar, Remotes, the editor's file tree | `components/context-menu.tsx` (`.ctx-menu`) | |
| Every modal-layer modal and global modal (command palette, user input, message, replace-pane confirm, layout preview, widget install prompt) | `element/modal-layer.ts`, `modals/` | Crop to `.modal-panel`. |
| The in-page startup screen | `#startup-loading` in `index.html` | |
| Error toasts | `.flash-error-container` | Notification bubbles render only in dev builds. |

**Capturable, but in a different window** (needs a second capture target, §5.5):

| Window | Opened by |
|---|---|
| SSH approval (`.ssh-approval-window`) | agent SSH use, terminal ssh prompts, Remotes add / test / remove helper / end session |
| Saved-credential approval (`.credential-approval-window`) | a browser pane hitting HTTP auth for a site with a saved credential |
| Memory adoption approval (`.memory-adoption-approval-window`) | Memory → Personal → Adopt selected… / Release… |
| A torn-off floating pane | dragging a tab out |

**Not capturable from the page** (class E):

| Surface | Why | Plan |
|---|---|---|
| Browser-pane web content | native child window (Windows, `crates/cef/src/browser_pane/creation.rs`) or CEF overlay view (macOS/Linux) | Composite its own capture (#162). |
| OS file dialogs | `rfd::FileDialog` (`crates/cef/src/commands/platform.rs`) | Start the flow one step later from a fixture. |
| OS notifications ("Send test notification", agent alerts) | OS toast | Text. |
| System tray icon and menu | OS | One OS-level shot on Installation. |
| macOS menu bar | `crates/cef/src/macos_menu.rs` | Text. |
| Window translucency | OS layered window | Optional OS-level shot. |
| Native pre-splash | `crates/launcher/src/splash.rs` (shows `user@host`) | Skip. |
| DevTools, "Online Docs" (external browser), "Open raw settings.json" (external editor) | external windows | Text. |
| `<select>` dropdown popups (inferred, not verified) | Chromium popup widget | Capture with the select closed. |
