# Bug Report — Pane-Tab Pill Colors Collapse to the Selected Tab's Color

**Date:** 2026-09-21
**Status:** analysis — one real, confirmed contributing bug found and fixed
(PR #3484); the user's exact live repro could not be reproduced afterward,
and a second live repro attempt on the unfixed code ALSO failed to
reproduce it — see §4. Root cause of the live symptom is not fully closed.
**Area:** `PaneChrome.tsx` / `PaneTabStrip.tsx` pane-tab pill coloring
(the feature added in PR #3484, itself a follow-up to #3476's header/border
pane-color consolidation)

---

## 1. Reported symptom

> I notice a bug, both dark and light mode: I have 5 tabs open in 1 pane,
> each tab has different colors. If I select the first tab (in this case
> "Swarm"), the colors of the last two tabs incorrectly change color to
> the same as the first tab (instead of retaining their own colors).

## 2. A real, confirmed bug found by code review — fixed

ReAgent's review of PR #3484 (2026-09-21T16:10:43Z) caught a genuine P1 in
the original implementation, matching this symptom's general *shape*
(multiple pills' colors collapsing to one shared value):

`PaneChrome.tsx`'s `tabColors` memo called
`computeFocusRingBorderColor(true, meta, tabMeta)` for **every** stack
member, passing the **same shared** `atoms.tabAtom()?.meta` each time.
That function checks `tabMeta["bg:activebordercolor"]` **first** and
returns immediately if present — before ever consulting the per-block
`frame:hue`/`frame:activebordercolor`. So whenever the workspace tab
itself carried a `bg:activebordercolor` override, every pill's underline
silently collapsed to that one tab-wide value, regardless of which block
it actually belonged to — defeating the whole point of per-block pill
colors.

**Fix** (same PR, follow-up commit): extracted
`computeBlockActiveBorderColor(blockMeta)` — the pure per-block
hue/`frame:activebordercolor` precedence, with **no** tab-level meta
consulted at all — and switched `tabColors` to call that instead. Added a
regression test (`PaneChrome.test.tsx`, "per-tab pane color" describe
block) proving distinct per-block colors survive independently, with a
meta-driven mock specifically chosen so a reintroduced shared-value bug
would fail it.

## 3. Why this likely is NOT the user's actual live trigger

`bg:activebordercolor`/`bg:bordercolor` are read in several places
(`blockframe.tsx`) but a repo-wide grep found **no call site that ever
writes either key** — they're vestigial fields from an older,
already-decommissioned env-var-driven color system (see
`SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md`). Without something
setting them, `tabMeta["bg:activebordercolor"]` is always `undefined` in
current usage, so the short-circuit ReAgent found never actually fires
today. It's a real bug worth fixing (a latent landmine — the day something
starts writing that key, this exact collapse comes back), but it doesn't
explain why the user saw the collapse live.

## 4. Live reproduction attempts — inconclusive

Connected directly to the running dev instance's CDP debug port
(`--remote-debugging-port=9223`, `agentmux-cef` dev build) and queried the
DOM for the actual 5-tab pane (`Swarm` / `Sysinfo` / `Terminal 1` /
`Terminal 2` / `Help` — the exact tab set + first-tab name from the
report), reading each pill's `--pane-tab-bg`/`--pane-tab-underline`
computed inline style values directly, before and after synthesizing real
`.click()` events on the pills (not React/Solid test-harness stubs — the
actual live app).

- **With the fix in place**: Swarm/Sysinfo/Terminal 1 each show their own
  distinct `hsl(...)` background/underline; Terminal 2/Help (no color
  assigned) show empty (falls through to the strip's default chrome). No
  collapse.
- **With the fix reverted** (`git stash` the three changed files, forced a
  full page reload so the pre-fix bundle was actually served, re-queried):
  **identical** result — still no collapse. This is consistent with §3:
  the tab-wide override path was never reachable here since nothing sets
  `bg:activebordercolor`.
- **Clicking between tabs** (Sysinfo → active, then back to Swarm →
  active), checked after each click: every pill's own color persisted
  correctly through both switches, on **both** the pre-fix and post-fix
  code.

Net result: this specific pane, in this specific running dev instance,
does not reproduce the reported collapse via a plain tab click, with or
without the ReAgent fix applied.

### Side finding (unrelated, noted for completeness)

The pill labeled "Sysinfo" relabels itself to "CPU" the moment it becomes
the active tab (and back to "Sysinfo" once it's deactivated again) — its
own color stayed correct and consistent throughout, so this looks like
expected behavior for a multi-metric sysinfo widget (a generic fallback
label while dormant, a live metric-specific name once its ViewModel
mounts), not a bug. Flagging only because it was surprising to see while
investigating, not because it's believed related to the color issue.

## 5. Open questions / next steps

The confirmed fix in §2 is real and worth keeping regardless, but §4 means
the user's live symptom likely has a different, still-unidentified
trigger. Things not yet ruled out:

- **Timing**: the user's repro may depend on the *very first* activation
  right after a fresh app/window load (before any other tab switch has
  happened this session) — the live test above started from an
  already-warm session with Swarm pre-active, which may not be equivalent.
- **A specific color-assignment path**: whether the 5 tabs' colors were
  set via the right-click "Pane Color" picker (`frame:hue`, explicit) vs.
  each being a distinct agent's passive identity color
  (`frame:activebordercolor`, seeded once at `agent.open`) vs. a mix of
  both — not confirmed for the exact pane the user saw this on.
- **Reactivity edge case** in `tabColors`' `createMemo` itself (e.g. a
  stale read racing `tabIds()`'s own recomputation) that only manifests
  under different timing than a manual CDP-driven click.

Recommend: next time this reproduces, capture the specific pane's 5
blocks' `frame:hue`/`frame:activebordercolor`/`agentId` meta (right before
and immediately after the collapse) rather than only the rendered colors —
that will show directly whether the underlying **data** changed (a real
write-path bug) or only the **rendering** picked the wrong data for a
given pill (a read-path bug), which this analysis wasn't able to
distinguish from the outside.
