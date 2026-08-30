# Spec: Agent Pane Zone Reorder + Enriched "Worked" Footer

**Date:** 2026-04-24
**Status:** implemented — shipped in #549 ("zone reorder + enriched Worked footer"). Verified 2026-08-23: `frontend/app/view/agent/agent-view.tsx` positions `<PendingMessagesPanel>` with a code comment explicitly citing this spec by filename; `AgentFooter.tsx` has the primary/secondary Worked-line split this spec designed.
**Owner:** AgentA
**Touches:** `frontend/app/view/agent/agent-view.tsx`,
             `frontend/app/view/agent/components/AgentFooter.tsx`,
             `frontend/app/view/agent/styles/_pending-footer.scss`

---

## 1. Problem

Two related UX gaps in the agent pane:

### 1.1 Queue sits in the wrong zone
When the user types a new message while the agent is still working,
the typed text lands in a **pending queue** (amber banner) that
renders *below* the status line and activity log:

```
┌ feed ──────────────────────────────────┐
│ …conversation transcript…              │
│                                        │
├ status line ──────────────────────────┤
│ ⠋ Working…                             │
├ activity log ──────────────────────────┤
│ (events)                               │
├ pending queue ────────────────────── ← │
│ “fix the thing”                        │
│ “then run tests”                       │
├ composer ──────────────────────────────┤
│ [type here…]                           │
└────────────────────────────────────────┘
```

Problem: the queue feels visually divorced from the feed it's queued
against. The user types, their message disappears into the composer,
and then reappears on the other side of two unrelated zones
(status + activity). The "pending" concept reads more clearly when
the queued items sit **directly under the live feed**, next to the
newest assistant message, with the "Working…" indicator right below
them — because the queue is what the agent will work on *next* after
the current turn.

### 1.2 "Worked" label lacks the numbers users want at a glance

At the end of a turn, the status line collapses from
`"Working… · ↑12k  ↓3k · 42s"` to `"Worked · $0.018 · 42s · 4 turns"`.
Tokens disappear from the completed-turn summary, even though they're
still the most-asked-about number during a post-mortem ("how much did
that cost us in tokens?"). The cost value is useful too but subservient
to tokens — tokens are what users budget against and what carry over
into the next turn's context window.

## 2. Goals

- **G1.** Move the pending queue directly below the feed and above
  the "Working…" status line in the pane's DOM order.
- **G2.** Enrich the "Worked" terminal-state label so it always
  shows **duration + total tokens** (input + output), with cost and
  turn count as secondary metadata (smaller type, same row).
- **G3.** No visual regression for running turns (the "Working…"
  animation + live token counters stay as-is).
- **G4.** Tokens-over-duration ordering matches the composer's
  live readout for continuity — the same glyphs (`↑`/`↓`), the same
  number formatting.

## 3. Non-goals

- No change to the pending-queue colour transition (amber → blue on
  accept) — that's governed by
  `AGENT_PANE_QUEUED_MESSAGE_FEEDBACK_SPEC.md` and stays intact.
- No change to the activity log panel, retry bar, or auth overlay
  positions — only the queue moves.
- No new metadata fields. Tokens + duration + cost + turn count are
  all already available in `SessionStats` and `TurnTokens`; this is
  a rendering change.
- No change to `AgentFooter`'s text input or slash picker.

---

## 4. Design

### 4.1 New DOM order

```
.agent-view
├── BookmarksPanel
├── AgentSearchBar
├── SessionDigestBanner
├── AgentFocusedPanel (overlay)
├── AgentDocumentView       ← the feed (unchanged)
├── AgentRetryBar           (unchanged; shows only on auth failure)
├── PendingMessagesPanel    ← MOVED HERE (was below activity log)
├── AgentStatusLine         ← "Working…" / "Worked …"
├── ActivityLogPanel        (unchanged position)
└── .agent-composer-region  (unchanged)
    └── …
```

Rationale: queue-then-status reads top-to-bottom as *"here's what
you've told the agent next → here's what it's currently doing"*.

### 4.2 Worked-footer layout

**Running turn (unchanged):**
```
⠋ Working…                     ↑12k ↓3k · 42s
```

**Completed turn (today):**
```
✓ Worked          $0.018 · 42s · 4 turns
```

**Completed turn (this spec):**
```
✓ Worked · 42s · ↑12k ↓3k            $0.018 · 4 turns
```

- Primary line (left group): state label, duration, total tokens
  with up/down glyphs. Same font size as the running-turn
  display so nothing jumps on transition.
- Secondary line (right group): cost + turn count, rendered in
  `--secondary-text-color` at the existing smaller size.
- If `TurnTokens` is unavailable for a completed turn (older CLI
  stream, missing `usage` field), fall back gracefully to
  `Worked · 42s · $0.018 · 4 turns` — identical to today.

### 4.3 Copy + formatting rules

- Duration: `XXs` when under a minute, `X.Xm` for 60-599s,
  `Xm Xs` for 10 minutes and above. Matches existing
  `statsText()` logic — reuse, don't duplicate.
- Tokens: reuse `tokenText()` in `AgentFooter.tsx:107`, which
  already emits `↑NNNk  ↓NNNk` with the same k-rounding we use in
  the running-turn readout.
- Cost: `$0.0XX` keep the 3-decimal precision that
  `statsText()` uses today.
- Turn count: `N turns` (pluralise — `1 turn` / `N turns`).
- Separator: middle dot `·` with `0.5ch` horizontal margins, same
  as today.

---

## 5. Implementation

### 5.1 `agent-view.tsx`

Move the `<PendingMessagesPanel>` JSX node from its current
position (between `<AgentStatusLine>` and composer) to sit
**immediately after `<AgentRetryBar>` and before
`<AgentStatusLine>`**. Line numbers per Explorer report: today the
panel lives around line 590; target position is just before the
status-line block near line 539. No prop changes, no new atoms.

### 5.2 `AgentFooter.tsx` → `AgentStatusLine`

Rewrite the fallback/"Worked" branch (today at lines 138-142) so it
pulls both `statsText()` and `tokenText()` and composes them into
left- and right-group spans matching §4.2. Concretely:

```tsx
// Existing:
<span class="agent-status-line-text">
    Worked {statsText()}
</span>

// New:
<span class="agent-status-line-text">
    <span class="agent-status-line-primary">
        Worked · {formatDuration(stats().duration_ms)} · {tokenText()}
    </span>
    <Show when={stats().cost_usd || stats().num_turns}>
        <span class="agent-status-line-secondary">
            {formatCostAndTurns(stats())}
        </span>
    </Show>
</span>
```

Extract `formatDuration()` / `formatCostAndTurns()` helpers from
the existing `statsText()` so the Working branch keeps using the
same formatters (no drift).

### 5.3 `_pending-footer.scss`

The moved position may need a tweak to the spacing/border: when
the queue sits *above* the status line, its visual weight should
be a bit lower than today's bottom-of-stack placement. Concretely:

- Change the queue's top border from solid to a 1px dashed
  border-top and a 1px solid border-bottom — visually "pinned to
  the feed above it" rather than "floating above the footer".
- Reduce its vertical padding by one `--space` token to tighten
  it against the feed.

No color changes. Existing amber-→-blue transition stays.

### 5.4 Status-line two-group flex

Update the status-line container so it's a `display: flex;
justify-content: space-between;` row with primary + secondary
groups as children. The current layout is already left-right-split
for tokens-right but the new Worked state uses the same pattern on
the completed branch too.

---

## 6. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Users relying on muscle memory ("queue is the amber thing above the input") | The queue is still amber and still visible. Its position change should be *more* intuitive (closer to the feed it queues against). If needed, a one-time "New layout" tooltip can be added later. |
| Token data missing for older stream replays | Fallback path in §4.2 keeps parity with today when `TurnTokens` is null. |
| Status-line width overflows on narrow panes | The `.agent-status-line` already has container queries (`@container`) in the responsive partial — if the combined primary+secondary string overflows, we can collapse to just primary. Spec-level placeholder; actual threshold determined empirically in review. |
| Two-row vs. one-row completed footer | The spec is one row. If the pane width can't hold both groups, wrap to two rows via `flex-wrap` — cheaper than hiding information. |

## 7. Validation

- ✅ `task build:frontend` succeeds
- ✅ `tsc --noEmit` clean
- ✅ Stylelint green
- ✅ Manual smoke (`task dev`):
  - Send a message, start a second one while first runs → queue
    appears **between** feed and status line
  - Let a turn complete → footer shows
    `Worked · 42s · ↑12k ↓3k` with `$0.018 · 4 turns` on the right
  - Start a new turn → live `Working…` shows tokens on the right
    as today
  - Resize pane narrow → cost+turns gracefully drop or wrap

## 8. Cross-references

- `frontend/app/view/agent/agent-view.tsx` — JSX order
- `frontend/app/view/agent/components/AgentFooter.tsx` —
  `AgentStatusLine` + `statsText()` + `tokenText()`
- `frontend/app/view/agent/components/PendingMessagesPanel.tsx` —
  moved component
- `frontend/app/view/agent/styles/_pending-footer.scss` — queue CSS
- `AGENT_PANE_QUEUED_MESSAGE_FEEDBACK_SPEC.md` — governs the queue's
  colour transitions (not changed here)
