# Agent pane "Working…" row: composer typeface, pane-border color, ambient summary, no token readout

**Status:** implemented (spec and change together).
**Date:** 2026-10-02
**Owner:** Manoz
**Component:** `AgentWorkingRow` (`frontend/app/view/agent/components/AgentFooter.tsx`)
**Styles:** `.agent-working-row-anchor .agent-working-row--loading` (`frontend/app/view/agent/styles/_control-bar.scss`)
**Supersedes in part:** `SPEC_AGENT_WORKING_ROW_TYPOGRAPHY_REFRESH_2026_09_03.md` (typeface and color) and the shimmer half of `SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md` §2.

## 1. Request, as given

> lets handle the "Working..." animation, part of the progress system in the agent pane. It should have the same font as the user input, which is a monospace type font. Also, get rid of the token count at the far right, since it already appears in the composer.

> it should also have the same color ... we also dont want the tool call, instead, Id like to integrate ambient haiku there

Clarified in the same session:

- "Ambient haiku" means the **ambient activity summary** (the Haiku-model text the app already generates), not a new generated poem.
- "Same color" means **the pane's border color**, as the agent pane shows it: the agent's color, with the theme accent as fallback. The faint unfocused-border grey was rejected as unreadable at 10px.
- The shimmer sweep goes (see 3.2).

## 2. What the row looked like

`AgentWorkingRow` in its `--loading` state: a pulsing dot, a left zone, and a right zone.

| Part | Before |
|---|---|
| Typeface | `--markdown-font-family`, the thinking-text face (chosen 2026-09-03 on the grounds that a status sentence "isn't code-shaped") |
| Color | `--main-text-color`, with a gradient shimmer sweeping between `--secondary-text-color` (dim, most of the time) and `--main-text-color` |
| Left zone | The cycling "Working…" phrase, or `tool · arg` (for example `Read · …/AgentFooter.tsx`) whenever a tool call was running and not yet promoted to an ActivityDock row |
| Right zone | `↑in ↓out` turn tokens, then the elapsed time |

## 3. Change

### 3.1 Typeface

`font-family: var(--font-mono)` on `--loading`. This is the composer's face: `.agent-input` declares `font-family: inherit`, and the nearest declaration above it is `.agent-view { font-family: var(--font-mono) }` (`agent-view.scss`). Naming the same variable keeps the two identical if the theme's mono face changes. Size (10px) and weight (700) are unchanged. `--worked` is untouched.

### 3.2 Color, and the shimmer

`color: var(--accent-color)`, overridden by `.has-agent-color & { color: var(--block-agent-color) }`.

- `.has-agent-color` is set on the block frame (`blockframe.tsx`) exactly when the pane has an agent color, and the same condition picks `--block-agent-color` over the accent for the focused border (`block.scss`).
- It cannot be a `var()` fallback: `blockframe.tsx` always sets `--block-agent-color`, to `"transparent"` when there is none, so a fallback would never apply and the text would vanish.
- The text is now one solid color. The gradient sweep, its keyframes and the `.is-typing` overlay (which existed only to cover the sweep during the reveal) are removed, along with the `typing` signal. A sweep between two stops of the same color would be invisible, and the dim half of the old sweep is what the request objected to. The pulsing dot beside the text still signals liveness, and the character-by-character type-out reveal of new text is kept.

### 3.3 Left zone: ambient summary instead of the tool call

Priority, first match wins:

1. Reconnecting, Compacting, Stopping, Rate limited, or a launch phase (unchanged).
2. **The ambient summary**, `readSwarmSummary(block.meta)`: the same line the Swarm row and pane tooltip show. That is `term:ambient_summary` (Haiku-written, validated by `isUsableTitle`), falling back to the CLI's own `term:osc_title` topic.
3. The cycling "Working…" phrase, until a summary exists.

What this text actually is: `useAgentActivitySummary.ts` keeps a stable, PR-title-style phrase for the session's **overall goal**, re-evaluated when the user submits a message. It is not a per-tool-call description. So the row reads, for example, "Fix the login redirect loop" for the whole turn rather than changing with every tool. A per-activity ambient call would be a new feature (prompt, rate limit and token cost in `crates/srv/src/ambient/`); it is out of scope here.

The pane header stopped showing this summary on 2026-09-20 (`agent-model.ts`, "pane-color unification"). This change surfaces it in the working row instead; the Swarm row is unaffected.

Consequences of dropping the tool text:

- `currentTool`, `currentToolArg` and `toolPromoted` are removed from `AgentWorkingRow` and its call site, along with the local `abbreviateArg` helper.
- The phrase-cycling effects no longer pause during a tool call, because nothing tool-specific is on screen to protect.
- `AgentBottomPanels` no longer takes `hasPromotedTool`. `useWorkingIndicator` still returns it and keeps its tests; only the unused plumbing is gone.
- The live tool call remains visible where it always had a better home: the ActivityDock rows and the document view.

### 3.4 Right zone: elapsed time only

The `↑in ↓out` turn-token readout is removed from the loading row, along with the `turnTokens` prop and its call-site argument. The elapsed time stays.

The "✓ Worked · 42s · ↑… ↓…" completion summary is **not** touched: the request named the far right of the working row.

## 4. Premise to check

The request says tokens "already appear in the composer". What the composer strip shows is **context-window usage** (`123k / 200k`, with an auto-compact warning band): a cumulative figure for the whole session. The per-turn `↑input ↓output` count was only ever in this row (the composer strip's own centered stats readout was dropped on 2026-08-31 for duplicating it). After this change the per-turn figure is visible nowhere while a turn runs; the completion row ("Worked") still prints it afterwards. If that per-turn figure is wanted somewhere, the natural home is the composer strip's stats zone, as a separate change.

## 5. Not changed

- Row layout, size, padding and the opaque background.
- The `--worked` completion row and its typography.
- The pulsing dot, the type-out reveal, and the reduced-motion handling of the reveal.
- The ambient summary's generation (`useAgentActivitySummary.ts`) and the Swarm row.

## 6. Tests

`AgentFooter.test.tsx`, "AgentWorkingRow ambient summary, elapsed-only right zone":

- the trimmed summary shows in the left zone;
- with no summary the cycling phrase shows;
- a status (Stopping…) wins over the summary;
- the right zone is elapsed time only (`/^\d+s$/`, no `↑` or `↓`);
- the left zone carries no shimmer or typing classes.

The typeface and color are CSS-only and were checked by compiling `agent-view.scss` and reading the emitted rules: `.agent-working-row--loading` gets `font-family: var(--font-mono)` and `color: var(--accent-color)`, and the `.has-agent-color …--loading` rule overrides the color; no `shimmer` selector remains.
