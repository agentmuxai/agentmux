# SPEC: Working-Indicator Shimmer + Mic Button Relocation

**Date:** 2026-07-08
**Status:** Draft — analysis only, no code changes made
**Scope:** Frontend only (SolidJS + SCSS). Two independent changes bundled because both touch the Agent pane's conversation chrome:

1. Give the "Working…" indicator a two-phase animation (type-out, then a back-and-forth shimmer sweep), confirm it's theme-bound.
2. Move the microphone button from the pane's top-right title bar to pinned beside the conversation composer.

Also answers a standing question about the light/white theme's status, since it came up while discussing the indicator's color.

---

## 1. "Working" indicator — current state

Component: `AgentWorkingRow` in `frontend/app/view/agent/components/AgentFooter.tsx:69-171`, rendered directly below `AgentDocumentView` in the conversation area. Structure:

```
<span class="agent-spinner-dot" />
<span class="agent-working-row-left">{phrase()}…</span>
```

CSS: `frontend/app/view/agent/styles/_control-bar.scss`:
- `.agent-working-row--loading` (~line 171-173): `color: var(--accent-color); background: color-mix(in srgb, var(--accent-color) 4%, transparent);`
- `.agent-spinner-dot` (~line 241-251): `background: var(--accent-color); box-shadow: 0 0 6px color-mix(in srgb, var(--accent-color) 60%, transparent);`

**Correction to the initial premise:** this is *already* a CSS custom property, not a hardcoded literal. `--accent-color` is defined per-theme — `frontend/app/theme.scss:49` sets the default to `rgb(65, 159, 224)` (blue), and every theme under `frontend/app/themes/` (`catppuccin.scss`, `dracula.scss`, `monokai.scss`, `nord.scss`, `tokyo-night.scss`, `gruvbox.scss`, `midnight.scss`, `high-contrast.scss`) overrides it to its own hue. So switching `window:theme` already recolors the Working indicator today — it *reads* as "always blue" only because the default theme's accent happens to be blue and most sessions run on default. No binding work needed here; flag as a live QA item (cycle through each theme, confirm the indicator visibly changes) rather than a code fix.

There's already a related animated element to build on: `.agent-pane-progress-bar` (`frontend/app/view/agent/agent-view.tsx:873-887`), a 2px gradient bar pinned to the pane top, active under the identical `status.isLoading() || workingFromPhase(...)` condition as the Working row. Its CSS (`_control-bar.scss`, ~line 97-140+) derives an "aurora" sweep via `color-mix()` and CSS relative-color syntax (`hsl(from var(--accent-color) calc(h + 180) ...)`) for a complementary-hue animation — the established house pattern for "animated, theme-adaptive, off of `--accent-color`." The shimmer effect below reuses this pattern rather than inventing a new one.

## 2. Two-phase "Anthropic-style" text animation

This describes the well-known "AI is thinking" text treatment: the label reveals character-by-character on first appearance, then — once fully revealed — a soft highlight band sweeps back and forth across it for as long as the turn is in progress (not a single one-shot pass; an oscillating loop, like a spotlight passing over embossed text).

### Phase A — type-out (once per phrase change)

`phrase()` already exists as a signal (`AgentFooter.tsx`, feeding the current `{phrase()}…` text — phrases rotate, e.g. "Working" → "Thinking" → tool-specific labels during a long turn per `SPEC_AGENT_STATUS_LABELS_2026_06_27.md`). On each phrase change:
- Reveal the string incrementally, e.g. via a small `createEffect` driving a `revealCount` signal on an interval (~25–35ms/char, tunable), slicing `phrase()` client-side — no new dependency needed, this is the same shape as any JS typewriter effect.
- Respect `prefers-reduced-motion`: skip straight to fully-revealed state.
- On unmount / phrase change mid-reveal, cancel the interval (standard `onCleanup`).

### Phase B — shimmer sweep (continuous, while `isLoading`)

Once Phase A completes, apply a `background-clip: text` gradient shimmer:

```scss
.agent-working-row-left.shimmer {
    background: linear-gradient(
        90deg,
        var(--secondary-text-color) 0%,
        var(--secondary-text-color) 40%,
        var(--accent-color) 50%,
        var(--secondary-text-color) 60%,
        var(--secondary-text-color) 100%
    );
    background-size: 220% 100%;
    background-clip: text;
    -webkit-background-clip: text;
    color: transparent;
    animation: agent-working-shimmer-sweep 2.4s ease-in-out infinite alternate;
}

@keyframes agent-working-shimmer-sweep {
    0%   { background-position: 200% 0; }
    100% { background-position: -20% 0; }
}
```

- `animation-direction: alternate` (folded into the shorthand above) is what produces the "back and forth" motion the user described, rather than a one-directional loop-and-snap.
- Base tone `--secondary-text-color` with `--accent-color` as the moving highlight keeps it theme-bound the same way the progress bar is; no new tokens needed.
- No `background-clip: text` usage exists anywhere else in the frontend yet (confirmed via search) — this would be the first, so it's worth a quick cross-platform (Windows/macOS/Linux CEF) render check before shipping, since `-webkit-background-clip: text` support is generally solid in Chromium/CEF but has occasional subpixel-AA quirks worth eyeballing.
- Gate the shimmer class on the same `status.isLoading()` condition the progress bar already uses, so both stop in lockstep when the turn ends.

### Open question for implementation (not resolved by this spec)

Whether the shimmer should restart from Phase A on *every* phrase change (e.g. "Working" → "Reading file…" re-types each time) or only on the very first phrase of a turn (subsequent phrase swaps just cross-fade under a continuously-running shimmer). The former matches "it types it out the first display" most literally per-phrase; the latter reads calmer during long multi-tool turns. Recommend the former for v1 (simpler, matches the literal request) with a note that it's revisitable.

## 3. Microphone button relocation

### Current state

The mic button is **not** Agent-pane-specific code — it's the shared `MicButton` component (`frontend/app/element/MicButton.tsx`), rendered from the generic frame chrome (`frontend/app/block/blockframe.tsx:255-267`) for *any* pane whose view model exposes `voiceHandle`:

- Conditional on `props.viewModel?.voiceHandle` (`blockframe.tsx:255`), placed in the pane's title-bar icon row (`.block-frame-end-icons`, `frontend/app/block/block.scss:272-306` — a `display:flex` group, `justify-content: space-between`; no `position:absolute`, it's just flex order).
- **Confirmed exactly two view models implement `voiceHandle`** — `frontend/app/view/term/termViewModel.ts:78,301` (Terminal) and `frontend/app/view/agent/agent-model.ts:43,49` (Agent). No other view model in the codebase defines it, so the mic button today appears **only** on Terminal and Agent panes — not "any pane" as this spec first stated. Every other pane type never shows a `MicButton` at all, so there's nothing to touch there.
- The Agent-specific tooltip ("Speak into this agent (Ctrl+Shift+V)") is chosen inline via `props.blockView === "agent"` (`blockframe.tsx:262-263`), so the component already has an agent-aware branch to hook into.

**Correction to the initial premise:** no other pane, modal, or composer in the codebase currently renders a mic button pinned beside a text input — `MicButton` has exactly one call site (`blockframe.tsx:256`). If "the standard elsewhere" refers to a specific screen, worth double-checking with whoever raised it before implementation, since it doesn't match what's on disk today. This spec proceeds treating the Agent pane as the first instance of the pattern rather than a port of an existing one.

**Confirmed scope (per follow-up):** only the **Agent pane's** mic moves. The **Terminal pane's** header mic stays exactly where it is today — the `blockframe.tsx` suppression added below must key specifically off `blockView === "agent"`, not remove/hide the shared header mic broadly.

### Target state

Agent pane's composer: `frontend/app/view/agent/components/AgentFooter.tsx:721-722`:

```
<div class="agent-footer">
    <div class="agent-input-container">   <!-- position: relative already, _pending-footer.scss:72-76 -->
        <textarea class="agent-input" ... />
    </div>
</div>
```

`.agent-input-container` is already `position: relative` and currently hosts no buttons at all (Enter submits — no send button, no icon row today), so a mic icon can sit here cleanly:

- Add a new `MicButton` render inside `.agent-input-container`, absolutely positioned to the input's right edge (`position: absolute; right: var(--space-1-5); bottom: ...` — vertically centered or bottom-aligned to match the composer's typical single-line resting height; needs a look at actual `.agent-input` line-height/padding to get this pixel-right, not fully specced here).
- Reserve right-padding on `.agent-input` (e.g. `padding-right: 28px`) so typed/pasted text doesn't run underneath the icon once the textarea grows past one line.
- In `blockframe.tsx`, suppress the header-row `MicButton` specifically when `props.blockView === "agent"` (the branch already exists at line 262-263 for tooltip text — extend it to also gate rendering: `<Show when={props.viewModel?.voiceHandle && props.blockView !== "agent"}>`). The Terminal pane's header mic is explicitly **left untouched** — it's the only other pane with `voiceHandle` at all, and this change must not affect it. This is a shared-component change (both panes route through the same `blockframe.tsx`/`MicButton`), so the gate needs to be scoped precisely to `blockView === "agent"`, not to "has voiceHandle" broadly.

### Why the move is safe (behavior-wise)

- `AgentFooter` already owns the voice wiring independent of where the button renders: it registers a `PaneVoiceHandle` on `AgentViewModel.voiceTargetRef` (`AgentFooter.tsx:489-542`, tied into `agent-model.ts:41-51`) so transcripts land in this textarea regardless of the button's DOM position. `MicButton` takes `blockId` + `handle` props and has no positioning assumptions itself.
- The global `Ctrl+Shift+V` shortcut (`frontend/app/store/keymodel.ts:555-581`) toggles voice independent of button position — no change needed, but its click semantics must stay in sync with `MicButton.tsx:46-63` if that logic is ever touched.
- No z-index conflict expected: the composer's `.slash-autocomplete` dropdown opens *above* the input (`_slash.scss:84-88`, `bottom: calc(100% + 2px)`), so a mic pinned at the input's right edge doesn't compete with it.
- `MicButton`'s internal `<Show>` (line 73) hides it entirely when voice is unavailable — the composer layout must not reserve fixed space assuming the icon is always present (avoid hard-coding a permanent gap that looks empty when voice is off).

---

## 4. Status of the light/white theme (context, not an action item here)

No light/white theme exists in the app today — `frontend/app/themes/index.scss` lists 8 themes (midnight, high-contrast, monokai, nord, dracula, catppuccin, tokyo-night, gruvbox), all dark-background. `high-contrast` is a black-background/white-text variant, not a light-background theme.

A research doc already scopes this: `docs/analysis/ANALYSIS_THEME_SYSTEM_LIGHT_THEME_AND_DEPTH_GAPS_2026_07_07.md` (dated 2026-07-07). Its conclusion: the token infrastructure is solid enough that a light theme is "mechanically straightforward" to add, gated behind fixing a handful of hardcoded-dark "nooks" first (phantom tokens, the hand-painted context menu, the pane title bar). It lays out a 4-step order; only step 1 (terminal picking up the app theme, `feat(term): terminal picks up the app theme by default`, #2010, merged 2026-07-07) has shipped since. The light theme itself and its remaining prerequisites are unbuilt — this is an active-but-early research trail, not abandoned or reverted work. Out of scope for this spec; flagging so it isn't re-litigated as a surprise gap.

---

## 5. Summary of what actually needs to change

| Item | Change needed? | Where |
|---|---|---|
| Working indicator theme-binding | **No** — already bound via `--accent-color` | n/a |
| Working indicator type-out + shimmer | **Yes** — new two-phase animation | `AgentFooter.tsx` (reveal signal), `_control-bar.scss` (shimmer keyframes) |
| Mic button relocation | **Yes** — move render site, suppress in shared header for agent panes only | `blockframe.tsx` (suppress), `AgentFooter.tsx` + its styles (add) |
| White/light theme | **No action** — pre-existing scoped research, unrelated to this spec | tracked in `ANALYSIS_THEME_SYSTEM_LIGHT_THEME_AND_DEPTH_GAPS_2026_07_07.md` |
