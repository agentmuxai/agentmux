# SPEC: tool previews — cap at one third of today's height, and always follow the latest output

**Author:** Agent4
**Date:** 2026-09-25
**Status:** proposed
**Related:** `SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md` (the agent-pane
follow work, PR #3652, tracking issue #3655 — Part B below is that spec's Phase 3,
pulled forward), `SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md` (the FLIP height
transition), `SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md` (wheel handoff to the
pane).

All line numbers are against `main` at `3cd1bd188`.

---

## 1. Request (user, 2026-09-25)

1. **Height.** Tool previews should keep sizing relative to window height, as they do
   today, but the maximum should be **one third** of today's maximum. "Today the tallest
   preview is about 40 lines; we want a third of that, 15 or so." Previews shorter than
   the maximum show at their natural height, as today.
2. **Follow the latest output.** A preview's inner scrollbar should stay at the bottom as
   new output arrives, and the user can still scroll up to read. Today it is often seen
   "wandering off to the middle". Bring it in line with the recent agent-pane follow work.

An earlier draft of this spec proposed an 8-line default with an expand button. The user
replaced that with (1) above. The draft is superseded, not partly kept (§7).

---

## 2. Part A — height: one third of today's cap

### A.1 Today

The cap is `max-height` in `vh` (window height), on `.agent-tool-panel`. Nothing is in
px or lines.

| Where | Rule | Applies to |
|---|---|---|
| `frontend/app/view/agent/styles/_document-nodes.scss` (`.agent-tool-block .agent-tool-panel`, ~line 375) | `max-height: 50vh` | agent pane < 900 px wide |
| `frontend/app/view/agent/styles/_responsive.scss` (Tier 4, `@container agent-pane (min-width: 900px)`, ~line 58) | `max-height: 60vh` | agent pane ≥ 900 px wide |
| `frontend/app/view/agent/styles/_shell-node.scss` (`.agent-shell-block .agent-tool-panel`, ~line 39) | `max-height: 50vh` | persistent-shell log (not a tool preview) |

Log lines are 12 px × 1.4 = 16.8 px (`_tool-overlay-portal.scss`, `.agent-tool-log-line`).
The panel has `padding: 0`; the log inside it (`.agent-tool-overlay-log`) is the single
scroll container and carries its own padding. Content shorter than the cap already
renders at natural height; that does not change.

The user's "about 40 lines" matches 60 vh on a ~1150 px-tall window or 50 vh on a
~1370 px-tall one. That is an inference from the numbers, not a measurement.

### A.2 Change

Keep the scheme; divide both values by three. Put the number behind one custom property
so the two tiers cannot drift apart.

```scss
// _document-nodes.scss, on the agent-pane container (or .agent-tool-block)
--agent-tool-panel-max-h: calc(50vh / 3);   // was 50vh  (≈16.7vh)

// _responsive.scss, Tier 4 container query
--agent-tool-panel-max-h: calc(60vh / 3);   // was 60vh  (= 20vh)

// .agent-tool-block .agent-tool-panel
max-height: var(--agent-tool-panel-max-h);
```

Writing it as `calc(50vh / 3)` keeps the "one third of what it was" intent readable in
the source.

Approximate visible lines at the cap (panel height ÷ 16.8 px, minus ~1 line for log
padding; a failed/denied/awaiting tool's header row takes about one more):

| Window height | < 900 px pane (16.7vh) | ≥ 900 px pane (20vh) |
|---|---|---|
| 800 px | ~7 | ~8 |
| 1080 px | ~10 | ~12 |
| 1200 px | ~11 | ~13 |
| 1440 px | ~13 | ~16 |

So "40 → about 13–15" holds on the user's setup. On small windows it gets short (~7 lines
at 800 px); no floor is added (§7 item 1).

### A.3 Scope

- **In:** tool previews (`.agent-tool-block .agent-tool-panel`), both width tiers.
- **Out:** the persistent-shell log (`.agent-shell-block`). It is a long-running
  build/dev log rather than a tool preview and keeps `50vh` (§7 item 2). It must not
  pick up the new custom property by
  accident: the property is set on the tool-block scope, not globally.
- **Out:** the px-based composer / decision / question panel caps.

### A.4 Knock-on edits

Comments that quote `50vh` as a number must change in the same PR, or they will mislead
the next reader. Their reasoning stays the same.

- `ToolOverlayLog.tsx` — the FLIP measurement comments (~lines 244–262).
- `resize-contract.ts` — the `DEFAULT_MEASURE` and `withHeightContinuity` doc comments
  (~lines 219–251).
- `_document-nodes.scss` — the `.agent-bash-output` comment (~line 850) and the others
  that say "bounded by `.agent-tool-panel`'s 50vh".

A smaller cap means the running→terminal FLIP animates a smaller rendered delta. That
only makes it cheaper; the magnitude gate (`MAX_ANIMATED_DELTA_PX`) is on the rendered
delta and will trip less often, not more.

---

## 3. Part B — always show the latest output

### B.1 Today (`frontend/app/view/agent/components/ToolOverlayLog.tsx`)

The follow logic is local to this component:

```ts
let stickToBottom = true;                        // ~line 139
const onScroll = () => {
    const dist = scrollHeight - scrollTop - clientHeight;
    stickToBottom = dist < 40;                   // geometry only
};
createEffect(() => {                             // ~line 212
    chunks();
    const hidden = panelHidden();
    if (stickToBottom && scrollRef)
        requestAnimationFrame(() => { scrollTop = scrollHeight; });
});
```

The scroller renders one of four `<Switch>` branches (~line 361): the live chunk log
while streaming, or the final result (`ToolOverlayResult`, e.g. `BashOutputViewer`)
once the tool finishes.

### B.2 Why it wanders — likely causes

These come from reading the code. **None is confirmed by a live repro yet**; B.4 step 1
exists to do that. They are ordered by how well they explain "ends up in the middle".

1. **No pin when the finished result replaces the live log.** The effect's only
   dependencies are `chunks()` and `panelHidden()`. When a tool completes, the `<Switch>`
   swaps `ChunkList` for `ToolOverlayResult`, a different DOM tree with a different
   height, and nothing re-pins. `scrollTop` keeps its old number, which inside the new
   content is usually somewhere in the middle. This fits the symptom best, since it
   happens at the end of every streamed tool call.
2. **Only chunk arrival triggers a pin.** Anything else that changes the content or the
   box's height doesn't: async rendering inside the result (syntax highlighting, lazy
   renderers), the 120 ms `max-height` transition, the FLIP animation, a pane or window
   resize. The single `requestAnimationFrame` pin can also run before such a change
   settles, landing short of the true bottom.
3. **Detach is decided from geometry, not from the user.** Any `scroll` event more than
   40 px from the bottom turns follow off, whoever caused it: a browser clamp when
   content shrinks, a scroll-anchoring adjustment, the output cap
   (`MAX_TOOL_OUTPUT_LINES`, tail-capped) dropping head lines. Once off, only the user
   scrolling back within 40 px turns it on again. The agent-pane spec names this exact
   rule, in this exact file, as the same bug class it fixed there (09-24 spec §5.1:
   "the same misclassification problem in a simpler form").
4. **State is lost on remount.** `stickToBottom` is a plain `let`. When the virtualized
   row remounts, it resets to `true` but nothing pins until the next chunk arrives. This
   shows the top, not the middle, so it is a minor contributor.

### B.3 Target behaviour

Adopt the agent pane's model from `SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md`
§5, scaled down to one small scroller:

| State | Behaviour |
|---|---|
| **FOLLOWING** (initial) | Keep the true bottom in view. Pin on **every** content or box resize (ResizeObserver on the log and on its content), including the running→result swap, async renders, transitions, and pane resizes. |
| **DETACHED** | The user is reading. Nothing moves the scroll position except the user. |
| **SUSPENDED** | Panel hidden (`.agent-tool-panel--hidden`, `content-visibility: hidden`). Ignore all input, read no layout. Restore the previous state when shown, and pin if that state is FOLLOWING. The existing `panelHidden` MutationObserver already tracks this. |

Transitions, with the same rules as the pane:

- FOLLOWING → DETACHED **only** on a user gesture that scrolls up: wheel up, a scrollbar
  drag (a `pointerdown` whose target is the scroller element itself), or PageUp/Up/Home
  with focus inside. A `scroll` event with no user gesture in the last 250 ms never
  detaches, whatever the geometry (the pane spec's invariants I1 and B2).
- DETACHED → FOLLOWING when a user scroll ends within **24 px** of the bottom (the pane's
  `REATTACH_PX`, which already allows for fractional positions at non-100% zoom).
- A tool finishing does **not** reattach a DETACHED reader. If the user scrolled up to
  read, the result appears without moving them.
- **Initial state depends on the kind of output** (decided 2026-09-25, §7 item 3).
  Streamed output and terminal-style results (Bash and similar) start FOLLOWING, so they
  open at the latest line. Document-like previews (Read, Write, Edit, Diff), which have
  no streaming phase, start DETACHED at the **top**, because the start of a file or diff
  is where reading begins. A document preview can still be followed: a user scroll to
  the bottom attaches it like any other.
- On remount, apply the same initial-state rule and pin once the element has size if
  that state is FOLLOWING. The row was off-screen, so there is no reading position to
  keep.

Must not break:

- The wheel hand-off to the outer pane at either scroll boundary (~lines 158–181,
  `SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md`). A wheel-down at the bottom still
  scrolls the pane, and must not count as a detach.
- The rule against reading layout inside a `content-visibility: hidden` subtree
  (~lines 183–197), and the existing test "node updates within the same branch read no
  geometry and no computed style" (`ToolOverlayLog.test.tsx:347`). Pins read geometry
  only in ResizeObserver callbacks and `requestAnimationFrame`, never in the update path
  or in input handlers (`tools/lint/check-input-handler-layout-reads.sh`).
- The FLIP transition (`withHeightContinuity`). While FOLLOWING, re-pin at the end of the
  transition as well as at the start, so the bottom stays in view as the box eases.

### B.4 Implementation path

The 09-24 spec already plans this as its **Phase 3**: move `ToolOverlayLog` (and
`SystemToolInstallInline`) onto a shared `createFollowScroll` primitive. That primitive
does not exist yet (`frontend/app/view/agent/scroll/` is absent; Phases 1–2 not
started).

Recommended order:

1. **Confirm the cause first.** Add the pane's I4 telemetry to this scroller, one
   `[scroll-follow] tool=<id7> <from>→<to> cause=<cause>` line per state change plus a
   `pin reason=<chunks|result-swap|resize>` debug line, and reproduce with the CDP
   harness (`scripts/ui-screenshots/`): stream a 200-line Bash, let it finish, and read
   `scrollTop`/`scrollHeight`/`clientHeight` before and after completion. This tells us
   which of B.2's causes is real before we write the fix.
2. **Build the pure reducer** (the 09-24 spec's Phase 1, `followReducer`), table-tested,
   with no DOM. It is small, the pane will need it anyway, and it keeps this from
   becoming a third copy of the follow rules.
3. **Wire it into `ToolOverlayLog` first**, through a minimal binding: RO pins,
   the user-intent window, and SUSPENDED from `panelHidden`. The tool preview is a much
   smaller surface than the transcript, so it is a good first user of the primitive.
   Moving the transcript onto it stays the 09-24 spec's Phase 2.

**Coordinate before starting step 2.** The 09-24 spec is owned by Agent2 and tracked in
issue #3655. Taking its Phase 1 and a slice of Phase 3 ahead of its Phase 2 needs a note
on #3655, or agreement with Agent2, so the two efforts don't build competing reducers.
If that primitive is about to land anyway, skip step 2 and consume it.

**Fallback if coordination stalls:** a Phase-0-style fix inside `ToolOverlayLog` only.
Replace the 40 px rule with a user-gesture gate, re-pin on a ResizeObserver, and re-pin
on the branch swap. It is roughly a 60-line change and can later be deleted in favour of
the primitive.

---

## 4. Tests

**Height (Part A)**

- jsdom can't lay out, so unit tests can't check `vh`. Assert the stylesheet contract
  instead: `.agent-tool-block .agent-tool-panel` reads `var(--agent-tool-panel-max-h)`,
  and the shell block keeps `50vh`. A small SCSS-source test, or skip it and rely on the
  pixel check.
- **Pixel check** (`scripts/ui-screenshots/`): a 200-line Bash output at window heights
  800 / 1080 / 1440 in both pane-width tiers. Assert the panel height is within ±2 px of
  `innerHeight × 0.5 / 3` (or `× 0.6 / 3` in the wide tier). A 3-line output renders at
  natural height with no empty gap. Repeat at a **non-100% app zoom**: I haven't verified
  how the app's zoom handling (`docs/specs/zoom-architecture.md`) interacts with `vh`, so
  check it rather than assume it.

**Follow (Part B)** — extend `ToolOverlayLog.test.tsx`, stubbing geometry the way its
FLIP tests and `AgentDocumentVirtualList.pin.test.tsx` already do.

1. Streaming chunks keep the log pinned to the bottom.
2. **Running → result swap while FOLLOWING ends at the bottom.** This is the regression
   test for B.2 cause 1 and must fail on today's code.
3. Content growing with no new chunk (a mocked RO callback) re-pins while FOLLOWING.
4. Wheel up → DETACHED; later chunks and the result swap do not move `scrollTop`.
5. A `scroll` event 200 px from the bottom with **no** preceding gesture does not detach.
   This is the regression test for cause 3 and must fail on today's code.
6. A user scroll back to within 24 px of the bottom reattaches, and the next chunk pins.
7. A wheel down at the bottom hands off to the pane (the existing chaining behaviour) and
   stays FOLLOWING.
8. Hidden panel: no geometry reads; on show, it pins if FOLLOWING and stays put if
   DETACHED.
9. Remount applies the initial-state rule: a Bash preview pins to the bottom once sized,
   and a Read/Diff preview stays at the top.
10. A Read/Write/Edit/Diff preview opens at the top (DETACHED), and a user scroll to its
    bottom attaches it.
11. All existing FLIP and no-layout-read tests in the file still pass unchanged.

Then a live check with the telemetry from B.4 step 1: run a few long Bash calls, scroll
up mid-stream in one of them, and confirm every `[scroll-follow]` transition has a user
cause.

---

## 5. Rollout

One PR for Part A. It's CSS plus comments and low risk. It can ship on its own and
ships first.

Part B is one or two PRs depending on the B.4 decision: telemetry and repro first, then
the fix with its tests. No setting or kill switch is needed for a single tool-preview
scroller. If the shared reducer is used, it follows the 09-24 spec's own rollout for the
transcript.

---

## 6. Implementation checklist

- [ ] A: `--agent-tool-panel-max-h` at `calc(50vh / 3)` and `calc(60vh / 3)`; panel reads it; shell block untouched.
- [ ] A: update the `50vh` comments listed in A.4.
- [ ] A: pixel check across window heights, both tiers, and one non-100% zoom.
- [ ] B: telemetry + CDP repro; record which B.2 cause(s) were confirmed, in this spec.
- [ ] B: coordinate with Agent2 / issue #3655 on the reducer.
- [ ] B: follow controller in `ToolOverlayLog` (shared reducer or the Phase-0-style fallback); tests 1–11.
- [ ] `task docs:index`; flip **Status** to `implemented — PR #NNNN`.
- [ ] PR body includes `<!-- agentmux:agent_id=agent4 -->`.

---

## 7. Decisions (resolved 2026-09-25, user: "use best recommendations")

1. **No floor for small windows.** The cap is exactly a third of today's. Revisit only if
   the pixel check (§4) looks cramped at ~800 px.
2. **The persistent-shell log is unchanged.** It keeps `50vh` (§2 A.3).
3. **Document-like previews open at the top.** Streamed and terminal-style output follows
   the bottom (§3 B.3).
4. **No expand button.** It was dropped with the 8-line default and is not part of this
   spec.
