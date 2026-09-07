# SPEC: Frosted-glass backdrop for the terminal pane tab strip

**Date:** 2026-09-07
**Status:** implemented — **Option A** (§2.1) shipped alongside this spec.
§2 records the decision and why the faithful-but-costly Option B (§2.2) was
rejected rather than deferred: it would put the top terminal row under glass,
breaking full-screen TUIs and changing the PTY's row count. If that tradeoff
is ever reconsidered, §2.2 is the starting point.
**Related:** `docs/specs/SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md`
(the agent-pane original — this spec deliberately reverses its "agent-pane-only"
scope decision, see §1), `docs/specs/SPEC_AGENT_PANE_TAB_STRIP_OVERLAY_2026_08_10.md`
(made the agent strip float), `docs/specs/SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`
(shrink-to-fit sizing, still in force for the terminal),
`docs/specs/SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12.md`
(why the strip's outer box is deliberately un-zoomed),
`docs/specs/SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md` §2.4
(the terminal strip's `animateWidth`, which interacts with §3.1 here),
`docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` (the shared strip)

---

## 0. The ask

> we want to replicate the same blur effect the agent pane tabs have into
> the terminal pane too.

---

## 1. Why this isn't a copy-paste, and what the original spec claimed

`SPEC_PANE_TAB_STRIP_TRAILING_BLUR_2026_08_12.md` §0 scoped the effect to
agent panes **on purpose**, and gave a structural reason rather than an
arbitrary one:

> **Editor/terminal:** the strip is a normal flex child at the top of a
> column layout — content starts *below* it (reserved row) … The space to
> the right of the strip's shrink-to-fit box, within that same 28px band,
> isn't covering any content — nothing is rendered there except the pane's
> own flat background. Nothing reads as "transparent" because there's
> nothing moving/scrolling behind it to notice.

That reasoning is still accurate as of `cb5cc301d`. Confirmed in the tree:

| | Agent pane | Terminal pane |
|---|---|---|
| Strip box | `position: absolute; left: 0; right: 0` (`agent-view.scss`, `.agent-pane-stack-content > .pane-tab-strip`) | normal flex child of `.view-term`, `display: inline-flex; align-self: flex-start` (shared `PaneTabStrip.scss`) |
| Width | full pane | shrink-to-fit `[tabs][+]` |
| Content below | `.agent-document` / `.agent-picker` pad their top by the strip height and scroll *underneath* | `.term-connectelem` (`flex-grow: 1`) starts *below* the reserved row |
| Behind the strip | the live, scrolling conversation | `.view-term`'s own opaque `var(--main-bg-color)` |

So `backdrop-filter: blur(2px)` applied to the terminal strip as-is would be
a **no-op blur**: it would blur a flat, uniform color, which is that same
flat color. What would still land is the `background: var(--tab-strip-bg)`
tint and the full-width box — a glass *band*, without any glass *effect*.

**This spec therefore has to decide what "the same blur effect" means**:
the same visual treatment (a tinted full-width band across the top), or the
same phenomenon (softened live content showing through). They are different
amounts of work and carry very different risk. §2 lays out the options; §2.4
is the recommendation.

---

## 2. Options

### 2.1 Option A — cosmetic only (tint + full-width band, blur is inert)

Give the terminal strip the agent pane's `background: var(--tab-strip-bg)`
and full-width box, keeping it a reserved flex row. Include the
`backdrop-filter` declaration so the treatment is literally identical, even
though with an opaque `.view-term` background behind it there is nothing for
it to soften.

- **Cost:** ~6 lines of SCSS, scoped to `.view-term > .pane-tab-strip`.
- **Risk:** near zero. No layout change, no terminal geometry change.
- **Delivers:** the panel/band look. **Not** the glass-over-content look.
- **Caveat:** the shrink-to-fit → full-width change conflicts with the
  terminal strip's `animateWidth` (see §3.1) and must be handled.

### 2.2 Option B — true overlay (faithful; terminal renders under the strip)

Mirror the agent pane exactly: make the terminal strip `position: absolute`,
full-width, `z-index: var(--z-pane-overlay, 4)`, pointer-events passthrough
on the trailing area — and let `.term-connectelem` occupy the **full** pane
height so live terminal output actually renders behind the strip and is
visibly softened by the blur.

This is the only option that reproduces the phenomenon. It also carries a
cost that has no agent-pane equivalent, and that cost is the reason this
spec exists rather than a one-line patch:

- **The terminal's top row would sit under glass.** The agent pane solves
  the same problem by *padding* its scroll container (`padding-top: calc(...
  + var(--pane-tab-strip-height))`) so no message is ever hidden. Padding
  the terminal identically would push `.term-connectelem` back down and
  restore a flat background behind the strip — i.e. it collapses back into
  Option A. So Option B only works by genuinely letting a row render
  underneath.
- **A terminal's top row is not decorative.** Full-screen TUIs on the
  alternate screen (`vim`, `less`, `htop`, `top`, editors' status lines) use
  every row they are given. Softening the top line of `htop` or a file being
  edited is a functional regression, not a visual flourish. Scrollback panes
  are more forgiving; alt-screen apps are not.
- **PTY geometry changes.** Growing `.term-connectelem` by ~28px means xterm
  fits ~1 more row and reports different dimensions to the PTY. That is a
  real `SIGWINCH`/resize to every running shell and TUI, and it changes what
  "full screen" means for them. This must not be treated as a cosmetic diff.

### 2.3 Option C — overlay only while the strip is genuinely empty

A conditional hybrid: reserved row when the pane has real tabs (today's
behavior), overlay + blur only for the lone-terminal case where the strip is
just the `+`. Rejected: it makes the terminal's row count depend on tab
count, so opening a second terminal tab would resize the PTY of the first.
Listed only so the option is on the record as considered and refused.

### 2.4 Decision — Option A (shipped)

**Option A**, unless the repo owner explicitly wants the top terminal row
rendered under glass. That was the call, and Option A is what landed.

Reasoning: the ask is framed visually ("the same blur effect"), and Option A
delivers the visual join-up — the terminal pane gets the same tinted
full-width glass band as the agent pane, so the two pane types stop looking
inconsistent. Option B buys a real blur at the price of degrading the top row
of every full-screen TUI and resizing live PTYs, which is a steep price for a
surface where, by the original spec's own analysis, there was nothing
distracting showing through in the first place.

If Option B is chosen anyway, §2.2's three consequences should be treated as
accepted tradeoffs, and the live verification in §5 becomes mandatory rather
than advisory.

---

## 3. Design (Option A)

### 3.1 Full-width box vs. `animateWidth` — must be resolved together

`term.tsx` passes `animateWidth` to `<PaneTabStrip>`, with this comment:

> Unlike the agent pane, this strip stays genuinely shrink-to-fit (no
> full-width override) — safe to animate.
> `SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md` §2.4.

`animateWidth` is a measured, FLIP-style JS width animation driven by tab
count. Making the box full-width (`left: 0; right: 0`, or `width: 100%`)
makes that animation animate a width that no longer changes — at best inert,
at worst fighting the inline width the animation writes.

**Required:** drop `animateWidth` for the terminal strip as part of this
change, and update that comment (it will otherwise assert something false).
The tab pills themselves still grow/shrink inside the now-fixed-width box;
only the box stops resizing. If the pill-level motion is judged a loss, that
is an argument for keeping shrink-to-fit and abandoning the full-width band —
which is a legitimate outcome of this spec, not a failure of it.

### 3.2 Scoping

Scoped override in `frontend/app/view/term/term.scss`, mirroring the agent
pane's own scoped override rather than touching shared `PaneTabStrip.scss`:

```scss
.view-term > .pane-tab-strip {
    width: 100%;                 // was shrink-to-fit; see §3.1
    background: var(--tab-strip-bg);
    backdrop-filter: blur(2px);
    -webkit-backdrop-filter: blur(2px);
}
```

`2px` matches the agent pane deliberately — the original spec records that
`8px` was tried and rejected live because letterforms dissolved. Do not
re-litigate the radius here; if it changes, it changes in both places.

The editor pane keeps today's behavior and is out of scope, so the two
consumers stay independently overridable.

### 3.3 Pointer events

Option A keeps the strip a reserved row, so nothing sits behind it and the
agent pane's `pointer-events: none` passthrough dance is **not** needed. Do
not copy it — it exists in `agent-view.scss` solely because that strip
overlays live, clickable content.

Under Option B it becomes mandatory, exactly as in `agent-view.scss`
(`pointer-events: none` on the box, `auto` restored on `.pane-tab-tip` and
the `+`).

---

## 4. Open question to verify before building: the `termBg` stacking order

`term.tsx` renders, when a terminal background image is configured:

```tsx
<Show when={termBg()}>
    <div class="absolute inset-0 z-0 pointer-events-none" style={termBg()} />
</Show>
```

That layer spans `inset-0` — the whole pane, **including the strip's row** —
and is positioned with `z-index: 0`, while the strip is a non-positioned flex
child appearing *earlier* in DOM order. Per CSS painting order, a positioned
`z-index: 0` box paints above non-positioned in-flow boxes, which suggests
the background image may already paint over the terminal tab strip today.

Two reasons this matters here rather than being a side note:

1. If it is true, it is a **pre-existing bug** independent of this spec
   (tabs obscured whenever a terminal background image is set), and it should
   be filed and fixed on its own rather than folded in silently.
2. It changes this spec's premise. With a background image set there *is*
   texture behind the strip's row, so Option A's blur would stop being inert
   for exactly those users — the effect would appear for some terminals and
   not others, which is worse than it uniformly not applying.

**Resolved defensively rather than by observation.** The implementation gives
the terminal strip an explicit `position: relative; z-index: 1`, which makes
the outcome deterministic whichever way the stacking currently falls: the
strip is above the `z-0` `termBg` layer and below the `z-index: 10` overlays
(search, links). That also closes the latent bug if it was real, without this
PR having to first prove it was — the two-line cost is lower than the cost of
being wrong about painting order. If it *was* real, it was pre-existing: tabs
would have been obscured whenever a terminal background image was set, with
or without this spec.

---

## 5. Acceptance criteria

Option A:

1. With one terminal tab open, the strip renders as a full-width band across
   the top of the pane, tinted with `--tab-strip-bg`, visually matching the
   agent pane's band.
2. Terminal content geometry is **unchanged** — same rows/cols before and
   after, confirmed by `stty size` (or `tput lines`) inside the pane rather
   than by eye.
3. Tabs, `+`, double-click rename, close, and middle-click close all behave
   exactly as before.
4. Light and dark themes both look correct (`--tab-strip-bg` is theme-scoped
   and inverts: white tint on dark themes, `rgba(0,0,0,0.03)` in `light.scss`).
5. The editor pane is visually unchanged.
6. `term.tsx`'s `animateWidth` comment no longer claims the strip is
   shrink-to-fit (§3.1).

Additionally, if Option B is chosen:

7. Live-verify against an alt-screen TUI (`htop`, `vim`) that the top row
   under the glass is still usable, and accept or reject on that evidence.
8. Confirm the PTY row-count change is intentional and that no running shell
   misbehaves across the transition.

## 6. Verification

Per this repo's practice, the visual criteria are **not** satisfiable by unit
test — they need `task dev` in an isolated worktree/branch, not the
instance the developer is working in. Criterion 2 (`stty size`) is the one
piece of hard evidence available and should be captured explicitly, since it
is what separates Option A from an accidental Option B.
