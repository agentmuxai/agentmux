# Spec: Status-Bar Token Usage Indicator + Per-Service Breakdown

**Date:** 2026-04-24
**Status:** Draft, ready to implement
**Owner:** AgentA
**Touches:** `frontend/app/statusbar/StatusBar.tsx` (+ SCSS),
             new `frontend/app/statusbar/TokenUsageIndicator.tsx`,
             new `frontend/app/statusbar/TokenBreakdownPopover.tsx`,
             a new token-aggregation store (see §5.1 — location TBD
             during impl, proposal: `frontend/app/store/token-usage.ts`)

---

## 1. Problem

AgentMux runs multiple agent CLIs concurrently (Claude Code, Codex,
Gemini, Copilot, …) and every turn consumes tokens, but today the
user has no single place to see *"how many tokens have I burned
today across everything?"* The per-turn readout in the agent pane's
status line (`↑12k ↓3k`) answers a local question; it doesn't
answer the budget question.

Users want:

1. A small at-a-glance total in the **bottom status bar** that
   increments as sessions run.
2. Clicking that total opens a **breakdown popover** that splits
   the total by service (Claude / Codex / Gemini / Copilot / …) so
   they can see where the spend is concentrated.

## 2. Goals

- **G1.** Persistent running total on the status bar, updated live
  as stream-json events arrive.
- **G2.** Popover (click to open, click-outside / Esc to close)
  showing per-service input + output token counts plus a total row.
- **G3.** Totals aggregate across all live agent panes + historical
  turns persisted since AgentMux started this session. A "Reset"
  option in the popover clears the running total to zero.
- **G4.** Zero backend work — per-turn `TurnTokens` and
  `SessionStats` are already parsed from stream-json; this spec
  aggregates existing data client-side.
- **G5.** Popover uses the existing `usePaneOverlay` primitive so
  it renders correctly over a browser pane (modal-v2 already does).

## 3. Non-goals

- No cost display on the status-bar total. Cost belongs in the
  per-turn footer (see
  `SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24.md`). Tokens
  are the budget unit here.
- No persistence across AgentMux restarts. Session-local only.
  (Stretch goal noted in §7.)
- No per-model breakdown (Opus vs. Sonnet vs. Haiku). Service-level
  only. Could extend later if demand surfaces.
- No rate-limit / remaining-budget integration. Those numbers live
  in a different realm (provider headers, subscription tiers) and
  belong in a separate spec.

---

## 4. Design

### 4.1 Status-bar indicator

Placement: the indicator sits in the status bar's **right group**,
alongside other passive readouts (backend status, connection
status). Compact form by default:

```
┌ status bar ───────────────────────────────────────────────────┐
│ …left-group things…          🪙 42k ↑ / 12k ↓   v0.33.385     │
└───────────────────────────────────────────────────────────────┘
```

- 🪙 icon (`fa-coins` from the font-awesome set AgentMux already
  bundles — same glyph family as other status icons).
- Numbers use the same `k` rounding as `tokenText()` in
  `AgentFooter.tsx` (`formatTokenCount(n)`: <1000 → raw, ≥1000 →
  `Xk` with one decimal when under 10k).
- On hover: subtle background highlight so it reads as clickable.
- Tab-index + `role="button"` + keyboard activation (Enter/Space).

When total is zero (no turns completed yet this session), the
indicator shows `🪙 0` — visible but muted. The zero state doubles
as a "I exist; click me to see the breakdown" affordance.

### 4.2 Breakdown popover

Opens on click below the indicator (popover anchor = indicator
bounding rect). Uses the existing modal-v2-adjacent overlay
primitive (`usePaneOverlay` for the Win32 airspace cut; same
pattern as MoreDropdown — see PR #544 / #545). If native
`<dialog>` positioning is painful, render via Solid's `<Portal>`
like MoreDropdown does.

Layout:

```
┌────────────────────────────┐
│ Token Usage · this session │
├────────────────────────────┤
│ Claude Code      ↑28k ↓ 8k │
│ Codex            ↑10k ↓ 3k │
│ Gemini           ↑ 4k ↓ 1k │
│ Copilot          ↑ 0  ↓ 0  │
├────────────────────────────┤
│ Total            ↑42k ↓12k │
├────────────────────────────┤
│ [Reset counter]            │
└────────────────────────────┘
```

- Header: `"Token Usage · this session"` + smaller subheader with
  the session start time (so the user knows what "this session"
  means — e.g. `"since 9:12 AM"`).
- One row per service that has contributed tokens this session.
  Services with zero so far are hidden (reduce noise) unless
  `debug` flag is on. Exception: always show all four services
  the user currently has registered agent definitions for, even if
  zero, so the popover looks complete.
- Services ordered by total tokens descending (biggest consumer
  first). Stable secondary sort = service name alphabetical.
- Total row is bold, same row layout, divider above it.
- "Reset counter" button at the bottom. Confirms via
  `ConfirmModal` (reuses modal-v2's preset) with destructive
  styling, since resetting loses the running total.

### 4.3 Interaction model

- Click indicator → popover opens.
- Click outside / Esc → closes.
- Click another DOM element → closes.
- Popover is **modal-equivalent for Win32 airspace** (calls
  `usePaneOverlay` on its root so pane HWNDs clip where it paints)
  but is not a true modal (no backdrop, doesn't block page
  interaction). Same pattern the MoreDropdown uses today.
- Keyboard: Enter on indicator opens; Esc closes; Tab cycles
  through "Reset counter" back to indicator trigger.

---

## 5. Implementation

### 5.1 Client-side aggregation store

Create `frontend/app/store/token-usage.ts` (or equivalent). It
exports a Solid store shaped roughly:

```ts
type ServiceId = "claude" | "codex" | "gemini" | "copilot" | string;

interface ServiceUsage {
    input: number;
    output: number;
}

interface TokenUsageState {
    sessionStartAt: number;     // epoch ms of first observation
    byService: Record<ServiceId, ServiceUsage>;
}
```

Exported methods:

- `recordTurn(serviceId, tokens: TurnTokens)` — called from the
  existing per-turn completion path (where `SessionEndEvent`
  fires). Increments the running totals.
- `getTotal()` — computed sum across all services.
- `resetSession()` — clears `byService` and bumps
  `sessionStartAt` to `Date.now()`.

**Plumbing:** find the location where `TurnTokens` becomes final
per turn (likely `useAgentStream.ts` where `SessionEndEvent` is
emitted — per the Explorer report). Call `recordTurn(provider,
tokens)` there. The `provider` string already lives on the agent
definition / catalog entry.

### 5.2 `TokenUsageIndicator.tsx`

New component rendered inside `StatusBar.tsx`. Reads the store's
total. Renders the compact `🪙 42k ↑ / 12k ↓` label. On activate,
toggles a `createSignal` flag that mounts the
`TokenBreakdownPopover`.

### 5.3 `TokenBreakdownPopover.tsx`

New component. Renders the breakdown layout in §4.2. Uses
`usePaneOverlay(() => rootRef)` for airspace. Anchors its
positioning to the indicator element's bounding rect (reuse or
generalise MoreDropdown's anchoring math).

### 5.4 Integration into `StatusBar.tsx`

Add the `<TokenUsageIndicator />` to the right-group children.
Ordering to be finalised in review — proposal: just left of the
version-number readout.

### 5.5 Styling

New SCSS partial `frontend/app/statusbar/_token-usage.scss`
(follows the Phase 5 split pattern). BEM classes:
`.token-usage-indicator`, `.token-usage-breakdown`,
`.token-usage-breakdown-row`, `.token-usage-breakdown-total`,
`.token-usage-breakdown-reset`. All colours via tokens
(`--secondary-text-color` for numbers, `--accent-color` for
hover/focus, `--error-color` for destructive-reset button).

### 5.6 Test plan

- [ ] `task build:frontend` succeeds
- [ ] `tsc --noEmit` clean
- [ ] Stylelint green
- [ ] Manual (`task dev`):
  - Launch an agent, run a turn → indicator updates as the turn
    completes
  - Launch a second agent of a different service, run a turn →
    both services appear in the popover breakdown
  - Click indicator → popover opens below it, over a browser pane
    if present (airspace works)
  - Reset counter → popover confirms → total returns to 0
  - Esc / click-outside closes popover
  - Open dev tools, inspect: token-usage store state matches
    what's visible in the UI

---

## 6. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Double-counting a turn if the stream-json replays on reconnect | Key turns by `(sessionId, turnIndex)` in the store and ignore duplicates. |
| Service IDs inconsistent between the catalog and the stream event | Centralise the normalisation in the store's `recordTurn` — map any recognised provider alias to the canonical id set. Unknown ids pass through as-is and get their own row. |
| Popover layout breaks on very narrow pane widths | The popover is fixed-width (320px). Status bar itself stays compact. If the pane is narrower than the popover, it anchors to the right edge of the viewport and clips gracefully (same pattern as MoreDropdown). |
| Reset button accidentally hit | Destructive `ConfirmModal` gate — matches the pattern used for delete-definition. |
| No persistence → restart wipes the counter | Spec limits scope to session-local. A follow-up could persist to `localStorage` keyed by date; not in scope here. |

## 7. Stretch goals (not in this PR)

1. **Daily/weekly cumulative totals** persisted to `localStorage`.
   Would let the indicator show *"42k today / 310k this week"*.
2. **Cost column** in the breakdown popover (not on the indicator
   itself) — reuses per-turn `cost_usd` the same way tokens
   aggregate. Needs a decision on whether to show estimated cost
   when the provider doesn't emit one.
3. **Rate-limit integration** — read provider headers the sidecar
   already captures and surface remaining quota next to the
   indicator. Separate spec.
4. **Per-model breakdown** (Opus vs. Sonnet). Indicator stays
   service-level; popover gains an expand-row per service.

## 8. Cross-references

- `frontend/app/statusbar/StatusBar.tsx` + `StatusBar.scss` —
  target for the new indicator.
- `frontend/app/view/agent/hooks/useAgentStream.ts` (per Explorer
  report) — where `TurnTokens` becomes final per turn; hook the
  store's `recordTurn` here.
- `frontend/app/view/agent/components/AgentFooter.tsx` —
  `formatTokenCount` / `tokenText` — reuse for consistent glyph
  style across the per-pane and global indicators.
- `frontend/app/platform/pane-overlay.ts` — `usePaneOverlay` for
  airspace (same pattern as modal-v2, MoreDropdown).
- `SPEC_AGENT_PANE_ZONE_ORDER_WORKED_FOOTER_2026_04_24.md` —
  companion spec for per-pane token rendering.
