# Findings: `[wave-scroll-shrink]` Live Data From Two Running Instances (2026-08-22)

Follow-up to `docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_21.md`
(which correlated a single dev-branch repro's 8 events). That dataset was
one pane, one short session. This one is two currently-running "shared"
(local `task package` build) instances, each with hours of real usage, and
is roughly two orders of magnitude larger. It surfaces two things the
smaller dataset couldn't: a lead worth chasing on a recurring
similar-magnitude shrink, and a distinct, more severe phenomenon the prior
analysis never saw at all.

**Both instances are actively running and their host logs are still
growing as this doc is written** — every count below is from a single
frozen snapshot taken via one atomic `grep` pass per file at
**2026-08-22T08:10:19Z**, not a live/moving total. (Two earlier verbal
figures shared before this doc — 158, then 1,434/575 — were each stale
the moment they were computed, one additionally clipped by `Grep`'s
default 250-line output cap. Codex's PR review correctly caught that the
1,434/575 figures didn't even sum to their own per-pane breakdown — same
root cause: the per-pane breakdown was computed in a second, later `grep`
call against an already-longer file. All numbers below come from one
`grep` invocation per instance, piped to a temp file, with every
downstream stat (totals, per-pane, histogram, `-> 0px` count) derived
from that same frozen copy, so they're internally consistent as of the
capture instant even though the live file has grown further since.)

## Data sources (frozen snapshot, 2026-08-22T08:10:19Z)

- **Instance A (`0.55.18`, local build channel `local-main-b28b7a-697d25a4`)**:
  `.../versions/0.55.18/logs/agentmux-host-v0.55.18.log.2026-08-22`, log
  span `00:00:04Z` – (still growing). **1,602**
  `[wave-scroll-shrink]` events across 3 panes at capture time.
- **Instance B (`0.55.19`, channel `local-main-b28b7a-b966d418`)**:
  `.../versions/0.55.19/logs/agentmux-host-v0.55.19.log.2026-08-22`, log
  span `00:00:18Z` – (still growing). **681** events across 3 panes at
  capture time.

Both are local `task package` builds (`docs/…CLAUDE.md`'s per-build
channel isolation — see "Data isolation is per-BUILD for local builds"),
not dev branches, so this is closer to real day-to-day usage than the
prior single dev-session dataset.

## 1. Per-pane volume — heavily skewed to one pane in Instance A

| Instance | Pane | Events |
|---|---|---|
| A | `43c1dea` (Camper) | **1,138** |
| A | `f5c4ae9` (Agent1 — this agent) | 370 |
| A | `0a8d11f` (AgentY) | 94 |
| B | `6365488` | 638 |
| B | `438cddb` | 40 |
| B | `34d8ae1` | 3 |

(A: 1,138 + 370 + 94 = 1,602. B: 638 + 40 + 3 = 681. Matches the
snapshot totals above by construction — both computed from the same
frozen copy.)

Instance A's Camper pane alone accounts for 71% of that instance's total
events — this is not evenly distributed across panes, and correlates with
Camper likely running a workload with a lot of live-updating content
(consistent with `docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md`
/ background-task work landed the same week — not confirmed here, just the
most plausible explanation for the skew; not chased down further in this
pass).

## 2. A recurring, near-identical ~251–252px net shrink — a lead, not a confirmed single element

Instance A: **59 events** at exactly `delta=251px` or `delta=252px`
(distributed 4/45/10 across panes `0a8d11f`/`43c1dea`/`f5c4ae9`
respectively — i.e. it hits every pane, not one). These recur steadily
across the sampled window, at irregular intervals ranging ~12s to ~9
minutes apart, overwhelmingly on the Camper pane (45 of 59). Instance B
has only 1 such event in 681 samples — this is heavily concentrated in A.

**Why this is worth chasing, and the important caveat on what it can and
can't establish (flagged in PR review — see below):** a magnitude this
consistent, recurring dozens of times across independent panes and
unrelated timestamps, stands out sharply from the rest of the histogram's
smooth, wide spread (205×2px, 188×4px, 144×3px, 127×12px, 87×73px,
81×35px, 58×141px, 50×6px, 45×36px, ...). That's a real pattern worth
investigating.

**However:** `AgentDocumentVirtualList.scrollToTrueBottom()` only updates
`lastKnownScrollHeight` when a pin-correction check runs — each logged
delta is therefore the **cumulative net change since the previous
pin-check**, not the size of one discrete DOM mutation (the same caveat
the prior FINDINGS doc already applied to its Class 2 example, §"Class 2"
correction there). A repeated net value of ~251-252px does **not** by
itself establish that one specific fixed-height element collapsed each
time — several intervening growth-and-shrink mutations between two
pin-checks could sum to the same net figure by coincidence, repeatedly.
The original wording here claimed this "reads as one specific UI element"
— that overstated what a net-delta measurement can show on its own.
**Correctly scoped: this is a pattern worth a targeted source-level
search, not yet an established single cause.**

**Not yet identified which element (if it is one).** No `[wave-turn]`
state transition was found within the immediate vicinity of the sampled
251/252px events checked (see §4) — so this doesn't obviously tie to
tool-call/turn lifecycle the way Class 1 did. A targeted search for any
component with a ~251-252px fixed/estimated height (candidates worth
checking: `ActivityDock` entries, a background-task dashboard card, an
`AgentDocumentVirtualList` row-height estimate default) is still the
right next step, but should be treated as testing a hypothesis, not
confirming one — mutation- or component-level instrumentation (as the
prior doc's own §"Recommended next step" already called for, for the same
reason) is what would actually confirm it.

## 3. A distinct, much more severe phenomenon: whole-pane collapse to 0px, multiple panes at once

Instance A logged **5 events where `scrollHeight` went to exactly `0px`**,
clustering into **2 incidents**:

```
07:39:08.638208Z  pane=0a8d11f  43921px -> 0px  (delta=43921px)
07:39:08.639745Z  pane=43c1dea  23327px -> 0px  (delta=23327px)
07:39:08.641382Z  pane=f5c4ae9  21243px -> 0px  (delta=21243px)

07:58:24.837494Z  pane=43c1dea  23096px -> 0px  (delta=23096px)
07:58:24.838751Z  pane=f5c4ae9  21098px -> 0px  (delta=21098px)
```

Each incident hits **every pane current-open in the instance simultaneously**
(all 3 at 07:39:08, 2 of 3 — `0a8d11f`/AgentY not among them this time — at
07:58:24), within 1–3ms of each other. The magnitudes (21K–44K px) are the
largest in either dataset by a wide margin — an order of magnitude past
even the original FINDINGS doc's 13,502px "Class 2" example.

**This is categorically different from §1/§2 and from the prior FINDINGS
doc's two classes — not a third data point for the same mechanism.** A
single pane's content reflowing, even drastically, doesn't explain
multiple independent panes' scroll heights all going to exactly zero
within milliseconds of each other. The natural read is something
application-wide (a window resize/minimize/restore, a display-mode
change, a CEF repaint/layout reset) rather than a per-pane document
mutation.

**A render-thread freeze longer than ~17-20s is ruled out; a shorter one
is not.** Checked for a timestamp gap in the host log's own logging
cadence around both incidents — none found. The log's periodic watchdog
ticks continued at their normal ~17-20s cadence straight through both
`07:39:08Z` and `07:58:24Z` with no unusual gap (largest gaps anywhere in
the full 8-hour log are the expected ~17-20s idle-tick intervals,
confirmed by scanning every consecutive timestamp pair in the file). **The
original wording here ("ruled out... not frozen") overstated this** (PR
review correctly caught it): an uninterrupted tick cadence only proves no
hang lasted long enough to delay one of those ~17-20s ticks. A transient
freeze shorter than that window, landing entirely between two ticks,
would leave exactly this evidence and isn't excluded by it. Correctly
scoped: **the watchdog cadence bounds any renderer freeze at these two
timestamps to well under ~17-20s** (if one occurred at all) — it doesn't
rule out a freeze in that shorter range, and a multi-pane simultaneous
0px reading is unusual enough that a brief freeze is still a live
candidate alongside the application-wide-event explanation above.

**Not present at all in Instance B.** Zero `-> 0px` events in 681 samples;
Instance B's delta histogram (top values: 65×4px, 61×2px, 57×10px,
52×3px, 50×9px, ...) shows only the smooth small-magnitude distribution,
and only 1 event at 251/252px (vs. 59 in Instance A). Whatever causes
both the §2 and §3 phenomena in Instance A did not occur (or didn't occur
as frequently/at all) in Instance B's session — consistent with them
being tied to something specific to Instance A's workload/panes (the
Camper-heavy skew from §1) rather than a universal per-build defect.

## 4. What wasn't established

- **Which specific component the 251/252px shrink (§2) belongs to** —
  needs a source-level search for a matching fixed/estimated height
  constant, not more log correlation.
- **The actual mechanism behind the 0px full-collapse (§3)** — a freeze
  longer than ~17-20s is ruled out; a shorter one isn't. Not yet
  identified what *did* cause it either way. Worth checking
  window/visibility-change event handlers, and whether these
  timestamps correlate with anything in the app's own window-focus/
  minimize logging (not checked in this pass — scoped to the
  `wave-scroll-shrink`/`wave-turn` diagnostics only).
- **Whether §3 is related to any user-visible freeze/hang report** — not
  established here. The timestamps (07:39:08Z, 07:58:24Z) are recorded for
  exactly this purpose, in case they need correlating against a separate
  freeze report later, but this doc does not itself claim that link.

## 5. Recommended next steps

1. **§2 (251/252px):** grep the frontend for a component with an
   estimated/fixed height in the 251-252px range (dashboard cards,
   `ActivityDock` rows, virtualization row-height defaults are the most
   likely candidates per the codebase areas already touched by recent
   work in this neighborhood).
2. **§3 (0px collapse):** add window-visibility-change / resize event
   logging alongside `[wave-scroll-shrink]` so a future occurrence can be
   directly correlated with an app-level cause, rather than inferred from
   absence of a log gap.
3. File §2 and §3 as separate tracked issues — they're different enough
   in mechanism and severity that bundling them risks either getting
   lost under the other.

## 6. Sources

- `docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_21.md`
  (the smaller prior dataset this doc builds on)
- `docs/analysis/ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md`
  (original `[wave-scroll-shrink]` diagnostic + three-source theory)
- Instance A: `C:\Users\asafe\.agentmux\channels\local-main-b28b7a-697d25a4\versions\0.55.18\logs\agentmux-host-v0.55.18.log.2026-08-22`
- Instance B: `C:\Users\asafe\.agentmux\channels\local-main-b28b7a-b966d418\versions\0.55.19\logs\agentmux-host-v0.55.19.log.2026-08-22`
