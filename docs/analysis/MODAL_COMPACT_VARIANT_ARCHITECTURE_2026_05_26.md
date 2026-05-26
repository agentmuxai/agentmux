# Modal compact-variant — architecture analysis + robust redesign

**Date:** 2026-05-26
**Author:** AgentA
**Status:** Analysis — proposed redesign, not yet implemented
**Trigger:** v0.38.10 install modal still renders wide with horizontal scrollbar in narrow agent panes, despite three iterative fixes.

---

## 1. Symptom and history

The user sees the install modal (`AgentInstallModalPanel`, the xterm-hosting npm-install runner) render at its default ~600px width with a horizontal scrollbar on the modal panel, when opened inside a pane narrower than 400px (the `COMPACT_THRESHOLD_PX`).

Prior attempts (chronological, all on `main`):

| PR | Change | Outcome |
|---|---|---|
| #1039 | Introduce `.modal-layer-mount--compact` class + per-body `min-width: 0` override; install modal SCSS added a `min-height: 200px` and `min-width: 0` block | Worked for body chrome; xterm container still pinned modal wide |
| #1056 | Audit + cover four other modals (launch, prereq, new-bundle, browser-auth) that missed the compact override | Closed the audit gap; install modal already had its override, untouched |
| #1059 (this PR) | Add `min-width: 0` to `.agent-install-modal-term` inside compact — break the flex-shrink trap on the xterm wrapper | Necessary but not sufficient (see §4) |

Three patches on the same symptom over five days — pattern strongly suggests we're attacking layers of an architectural mismatch, not isolated bugs. Per the team's "3-strike rule," it's time to step back and design a robust replacement rather than add a fourth patch.

---

## 2. Today's architecture (factual)

### 2.1 Class toggle (`frontend/app/element/ModalLayer.tsx`)

```ts
const COMPACT_THRESHOLD_PX = 400; // binary threshold

// ResizeObserver on the mount node:
const compact = w > 0 && w < COMPACT_THRESHOLD_PX;
if (compact !== isCompact()) setIsCompact(compact);

// Class applied to the mount div:
class={`modal-layer-mount${isCompact() ? " modal-layer-mount--compact" : ""}`}
style="display:contents"  // layout-transparent wrapper
```

### 2.2 Modal panel hardcoded to `size="fit"` (ModalLayer.tsx:201)

```tsx
<Modal
    open={current() != null}
    scope={props.scope}
    size="fit"             // ← ALL modals through ModalLayer get content-driven width
    ...
>
```

### 2.3 `data-size` rules (`frontend/app/element/modal.scss`)

```scss
.modal-panel {
    overflow: auto;        // ← horizontal scrollbar source
    &[data-size="sm"]  { width: 360px; }
    &[data-size="md"]  { width: 520px; }
    &[data-size="lg"]  { width: 720px; }
    &[data-size="xl"]  { width: 960px; }
    &[data-size="fit"] { width: auto; }   // ← used by every ModalLayer modal
}

// Compact override (line 303):
:where(.modal-layer-mount--compact) {
    .modal-panel[data-size] {
        min-width: 0;
        width: 100%;       // ← of containing block (.modal-root)
        max-width: 100%;
    }
    .modal-panel-body { padding: var(--space-2); }
    .modal-panel-footer { flex-direction: column; ... }
}
```

### 2.4 Per-body opt-in (each modal's SCSS)

```scss
// _install-modal.scss
.agent-install-modal-body {
    min-width: 560px;
    :where(.modal-layer-mount--compact) & {
        min-width: 0;
        min-height: 200px;
        // #1059: .agent-install-modal-term { min-width: 0; }
    }
}
```

Five modals have this opt-in today: launch, prereq, new-bundle, browser-auth, install.

### 2.5 The xterm container (`_install-modal.scss`)

```scss
.agent-install-modal-term {
    flex: 1 1 auto;        // ← default min-width: auto (intrinsic content)
    min-height: 240px;
    overflow: hidden;
    .xterm { height: 100%; }
}
```

### 2.6 xterm.js boot (`AgentInstallModal.tsx`)

```ts
terminal = new Terminal({
    cursorBlink: false,
    scrollback: 5000,
    fontSize: 12,
    // No cols/rows — defaults to 80 × 24 (xterm.js library default)
    ...
});
// ...
fitAddon = new FitAddon();
terminal.loadAddon(fitAddon);
terminal.open(termRef);              // ← canvas painted at 80 × cellWidth here
const tryFit = () => fitAddon?.fit();
void fontsReady.then(tryFit);
tryFit();                            // best-effort initial fit
resizeObserver = new ResizeObserver(() => tryFit());
resizeObserver.observe(termRef);
```

---

## 3. Failure mode cross-product

Six independent failure modes coexist. Today's patches address subsets:

| # | Failure mode | What it breaks | Patched today? |
|---|---|---|---|
| 1 | **Per-body opt-in fragility** — every new modal must remember to add `:where(.modal-layer-mount--compact) & { min-width: 0 }` | Future modals will forget; existing ones discovered in audits | Mitigated reactively (#1056) — recurs |
| 2 | **Flex-shrink trap** — flex children default to `min-width: auto`, pinning parent to intrinsic content width | Body shrinks but child re-expands it | Mitigated case-by-case (#1059 install) |
| 3 | **Content-managed pixel widths** — xterm canvas, monaco editor, browser webview own their own `<canvas>` / DOM dimensions; CSS can't shrink them | xterm at default 80 cols paints a 600px-wide canvas regardless of CSS parent constraints | Not addressed |
| 4 | **`size="fit"` panel sizes to content** — modal panel width = max(content widths). If content is 600px and mount is 200px, panel wants 600px. The compact override sets `width: 100%`, but `100% of containing block` requires the containing block to bind tightly | Panel may still exceed mount width even with compact override | Partially addressed (compact `width: 100%`) — verification needed |
| 5 | **Bootstrap timing race** — xterm Terminal is created at default 80 cols → canvas painted at 600px → modal lays out around 600px content → FitAddon's first ResizeObserver tick fires, but container is already shaped to the wide content → FitAddon computes "fits as 80 cols," no shrink | First paint of every install-modal-open is wide regardless of CSS | Not addressed |
| 6 | **Binary threshold (`< 400px`)** — modal renders compact at 399px, full at 401px; near-threshold panes flicker | Visual jank during pane drag near the threshold | Acceptable per spec |
| 7 | **Wasted edge padding** — `.modal-root { padding: var(--space-6) }` reserves 24px on every side. In a 200px pane that's 48px ≈ 24% of horizontal real estate burned on whitespace before the panel even renders | Compact panel sits in the middle with large unused gutters; user expects modal to hug the pane | Not addressed |

#1059's fix unblocks #2 for the install modal, but **#3, #4 (partial), and #5 remain** — which is consistent with the user's observation that it's "still the same" in v0.38.10.

---

## 4. Why #1059 is necessary but not sufficient

`min-width: 0` on `.agent-install-modal-term`:
- ✅ Lets the flex child shrink below its intrinsic content width (defeats trap #2)
- ❌ Does NOT shrink the xterm `<canvas>` — that element has a `width="600"` HTML attribute set by xterm.js, derived from `cols × cellWidth`
- ❌ Does NOT race FitAddon ahead of the first paint — FitAddon runs after `terminal.open()`, which has already painted the canvas

What #1059 unlocks: once the parent allows shrinking, the FitAddon ResizeObserver SHOULD eventually fire and call `terminal.resize(newCols, newRows)`. But this happens on a later frame than the initial mount, and the initial-mount paint shows the wide canvas. If the modal-panel's `overflow: auto` records the overflow before the resize lands, the scrollbar persists (the browser's `overflow: auto` reservation is sticky in some Chromium versions until reflow).

So the *steady state* after a tick should be compact — but the *initial paint* and possibly the *recorded overflow* stay wide, which is what the user sees and reports as "still the same."

---

## 5. Diagnostic gap (what we don't yet know from this analysis)

Cannot confirm from source alone:
1. Is `.modal-layer-mount--compact` actually being applied when install modal opens in the narrow pane? (Verify via DOM inspect.)
2. What's the *computed* `width` and `min-width` of `.modal-panel` at runtime? (Verify via Computed-Style panel.)
3. What's the rendered pixel width of the `<canvas>` inside `.agent-install-modal-term`? (Verify via Elements panel.)
4. After FitAddon's first tick, do the numbers from (2) and (3) converge, or does one of them stay wide?

Action: use the Inspect-Element feature (shipped in 0.38.x) to right-click the modal in a narrow pane and capture these. This is the single highest-value diagnostic before committing to a fix path.

---

## 6. Robust-solution principles

For the redesign to count as "robust" (per the user's brief — "no hacks"):

1. **Structural, not opt-in.** Compact behavior must apply to every modal automatically. Adding a new modal must require zero compact-mode CSS.
2. **Width math cannot overflow the mount, ever.** Independent of content. Independent of timing.
3. **Content-managed surfaces (xterm / monaco / webview) need a coordinated shrink path** — CSS alone is necessary but not sufficient.
4. **Continuous, not binary.** A 401px modal and a 399px modal should look almost identical; a 200px modal should look proportionally different. No `< 400 → compact` cliff.
5. **Native browser primitives where possible.** Container queries, `clamp()`, `min()`, `inset` — over JS observers and class toggles.

---

## 7. Proposed redesign — four layers

The redesign decomposes into four independent layers. Each can land in a separate PR.

### Layer A — Replace class toggle with container queries

```scss
// modal.scss
.modal-layer-mount {
    container-type: inline-size;
    container-name: modal-mount;
}

@container modal-mount (max-width: 400px) {
    // Drawer-style edge clearance — the modal hugs the pane. The
    // 1px padding leaves room for the panel's 1px border (otherwise
    // it lands on the pane clip edge and sub-pixel-renders on HiDPI).
    // Failure mode #7 — reclaim 48px of horizontal real estate.
    .modal-root { padding: 1px; }

    .modal-panel-header { padding: var(--space-1) var(--space-2); }
    .modal-panel-title  { font-size: 14px; line-height: 1.3; }
    .modal-panel-body   { padding: var(--space-2); }
    .modal-panel-footer { flex-direction: column; gap: var(--space-1); }
    .modal-panel-footer .button { width: 100%; }
}

// Optional ultra-narrow drawer mode — true edge-to-edge sheet for
// very narrow panes where even a 1px border + drop-shadow looks
// out of place. Two-tier responsive behavior.
@container modal-mount (max-width: 250px) {
    .modal-root { padding: 0; }
    .modal-panel {
        border-left: none;
        border-right: none;
        box-shadow: none;  // shadow against pane edges looks broken
    }
}
```

Then **delete** in `ModalLayer.tsx`:
- `COMPACT_THRESHOLD_PX`
- `isCompact` signal
- the `ResizeObserver` on the mount node
- the `--compact` class concatenation

**Wins:** declarative, continuous, no JS, no flicker near threshold, applies to all descendants automatically. Chromium 105+ — we're CEF 146, supported.

**Caveat:** `display: contents` on the mount disables containment in some browser versions. We may need to drop `display: contents` and replace with a less-invasive layout-passthrough (e.g. `display: grid` with `grid-template: 100% / 100%`).

### Layer B — Fluid panel widths

```scss
// Replace each fixed width with a clamp:
.modal-panel {
    &[data-size="sm"]  { width: min(360px, 100%); }
    &[data-size="md"]  { width: min(520px, 100%); }
    &[data-size="lg"]  { width: min(720px, 100%); }
    &[data-size="xl"]  { width: min(960px, 100%); }
    &[data-size="fit"] { width: auto; max-width: 100%; }
}
```

The panel **structurally cannot exceed its containing block** — `min(...)` enforces it independent of content width.

For `size="fit"` specifically (which ModalLayer hardcodes), the panel's content-sizing intent stays, but `max-width: 100%` clips it.

**Wins:** Failure mode #4 (size="fit" content sizing) eliminated. Modal can never overflow its mount.

### Layer C — Universal `min-width: 0` for body descendants

```scss
.modal-panel-body,
.modal-panel-body > * {
    min-width: 0;
}
```

Eliminates the per-body opt-in (Failure mode #1) and the flex-shrink trap one level deeper (Failure mode #2). The five existing per-modal `:where(.modal-layer-mount--compact) & { min-width: 0 }` blocks become unnecessary — delete them.

**Risk:** Too broad? Mitigated by:
- Scope is `.modal-panel-body` — already-isolated DOM
- `> *` is direct children only (not all descendants)
- Modals that have *intentional* min-widths on their body children can override

**Wins:** Failure modes #1 and #2 collapse to zero ongoing maintenance.

### Layer D — Content-shrink coordination for xterm-class surfaces

The xterm canvas is JS-sized, not CSS-sized. Three options, ranked by robustness:

**D1. Deferred mount (recommended).** Don't instantiate xterm until the container has its final size:

```ts
onMount(() => {
    if (!termRef) return;
    // Wait one ResizeObserver tick before creating the terminal so
    // the container's compact-mode dimensions are already settled.
    const ro = new ResizeObserver(() => {
        if (!terminal) {
            const cs = getComputedStyle(termRef);
            const cellW = parseFloat(cs.getPropertyValue("--xterm-cell-width") || "7.2");
            const initialCols = Math.max(20, Math.floor(termRef.clientWidth / cellW));
            terminal = new Terminal({
                ...,
                cols: initialCols,
                rows: 24,
            });
            terminal.open(termRef);
            // ...
        } else {
            fitAddon?.fit();
        }
    });
    ro.observe(termRef);
});
```

xterm boots at the *correct* width on first paint. No flash, no scrollbar reservation.

**D2. Boot-then-fit-then-write.** Create xterm with `cols: 1, rows: 1`, call `fitAddon.fit()` synchronously after `terminal.open()`, then start writing:

```ts
terminal = new Terminal({ ..., cols: 1, rows: 1 });
terminal.open(termRef);
fitAddon.fit();              // synchronous resize before any writes
terminal.writeln("...");
```

Simpler, but `cols: 1` may break xterm's internal assumptions in edge cases.

**D3. CSS `aspect-ratio` clamp on canvas.** Override `<canvas>` width via CSS:

```scss
.agent-install-modal-term .xterm canvas {
    width: 100% !important;
    height: auto !important;
}
```

This is the **hack option** — fights xterm's own sizing, breaks under HiDPI, breaks selection/click coordinates. Reject.

**Recommended: D1** — fully addresses Failure modes #3 and #5.

---

## 8. Migration path

### Phase 1 — Ship a robust patch (~1 PR, low risk)

- **Layer C** (universal `min-width: 0` cascade)
- **Layer B** (`min(<size>, 100%)` panel widths)
- **Layer D1** (deferred xterm mount in `AgentInstallModal.tsx`)
- **Edge-padding fix** — ride the existing class toggle (`:where(.modal-layer-mount--compact) .modal-root { padding: 1px }`) so we don't have to wait for Phase 2's container-query migration

Outcome: install modal compacts correctly to any pane width on first paint, hugs the pane edge with a 1px hairline. Other modals get the structural fixes for free. Five per-body compact opt-in blocks become deletable (do so in the same PR).

**Estimated diff:** ~85 lines added, ~30 lines deleted.

Note: Phase 1 keeps the class-toggle infrastructure (`isCompact`, the mount ResizeObserver, the `--compact` class) — they get deleted in Phase 2 when container queries replace them. The edge-padding rule moves from `:where(.modal-layer-mount--compact)` to `@container modal-mount` at that point. Zero behavior change between Phase 1 and Phase 2 from the user's perspective; Phase 2 is pure internal cleanup.

### Phase 2 — Architectural cleanup (~1 PR, medium risk)

- **Layer A** (container queries replace class toggle)
- Delete `COMPACT_THRESHOLD_PX`, `isCompact`, the mount ResizeObserver, the `--compact` class
- Migrate the compact rule block in `modal.scss` from `:where(.modal-layer-mount--compact)` to `@container modal-mount`

**Estimated diff:** ~50 lines changed (mostly selector swaps). Net DELETE of JS observer + class wiring.

### Phase 3 — Audit other content-managed surfaces (separate, out of scope here)

- Browser pane (native webview) — likely has the same shrink issue at narrow widths
- Monaco editor — similar canvas-pixel sizing
- Establish a "shrinkable content" contract: views that own their pixel dimensions must accept a width hint via prop / signal / RPC

---

## 9. Revert option

If Phase 1 hits unexpected regressions during smoke or bot review:
- Revert to current state (post-#1059)
- Document in CLAUDE.md: *"modals containing xterm/canvas/monaco content do not reliably compact below 400px until Phase 1 of `MODAL_COMPACT_VARIANT_ARCHITECTURE_2026_05_26.md` lands"*
- Track as a known limitation rather than a bug; revisit when Phase 1 has dedicated time

The revert is fully reversible — Phase 1 doesn't change any backend, IPC, or persistence.

---

## 10. Acceptance criteria for the robust solution

A robust redesign must satisfy ALL of:

1. **No horizontal scrollbar.** Install modal opened in a 200px-wide pane → `modal-panel.scrollWidth === modal-panel.clientWidth`.
2. **No first-paint flash.** xterm renders at its compact width on the FIRST paint, not after a ResizeObserver tick. Verifiable with `requestAnimationFrame(() => measure())`.
3. **No per-modal opt-in.** Adding a hypothetical new modal type (e.g. `AgentMcpAuthModal`) requires zero changes to compact-mode SCSS — it works structurally.
4. **All five existing modals still work.** launch / prereq / new-bundle / browser-auth / install all render correctly at pane widths 200px / 350px / 500px / 800px / 1200px.
5. **Width math is monotonic.** A 350px modal is wider than a 250px modal; no `< threshold` cliff in either direction.
6. **No JS-driven class toggle in the hot path.** Mount ResizeObserver is gone; compact behavior is purely CSS (container queries).
7. **Reagent + codex both green** on the resulting PR(s) without manual override.
8. **Edge-padding reclaimed.** Compact modal panel border is within 1px of the pane edge; in `< 250px` panes the panel goes true edge-to-edge with no side border.

---

## 11. Open questions for the user before implementing

1. **Container-query browser support.** We're on CEF 146 (Chromium 146); container queries are supported since 105. Any concern? *Answer expected: no.*
2. **`display: contents` on `.modal-layer-mount`.** Does our codebase rely on this for layout pass-through anywhere we'd break by changing it? Quick grep: only used here. *Answer expected: safe to replace.*
3. **`size="fit"` semantic.** ModalLayer hardcodes it for all modals. Is that intentional, or should some modals use `size="md"` / `size="lg"`? *Answer expected: keep `fit`, with `max-width: 100%` added.*
4. **Phase ordering.** Does Phase 2 (container queries) need to wait for Phase 1, or can they ship in parallel? *Recommendation: Phase 1 first — it's the user-visible fix; Phase 2 is internal cleanup.*

---

## 12. Files this redesign would touch

Phase 1:
- `frontend/app/element/modal.scss` (Layers B + C)
- `frontend/app/view/agent/components/AgentInstallModal.tsx` (Layer D1)
- `frontend/app/view/agent/styles/_install-modal.scss` (delete per-body compact block — Layer C makes it redundant)
- `frontend/app/view/agent/styles/_launch-modal-body.scss` (same)
- `frontend/app/view/agent/components/AgentNewBundleModal.scss` (same)
- `frontend/app/view/agent/components/AgentPrereqModal.scss` (same)
- `frontend/app/view/browser/components/BrowserAuthModal.scss` (same)

Phase 2:
- `frontend/app/element/ModalLayer.tsx` (delete COMPACT_THRESHOLD_PX, isCompact, mount ResizeObserver)
- `frontend/app/element/modal.scss` (`:where(.modal-layer-mount--compact)` → `@container modal-mount`)

---

## 13. Spec updates after Phase 1 lands

- `docs/specs/SPEC_MODAL_COMPACT_VARIANT_2026_05_25.md` — update §3 to reflect Layer C + D as canonical, mark per-body opt-in section as deprecated
- Add a §5: "Content-managed surfaces and the shrinkable-content contract"

---

*End of analysis. Ready for review + go/no-go decision on Phase 1.*
