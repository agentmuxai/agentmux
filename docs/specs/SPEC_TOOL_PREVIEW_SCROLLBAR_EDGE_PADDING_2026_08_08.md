# SPEC: Tool preview scrollbar-to-edge padding removal

**Date:** 2026-08-08
**Status:** Implemented (same day)
**Scope:** `frontend/app/view/agent/styles/_document-nodes.scss`,
`frontend/app/view/agent/styles/_tool-overlay-portal.scss`
**Related:**
- `SPEC_TOOL_PREVIEW_REFINEMENTS_2026_06_26.md` (prior preview-polish pass)
- `SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md` (established
  `.agent-tool-overlay-log` as the single scroll container per tool block)
- `SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06.md` /
  `SPEC_TERM_SCROLLBAR_ZERO_GAP_2026_06_10.md` (the two prior
  scrollbar-at-edge fixes; same design goal — the scrollbar should read as
  sitting AT the surface's edge, not floating inside it)

This is refinement #1 of a tool-preview polish series (user ask: "refine the
preview for Read, Bash, Write, and any other that has a preview; first,
get rid of the extra padding between the preview's scrollbar and the edge").
Further refinements will get their own specs.

---

## 1. Report

In the agent pane, every expanded tool-call preview (Read, Bash, Write,
Edit, Grep/Glob, Agent, Task, and the compact default) shows its vertical
scrollbar floating ~8px inboard of the preview surface's right edge, with
dead space on **both** sides of the scrollbar track. The preview surface
(the subtly-shaded panel) visibly continues past the scrollbar, which reads
as misaligned — a native scroll surface puts its scrollbar flush at the
container's edge (editor pane, terminal, and the main conversation scroll
all already do this in AgentMux).

## 2. Structure and measurements (traced, not guessed)

All tool previews share **one** scroll container. There are no per-tool
scrollbars (deliberate — the double-scrollbar bug fix, see
`SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md` §"single scroll
container"). The DOM/CSS chain:

```
.agent-view .agent-tool-block
└── .agent-tool-panel                      _document-nodes.scss:349
      padding: 6px 8px;                    ← outer inset, all four sides
      max-height: 50vh; overflow: hidden;
      background: color-mix(...)           ← the visible "preview surface"
    ├── .agent-tool-overlay-header         _tool-overlay-portal.scss:17
    │     padding: var(--space-1) var(--space-2);   (4px 8px)
    └── .agent-tool-overlay-log            _tool-overlay-portal.scss:40
          overflow-x: auto; overflow-y: auto;
          padding: var(--space-1-5) 10px var(--space-1-5) var(--space-2);
          (6px top/bottom, 10px right, 8px left)
        └── per-tool content (.agent-tool-read / .agent-bash / .agent-diff
            / .agent-tool-write / .agent-tool-search / CompactResult ...)
```

The scrollbar is a real (layout-space-reserving, non-overlay) WebKit
scrollbar, 7px wide via the universal rule `*::-webkit-scrollbar { width:
7px }` (`app.scss:87-90`); its track is transparent
(`--scrollbar-background-color`, `theme.scss`). A `::-webkit-scrollbar`
renders inside its owning element's own box, at the right edge of
`.agent-tool-overlay-log` — **not** at the edge of the panel.

So the right-edge stack, from panel edge inward, is:

| Layer | Width | Source |
|---|---|---|
| `.agent-tool-panel` right padding | 8px | `_document-nodes.scss:349` (`padding: 6px 8px`) |
| scrollbar (in the log's box) | 7px | `app.scss:87` |
| `.agent-tool-overlay-log` right padding | 10px | `_tool-overlay-portal.scss:46` |
| → content | | |

**The reported "extra padding between the scrollbar and the edge" is the
panel's 8px right padding** — it insets the log's entire box (scrollbar
included) from the visible surface edge. The log's own 10px sits on the
*other* side of the scrollbar (content ↔ scrollbar breathing room — less
objectionable, but oddly larger than the 8px left inset, and stacked with
the 8px it produces a ~25px total right inset for content vs 16px on the
left).

The same 8px story repeats vertically: the panel's 6px bottom padding
insets the horizontal scrollbar (the log is also `overflow-x: auto`) and
cuts the vertical track 6px short of the surface's bottom edge.

## 3. Goal

The scroll container's border box should coincide with the preview
surface's edges, so:

- the vertical scrollbar sits flush at the panel's right edge (zero gap
  outboard of the track), running the full height of the scrollable region;
- the horizontal scrollbar (when present) sits flush at the panel's bottom
  edge;
- content keeps sane breathing room on the inner side of the scrollbar and
  a **symmetric** left/right text inset;
- no behavior change to scrolling, collapse animation, or the
  single-scroll-container invariant.

## 4. Proposed change

Move the panel's inset onto its children so the log's box (and therefore
its scrollbars) reaches the panel edges:

### 4.1 `.agent-tool-panel` (`_document-nodes.scss:349`)

```scss
- padding: 6px 8px;
+ padding: 0;
```

The `--hidden` collapse variant already only zeroes `padding-top`/
`padding-bottom` (`_document-nodes.scss:~397`); with base padding 0 those
declarations become no-ops — harmless, and the `padding 120ms` entry in the
`transition` list simply has nothing to animate. Leave both in place (the
shell-block mirror in `_shell-node.scss:39` never had padding, so this also
converges the two panel variants).

### 4.2 `.agent-tool-overlay-log` (`_tool-overlay-portal.scss:46`)

```scss
- padding: var(--space-1-5) 10px var(--space-1-5) var(--space-2);
+ padding: var(--space-1-5) var(--space-1) var(--space-1-5) var(--space-2);
```

- Left stays `--space-2` (8px) — content keeps its text inset, now measured
  from the true surface edge (content inset goes 16px → 8px, gaining ~8px
  of usable preview width per side; a win for code lines).
- Right becomes `--space-1` (4px) — this is now purely the content ↔
  scrollbar gap; the scrollbar itself provides the edge separation. 4px
  matches the visual role (compare: the main conversation's content ↔
  scrollbar gap). If 4px feels tight against the 7px thumb in practice,
  `--space-1-5` (6px) is the fallback — decide by eye at implementation
  time, not in this spec.
- Top/bottom stay `--space-1-5` (6px) — the vertical inset the panel used
  to provide, now carried by the log itself so the track runs edge to edge
  while the *content* keeps its vertical breathing room.

### 4.3 `.agent-tool-overlay-header` (`_tool-overlay-portal.scss:17-23`)

No change to its own `padding: var(--space-1) var(--space-2)` — but note
two knock-on effects of 4.1, both desirable:

- header text inset drops 16px → 8px, now aligning with the log content's
  left inset instead of sitting 8px further in (the old 16px was two
  paddings stacking, not a design choice);
- the header's `border-bottom` extends to the panel's full width instead of
  stopping 8px short of each edge (full-bleed divider — consistent with how
  `.agent-bash-cmd` / `.agent-diff-header` section bars will now also reach
  the edge).

### 4.4 Explicitly out of scope for the CSS change

- `.agent-activity-log` (`_shell-node.scss:326`) — ActivityRow's live
  activity feed reuses the `agent-tool-overlay-log` class
  (`ActivityRow.tsx:211,235,254`) but overrides `padding`, `max-height`,
  and `overflow-y` with its own single-class rule that wins on cascade
  order (`shell-node` is `@use`d after `tool-overlay-portal`,
  `agent-view.scss:10` vs `:33`). It is therefore **unaffected** by 4.2.
  Its own 8px-right-padding-inside-a-scroll-container has the same smell,
  but it renders inside a differently-shaped surface (the shell/activity
  block) — align it in a follow-up if the visual result of this spec looks
  right, rather than batching it in blind.
- `PersistentShellBlock.tsx:154` — uses the log class inside
  `.agent-shell-block .agent-tool-panel`, whose panel mirror never had
  padding (`_shell-node.scss:39`). It gets 4.2's right-padding tightening
  automatically and needs no separate edit. Include it in the visual pass.
- The 7px scrollbar width/track/thumb styling (`app.scss`) — unchanged;
  this spec repositions the scrollbar, it doesn't restyle it.

## 5. Risks / things the implementer must check

1. **FLIP height animation** (`ToolOverlayLog.tsx:342`, `flipHeight`) —
   measures and animates the log's `height` and briefly forces
   `overflow-y: hidden`. Padding is inside the measured box; changing it
   changes measured heights but not the mechanism. No code change expected;
   verify expand/collapse still animates smoothly.
2. **Preview zoom** (`fontScale` inline `font-size` on the log,
   `ToolOverlayLog.tsx:308`) — padding is px-based and unaffected by
   font-size scaling; confirm Ctrl+scroll zoom inside a preview doesn't
   reveal layout oddities at the new tighter right edge.
3. **Stick-to-bottom scroll logic** (`onScroll`, `ToolOverlayLog.tsx`)
   — reads scrollTop/scrollHeight; padding-neutral. No change expected.
4. **Per-tool inner chrome now reaching the edge**: `.agent-bash-cmd`'s
   command bar, `.agent-bash-exit`, `.agent-diff-header`, and bordered
   content boxes (`.agent-tool-read-content`'s 1px border,
   `.agent-tool-search-results`) will sit 8px closer to the panel edge on
   both sides. This is the intended full-bleed direction, but eyeball each
   of Read (code + markdown variants), Bash, Write, Edit/diff (plain +
   Shiki-highlighted), Grep/Glob, and the compact default — the spec's
   whole premise is that these share one shell, so one CSS edit moves all
   of them at once.
5. **Responsive tiers** (`_responsive.scss:58`) — only overrides
   `max-height` at wide widths; no padding interaction.
6. **Awaiting-approval / failed states** — the header renders only in
   non-success states; check one of these states so the header's new
   full-width divider and 8px inset are seen together with the log.

## 6. Test plan

Visual (via `task dev`, or `scripts\dev-agent.cmd` from an agent shell),
with an agent producing real tool calls:

- [ ] Read (a `.rs`/`.ts` file → Shiki path, and a `.md` file → markdown
      path): scrollbar flush at panel right edge; no dead strip outboard of
      the track; content ↔ scrollbar gap looks intentional.
- [ ] Bash with long output: vertical scrollbar flush; cmd header bar and
      exit-code bar span edge to edge; with a long unwrapped line,
      horizontal scrollbar flush at the panel bottom edge.
- [ ] Write: same checks as Read.
- [ ] Edit (diff, both plain and highlighted): add/del line backgrounds
      extend to the new content width; no clipped diff markers.
- [ ] Grep/Glob + a default/compact tool: bordered result boxes look right
      at the new inset.
- [ ] Expand/collapse a panel (pin + auto-expand paths): 120ms collapse
      still animates cleanly; no flash from the padding change.
- [ ] Ctrl+scroll preview zoom at min/max scale: no overlap of content and
      scrollbar.
- [ ] Persistent shell block's build log (reuses the log class): unchanged
      or improved; no regression from 4.2's right-padding change.
- [ ] Activity feed (ActivityRow): confirm visually unchanged (its own
      padding override wins), guarding §4.4's cascade claim.
- [ ] One non-success state (denied/failed) for the header divider width.
- [ ] `npx tsc --noEmit` + frontend tests — CSS-only change, expect clean.

## 7. Summary

One 8px `padding` on `.agent-tool-panel` is the entire reported bug: it
insets the shared scroll container — scrollbar and all — from the preview
surface's edge, and every tool preview inherits it because they all render
inside that one panel + one scroll container. Zero the panel's padding,
re-home the vertical inset onto the log, and tighten the log's
scrollbar-side padding; the header and per-tool section bars then land
full-bleed as a natural consequence. One SCSS file edit for the mechanism
(4.1), one for the log (4.2), everything else is verification.
