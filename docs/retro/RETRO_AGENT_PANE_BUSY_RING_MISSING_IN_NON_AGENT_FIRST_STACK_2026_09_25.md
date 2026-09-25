# RCA: busy ring never shows for an agent tab in a stack that started as another pane type

**Date:** 2026-09-25
**Severity:** Medium: the only at-a-glance "this agent is working" signal is silently missing.
**Status:** root cause confirmed in a live 0.57.3 instance; fix not yet written.

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

## Scope: every stack whose first hoisted member isn't an agent

This isn't specific to Swarm. Every view type in `HOISTS_OWN_CHROME`
(`pane-leaf-chrome.tsx:59`) other than `agent` produces the same result when
it's the first member: term, browser, editor, sysinfo, swarm, armory, media,
drone, help, warden. `term` has its own `PaneChromeModel`, but it doesn't
provide `renderBelowHeader` either.

Agent-specific chrome capabilities lost in these stacks for the same reason:

- the busy ring (this report);
- the agent's `extraTabs` (forked-conversation pills from other panes);
- the `agent-pane-stack` / `-content` root classes, which carry
  `--progress-bar-color` and the floating tab-strip overlay rules.

Also related, but a separate mechanism: `KEEP_ALIVE_TYPES` is latched the same
way (`pane-leaf-chrome.tsx:144-150`). A Swarm-first stack therefore never
keeps its agent tab alive, and switching back to the agent tab remounts its
`<Block>`.

## Why it wasn't caught

Every existing spec and test for the ring assumes the pane's chrome is agent
chrome. The pane-tab work that made mixed-type stacks possible
(`SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md`, #3706's
"drop a Pane Tab anywhere") never re-checked the progress-bar handoff for
an agent that is not the chrome owner.

## Proposed fix

Make the slot a **chrome** feature, not an agent-chrome feature:

- `PaneChrome.tsx` always renders a generic progress-bar slot, whatever the
  latched vm is.
- The mount handoff follows the **active** member, not the latched one:
  whenever `nodeModel.activeViewModel()` exposes `setProgressBarMount`, point
  it at the slot, and clear it on change. This is the same
  effect/cleanup idiom as `agent-view.tsx:485-492`, moved into the shared
  chrome.
- Move the slot/bar positioning and `--progress-bar-color` from
  `.agent-pane-stack` onto `.pane-stack`, so the ring is styled identically
  in every chrome.

Regression test: a stack whose first member is `swarm` (and one that starts
with `term`), plus an agent member. Activate the agent, make it busy, and
assert that `.agent-pane-progress-bar--active` renders.

Out of scope for this fix, to track separately: agent `extraTabs` and
keep-alive in non-agent-first stacks.

## Workaround until fixed

Keep agents in panes that started as agent panes. Dragging the agent tab out
into its own pane gives it agent chrome and the ring.
