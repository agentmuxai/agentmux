# SPEC: consolidate pane/tab color systems — persist explicit agent-pane picks, unify the tab-select indicator

**Date:** 2026-09-20 (revised 2026-09-24)
**Status:** active — §2.2 shipped in PR #3476, §2.3 (pane tabs) in PR #3484
and PR #3492; §3 (remove the dead `bg:*` border tier) is the one remaining
item, in the PR that commits this document.
**Author:** Camper
**Trigger:** direct user request, same session as
`SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md` (that spec's decommission
work surfaced most of the inventory this one builds on)
**Related:** `docs/specs/SPEC_AGENT_COLOR_2026_08_08.md`,
`docs/specs/SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md`,
`docs/specs/SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md`,
`docs/analysis/ANALYSIS_PANE_TAB_COLOR_COLLAPSE_2026_09_21.md`,
`docs/specs/SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md` (Revision 3)

---

## 0. Revision 2026-09-24 — what happened since this was drafted

This document was written 2026-09-20 and kept local (no implementation yet).
Most of it has since been delivered by other PRs, and one repo-owner
decision settled its main open question. Re-verified against `main` at
`eabfffd96`:

| Section | Ask | Outcome |
|---|---|---|
| §2.1 | Header inherits the agent's color | Delivered before this spec (header-unification). Then refined by #3476: explicit hue and agent identity color now go through ONE header rule, `headerBgForEffectiveColor` (`pane-color-menu.ts`), dark/muted on dark themes, full-strength on light themes. |
| §2.2 | An explicit pick on an agent pane persists to the agent | **Shipped in #3476, as this spec's option 1.** `setHue(blockId, hue, agentId)` writes `frame:hue` on the block and, for an agent pane, also `SetAgentContentCommand` `ui:color` = `hueToAgentIdentityColor(hue)` (same HSL as the border). Clearing ("Default") does not reset the agent's identity color. Other already-open panes of the same agent are not repainted, as recommended. §4 Q1 is therefore answered: "persist" means "follow the agent". |
| §2.3 | The selected-tab indicator matches the pane's resolved color | **For pane tabs: shipped.** #3484 gives every pane-tab pill its own block's color (`computeBlockActiveBorderColor`: `frame:hue` first, then `frame:activebordercolor`). The active pill's underline is that color, and #3492 made the active pill paint its own color too (`SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md` §2.2). **For window tabs: not pursued**, see §2.3 below. |
| §3 | Remove the dead `bg:activebordercolor` / `bg:bordercolor` tier | **Still open — the remaining work.** See §3. |

## 1. Inventory (current, 2026-09-24)

| # | Key | Scope | Set by | Read by |
|---|---|---|---|---|
| 1 | `frame:hue` | block meta | `setHue` (`pane-color-menu.ts`, "Pane Color" menu) | `computeBlockActiveBorderColor` / `computeFocusRingBorderColor` (border, pill underline), `headerBgForEffectiveColor` / `paneTabBgForEffectiveColor` (header, pill background) |
| 2 | `frame:activebordercolor` / `frame:bordercolor` | block meta | seeded at agent launch from `ui:color` | same readers as #1, below `frame:hue` |
| 2a | `ui:color` | agent (agent_content) | agent creation; **and now `setHue` on an agent pane (#3476)** | agent launch seeding of #2 |
| 3 | `tab:color` | window-tab meta | `tab.tsx` swatch picker (`TAB_COLORS`) | `tab.tsx`/`tab.scss` only (`--tab-color`) |
| 4 | `bg:activebordercolor` / `bg:bordercolor` | window-tab meta | **nothing** (see §3) | `computeFocusRingBorderColor` only, as its top tier |

## 2. The three asks

### 2.1, 2.2 — delivered (see §0)

### 2.3 — selected-tab indicator

**Pane tabs (the pills in a pane's header):** delivered by #3484/#3492. The
selected pill shows its block's own color and underline. That's the
"combined system" the ask was about: the indicator comes from the same
resolved pane color as the border and header, with no second pick.

**Window tabs:** deriving a window tab's color from the pane(s) inside it is
**not pursued.** The stacked-pane question (§4 Q2 of the draft: whose color
wins in a window tab that holds several differently-colored panes) now
applies to almost every window tab, because universal pane tabs made
multi-pane, multi-tab layouts normal. And the repo owner set the direction
on 2026-09-24 (`SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md` Revision 3):
a window tab never takes a pane's color; "the pane's color belongs to its
pill." A window tab keeps its own `tab:color` (explicit swatch pick), which
is a different thing, the color of the *workspace tab*, not of any pane.

## 3. Remaining work: remove the dead `bg:*` border tier

`bg:activebordercolor` / `bg:bordercolor` (inventory #4) are read by
`computeFocusRingBorderColor` (`frontend/app/block/blockframe.tsx`) as the
highest-priority tier for the focused and unfocused border, and declared in
`frontend/types/srv-types.d.ts`. Re-verified 2026-09-24 across the whole
repo (frontend, `agentmux-srv`, `agentmux-common`, `schema/`, all JSON):
**no writer exists.** They are a leftover of the Wave-era tab *background
presets* (`bg@…` presets that set `bg`, `bg:opacity`, `bg:activebordercolor`
on a tab). AgentMux no longer applies any `bg@` preset to a tab, and nothing
else writes these keys. The only way to set one today is a raw metadata write
to a tab, which no UI offers.

Keeping the tier isn't free. It's a trap in the one function every pane's
border goes through, and it already caused one bug: pills computed through it
would all collapse to a single tab-wide color (reagent P1 on #3484, which is
why `computeBlockActiveBorderColor` exists as a separate, tab-agnostic
function).

**Change:**
- `computeFocusRingBorderColor(isFocused, blockMeta)`: drop the `tabMeta`
  parameter and both `bg:*` reads. The focused branch becomes exactly
  `computeBlockActiveBorderColor(blockMeta)`. The unfocused branch keeps
  `frame:hue` → `hueToBorder`, then `frame:bordercolor`.
- Update its three callers (`BlockMask`, PaneChrome's ring color, and any
  other `atoms.tabAtom()?.meta` pass-through) to stop passing tab meta.
- Remove `"bg:bordercolor"` and `"bg:activebordercolor"` from
  `srv-types.d.ts`. The other `bg`, `bg:*`, `bg:opacity`, `bg:blendmode` keys
  stay: `computeBgStyleFromMeta` still reads them for tab/pane backgrounds.
- Reword the doc comments on `computeBlockActiveBorderColor`,
  `computeFocusRingBorderColor` and in `PaneChrome.tsx`/`PaneChrome.test.tsx`
  that explain the pills' separate path in terms of the tab-wide override.
  The separation stays (pills must never take a tab-level color), only the
  reason changes.
- A tab that somehow still carries one of these keys just stops overriding
  its panes' borders. Nothing else reads them.

**Tests:** `frontend/app/block/focus-ring-color.test.ts`, unit cases for
`computeFocusRingBorderColor`:
- focused: hue, then activebordercolor, then undefined; and it equals
  `computeBlockActiveBorderColor` (one rule for ring and pill)
- unfocused: hue-derived dim, then bordercolor, then undefined
- a cleared (`null`) hue falls through to the agent color

"Tab meta can no longer affect the ring" is enforced by the signature
itself: the function no longer takes tab meta.

## 4. Open questions

None left. Q1 was answered by #3476 (§0). Q2 (whose color a stacked window
tab takes) is moot because window tabs don't take pane colors (§2.3). Q3
(remove `bg:*` in the same PR or separately) is settled: separately, in the
PR that commits this document, since §2 shipped elsewhere.

## 5. Out of scope

- Re-deciding anything shipped in `SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md`,
  `SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md`, or the activity-flash spec.
- A `ui:color` picker outside the pane header menu (e.g. agent creation),
  still a separate follow-up per `SPEC_AGENT_COLOR_2026_08_08.md` §4.
- Changing `tab:color` or the window-tab swatch picker.
