# SPEC: My Agents row delete — exit animation + reflow

**Status:** proposed — not implemented. §5.1 (library vs. hand-rolled FLIP)
resolved 2026-09-16, repo-owner-confirmed direct Q&A: "use the most
robust-performance solution, engineering cost isn't important" — see §4.3
and §5.1.
**Date:** 2026-09-16
**Author:** Manoz
**Related:** `docs/specs/SPEC_AGENT_DELETE_2026_09_16.md` (the delete feature
this animates — backend + wiring already shipped, verified clean end-to-end
against a live instance the same day this spec was written, see §0),
`frontend/app/view/agent/components/MyAgentsList.tsx` (`rows`/`createResource`
at line 336, the `"agents:changed"` subscription at 406-407, `sortedRows` at
775-778, the `<For>` render at 837, `confirmDelete` at ~643-710),
`frontend/app/view/agent/styles/_recent-sessions.scss` (`.agent-recent-sessions-list`
at 89-103, `.agent-recent-sessions-row` at 105-112), `frontend/app/theme.scss`
(`--motion-spring` at 417), `frontend/app/view/drone/drone-model.ts` (91-97,
the `reconcile`-keyed-by-id precedent this spec's fix borrows), `frontend/app/store/global.ts`
(`prefersReducedMotionAtom`, 83), `frontend/app/app.scss` (295-311, the
documented `transition-*`-only gap in the app-wide reduced-motion class and
the two-layer JS-gate + CSS-backstop fix already used for view transitions).

---

## 0. Origin

Manoz pulled `origin/main` (v0.56.2) and verified the newly-shipped delete
feature (`SPEC_AGENT_DELETE_2026_09_16.md`) end-to-end by introspecting the
databases and shared registry directly — before/after row counts across all
twelve dependent tables (all zero either way, since the test agent, "My
Claude Agent," turned out to be a cross-channel-only record with no local
`db_agents` row — see §5.1a of that spec), the global definition file moving
`shared/agents/definitions/` → `.../retired/` (tombstoned), and the registry
instance file being fully removed rather than retired. Confirmed clean, no
errors in the srv log.

The repo owner then deleted a real row through the actual UI and reported the
result was **too fast to perceive** — "the change is so fast, I can't tell"
— and asked for a "rubber poof" disappear animation on the deleted card,
followed by a "rubbery shift" of the surviving cards into place. This spec
covers that animation only; the delete mechanism itself is unchanged.

## 1. What exists today

| Concept | Mechanism | Notes |
|---|---|---|
| **Row list container** | `.agent-recent-sessions-list` (`_recent-sessions.scss:89-103`) | **A CSS Grid**, not a single-column list: `display: grid; grid-template-columns: repeat(auto-fill, minmax(min(var(--agent-tile-min, 240px), 100%), 1fr));`. Cards flow left-to-right, wrapping into rows. This matters for §4.2 — "shift" here means cards reflowing across a multi-column tile grid, not sliding up a single column. |
| **Row rendering** | `<For each={sortedRows()}>` (`MyAgentsList.tsx:837`), fed by `sortedRows` (775) ← `filteredRows` (758) ← `rows()`, a `createResource<RecentSessionRow[], string>` (336). | `<For>` in SolidJS reconciles **by item reference**, not by a value key. |
| **List refresh on delete** | `waveEventSubscribe({ eventType: "agents:changed", ... })` (406-407) triggers the resource's `refetch`. `confirmDelete` (~643-710) awaits `DeleteAgentDefinitionCommand`, then does its own pane sweep — it does **not** touch `rows` directly; the row's disappearance today is entirely a side effect of the resource wholesale-refetching and returning an array that no longer contains it. | See §4.1 — this is the actual blocker for any exit animation, independent of which animation technique is chosen. |
| **Existing per-row transition** | `.agent-recent-sessions-row { transition: filter 0.15s ease, opacity 0.15s ease; }` (`_recent-sessions.scss:111`) | Already used for the spotlight-blur effect when a sibling row is expanded (127-131) — proves `filter`/`opacity` transitions on this exact element already work and read fine visually; nothing currently transitions row *removal*. |
| **Motion tokens** | `--motion-fast: 100ms`, `--motion-base: 160ms`, `--motion-slow: 280ms`, all `cubic-bezier(0.2, 1, 0.3, 1)` (ease-out, no overshoot) — and `--motion-spring: 400ms cubic-bezier(0.2, 0.9, 0.2, 1.2)` (`theme.scss:414-417`). | Confirmed by direct grep: **`--motion-spring` is defined but not referenced anywhere in the frontend today.** It is the only one of the four tokens with overshoot in its curve (the `1.2` at the end passes 100% before settling) — i.e. it already *is* "rubbery." This animation would be its first real use, not a new token. |
| **Reduced motion** | Two independent mechanisms: (1) an app-wide `.prefers-reduced-motion` ancestor class (`app.scss:193-205`) that zeroes `transition-*` properties `!important` on every descendant; (2) `atoms.prefersReducedMotionAtom` (`store/global.ts:83`), consumed directly by individual components via an `"is-reduced-motion"` classList entry (e.g. `agent-view.tsx:306`, `block.tsx:452`). | **Documented gap, already hit once:** `app.scss:295-311`'s own comment explains that (1) only zeroes `transition-*`, not `animation-*` — a reagent P1 finding on PR #3239 for `startViewTransition`'s pseudo-elements, fixed with a JS-side gate (skip calling the animation API at all) plus a narrow `@media (prefers-reduced-motion: reduce)` CSS backstop. **A `@keyframes`-driven "poof" hits the identical gap** — the ancestor class alone will not suppress it. §4.4 applies the same two-layer fix here. |
| **Prior art for the reconciliation problem this spec runs into** | `drone-model.ts:91-97`: `setDraft()` calls `this.setDraftStore(reconcile(next))` from `solid-js/store`, keyed on node/edge `id`, specifically so "surviving DOM nodes and edges are diffed in place rather than torn down + rebuilt" when a fresh, structurally-new object graph replaces the old one. | **Exactly this spec's problem, already solved once in this codebase.** `solid-js/store` is not a new dependency — it ships inside the already-installed `solid-js` package (`package.json:116`) and is already imported in five other files (`mcp-capabilities.ts`, `toolchain-capabilities.ts`, `launch-flow-store.ts`, `drone-model.ts`). |
| **This codebase's own `<For>`/reconciler crash history** | `AgentDocumentVirtualList.tsx:1103-1121`, `stream-flush-queue.ts:202-211`, `DiffViewer.tsx:307`, `MarkdownBlock.tsx:137` all carry comments about `reconcileArrays`/`replaceChild` crashes from `<For>`'s reference-identity semantics being violated under streaming updates. | Not this spec's failure mode directly (nothing here streams), but it's the reason to treat `<For>`'s reference semantics as a real, previously-costly subtlety in this codebase rather than a theoretical concern — see §4.1. |

## 2. Goals

1. When a delete is confirmed, the deleted card visibly **"poofs"** —
   a distinct, brief exit animation — instead of vanishing on the next
   render tick.
2. The surviving cards **visibly glide** into the vacated grid position
   (the "rubbery shift") rather than snapping there instantly.
3. Both motions read as part of the same "rubbery" language — reuse
   `--motion-spring`'s overshoot curve for both, rather than inventing a
   second easing curve.
4. Respect reduced-motion (§4.4) — this is a delete confirmation, not
   incidental chrome; a user who has asked for reduced motion should still
   get an immediate, legible "it's gone," just without the flourish.
5. Stay within `MyAgentsList.tsx` / `_recent-sessions.scss` — no new
   component, no new modal, no change to `confirmDelete`'s RPC sequencing
   or the pane-sweep logic (`SPEC_AGENT_DELETE_2026_09_16.md` §5.2), both of
   which are unrelated to how the row *looks* while it leaves.

## 3. Non-goals

- **An entrance animation for newly-created/duplicated cards.** Not asked
  for; out of scope. (`--motion-spring` being unused elsewhere means there's
  room to reuse it there later, but that's a separate ask.)
- **Fixing the cross-window pane-sweep gap** (`SPEC_AGENT_DELETE_2026_09_16.md`
  §5.2's "does NOT reach another window" limitation). Unrelated to how the
  row animates in the window where the delete was actually triggered.
- **The Templates grid** (`AgentCard.tsx`) — this spec is scoped to
  `MyAgentsList.tsx`'s "My Agents" rows only, same scoping as the delete
  feature itself (`SPEC_AGENT_DELETE_2026_09_16.md` §3).
- **Changing `--motion-spring`'s value.** It's used as-is; if the timing
  feels wrong once built, that's a tuning pass on the existing token, not a
  reason to fork a new one for this one call site.

## 4. Design

### 4.1 The real blocker: row identity across a wholesale refetch

`confirmDelete` doesn't remove the row from `rows()` itself — it relies on
the backend's `"agents:changed"` broadcast to trigger the resource's
`refetch`, which re-fetches and replaces the **entire** array with freshly
RPC-deserialized objects. `<For>` reconciles by item reference
(`MyAgentsList.tsx:837`), and a brand-new array of brand-new objects gives
`<For>` no way to tell "24 unchanged rows + 1 removed" from "25 rows
replaced by 24 different ones" — in practice this means **every** card is
liable to unmount and remount on every delete-triggered refetch today, not
just the deleted one, which is a second, independent reason the current
delete reads as an instant flicker rather than one card leaving. Any
per-row exit animation added naively (e.g. just adding a CSS class to the
row markup) would be fighting this: by the time the animation's `class`
prop would apply, the node it was meant to apply to may already have been
torn down and a new one built in its place with no class on it.

**Fix, mirroring the existing `drone-model.ts:91-97` idiom exactly:** wrap
the resource's setter (or the `rows` signal, however `createResource`'s
mutate is threaded through this file) with
`reconcile(data, { key: "definition_id" })` from `solid-js/store` on
refetch. This makes the diff **value-keyed** instead of reference-keyed —
unrelated rows keep their existing object identity (and thus their DOM
nodes) across a refetch, and the deleted row is the only one `<For>` sees
as removed. This is a prerequisite for §4.2/§4.3 regardless of which exit
technique is chosen — without it, there is no stable DOM node to animate
in the first place.

**Optimistic local removal, matching the "optimistic collapse" philosophy
already used elsewhere in this file** (`SPEC_AGENT_DELETE_2026_09_16.md`
§4.2, for the menu closing immediately rather than waiting on an async
result): don't wait for the `"agents:changed"` round-trip to *start* the
exit animation. On `DeleteAgentDefinitionCommand` resolving successfully in
`confirmDelete`, mark that one `definition_id` as "leaving" in a local
signal immediately (a `Set<string>` or similar, separate from `rows()`) —
this is what actually drives the animation class in §4.2. Whether that same
signal then also *filters* the row out of `sortedRows()` once the animation
finishes (rather than waiting for the real refetch, which may land at an
arbitrary point during or after the animation) is an implementation
decision, not a design one — either is fine as long as the row is never
visible again once the animation completes and never double-removed if the
refetch lands mid-animation.

### 4.2 The poof

A new CSS class (e.g. `.agent-recent-sessions-row.is-leaving`), driven by
the local "leaving" signal from §4.1, applied to the existing
`.agent-recent-sessions-row` element — no new wrapper needed, the row
already has `position: relative` (`_recent-sessions.scss:110`) and an
existing `transition: filter …, opacity …` (111) to build on.

```scss
.agent-recent-sessions-row.is-leaving {
    pointer-events: none; // matches the existing blurred-sibling rule (130)
    animation: agent-row-poof var(--motion-spring) forwards;
}

@keyframes agent-row-poof {
    0%   { transform: scale(1);    opacity: 1;    filter: blur(0); }
    35%  { transform: scale(1.06); opacity: 1;    filter: blur(0); } // the "rubber" overshoot before collapsing
    100% { transform: scale(0.4);  opacity: 0;     filter: blur(3px); }
}
```

The 35%-overshoot-then-collapse shape mirrors `--motion-spring`'s own curve
(overshoots past 100% before settling) rather than fighting it with a
monotonic keyframe — and reuses `filter: blur()` on this exact element,
which is already established as this row's own "going away" visual
language via the spotlight-blur rule (`_recent-sessions.scss:127-131`), so
the poof reads as a faster, more emphatic version of a motion this row
already makes, not a foreign effect.

`animation`, not `transition` — deliberate: this is a one-shot multi-keyframe
motion (scale up, THEN down), which `transition` cannot express as a single
declaration the way `animation` + `@keyframes` can. This is also why §4.4's
reduced-motion handling needs its own explicit case (transition-only
suppression doesn't touch it — see §1's table entry on the app.scss gap).

### 4.3 The shift

**This needs more than a CSS `transition` on the surviving rows.** Because
`.agent-recent-sessions-list` is a CSS grid with `auto-fill` columns
(§1), removing a card doesn't just change one row's height the way it
would in a single-column flex list — every card *after* the removed one in
document order can shift both column and row position (up-and-left, in the
common case). CSS does not animate grid-position/placement changes the way
it animates `top`/`left`/`transform`; a plain `transition: transform …` on
`.agent-recent-sessions-row` does nothing for a card whose *grid cell*
changed, because nothing about that card's own CSS properties changed
continuously — its computed position just snapped to a new cell on the
next layout.

Getting a genuinely smooth "glide into the new cell" therefore needs a
FLIP-style approach (First–Last–Invert–Play: measure each surviving row's
`getBoundingClientRect()` before the DOM update, let the removal/reflow
happen, measure again, then apply an inverse `transform` and transition it
back to identity so the browser paints a smooth move instead of a snap).

**Decision (2026-09-16, repo-owner-confirmed): add `solid-transition-group`**
rather than hand-roll the FLIP measurement. Resolved in favor of the
library on robustness grounds, not just lower engineering cost (the repo
owner explicitly said cost wasn't the deciding factor) — a hand-rolled
`getBoundingClientRect()`/RAF recipe is exactly the kind of timing-sensitive
DOM-measurement-vs-reconciliation code this codebase's own crash history
(§1's `<For>`/`reconcileArrays` row) shows is easy to get subtly wrong under
real-world timing, and a purpose-built, widely-used library gets that
measurement/RAF sequencing right once instead of once per call site that
needs it. `<TransitionGroup>` wraps the existing `<For>` directly, computes
the FLIP transform automatically via its `move-class` on every surviving
row, and handles the exit animation (§4.2's poof) through the same
component via `exit-active`-class hooks — no separate exit-class wiring
needed alongside it.

This still depends on §4.1's `reconcile`-by-key fix — a FLIP measurement,
library-driven or not, is meaningless if `<For>` might tear down and rebuild
an unrelated row's DOM node at the same moment.

### 4.4 Reduced motion

Two-layer fix, mirroring the existing `startViewTransition` precedent
(`app.scss:295-311`) exactly, since this spec hits the identical
`transition-*`-vs-`animation-*` gap in the app-wide `.prefers-reduced-motion`
class:

1. **JS gate**: read `atoms.prefersReducedMotionAtom()` where the "leaving"
   signal is set (§4.1) and where §4.3's shift is triggered. When true,
   skip straight to removing the row from `sortedRows()` (or wherever §4.1
   lands the actual filter) with no animation class applied and no FLIP
   measurement taken — the row simply isn't there on the next render,
   which is the existing (pre-this-spec) behavior and is itself already an
   acceptable reduced-motion experience.
2. **CSS backstop**, for any path that reaches the class regardless (mirrors
   `app.scss:306-311`'s own comment on why the JS gate alone isn't treated
   as sufficient):
   ```scss
   @media (prefers-reduced-motion: reduce) {
       .agent-recent-sessions-row.is-leaving {
           animation-duration: 1ms;
       }
   }
   ```

## 5. Open questions for the repo owner

1. ~~`solid-transition-group` vs. hand-rolled FLIP for the shift (§4.3).~~
   **Resolved 2026-09-16** (direct Q&A): use `solid-transition-group` —
   repo owner explicitly prioritized robustness over engineering/dependency
   cost. See §4.3 for the reasoning.
2. **Exact poof keyframe shape (§4.2)** — the 0%/35%/100% numbers above are
   a first draft matching `--motion-spring`'s own overshoot, not a
   pixel-tuned final. Worth a quick visual pass once built before calling
   it done.
3. **Does the shift apply to EVERY surviving card, or only the ones whose
   position actually changed** (i.e. cards before the deleted one in the
   same row never move)? Functionally the same either way if the FLIP
   delta for an unmoved card is `(0, 0)` (no visible difference, negligible
   cost) — flagging only in case the repo owner wants the effect visually
   contained to "the row the deletion happened in" rather than a
   full-grid reflow when the list is long.
