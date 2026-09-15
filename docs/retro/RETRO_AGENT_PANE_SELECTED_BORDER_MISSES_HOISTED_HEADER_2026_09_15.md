# Retro: Agent pane's selected border no longer wraps the header/progress bar

**Date:** 2026-09-15
**Severity:** Low-medium — cosmetic, but visible on every agent pane, every
session; makes the currently-focused agent pane harder to spot at a glance
compared to every other pane type (terminal, editor, browser), which still
get a full-perimeter selection ring.
**Status:** implemented — Agent pane + Terminal pane, both confirmed to
share the same root cause. See "Fix" below for what shipped.

## What happened

Reported by the user: "the selected border should surround the entire pane
including the header, like the other panes." On an Agent pane, the
accent-colored focus ring now stops at the top of the conversation content —
the header (agent name/avatar/controls) and the progress-bar row above it
render outside the ring, so a focused Agent pane's border visually looks like
it starts partway down the pane instead of at the very top. Other pane types
(terminal, editor, browser) still get a ring around the whole pane.

## Root cause

The selection ring is not a border on the pane's outer box — it's a separate
overlay element, `.block-mask` (`frontend/app/block/block.scss:242-334`),
absolutely positioned (`inset: 0`) *relative to `.block`*
(`.block { position: relative; ... }`, same file, line 13), and painted
accent-colored via `.block-focused .block-mask { border-color:
var(--accent-color); }` (lines 297-310). It is rendered as the last child of
`.block`'s own root div by `BlockMask` (`frontend/app/block/blockframe.tsx`).

For a normal pane, `.block-frame-default-header` renders *inside*
`.block-frame-default-inner`, which is *inside* `.block` — so `.block-mask`'s
`inset: 0` naturally spans header + body together, and the ring wraps the
whole pane.

**For an Agent pane specifically, this stopped being true as of commit
`58a8c3e03`** ("fix(agent): stop the pane header/tab-strip flash on Agent <->
History switch", PR #3136, merged 2026-09-09, implementing
`SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`). That change introduced
`AgentPaneChrome` (`frontend/app/view/agent/agent-view.tsx`, `AgentPaneChrome`
starting at line 349) and a new hoisting mechanism
(`frontend/app/tab/pane-leaf-chrome.tsx`) so an agent pane's header/tab-strip
survive a tab switch as a stable outer shell instead of remounting each time.
The resulting DOM, from `agent-view.tsx:786-816`:

```tsx
<div class="agent-pane-stack" data-blockid={activeBlockId()} ...>
    <ErrorBoundary fallback={headerElemNoView}>{headerElem}</ErrorBoundary>   {/* line 808 */}
    <div class="agent-pane-progress-bar-slot" ref={(el) => setSlotEl(el)} />  {/* line 815 */}
    <div class="agent-pane-stack-content">
        {content /* = the switch-scoped <Block>, carrying .block + .block-mask */}
    </div>
</div>
```

The header (`headerElem`) and the progress-bar slot are now **siblings
rendered before** `.agent-pane-stack-content`, which is the *only* wrapper
around `content` (the `<Block>`/`.block`/`.block-mask` subtree).
`AgentViewModel.noHeader` (`agent-model.ts:203`,
`this.noHeader = () => this.nodeModel.paneChromeHoisted === true;`)
suppresses `BlockFrame`'s own inline header so it isn't duplicated inside
`.block-frame-default-inner`. The net effect: `.block` (and therefore
`.block-mask`) now spans only the conversation-content region — the header
and the progress-bar row sit structurally outside the box the selection ring
covers.

The user's framing — "this was introduced when the progress bar was moved" —
is accurate in substance and points at the right commit: the progress-bar
slot's relocation (from inside the old, still-in-border `.agent-pane-stack`
wrapper to its current position as a sibling of the hoisted header, both now
outside `.block`) happened in this same commit, as a visible part of the same
refactor that hoisted the header out. It's a symptom of the same structural
change, not a separate regression.

A follow-up commit, `cecd813a2` ("fix(agent): restore pane tab strip + header
layout, and drop the remaining new-tab flash", PR #3151, 2026-09-10), fixed
the hoisted header's *visual* styling (it initially rendered as unstyled
stacked divs) by extracting a shared `block-frame-default-header-layout()`
mixin (`frontend/app/mixins.scss`) and applying it to the hoisted header. It
did not touch `.block-mask`/`.block-focused` or add any equivalent border
around `.agent-pane-stack` as a whole — the selection-ring gap was not part of
what got restored, and nothing since has addressed it. A repo-wide search for
`block-mask`/`block-focused` under `frontend/app/view/agent/` and
`frontend/app/tab/` turns up no compensating rule.

## Why this wasn't caught at the time

PR #3136/#3151 were explicitly scoped to fixing a visual *flash* on tab
switch (`SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`) — a real,
narrowly-defined bug with its own before/after repro. The selection ring
wrapping the whole pane was working correctly before this refactor and simply
wasn't part of that change's checklist, so nobody was specifically looking at
`.block-focused` styling while validating it. This is a common shape for this
kind of regression: a structural DOM refactor (moving what's inside vs.
outside a positioned container) silently breaks a CSS rule that depends on
containment, with no error, no test failure, and no visual difference unless
you specifically compare a *focused* pane's border before and after.

Also relevant: `TermPaneChrome` (`frontend/app/view/term/term.tsx`) does the
identical hoist for terminal panes with an in-pane tab strip (same
`pane-leaf-chrome.tsx` mechanism, same `HOISTS_OWN_CHROME` gate). That path
has the same latent structural gap — a stacked terminal pane's header/tab
strip likely sit outside `.block-mask` too — but wasn't what the user
reported, and hasn't been independently verified here. Worth checking
alongside any fix to the Agent pane case, since a fix scoped to `agent-view.tsx`
alone would leave the terminal case unaddressed if it does turn out to have
the same gap.

## How this looks — current (broken) vs. expected

The progress-bar slot is `position: absolute; top: 0` against
`.agent-pane-stack` (`agent-view.scss:265-271`) — despite coming *after* the
header in DOM order, it's pulled out of normal flow and pinned as a thin 3px
strip flush against the very top edge of the pane, so it renders visually
**above** the header/title row, not below it (confirmed against the CSS, not
assumed).

Current: the accent ring (`█`) only wraps `.agent-pane-stack-content`. The
progress-bar strip and the header both sit above it, outside the ring,
bordered in the pane's normal dim `--border-color` (`░`) if bordered at all:

```
┌ ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ ┐
░  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░ ░   ← progress bar (outside ring, top edge)
░  🤖 agent-name          [tab] [tab] [+]  ⚙  ⤢  ░   ← header/title (outside ring)
└ ░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ ┘
  ┌███████████████████████████████████████████████┐
  █                                                 █
  █   > conversation content                        █  ← .block / .block-mask
  █     assistant reply...                          █    (ring starts HERE,
  █                                                 █     not at the top)
  █   [ type a message...                    ▷ ]    █
  █                                                 █
  └███████████████████████████████████████████████┘
```

Expected (matches every other pane type — terminal, editor, browser): the
ring wraps the header and the content as one continuous box, and the
progress bar becomes a thin *inset* overlay sitting just inside that ring's
top edge — layered on top of the header's own top few pixels (so it still
costs no extra layout height, same as today), but offset in from the ring by
the ring's own stroke width so it never paints over/through the border
itself:

```
┌███████████████████████████████████████████████┐
█ ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░ █   ← progress bar, inset 2px from
█  🤖 agent-name          [tab] [tab] [+]  ⚙  ⤢  █     the ring on all 3 sides —
█ ───────────────────────────────────────────── █     overlays the header's top
█                                                 █     edge, ring stays visible
█   > conversation content                        █     all the way around
█     assistant reply...                          █
█                                                 █
█   [ type a message...                    ▷ ]    █
█                                                 █
└███████████████████████████████████████████████┘
```

## Fix (not yet applied)

Not implemented as part of this retro — this document is investigation and
root-cause only, per the user's request ("take a look, write a retro to
file"). The most direct options, for whoever picks this up:

1. Move (or duplicate) the selection-ring painting so it covers
   `.agent-pane-stack` as a whole, not just the nested `.block`. Cheapest
   version: give `.agent-pane-stack` its own focused-border rule keyed off
   the same focus state `.block-focused` already reads, since `.block-mask`
   itself can't simply be relocated (it's rendered inside `BlockFrame`,
   shared by every pane type, and still needs to draw the *unfocused* 2px
   border around just the content box in other contexts).
2. Once the ring wraps `.agent-pane-stack`, `.agent-pane-progress-bar-slot`
   (`agent-view.scss:265-271`, currently `top/left/right: 0` against
   `.agent-pane-stack`) needs to inset from all three edges by the ring's
   own border width (2px, `block.scss:249`) instead of sitting flush at
   `0` — otherwise the progress bar's fill paints directly over the new
   top/left/right border rather than nesting inside it. Confirmed design:
   the bar still overlays the header's own top pixels (no reserved height,
   same as today, per `SPEC_AGENT_PANE_PROGRESS_BAR_OVERLAY_NO_GAP_
   2026_08_25.md`'s original no-layout-shift intent) — only the offset from
   the ring itself is new.
3. Confirm/fix `TermPaneChrome`'s identical case at the same time if it
   reproduces, so the fix isn't Agent-pane-specific when the underlying gap
   (hoisted chrome living outside `.block`) isn't.
4. Whatever the shape of the fix, verify visually (screenshot/UIQuery of a
   focused Agent pane, and a focused Term pane with 2+ tabs) rather than by
   code inspection alone — this exact class of bug (a rule that's
   structurally correct on paper but wrong once containment changes) is easy
   to reintroduce without a visual check.
