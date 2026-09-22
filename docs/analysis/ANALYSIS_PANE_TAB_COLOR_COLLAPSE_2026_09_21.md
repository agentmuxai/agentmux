# Bug Report — Pane-Tab Pill Colors Collapse to the Selected Tab's Color

**Date:** 2026-09-21
**Status:** implemented in #3484 — see §7 for the confirmed root cause and
fix. §2 and §4 are kept as-written below (including their now-superseded
"could not reproduce" conclusion) since the process that got from "can't
reproduce" to "found it live" is itself worth keeping.
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

## 6. Root cause found — it's §5's read/data question answered: neither

The user reported seeing the bug live a second time, in the SAME running
dev instance this analysis's §4 had just tested against. Rather than
speculate further, connected to that instance directly via its CDP debug
port (`http://localhost:9223`, the dev build's
`--remote-debugging-port`), read each pill's **actual computed**
`background-color` (not just the `--pane-tab-bg` custom property §4
checked) via `getComputedStyle`, and captured a real screenshot
(`Page.captureScreenshot`) of the live window.

The computed backgrounds were, in fact, all distinct:
`Swarm=rgb(29,52,52)`, `Sysinfo=rgb(52,41,29)`, `Terminal 1=rgb(52,29,52)`
— confirming §5's "data vs. rendering" question with a third answer
neither option anticipated: **the data and the rendering were both
already correct.** The screenshot showed why it still looked broken:
`hueToHeaderBg`'s `hsl(hue, 28%, 16%)` — reused as-is for pane-tab pill
backgrounds — is tuned for a large, full-width header bar, where even a
subtle tint reads clearly. On a ~20px pane-tab pill, three different hues
at 16% lightness are close enough to black, and close enough to *each
other*, that they're not reliably distinguishable at a glance — especially
next to an uncolored pill's fully-transparent background and an *active*
colored pill's plain `--block-bg-color` (itself a similar near-black),
both of which paint as "no visible tint" for a different reason. Selecting
a different tab changes *which* pill shows which of these three
visually-similar-but-technically-different "looks no different from
black" treatments — reading, at a glance, exactly like "the other tabs'
colors changed to match."

**Fix**: added `hueToPaneTabBg` (`hsl(hue, 42%, 24%)` — higher saturation
and lightness than the header's 28%/16%) and a parallel
`paneTabBgForEffectiveColor`/`computeBlockTabPillBg`, sharing
`headerBgForEffectiveColor`'s exact light/dark-theme branching logic (now
parameterized by which dark-theme deriver to use) rather than duplicating
it. `PaneChrome.tsx`'s `tabColors` memo now calls the pill-specific
variant; the header itself (`blockframe.tsx`'s `headerStyle`) is
untouched, still using the original `hueToHeaderBg` treatment the user had
already confirmed looks right. Re-verified live via the same CDP
screenshot technique: `Swarm/Sysinfo/Terminal 1` now render as clearly
distinct teal/brown/purple, not uniform near-black.

Only the dark-theme treatment was addressed here — light theme was
already flagged as needing further refinement in a separate pass, and the
light-theme branch (`hueToActiveBorder`/raw hex, full vivid strength) is
untouched by this fix.

## 7. Actual root cause — transparent uncolored pills show the header's tint

§6's contrast fix was real but did not resolve the report: the user still
saw the collapse after it landed. A live CDP read of every pill's ancestor
chain showed why. `BlockFrame_Header`'s `headerStyle` paints the whole
header row with the **active** block's color (e.g. `rgb(52,29,52)` while
Terminal 1 is active), and the tab strip sits on top of that row. A pill
whose block has **no color of its own** (Terminal 2, Help) resolved to
`background: var(--pane-tab-bg, transparent)` — fully transparent — so it
displayed the header's color through itself. Selecting Swarm retints the
header teal, and every uncolored pill turns teal with it. That is exactly
the reported symptom: "the last two tabs change to the same color as the
first tab."

**Fix**: every pill in `PaneChrome` now gets an opaque `neutralBackground`
(`computeBlockTabPillNeutralBg`: on a dark theme, the fixed
`NON_AGENT_DEFAULT_HEADER_BG` its own header would show; on a light theme
or for an agent pane, `var(--block-bg-solid-color)`), emitted as
`--pane-tab-neutral-bg` and used as the fallback when `--pane-tab-bg` is
absent. The plain-hover white tint is now layered over that neutral
background instead of replacing it, so hovering doesn't expose the tint
either. Other `PaneTabStrip` consumers (editor file tabs, the agent History
strip) don't set the variable, so they keep the old transparent behavior.

Verified live on the dev instance: with Terminal 1 active, Terminal 2 and
Help show `rgb(36,39,46)`. After clicking Swarm they stay that same neutral
gray (screenshot confirmed) instead of turning teal.
