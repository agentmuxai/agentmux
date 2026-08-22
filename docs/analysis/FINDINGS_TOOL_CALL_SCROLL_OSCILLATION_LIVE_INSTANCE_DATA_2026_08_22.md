# Findings: `[wave-scroll-shrink]` Live Data From Two Running Instances (2026-08-22)

Follow-up to `docs/analysis/FINDINGS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_21.md`
(which correlated a single dev-branch repro's 8 events). That dataset was
one pane, one short session. This one is two currently-running "shared"
(local `task package` build) instances, each with hours of real usage, and
is roughly two orders of magnitude larger — **1,434 events in one instance,
575 in the other**. It surfaces two things the smaller dataset couldn't:
a much stronger case for a recurring fixed-size shrink, and a distinct,
more severe phenomenon the prior analysis never saw at all.

**Correction on the earlier interim numbers shared verbally before this
doc:** an initial pass under-reported this instance's totals (158 events,
3 panes) — that was `Grep`'s default 250-line output cap silently
truncating a larger match set, not the real total. All numbers below are
from unlimited `grep -c` / full-file counts, verified twice.

## Data sources

- **Instance A (`0.55.18`, local build channel `local-main-b28b7a-697d25a4`)**:
  `.../versions/0.55.18/logs/agentmux-host-v0.55.18.log.2026-08-22`, full
  session span `00:00:04Z` – `08:01:16Z` (~8 hours). **1,434**
  `[wave-scroll-shrink]` events across 3 panes.
- **Instance B (`0.55.19`, channel `local-main-b28b7a-b966d418`)**:
  `.../versions/0.55.19/logs/agentmux-host-v0.55.19.log.2026-08-22`, span
  `00:00:18Z` – `08:00:43Z`. **575** events across 3 panes.

Both are local `task package` builds (`docs/…CLAUDE.md`'s per-build
channel isolation — see "Data isolation is per-BUILD for local builds"),
not dev branches, so this is closer to real day-to-day usage than the
prior single dev-session dataset.

## 1. Per-pane volume — heavily skewed to one pane in Instance A

| Instance | Pane | Events |
|---|---|---|
| A | `43c1dea` (Camper) | **1,138** |
| A | `f5c4ae9` (Agent1 — this agent) | 206 |
| A | `0a8d11f` (AgentY) | 94 |
| B | `6365488` | 540 |
| B | `438cddb` | 40 |
| B | `34d8ae1` | 3 |

Instance A's Camper pane alone accounts for 79% of that instance's total
events — this is not evenly distributed across panes, and correlates with
Camper likely running a workload with a lot of live-updating content
(consistent with `docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md`
/ background-task work landed the same week — not confirmed here, just the
most plausible explanation for the skew; not chased down further in this
pass).

## 2. A recurring, near-identical ~251–252px shrink — new, not in the prior dataset

Instance A: **54 events** at exactly `delta=251px` or `delta=252px`
(42 at 251, distributed 4/45/5 across panes `0a8d11f`/`43c1dea`/`f5c4ae9`
respectively — i.e. it hits every pane, not one). These recur steadily
from `02:34:05Z` through `03:27:42Z` and beyond (the sampled window), at
irregular intervals ranging ~12s to ~9 minutes apart, overwhelmingly on
the Camper pane (45 of 54).

**Why this is a stronger lead than anything in the prior analysis:** a
magnitude this consistent (251 vs 252px — a 1px rounding-level difference,
effectively identical), recurring dozens of times, across independent
panes and unrelated timestamps, is very hard to explain as
content-dependent (real markdown/tool-output shrinkage would produce a
much wider, essentially continuous spread of magnitudes — which is
exactly what the rest of the histogram shows: 174×2px, 162×4px, 134×3px,
114×12px, 80×73px, 73×35px, 46×6px, 39×36px, 38×41px, ... a smooth
long-tail, with this one value spiking sharply out of that pattern). This
reads as **one specific UI element with a fixed rendered height around
251–252px that periodically collapses/disappears** — a widget, a
dashboard card, or a fixed-height placeholder — not the general
markdown/tool-block-reflow class of shrink the prior FINDINGS doc's Class
1/Class 2 taxonomy was built around.

**Not yet identified which element.** No `[wave-turn]` state transition
was found within the immediate vicinity of the sampled 251/252px events
checked (see §4) — so this doesn't obviously tie to tool-call/turn
lifecycle the way Class 1 did. Needs a targeted search for any component
with a ~251-252px fixed/estimated height (candidates worth checking:
`ActivityDock` entries, a background-task dashboard card, an
`AgentDocumentVirtualList` row-height estimate default) rather than more
log correlation — the log alone can't identify *which* element without
that height already being a known constant to grep for.

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

**Ruled out: a render-thread freeze causing this.** Checked for a
timestamp gap in the host log's own logging cadence around both
incidents — none found. The log's periodic watchdog ticks continued at
their normal ~17-20s cadence straight through both `07:39:08Z` and
`07:58:24Z` with no unusual gap (largest gaps anywhere in the full
8-hour log are the expected ~17-20s idle-tick intervals, confirmed by
scanning every consecutive timestamp pair in the file). **If the UI
thread had actually hung, the periodic tick logging would have hung with
it** — it didn't, so whatever caused the 0px readings, the process
producing this log was still alive and ticking through it, not frozen.

**Not present at all in Instance B.** Zero `-> 0px` events in 575 samples;
Instance B's delta histogram (top values: 59×4px, 57×10px, 53×2px,
46×9px, 45×3px, ...) shows only the smooth small-magnitude distribution,
no 251/252px spike either. Whatever causes both the §2 and §3 phenomena in
Instance A did not occur (or didn't occur as frequently/at all) in
Instance B's session — consistent with them being tied to something
specific to Instance A's workload/panes (the Camper-heavy skew from §1)
rather than a universal per-build defect.

## 4. What wasn't established

- **Which specific component the 251/252px shrink (§2) belongs to** —
  needs a source-level search for a matching fixed/estimated height
  constant, not more log correlation.
- **The actual mechanism behind the 0px full-collapse (§3)** — ruled out
  a render-thread freeze; not yet identified what *did* cause it. Worth
  checking window/visibility-change event handlers, and whether these
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
