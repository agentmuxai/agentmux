# RCA: busy ring never shows for an agent tab in a stack that started as another pane type

**Date:** 2026-09-25
**Severity:** Medium: the only at-a-glance "this agent is working" signal is silently missing.
**Status:** implemented. Both parts of the cause were confirmed live; the fix
was verified in a `task dev` instance (see "Verification").

## What was reported

In a live 0.57.3 portable, the marching-ants ring that goes around an agent
pane's border while the agent is busy never appeared on the Manoz pane.
The pane to its left (AgentA) showed it normally.

## Evidence (live, 0.57.3, while the Manoz agent was mid-turn)

Queried from inside the affected pane (`UIQuery`):

- `.agent-pane-progress-bar`: **no element at all**. It isn't merely
  inactive; it was never rendered.
- `.agent-pane-progress-bar-slot`: **no element**.
- `[class*="-pane-stack"]` (the `rootClass` the agent chrome puts on the pane
  root): **no element**.
- `.pane-tab-strip` text: `Swarm × Manoz × +`. The pane is a two-member
  stack whose first member is a **Swarm** view, with the agent added later.
- The agent itself was clearly busy: `.agent-working-row--loading` and an
  `agent-tool-block ... running` were present in the same pane.

AgentA's pane is an agent-first pane, so it has the agent chrome and the slot.

## Root cause

The busy ring lives in pane **chrome**, and chrome belongs to whichever view
was active when the pane first hoisted. It is never re-derived after that.

1. `frontend/app/tab/pane-leaf-chrome.tsx:414-420`: `chromeVm` latches the
   **first** non-null `ViewModel` it ever sees and renders
   `vm.renderPaneChrome(...)` from it for the pane's whole life. This is
   deliberate: it prevents chrome remount flashes on tab switch.
2. `frontend/app/element/PaneChrome.tsx:69`: the shared chrome resolves the
   `PaneChromeModel` **once**, from that latched vm's `paneChromeModel`.
3. Only the agent's `PaneChromeModel` provides `renderBelowHeader`
   (`frontend/app/view/agent/agent-view.tsx:515`), which renders
   `.agent-pane-progress-bar-slot`. The same model's effect
   (`agent-view.tsx:484-492`) is also what calls
   `setProgressBarMount(slotEl)` on the active agent vm.
4. The agent content portals `.agent-pane-progress-bar` into
   `progressBarMount()` behind `<Show when={progressBarMount()}>`
   (`agent-view.tsx:2509-2514`).

In a Swarm-first stack the latched vm is the Swarm vm. It has no
`paneChromeModel`, so there is no slot, `setProgressBarMount` is never called,
`progressBarMount()` stays `null`, and the `<Show>` never renders the bar.
The agent does become busy (`paneBusy()` flips), but nothing is mounted to
show it.

### Part 2, found while verifying the first fix

Moving the slot into the shared chrome wasn't enough on its own. In a live
`task dev` instance, the slot rendered in a Swarm-first pane, but the agent
tab's bar still never reached it. The cause is a second latch in
`pane-leaf-chrome.tsx`:

- `keepAlive` latches on the first time an agent (or term) member is
  active (`pane-leaf-chrome.tsx:144-150`). A Swarm-first pane starts with
  it off and flips it on mid-life, when the agent tab is first activated.
  Live: the pane went from 0 to 2 `.pane-leaf-keepalive-slot`s.
- Under keep-alive, each member reports its vm to its **own per-id slot**
  (`keepAliveNodeModelFor`), not to the leaf NodeModel.
- The chrome renders once and was handed a **snapshot** of
  `chromeNodeModel()` from first hoist: the leaf's own NodeModel, because
  keep-alive was still off then. Its `activeViewModel()` stopped following
  the active tab (it read `null`), so the chrome's slot handoff never reached
  the agent.

The same stale `activeViewModel()` also feeds the header's `viewModel` and
the tab strip's `liveViewModel` in these panes.

**Already fixed on main by #3714.** While this PR was in progress, #3714
(pane tabs keep their own name, icon and content) rewrote `chromeNodeModel`
to read the per-id slots unconditionally, whether or not keep-alive is on.
That fixes Part 2 on its own. This PR's own Part 2 change (a
delegating NodeModel handed to the chrome) became a no-op after rebasing onto
it, which ReAgent caught on #3722, and was dropped. The Part 2 regression
tests stay: they fail on pre-#3714 main and pass on current main.

## Fix

1. `PaneChrome.tsx` renders `.pane-progress-bar-slot` on every pane and
   hands it to `nodeModel.activeViewModel()?.setProgressBarMount`,
   re-pointing on every switch. The agent's `PaneChromeModel` no longer
   supplies the slot. The slot's styles and `--progress-bar-color` move from
   `.agent-pane-stack` to `.pane-stack` (`PaneChrome.scss`).
2. Part 2: already fixed on main by #3714 (see above). No change in this
   PR beyond regression tests.

## Verification

- Unit tests: `PaneChrome.test.tsx` (the slot exists without a
  `PaneChromeModel`; it's handed to, taken back from, and moved between
  active vms) and `pane-leaf-chrome.test.tsx` (the chrome's
  `activeViewModel()` reports the agent tab in a Swarm-first pane after
  keep-alive latches, and follows later switches). The four `PaneChrome`
  tests fail without Part 1's fix. The two `pane-leaf-chrome` tests failed
  on the pre-#3714 base this was first written against, and pass on current
  main.
- Live, `task dev`, driven over CDP. In a pane with the Swarm tab active at
  load, switching to an agent tab (Mopeo) put the agent's
  `.agent-pane-progress-bar` into that pane's `.pane-progress-bar-slot`.
  Before part 2 there was none. With the `--active` class forced on,
  the ring rendered in the pane's ring color (`--progress-bar-color`
  resolved on `.pane-stack`) with `agent-ant-march` running. A real busy
  turn couldn't be driven, because no agent in the dev data dir has valid
  credentials. The `paneBusy()` → `--active` path itself is unchanged by
  this fix.

## Scope: every stack whose first hoisted member isn't an agent

This isn't specific to Swarm. Every view type in `HOISTS_OWN_CHROME`
(`pane-leaf-chrome.tsx:59`) other than `agent` produces the same result when
it's the first member: term, browser, editor, sysinfo, swarm, armory, media,
drone, help, warden. `term` has its own `PaneChromeModel`, but it doesn't
provide `renderBelowHeader` either.

Agent-specific chrome capabilities lost in these stacks for the same reason:

- the busy ring (this report, fixed);
- the agent's `extraTabs` (forked-conversation pills from other panes; still
  open);
- the `agent-pane-stack` / `-content` root classes, which carry the
  floating tab-strip overlay rules (still open). `--progress-bar-color`
  used to live there too; it's on `.pane-stack` now.

Keep-alive is latched too (`pane-leaf-chrome.tsx:144-150`), but on the
first time a keep-alive type is active, not only at first hoist. A
Swarm-first stack does switch to keep-alive once its agent tab is activated.
That mid-life switch is the second half of this bug (Part 2, above).

## Why it wasn't caught

Every existing spec and test for the ring assumes the pane's chrome is agent
chrome. The pane-tab work that made mixed-type stacks possible
(`SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md`, #3706's
"drop a Pane Tab anywhere") never re-checked the progress-bar handoff for
an agent that is not the chrome owner.

A unit test for the chrome's slot handoff alone would also have passed while
the live bug remained. Only driving a real Swarm-first pane exposed Part 2,
because it lives in how `pane-leaf-chrome.tsx` and the chrome compose, not
in either one alone.

## Still open (separate follow-ups)

- Agent `extraTabs` and the `agent-pane-stack` overlay rules in
  non-agent-first stacks.

## Workaround on builds without the fix

Keep agents in panes that started as agent panes. Dragging the agent tab out
into its own pane gives it agent chrome and the ring.
