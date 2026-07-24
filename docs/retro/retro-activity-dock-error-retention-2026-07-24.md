# Retro — ActivityDock error rows persisted forever ("racking up above the composer")

**Date:** 2026-07-24
**Trigger:** User report, live: "i see the docks racking up above the model
selector bar." Reported mid-session, after a stretch of background-shell-heavy
work (several `mcp__agentmux__Shell` invocations that failed) in this same
agent pane.
**Audience:** anyone touching `ActivityDock.tsx`, `ActivityRow.tsx`, or
`activity/types.ts`, and anyone about to hand-roll a "flash to draw attention"
animation instead of checking for an existing one first.

---

## 1. What was wrong

`ActivityDock` (spec: `SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md`) pins
one row per long-running shell/subagent above the composer. Its D4 retention
policy (`activity/types.ts`'s `RETENTION_MS`) auto-clears terminal rows after a
short window — **except `error`, which was `Infinity`**, i.e. "persists until
the user manually dismisses it," by original design ("error → until
acknowledged," per the code comment).

In practice, a session that spawns several background shells that fail (this
one: two failed `task dev` attempts via the MCP Shell tool, exit codes 201 and
127; a diagnostic check, exit 1) accumulates one permanent row per failure,
each requiring a manual click on its own × dismiss button. Over a long agent
session doing real debugging work — which fails plenty, by nature — this reads
as an ever-growing, cluttered strip crowding the space right above the
composer, not a helpful error log.

## 2. The fix

- **`RETENTION_MS.error`: `Infinity` → `15_000`** (`activity/types.ts`). Long
  enough to actually read the error (well above `done`'s 8s / `stopped`'s 3s),
  short enough that a bad debugging session doesn't leave a permanent wall of
  dead rows. The manual × dismiss button still exists for clearing one early.
- **A landing/departure flash**, so a row's appearance and (now genuinely
  imminent) disappearance are both conspicuous rather than a silent DOM
  mutation the user might miss:
  - On mount (`ActivityRow.tsx`): a local `entering` signal, true for
    `EXIT_FLASH_MS` (400ms) after `onMount`.
  - Just before removal: `ActivityDock`'s `visible` memo now keeps a row
    mounted for `RETENTION_MS[status] + EXIT_FLASH_MS` instead of exactly
    `RETENTION_MS[status]`, and exposes an `isLeaving(id)` helper that's true
    only during that extra grace window. `hasExpiring`'s precise-retimer
    effect was extended the same way so it still fires exactly when needed.
  - Both apply the **same** animation: `tab-bounce` (`tabbar.scss`), the
    spring-bounce already used for a tab landing after a drag/drop. Reused
    directly by name (SCSS `@use`/component-colocated imports don't scope
    `@keyframes` — it's a plain CSS-level name, visible from any stylesheet in
    the bundle) rather than duplicating the keyframe, per the user's explicit
    ask to reuse "the same flash as the tab landing in the chrome."

Net effect: an error row now flashes in, sits for 15s (dismissible early), then
flashes out on its own. No more permanent accumulation.

## 3. A pattern worth abstracting — scoped, not built, this pass

The user asked to look into whether the flash-on-mount/flash-before-removal
pattern should become a standard, reusable primitive, separately from this
fix. Worth doing — the codebase already has **at least four independent,
structurally near-identical** "flash to draw attention" implementations, none
aware of the others:

| Where | Keyframe | Trigger mechanism | Duration |
|---|---|---|---|
| `tabbar.scss` | `tab-bounce` | signal (`bouncingTabId`) + `setTimeout` clear | 300-400ms |
| `block.scss` (terminal activity) | `term-activity-flash` | conditional class, `--flash` modifier | 500ms |
| `swarm-view.scss` | `swarm-activity-flash` | conditional class, `--flash` modifier | 500ms |
| `_search.scss` (match nav) | `search-match-flash` | plain CSS, fires on selector match | 250ms |
| `_shell-node.scss` (this fix) | reuses `tab-bounce` | local `entering` signal / parent-supplied `leaving()` | 300ms |

Each reinvents: a keyframe, a class-toggle trigger (either an imperative
signal + timeout, or a CSS-only conditional class), and its own duration/easing
choice — with no shared vocabulary for "this UI element just appeared/is about
to matter/is about to disappear, draw the eye."

**Recommendation for a follow-up (not done here):** a single `useFlash()`
hook or `<Flash>` wrapper in a shared location (candidate:
`frontend/app/hook/` alongside `useTick`) that takes a duration and an
`active: () => boolean` accessor, applies a standard flash class, and lets
call sites pick from a small set of pre-defined visual treatments (bounce,
color-pulse, invert-strobe) rather than each hand-rolling a keyframe. Should
absorb `tab-bounce` and this fix's usage first (both are the same visual,
literally), then evaluate whether `term-activity-flash`/`swarm-activity-flash`
(same color/opacity shape as each other already) and `search-match-flash` are
close enough to consolidate too, or different enough (different semantic
purpose: landing vs. draw-attention vs. match-nav) to stay separate by design.
Also worth checking the already-WCAG-2.3.1-hardened `tab-drop-invert-strobe`
(400ms steps(1,end) 2, capped at 2.5Hz per PR #2105's review finding) — any
generalized primitive needs to inherit that flash-rate ceiling by default, not
let a new call site accidentally reintroduce a seizure-risk flash rate.

## 4. What this retro is explicitly not

Not a claim that `done`'s 8s / `stopped`'s 3s retention windows are also
wrong — only `error`'s `Infinity` was the reported problem. Not a claim the
generalized flash primitive is trivial — the table above undersells the real
work (auditing every existing call site's actual visual intent before
consolidating, and getting the reduced-motion/seizure-safety behavior right
for all of them at once) — it's scoped as a separate follow-up specifically
because it's real design work, not a quick refactor to bundle into this fix.
