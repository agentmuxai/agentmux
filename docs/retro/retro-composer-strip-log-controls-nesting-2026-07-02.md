# Retro — Clicking "Log" in the agent pane composer strip also reveals a nested "Controls" panel

**Date:** 2026-07-02
**Severity:** Medium (UX regression/incompleteness — not data-loss, but the requested behavior was
explicitly specified and shipped incorrectly)
**Status:** Root-caused; fix not yet implemented (design in
`SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02.md`)
**Reporter:** asaf
**Component:** agent pane composer strip / details region (`frontend/app/view/agent`)

---

## 1. What happened

In the agent pane, there's a small strip directly above the text input (`AgentComposerStrip`). It has
a **Log** button that expands a details region below the strip. Clicking **Log** is supposed to show
only the activity log. Instead, the details region also renders a second, independently-collapsible
**"Controls"** block — clicking its own "▸ Controls" chevron reveals *another* set of Mode / Model /
Effort dropdowns, duplicating the Model and Effort selects that already exist at the top level of the
strip.

So today: click **Log** → see log entries **and** a collapsed "▸ Bypass · Sonnet 4.6 · Effort: xhigh"
summary row → click that row → see Mode/Model/Effort dropdowns. Two clicks and two concepts
(log vs. runtime controls) conflated behind one "Log" toggle.

The user had already described this exact desired end-state to the previous agent working this
codebase: **Log should show only logs; Mode, Model, and Effort should be top-level dropdowns next to
Log**, not nested inside anything the Log button opens.

---

## 2. How the flow is supposed to work (per the most recent spec)

`docs/specs/SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md` Part C ("Log button — one toggle, no
middle level") documents this exact nesting problem and explicitly names three levels:
- **L1** — the Log button (`AgentComposerStrip.tsx`) → `detailsOpenAtom` → renders
  `.agent-composer-details`, which mounts **both** `<ActivityLogPanel>` and `<AgentControlBar>`
  (`agent-view.tsx:1005-1016`).
- **L2** — `ActivityLogPanel`'s own internal header/chevron/summary toggle.
- **L3** — per-entry row expand (truncated ↔ full text).

The spec's fix (commit `1db3a8a2`, PR #1899) **removed L2 only** — `ActivityLogPanel` now renders its
entries unconditionally once mounted (`ActivityLogPanel.tsx:48-56`). That part is done and correct.

The spec explicitly flagged a second, *separate* decision and left it open:

> **Decision to resolve: the control bar shares the details region.**
> - **(a, recommended)** Log toggles only the log entries; leave `AgentControlBar` where it is (still
>   revealed alongside).
> - **(b)** Split them: Log reveals only entries; move/retire `AgentControlBar`'s Model/Effort (duplicate
>   of the strip).

**Option (a) is what shipped.** `AgentControlBar` — with its own `expanded` signal, its own
"▸/▾ Controls" chevron header (`AgentControlBar.tsx:60-61,257-273`), and its own Mode/Model/Effort
selects (`AgentControlBar.tsx:274-320`) — is still mounted directly inside `.agent-composer-details`
(`agent-view.tsx:1010-1014`), alongside `<ActivityLogPanel>`. That is the second expansion the user is
now reporting.

---

## 3. Root-cause analysis

### Primary — option (a) was picked without being confirmed with the user
The spec surfaced (a) vs (b) as an open decision (§"Decision to resolve", §"Open questions" item 4) but
the implementing pass defaulted to (a) — the smaller diff — and shipped it as if resolved. (a) directly
reproduces the "Log also shows Controls" behavior the user is now reporting, because it leaves
`AgentControlBar`'s self-contained "Controls" expansion nested inside the region the Log button opens.
The user's now-stated requirement ("only logs under Log; Mode/Model/Effort as top-level dropdowns") is
functionally a request for **option (b)**, plus one refinement not in the original spec: **Mode** (today
only a read-only colored pill at the strip's top level, `AgentComposerStrip.tsx:269-277`) needs to
become an actual top-level dropdown, not just Model and Effort.

### Contributing — Mode has never been a top-level dropdown at all
`AgentComposerStrip.tsx` already promoted Model and Effort to top-level `<select>`s
(`:216-239`), but Mode only exists there as a **read-only pill** (`:269-277`) driven by a
`permissionMode` prop — there is no `onChange` path. The only *editable* Mode control in the whole pane
is inside `AgentControlBar`'s nested Controls body (`:276-290`). So even implementing spec option (b)
literally (retire `AgentControlBar`'s Mode/Model/Effort block) would have **removed the only way to
change Mode** unless a replacement top-level Mode dropdown is added at the same time. This is why the
user is asking for Mode specifically, not just de-duplication of Model/Effort.

### Secondary — `AgentControlBar` bundles two unrelated concerns behind one toggle
`AgentControlBar` renders both the runtime controls (Mode/Model/Effort, lines 274-320) **and**
session-management banners/buttons (interrupted-session banner, large-session warning, archived badge,
Archive/Export/Restore, lines 199-256) behind the same `expanded` chevron. Once the runtime controls
move out (per the fix), the session-management pieces still need a home — they don't disappear, and
they're still mounted inside the Log-gated `.agent-composer-details` today. This wasn't part of the
original ask but is a direct consequence of removing the "Controls" chevron, so it's called out as an
explicit scope decision in the accompanying design doc rather than silently resolved.

---

## 4. Contributing factors
- **An explicitly-flagged decision point was defaulted instead of confirmed.** The spec did the right
  thing by naming (a)/(b) as an open question; the gap was in not stopping to get the answer before
  shipping the smaller option.
- **Mode lagged Model/Effort's promotion to the strip.** Model and Effort were promoted to top-level
  selects in an earlier pass (`SPEC_AGENT_COMPOSER_STRIP_REDESIGN_2026_06_23.md`); Mode was left behind
  as a read-only pill, so "promote the remaining runtime controls" was incomplete even before this spec.
- **Two unrelated feature sets (runtime controls, session management) share one component and one
  toggle**, so a fix aimed at "controls" can't cleanly avoid touching session management too.

---

## 5. Recommended fixes (ranked)

See `docs/specs/SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02.md` for the full design. Summary:

1. **Add an editable Mode `<select>` to `AgentComposerStrip.tsx`**, top-level, alongside Model and
   Effort, before the Log button. Replace the read-only permission pill (redundant once Mode is an
   editable dropdown in the same row).
2. **Stop mounting `AgentControlBar`'s Mode/Model/Effort block inside `.agent-composer-details`.** Once
   (1) ships, that block is a pure duplicate — delete it (and its `expanded` chevron/"Controls" header,
   which no longer has a reason to exist once it's not gating dropdowns).
3. **Decide and implement where session management (banners + Archive/Export/Restore) lives** now that
   it's the only thing left in `AgentControlBar`. Recommend: keep it inside `.agent-composer-details`
   (still reachable via Log) but rendered unconditionally with no chevron — see design doc §"Fix 3" for
   the two options and trade-offs.

## 6. Prevention
- When a spec leaves an option explicitly open ("Decision to resolve", "Open questions"), **do not
  default silently** — surface the choice back to the requester before implementing, or implement the
  smaller option but say so plainly in the PR description so it can be caught in review rather than
  reported back as a bug.
- When promoting controls from a nested panel to a top-level strip, promote **all** of them in the same
  pass (Mode was left behind when Model/Effort moved up) — a partial promotion just relocates the
  inconsistency instead of resolving it.

## 7. Diagnostic to confirm at next repro
1. Open any Claude agent pane, click **Log**.
2. Confirm current (buggy) behavior: log entries appear, plus a collapsed
   "▸ &lt;mode&gt; · &lt;model&gt; · Effort: &lt;level&gt;" row below them.
3. Click that row: confirm it expands to show Mode/Model/Effort `<select>`s — these are the same
   controls already visible at the top of the strip (minus Mode, which only exists here).

## 8. References
- Frontend: `view/agent/components/AgentComposerStrip.tsx:213-250` (top-level controls + Log button),
  `:269-277` (read-only Mode pill); `view/agent/components/AgentControlBar.tsx:60-61,257-320`
  (nested Controls chevron + Mode/Model/Effort body); `view/agent/agent-view.tsx:1005-1016`
  (`.agent-composer-details` mounts both `ActivityLogPanel` and `AgentControlBar`);
  `view/agent/components/ActivityLogPanel.tsx:48-56` (L2 fix, already shipped).
- Specs: `docs/specs/SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md` Part C (names this exact
  nesting, ships (a) instead of (b)); `docs/specs/SPEC_AGENT_COMPOSER_STRIP_REDESIGN_2026_06_23.md`
  (earlier pass that promoted Model/Effort but not Mode, and explicitly called the
  `AgentControlBar` duplication "acceptable" at the time); `docs/specs/SPEC_COMPOSER_STRIP_AND_HOST_POLISH_2026_06_25.md`
  (renamed Shell → Log, removed the strip-level chevron).
- Commits: `1db3a8a2` (#1899, removed L2 middle collapse layer), `1087eec8` (#1900, registry-driven
  model dropdowns).
