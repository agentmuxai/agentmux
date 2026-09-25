# Spec: Browser pane identities — shared by default, private (unique incognito) per pane tab, named profiles later

**Status:** proposed — nothing in this spec is implemented. Written
2026-09-22 against `main` @ `575947270` (v0.56.12). Every file:line
citation below was read on that commit; spot-verify before trusting.

## 0. The ask, verbatim

> we want to implement a muple identity mode for our browser, but moreso
> along the line as multiple identities, where in one pane tab I can be
> logged in one oauth session and in another pan tab I can be in an
> entirely different one. we already have a cross window demarcation, but
> currently, from my understanding, if 2 browser panes share the same
> window, they share the oauth identity. we want to keep that as the
> default, but add some UX that makes it possible for an alternate,
> perhaps an incognito, but each incognito would be unique

Restated as requirements:

1. **Default stays shared.** Two browser pane tabs with no opt-in keep
   sharing one login session, exactly as today.
2. **Opt-in per pane tab.** A browser pane tab can be put in its own
   identity so a second OAuth session coexists with the first.
3. **Each opt-in is unique.** Two "incognito" pane tabs do not share with
   each other either. There is no single "the incognito jar".
4. **Discoverable.** There is UX to choose this, and to see which pane
   tabs are in which state.

## 1. Where things actually stand (verified 2026-09-22)

The ask's premise — "cross window demarcation exists, same-window panes
share" — is only half right, and differs by platform. This matters
because the design has to start from what is true, not from the mental
model.

### 1.1 Windows: every browser pane, in every window, shares ONE jar

The Windows native-child path passes `None` for `request_context`, i.e.
the global default, disk-backed context:

- Docked panes: `agentmux-cef/src/browser_pane/creation.rs:186-193`
  (`None, // request_context`).
- Floating / torn-off panes: `agentmux-cef/src/floating_pane.rs:264-271`
  and `:484-496`, also `None`.

The per-window isolated `RequestContext`
(`agentmux-cef/src/commands/mod.rs:54-93`) exists, but it is created
only for a secondary window's **main renderer**
(`agentmux-cef/src/ui_tasks/window.rs:1635-1653`) so that window gets
its own SolidJS process. Browser panes on Windows never touch it. There
is **no cross-window cookie demarcation for browser panes on Windows**
today; log in to GitHub in any browser pane and every other browser pane
in every window is logged in too.

### 1.2 Linux / macOS: panes inherit their window's context, by accident of a crash fix

The Views path resolves the parent window's context and passes it
through (`agentmux-cef/src/browser_pane/creation_views.rs:154-173`).
That was done to stop a FATAL, not to give users isolation
(`docs/specs/multi-window-pane-and-newwindow-fixes-linux-2026-05-15.md`
§Bug 1). The consequence:

| Window | Context the pane gets | Persistence |
|---|---|---|
| main | global default | disk-backed, survives restart |
| any secondary window | that window's unique off-the-record profile | **in-memory; login lost when the window closes** |

So on Views platforms there *is* a cross-window demarcation, but a
secondary-window browser pane is silently incognito, which is a
data-loss surprise nobody chose. This spec fixes that as a side effect
(§4.2).

### 1.3 No identity concept exists anywhere in the pane pipeline

- Host IPC `browser_pane_create` args: `block_id, url, x, y, width,
  height, window_label` — `agentmux-cef/src/ipc.rs:443-461`.
- Frontend caller: `frontend/app/view/browser/use-pane-rect-sync.ts:164-187`.
- Block meta for a browser block is `view` + `url` (+
  `browser:show_controls`): `agentmux-srv/src/server/app_api/pane.rs:246-251`,
  `agentmux-srv/src/config/widgets.json` (Discord/Slack/Telegram/
  WhatsApp/Teams widgets).
- The word "identity" in browser code today means the **credential
  broker**: `block_id → owning window → identity_id → OS-keychain
  HTTP-auth/form credentials`
  (`agentmux-cef/src/credential_broker/mod.rs:312-391`,
  `agentmux-srv/src/server/service/credential.rs:149-194`). That is
  orthogonal to the Chromium cookie jar and is left untouched here.

### 1.4 Prior decisions this spec reopens

- `SPEC_NATIVE_BROWSER_PANE_2026_04_17.md` §Cookie Isolation: shared
  cookies for v1, per-pane `RequestContext` "deferred as a future
  opt-in". This is that opt-in.
- `SPEC_BROWSER_AND_EDITOR_PANES_2026_04_16.md`: P1 "separate cookie jar
  per pane" — never built.
- `REPORT_ARMORY_ZOOM_AND_PER_PANE_BROWSER_ZOOM_2026_07_20.md`: per-pane
  context judged "likely undesirable for most uses" → CSS zoom instead.
  Still true as a *default*; unchanged by this spec.

### 1.5 The two hard constraints

**(A) Disk-backed per-context profiles stall browser creation.** A
`RequestContextSettings.cache_path` that is a valid direct child of
`root_cache_path` makes Chrome materialize a real profile, after which
browser creation never completes — no `on_after_created`, no load, no
error (`agentmux-cef/src/commands/mod.rs:39-49`,
`docs/specs/SPEC_CEF_LOG_ROBUSTNESS_2026_06_20.md` §1.6(b), verified
live 2026-07-09). A grandchild path silently degrades to a unique
off-the-record profile. An **empty** `cache_path` reliably yields a
unique, in-memory off-the-record profile and is what every secondary
window already uses in production.

Consequence: a *persistent* second identity is blocked on solving the
stall. An *ephemeral* second identity is not blocked at all — it is the
proven code path.

**(B) On Views platforms, a pane whose `Profile*` differs from its
window's trips a CHECK.** Every distinct `RequestContext` is a distinct
`Profile*`, but all share one `ThemeService`;
`CefWidgetImpl::AddAssociatedProfile` re-adds the widget as an observer
and hits "Observers can only be added once!" → FATAL
(`creation_views.rs:140-153`). The Windows native-child path never
reaches `AddedToWidget`, so it is unaffected
(`multi-window-pane-and-newwindow-fixes-linux-2026-05-15.md` §Windows
impact).

Consequence: per-pane contexts ship on Windows first; Views platforms
need either a CEF-fork patch or a structural workaround (§4.4).

## 2. Goals and non-goals

**Goals**

- G1. A browser pane tab can be created as, or switched to, a **private**
  identity: its own cookie jar, localStorage, cache and service workers,
  shared with nothing else, discarded when the pane closes.
- G2. Every private pane tab is unique (requirement 3).
- G3. Default unchanged: no opt-in means the global shared jar on every
  platform, in every window — which *changes* Linux/macOS secondary
  windows from accidental-incognito to shared (§1.2, §4.2).
- G4. The identity is visible on the pane tab and pane header, not just
  inside the nav bar (the messaging widgets hide the nav bar via
  `browser:show_controls=false`).
- G5. Agents can request it through the App API (`pane.open`) and
  widgets can declare it in `widgets.json`.
- G6. OAuth popups opened from a private pane stay in that pane's
  identity. This is the whole point; a login flow that pops a window
  into the shared jar would defeat the feature.

**Non-goals (this spec)**

- Named, persistent profiles ("Work" / "Personal"). Wanted, and the
  most likely follow-up, but blocked on constraint A. Sketched as Phase
  3 so the data model does not paint us into a corner.
- Window-level default identity ("everything in this window is private").
  Cheap to add later on top of the block-level model; not in v1.
- Anti-fingerprinting, tracking protection, or anything "incognito"
  implies beyond a separate storage partition. The user-facing word is
  **Private**, and the tooltip says exactly what it is.
- Changing the credential broker's identity model (§1.3).

## 3. Model

### 3.1 Identity kinds

An identity is a property of the **browser block** (= the pane tab's
content, which is what the ask calls a "pane tab").

| kind | meta value | cookie jar | survives pane close | survives app restart |
|---|---|---|---|---|
| shared (default) | key absent | global default context | yes | yes |
| private | `"browser:identity": "private"` | a fresh off-the-record context created for this block | no | no (comes back logged out) |
| named (Phase 3) | `"browser:identity": "profile:<id>"` | disk-backed profile shared by every block naming it | yes | yes |

A single string key keeps `widgets.json`, `pane.open`, and session
restore trivial and leaves room for `profile:<id>` without a schema
change. No separate boolean.

### 3.2 Semantics that fall out

- **Duplicate pane** of a private pane → a *new* private pane, logged
  out. (Requirement 3. Copying a live jar is not offered.)
- **Move / tear-off / redock** within one app run → same jar. The host
  keeps the context keyed by `block_id` (§4.1), so any re-create of the
  native browser for the same block reattaches it.
- **Close pane** → the context is dropped; Chromium tears the OTR profile
  down when its last browser goes away.
- **Session restore** → a private block is recreated with a *fresh*
  context. A one-line inline notice in the pane ("Private session was
  reset when AgentMux restarted") tells the user why they are logged
  out; it dismisses on first navigation.
- **Switching a live pane** shared ↔ private → destroy and recreate the
  native browser at the current URL under the new context. The user is
  told in the menu item's confirmation that this logs the pane out. No
  attempt to migrate cookies.
- **Popups** (`window.open`, OAuth) inherit the opener's context. CEF
  does this by default for popups; both platforms' popup paths must be
  checked in the spike (§6, Phase 0) because the Views delegate turns
  popups into top-level windows
  (`creation_views.rs:126-130`) and those may currently be created with
  an explicit (different) context.
- **Per-pane zoom** is unaffected: it is CSS-injected precisely because
  it must not depend on the context
  (`agentmux-cef/src/state/mod.rs:554-565`).
- **Credential broker** is unaffected: keyed by `block_id`, not by
  context.

## 4. Host (`agentmux-cef`)

### 4.1 Context registry

Add to `AppState` (or to `BrowserPaneManager`, which already owns
per-block state) a `pane_contexts: Mutex<HashMap<String /* block_id */,
cef::RequestContext>>`.

`resolve_pane_request_context(state, block_id, identity, window_label)
-> Option<RequestContext>`:

```text
shared   → Windows: None (global default) — unchanged
           Views:   the parent window's context IF that window is main,
                    else the global default (see §4.2) — CHANGED
private  → pane_contexts[block_id] if present, else create one with
           RequestContextSettings { cache_path: "", persist_session_cookies: 0 }
           (the exact settings `create_isolated_request_context` already
           uses, commands/mod.rs:68-72), store it, return it
profile: → Phase 3
```

Use it at both Windows creation sites (`creation.rs:186-193`,
`floating_pane.rs:264-271` and `:484-496`) and the Views site
(`creation_views.rs:164-173`). Remove the entry in the pane-close path.
The context is refcounted; holding it in the map is what keeps the jar
alive across a redock re-create.

`browser_pane_create` gains an optional `identity` arg
(`ipc.rs:443-461`); absent → shared.

### 4.2 Fixing the Views accidental-incognito

For `shared` on Views platforms, a secondary-window pane must **not**
inherit that window's OTR context any more (§1.2). But constraint B says
the pane's `Profile*` must match its window's. These conflict, and the
resolution is the same as for `private` (§4.4): either the CEF-fork
patch lands, or a secondary-window browser pane on Views keeps today's
behavior and the pane shows the private indicator so it is at least
honest. Phase 1 (Windows) does not have this problem.

### 4.3 Cost

Each distinct context is a distinct profile, and Chromium will not share
a renderer process across profiles, so each private pane is at least one
extra renderer process. That interacts with the memory-pressure work
(#3467, `mem_attribution`). Phase 0 measures it; Phase 1 logs the count
of live private contexts and caps it (proposal: 8, configurable) with a
clear error in the menu when exceeded, rather than letting a user open
thirty and wonder why the host is being evicted.

### 4.4 Views platforms (Linux / macOS)

Two candidate routes, decided by a spike, in this order of preference:

1. **CEF-fork patch.** The repo already ships a patched CEF
   (`docs/cef-patches/`, `--features patched-libcef`, fork
   `agentmuxai/cef@agentmux/7977-…`). A patch making
   `CefWidgetImpl::AddAssociatedProfile` tolerate a widget that already
   observes the shared `ThemeService` (check `HasObserver` before
   `AddObserver`, or key the associated-profile map on the original
   profile) removes constraint B for both `private` and §4.2. This is
   a small, local change of the same class as the existing
   mach-rendezvous patch.
2. **Structural fallback.** A private pane on Views must be the sole
   tenant of its own top-level `CefWindow` — i.e. choosing Private
   tears the pane off into a floating window whose main renderer is
   created with that same context. Ugly, but it satisfies constraint B
   without touching CEF. Only if route 1 fails.

Until one of these ships, the `identity` arg on Views is accepted,
logged at `warn`, and coerced to `shared`; the frontend hides the menu
item on those platforms (§5.3), so users never see a control that
silently does nothing.

## 5. srv, App API, frontend

### 5.1 srv

- `build_pane_meta` (`agentmux-srv/src/server/app_api/pane.rs:246-251`)
  accepts an optional `identity` on `view=browser`, validated against
  `shared | private | profile:<id>` (`profile:` rejected until Phase 3),
  and writes `browser:identity`. `CommandPaneOpenData` gets the field.
- `widgets.json` widget blockdefs may carry `"browser:identity":
  "private"`. Add one hidden-by-default widget, "Private Browser", so
  there is a discoverable creation path in the widget bar.
- No new tables. No migration. Session restore already round-trips
  block meta.

### 5.2 App API / MCP

`pane.open` with `view: "browser", url, identity: "private"`. Surfaced
through the existing MCP pane-open tool schema
(`agentmux-mcp/src/tool_schemas.rs`) as an optional enum. This is how
an agent runs "log into service X as a second account without
disturbing the user's session".

### 5.3 Frontend UX

**Creating.** Two entry points, both reusing existing primitives:

- Widget bar: the new "Private Browser" widget (§5.1).
- Browser nav bar `FlyoutMenu` (`frontend/app/view/browser/browser-nav-bar.tsx`,
  the same primitive as the bookmark menu): a new **Identity** group —
  `Shared (default) ✓` / `Private — this pane only`. Selecting the
  other one shows a one-line confirm ("Reopens this pane logged out.
  Continue?") then does the recreate (§3.2).

**Seeing.** Because `browser:show_controls=false` hides the nav bar,
the indicator lives in pane chrome:

- Pane tab pill and pane header: a `fa-user-secret` glyph before the
  title for `private`. The header text already moved into a hover
  tooltip (#3488); the tooltip for a private pane appends "Private
  session — cookies and logins are not shared with other panes and are
  discarded when this pane closes." Use the existing pane-color system
  (`SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md`, #3476); do **not**
  introduce a new colour for this. Glyph plus tooltip is enough.
- Nav bar (when shown): the same glyph as a non-interactive badge at the
  left of the URL field.
- Session-restore notice (§3.2).

**Model plumbing.** `BrowserViewModel` reads
`meta["browser:identity"]` the way it reads `browser:show_controls`
(`frontend/app/view/browser/browser-model.ts:363`), and
`use-pane-rect-sync.ts:168` forwards it in the `browser_pane_create`
args. The menu writes it through the existing `SetMetaCommand`
(`frontend/app/store/rpc-api/workspace.ts:139`), then triggers the
recreate.

**Platform gating.** The menu group and widget are hidden when the host
reports the capability absent; add `browser_pane_identity: bool` to
whatever host-capabilities payload the frontend already reads (or a
one-off `browser_pane_capabilities` IPC if none exists — check first).

## 6. Phasing

**Phase 0 — spike (Windows dev build, ~1 day).** Hard-code a second
pane onto an empty-`cache_path` context and confirm, with two panes on
`github.com/login`: (a) independent logins; (b) an OAuth popup flow
(e.g. "Sign in with GitHub" on a third-party site) completes inside the
private jar; (c) closing and re-creating the pane within one run via
tear-off/redock keeps the login when the context is held in a map, and
loses it when it is not; (d) per-private-pane RSS delta from
`mem_attribution`. On Linux, confirm constraint B fires for a per-pane
OTR context, so §4.4 is grounded in a fresh repro rather than a
four-month-old note. Output: a short report under `docs/reports/` and
a Status bump here.

**Phase 1 — Private identity, Windows.** §4.1, §5.1, §5.2, §5.3, the
cap in §4.3, platform gating. Views platforms coerce to shared. Tests:
unit tests for `build_pane_meta` validation and for the context
registry's create/reuse/drop lifecycle; a vitest for the menu writing
the meta and for the indicator rendering from meta.

**Phase 2 — Views platforms.** §4.4 route 1 spike on the CEF fork; if
it works, ship it and also apply §4.2. If not, route 2. Either way, the
Linux/macOS secondary-window behaviour becomes deliberate and labelled
instead of accidental.

**Phase 3 — Named persistent profiles.** Separate spec once Phase 0 of
*that* work solves constraint A. Starting hypotheses for the stall, so
nobody re-runs the 2026-07-09 experiment blind: Chrome profile creation
is asynchronous (`ProfileManager::CreateProfileAsync`) and the host
creates the browser on the UI thread immediately; the 2026-05-02
window-creation runner serialises UI-thread work and may be what wedges
(`SPEC_HOST_WINDOW_CREATION_RUNNER_2026-05-02.md`). Try: create the
context ahead of time and wait for it to initialise (any
callback-taking `RequestContext` call, e.g. `get_cookie_manager` with a
completion callback, resolves only once the profile is ready) before
posting the create; and note the stall was only ever observed for
top-level Views windows, never for the Windows native-child pane path,
which may simply not stall. Data model: a profile registry in srv
settings (`id`, `label`), directory `<cef-cache>/profile-<id>` (direct
child of `root_cache_path`, the one layout Chrome accepts), and an
answer to whether profiles live inside the per-version data dir
(`SPEC_VERSION_ISOLATION_2026_06_01.md`) or in the shared tree the
archived data-dir-unification spec planned for cookies
(`archive/SPEC_DATA_DIR_UNIFICATION_2026-05-05.md` §chromium-cookies).

## 7. Security notes

- A private identity is a **storage-partition boundary, not a
  sandbox boundary**. Same OS user, same renderer sandbox policy, same
  `--password-store=basic` obfuscation of on-disk cookies
  (`SPEC_SUPPRESS_OS_CREDENTIAL_PROMPTS_2026_05_30.md` §4). An agent with
  deep browser-pane control (`SPEC_AGENT_BROWSER_PANE_DEEP_CONTROL_2026_09_20.md`)
  can drive a private pane exactly as it can a shared one. Say this in
  the tooltip's longer form, not just in this doc.
- Private contexts are in-memory, so there is nothing new on disk in
  Phase 1. Phase 3 profile dirs need the same handling as `cef-cache`
  (the legacy-dir sweep in `commands/mod.rs:109-134` shows the pattern
  for cleaning up orphans).
- `pane.open identity=private` over the App API is not a privilege
  escalation: it grants an agent *less* shared state, not more. No new
  authorization tier needed.

## 8. Open questions

1. **Name.** "Private" (this spec) vs "Incognito" (the ask) vs
   "Isolated". "Private" avoids implying Chrome-incognito tracking
   guarantees. Decide before the widget label ships.
2. **Cap value** for concurrent private contexts (§4.3). Proposal 8;
   Phase 0's memory number should set it.
3. **Should a pane-tab drag between windows preserve the private
   jar** on Views if the fallback route 2 (§4.4) is what ships? With
   route 2 the pane cannot be docked at all, so the question only
   exists if route 1 fails.
4. **Window-level default identity** — worth doing after Phase 1 if
   users mostly want "this whole window is my other account". Not in
   scope, but the block-level meta key composes with it.

## 9. Acceptance (Phase 1)

- Two browser pane tabs in the same window, one shared and one private:
  logging into GitHub in one leaves the other logged out, and vice
  versa. Two private panes are independent of each other.
- An OAuth popup started in a private pane lands its session in that
  pane, not in the shared jar.
- Tear-off and redock of a private pane keep its login within one run.
- Closing a private pane and opening a new one gives a logged-out pane.
- After app restart a private pane comes back logged out with the
  notice shown once.
- Pane tab, pane header tooltip and nav bar badge all show the private
  state; nothing changes visually for shared panes.
- `pane.open view=browser identity=private` from the App API creates a
  private pane; `identity=profile:x` is rejected with a clear error.
- On Linux/macOS the option is hidden and the arg is coerced with a
  warn log; no CHECK crash is reachable.
- No behaviour change for any browser pane that does not opt in, on any
  platform, on Windows. (Views platforms' secondary-window change is
  Phase 2.)
