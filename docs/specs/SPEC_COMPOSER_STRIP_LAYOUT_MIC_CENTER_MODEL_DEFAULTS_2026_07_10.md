# SPEC: Composer-strip layout fixes, mic vertical centering, curated model defaults

**Date:** 2026-07-10
**Status:** Draft — investigated and designed, not yet implemented
**Author:** AgentA
**Trigger:** User request — *"lets have the dropdown wider, it is showing ellipsis even though there is plenty of room. it should only show ellipsis if there is no room for the control. On the same bar, move the shell button to the far right. Have the stats centered in the middle. also, the new microphone icon in the conversation input, have it vertically centered as the input height increases as more lines are added. finally, get sonnet 5, and fable 5 as the hard coded default so if the endpoint fails we have the latest .. investigate, put it all together into a spec, write to file"*

This bundles four independent, small UI fixes discovered/designed in one investigation pass. Each is scoped so it can land as its own commit/PR; there is no dependency between them except items 1–2 touching the same file (`AgentComposerStrip.tsx` / `_composer-strip.scss`).

---

## 1. Runtime dropdown shows ellipsis with room to spare

### Current behavior

`AgentRuntimeDropup`'s trigger button (`frontend/app/view/agent/components/AgentRuntimeDropup.tsx:281-295`) renders a live `"Mode · Model · Effort"` summary (e.g. `"Auto · Sonnet 5 · Medium"`) inside `.agent-runtime-dropup-trigger-label`. The trigger and label are styled in `frontend/app/view/agent/styles/_composer-strip.scss:48-79`:

```scss
.agent-runtime-dropup-trigger {
    display: inline-flex;
    ...
    max-width: 160px;   // <- fixed cap, unconditional
    ...
}

.agent-runtime-dropup-trigger-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
}
```

`max-width: 160px` is a **blanket cap** — it applies regardless of how much room `.agent-composer-strip` actually has. `.agent-composer-strip-controls` (the trigger's flex parent) is `flex-shrink: 0`, and the strip's right zone is pinned via `margin-left: auto`, so in a normal-width or wide pane there is plenty of unused space in the middle of the bar — the trigger still clips at 160px and shows `…` for no reason.

### Fix

Drop the unconditional cap; let the button size to its content (its neighbors already don't compete for the same space — `flex-shrink: 0` on both `.agent-composer-strip-controls` and `.agent-composer-strip-right` means the trigger is never squeezed in normal layouts). Keep `overflow/text-overflow/white-space` on the label as a defensive no-op most of the time, and only reintroduce a `max-width` inside the existing narrow-pane container query, where there genuinely isn't room:

```scss
.agent-runtime-dropup-trigger {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: 3px;
    // max-width removed — size to content; only constrained under real
    // space pressure (see the ≤320px container query below).
    ...
}

// _composer-strip.scss already has this breakpoint (line 236) — add the cap here instead of unconditionally:
@container modal-mount (max-width: 320px) {
    .agent-runtime-dropup-trigger { max-width: 120px; }
    .agent-composer-strip-log-btn { font-size: 9px; padding: 1px 3px; }
}
```

This satisfies "only show ellipsis if there is no room for the control" literally: unconstrained above 320px pane width, truncated only below it.

**Files:** `frontend/app/view/agent/styles/_composer-strip.scss` (lines ~48-53, ~236-243).

---

## 2. Shell button to the far right; stats centered in the bar

### Current behavior

`AgentComposerStrip.tsx:155-215` renders two zones:

- `.agent-composer-strip-controls` (left) — the runtime dropdown.
- `.agent-composer-strip-right` (`margin-left: auto`) — in DOM order: **Shell button**, stats (`rightText()`), process badge, context text. All four are clustered together at the right edge; "centered" today only means "centered *within that right-edge cluster*," not centered in the bar.

```
┌──────────────────────────────────────────────────────────────────┐
│ [Auto · Sonnet 5 · Medium ▴]      [Shell] ↑2.1k↓480 1m12s ⚙3 12k/64k │
└──────────────────────────────────────────────────────────────────┘
```

### Requested

- Shell button moves to the absolute right edge of the bar (last, not first).
- Stats (`rightText()` — tokens/elapsed) sit visually centered in the *whole bar*, not just within the right cluster.

### Fix — 3-column grid

A flex row with one auto-margin zone can't put something at the mathematical center of the bar while an unrelated-width item sits at the far right — the center point drifts with whatever's on either side. The standard fix (same pattern as a title bar: left controls / centered title / right actions) is a 3-column grid where the outer columns are symmetric (`1fr` each), guaranteeing the center column sits at true center regardless of how wide the left/right content is:

```scss
.agent-composer-strip {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    // flex-wrap/justify-content/gap rules below adapt accordingly — see
    // the narrow-width note at the end of this section.
    gap: var(--space-2);
    ...
}

.agent-composer-strip-controls {
    justify-self: start;
    // unchanged otherwise
}

// NEW middle column — just the stats text, nothing else.
.agent-composer-strip-stats-zone {
    justify-self: center;
}

// Right column now holds process badge + ctx text + Shell, in that order —
// Shell last so it's the rightmost element in the bar.
.agent-composer-strip-right {
    justify-self: end;
    display: flex;
    align-items: center;
    gap: var(--space-1-5);
    // margin-left: auto no longer needed — the grid column does the pinning.
}
```

Component change in `AgentComposerStrip.tsx` (`return` block, lines 155-215): pull `rightText()`'s `<span class="agent-composer-strip-stats">` out into its own top-level grid child, and reorder `.agent-composer-strip-right`'s children so the Shell button renders **last**:

```tsx
<div class="agent-composer-strip" ...>
    <span class="agent-composer-strip-controls">...</span>

    <Show when={rightText()}>
        <span class="agent-composer-strip-stats-zone">
            <span class="agent-composer-strip-stats">{rightText()}</span>
        </span>
    </Show>

    <span class="agent-composer-strip-right">
        <Show when={(props.processCount ?? 0) > 0}>...process badge...</Show>
        <Show when={ctxText()}>...ctx text...</Show>
        <button class="agent-composer-strip-log-btn" ...>Shell</button>
    </span>
</div>
```

When `rightText()` is empty (no turn in flight, no session totals yet), the middle grid column simply has no content and collapses to its content width (0) — the two `1fr` side columns are unaffected, so left/right stay correctly positioned either way.

### Open question — narrow-pane fallback

Today, `.agent-composer-strip { flex-wrap: wrap }` lets the right zone drop to its own row below the controls zone once both zones no longer fit on one line (the strip's own header comment describes this). A 3-column grid has no built-in equivalent of "wrap to a second row" — `1fr` columns just shrink toward zero instead of reflowing. Two options for whoever implements this:

1. **Minimal:** keep the existing container-query breakpoints (`≤240px` hides stats/ctx, `≤320px` shrinks the trigger/Shell button — already in `_composer-strip.scss`) and accept that they now do the narrow-width work the old flex-wrap did. Likely sufficient in practice since those breakpoints already strip enough content that a 3-column squeeze shouldn't look broken.
2. **Safer:** add one more breakpoint (e.g. `≤400px`) that overrides `.agent-composer-strip` back to `display: flex; flex-wrap: wrap` with the old margin-left:auto right-zone rule, preserving the exact old two-row fallback at extreme widths, with the grid layout only active above that threshold.

Recommend starting with option 1 and only adding option 2 if manual testing at very narrow pane widths shows clipping/overlap.

**Files:** `frontend/app/view/agent/components/AgentComposerStrip.tsx` (lines 155-215), `frontend/app/view/agent/styles/_composer-strip.scss` (lines 13-34, 161-172).

---

## 3. Mic icon should vertically center as the composer grows

### Current behavior

`frontend/app/view/agent/styles/_pending-footer.scss:78-88`:

```scss
.agent-input-mic {
    position: absolute;
    top: 3px;
    right: 5px;
    z-index: 1;
}
```

This is a **deliberate** existing choice, per the comment directly above it — the mic was pinned to the top-right corner specifically so it would *not* drift as the textarea grows past one line (this reverses that choice; the comment should be updated, not just the rule).

The textarea (`.agent-input`, same file, lines 102-138) auto-grows via `field-sizing: content` (no JS) between `min-height: 20px` and `max-height: 200px`, and `.agent-input-container` (line 72) is `position: relative` — the containing block the mic's `position: absolute` resolves against, and it grows in lockstep with the textarea.

### Fix

Track the container's vertical center instead of pinning to a fixed offset from the top — pure CSS, consistent with the codebase's existing zero-JS auto-grow approach (`docs/analysis/agent-typing-lag-trace-2026-04-12.md`, referenced at line 121):

```scss
// Pinned mic — right edge of the composer, vertically centered against the
// input's current (growable) height so it tracks the middle of the box as
// more lines are typed, rather than staying fixed to the top-right corner.
.agent-input-mic {
    position: absolute;
    top: 50%;
    right: 5px;
    transform: translateY(-50%);
    z-index: 1;
}
```

At the single-line resting height (`min-height: 20px`) this lands within a couple pixels of the old `top: 3px` pin, so the common case looks effectively unchanged; the visible difference only appears once the box grows past one line, which is exactly the behavior requested.

No change needed to the `:has(.mic-button-wrap) .agent-input { padding-right: 22px }` rule (line 98-100) — that reserves horizontal space only, unaffected by vertical positioning.

**Files:** `frontend/app/view/agent/styles/_pending-footer.scss` (lines 78-88, plus the comment above it).

---

## 4. Hardcode Sonnet 5 / Fable 5 as curated defaults (endpoint-failure fallback)

### Why this matters right now

Confirmed live in this session: the srv log shows `model catalog: HTTP 401 Unauthorized from /v1/models` (`agentmux-srv/src/backend/model_catalog.rs:183-187` treats any non-2xx, including an expired OAuth token, as "fall back to the bundled curated catalog"). When that fetch fails, the dropdown shows whatever is hardcoded in the static list — today that's stale.

### Current curated list

`frontend/app/view/agent/providers/index.ts:198-202`:

```ts
models: [
    { value: "opus", label: "Opus 4.8", description: "Claude Opus 4.8 — highest quality", aliases: ["claude-opus"] },
    { value: "sonnet", label: "Sonnet 4.6", default: true, description: "Claude Sonnet 4.6 — balanced", aliases: ["claude-sonnet"] },
    { value: "haiku", label: "Haiku 4.5", description: "Claude Haiku 4.5 — fastest", aliases: ["claude-haiku"] },
],
```

`value: "sonnet"` is the CLI's own generic alias (always resolves to whatever Sonnet is current), so `--model sonnet` already invokes Sonnet 5 today regardless of this file — only the **label** ("Sonnet 4.6") is stale. Fable has no curated entry at all, so it's invisible whenever the API overlay hasn't (yet, or ever) succeeded.

### Fix

```ts
models: [
    { value: "opus", label: "Opus 4.8", description: "Claude Opus 4.8 — highest quality", aliases: ["claude-opus"] },
    { value: "sonnet", label: "Sonnet 5", default: true, description: "Claude Sonnet 5 — balanced", aliases: ["claude-sonnet"] },
    { value: "haiku", label: "Haiku 4.5", description: "Claude Haiku 4.5 — fastest", aliases: ["claude-haiku"] },
    { value: "claude-fable-5", label: "Fable 5", description: "Claude Fable 5" },
],
```

- Sonnet's `value` stays the generic alias `"sonnet"` (unchanged behavior for `--model`); only the label/description text is bumped.
- Fable has no documented generic alias (unlike opus/sonnet/haiku), so its `value` is the concrete model id `claude-fable-5` — confirmed as the right id via existing precedent already in the codebase: `context-window.ts:38` and its test (`context-window.test.ts:15`) already special-case `"claude-fable-5"` for the 1M context-window band. **Not fabricated** here: no `aliases` entry is added, since there's no confirmed short alias for Fable in the CLI — verify against the installed Claude Code CLI's actual accepted `--model` values before shipping if one exists (would let the entry read as a proper alias like the other three, but isn't required for correctness).

### Follow-on correctness issue this surfaces

`setProviderModels()` (`providers/index.ts:569-601`) is what turns "Sonnet 4.6" into "Sonnet 5" automatically once the live API succeeds, by substring-matching each curated entry's `value` against API-returned ids:

```ts
const family = m.value.toLowerCase();                                    // e.g. "sonnet"
const matches = apiModels.filter((a) => a.value.toLowerCase().includes(family));
```

This works indefinitely for opus/sonnet/haiku because their `value` is a short, version-agnostic word — `"claude-sonnet-6".includes("sonnet")` will still match a future version. It does **not** work the same way for the new Fable entry, because its `value` is the *concrete, version-pinned* id `"claude-fable-5"` — a future `"claude-fable-6"` would **not** contain that exact substring, so:

1. The curated Fable row would never auto-refresh to "Fable 6" (stays stuck on "Fable 5" even once the API is reachable and reports a newer version).
2. Worse, since the new API model wouldn't be `consumed` by step 1, it falls through to step 2 (`byFamily`, grouped by `familyKey()`) and gets **appended as a second, separate "Fable 6" entry** — a visible duplicate in the dropdown.

**Recommended accompanying fix:** change step 1's matching from raw substring (`m.value.toLowerCase()`) to `familyKey()` equality (the same helper step 2 already uses, `providers/index.ts:545-548`):

```ts
const curated = base.models.map((m) => {
    const family = familyKey(m.value);   // was: m.value.toLowerCase()
    const matches = apiModels.filter((a) => familyKey(a.value) === family);
    ...
});
```

`familyKey("sonnet")` → `"sonnet"` (alphabetic tokens, no `-` to split, nothing to strip) and `familyKey("claude-fable-5")` → `"fable"` — both still match their respective families exactly as before for opus/sonnet/haiku, but now Fable (and any future curated entry pinned to a concrete id) auto-refreshes across version bumps the same way, with no duplicate-entry risk. This is a small, low-risk change confined to `setProviderModels()`; recommend shipping it in the same PR as the curated-list addition so the new Fable entry doesn't reintroduce the exact "stale label" bug this whole feature exists to fix.

**Files:** `frontend/app/view/agent/providers/index.ts` (lines 198-202, and optionally 578-584 for the `familyKey()` fix).

---

## Summary of files touched

```
frontend/app/view/agent/styles/_composer-strip.scss             # 1, 2
frontend/app/view/agent/components/AgentComposerStrip.tsx       # 2
frontend/app/view/agent/styles/_pending-footer.scss              # 3
frontend/app/view/agent/providers/index.ts                       # 4
```

No backend/Rust changes — `agentmux-srv/src/backend/model_catalog.rs` has no curated fallback of its own; the only hardcoded list lives in the frontend file above.

## Verification plan (once implemented)

1. `npx tsc --noEmit` — no type errors from the `AgentComposerStrip.tsx` JSX restructure.
2. `task dev` — visually confirm: (a) the runtime trigger shows its full summary un-truncated in a normal-width pane and only ellipsizes below ~320px; (b) Shell sits at the far right with stats visibly centered in the bar at a few different pane widths; (c) the mic icon visibly tracks the vertical center as the composer grows past one line; (d) with the OAuth token still expired (current live state), the `/model` picker shows "Sonnet 5" and "Fable 5" rather than stale/missing entries.
3. Re-run once Claude Code auth is refreshed to confirm the live API overlay still refreshes labels correctly on top of the new curated defaults (no duplicate Fable entries).

---

*End of spec. Not yet implemented — ready for review/go-ahead per item.*
