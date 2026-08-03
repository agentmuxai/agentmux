# Plan: consolidate duplicated display-formatting utilities into `frontend/util/`

**Status:** Plan — not yet implemented.
**Trigger:** user request while discussing the composer strip's `↑in ↓out` token display — the abbreviated form never rolls over past "k" (a heavy session could show `12345.6k` instead of `12.3m`), and there's no comma-grouped exact form for tooltips/full-precision contexts. Broadened per follow-up request into a sweep for other duplicated utility-shaped logic (§7) worth building out the same way.

`frontend/util/` already exists as this codebase's convention for exactly this (`menu-position.ts`, `settle-detector.ts`) — everything below lands as new siblings there, not a novel location.

## 1. Problem

Searched every token/context-count display in the frontend. The "format a token count compactly" logic is **independently duplicated 7 times across 6 files** — no shared utility exists. Every copy shares the same two gaps: no k→m→b rollover, and inconsistent (or absent) precision rules.

| # | File | Function | Current logic | Used for |
|---|------|----------|----------------|----------|
| 1 | `frontend/app/view/agent/components/AgentComposerStrip.tsx:33-36` | `fmtTokens`'s inner `fmt` | `n >= 1000 ? (n/1000).toFixed(1)+'k' : String(n)` | Composer strip `↑in ↓out` (turn/session totals) |
| 2 | `frontend/app/view/agent/components/AgentComposerStrip.tsx:43-45` | `fmtK` | `Math.round(n/100)/10 + 'k'` | Composer strip context-fill text (`12.1k / 64k`) |
| 3 | `frontend/app/view/agent/components/AgentFooter.tsx:36-39` | `fmtTokens`'s inner `fmt` | **Byte-identical to #1** | `AgentWorkingRow`'s `↑in ↓out` |
| 4 | `frontend/app/statusbar/TokenUsageIndicator.tsx:16-20` | `formatTokenCount` | Tiered: raw <1000, 1-decimal `k` <10k, integer `k` ≥10k | Status-bar running total |
| 5 | `frontend/app/statusbar/TokenBreakdownPopover.tsx:31-32` | (inline, unnamed) | **Same tiered logic as #4**, copy-pasted | Per-service breakdown rows |
| 6 | `frontend/app/view/swarm/swarm-view.tsx:220-222` | `fmtCtx` | `Math.round(tokens/100)/10 + 'k'` — **same as #2** | Swarm pane per-agent context-fill |
| 7 | `frontend/app/view/agent/virtualization/DocumentRow.tsx:261` | inline `fmt` | `tok >= 1000 ? Math.round(tok/1000)+'k' : String(tok)` (integer only, no decimal) | "Context compacted: Xk → Yk" transcript node |

None of the 7 ever produce `m` or `b` — a heavy session's cumulative totals (`sessionTotals` in #1/#3, the status-bar running total in #4/#5) can realistically cross 1M+ tokens over a long day of use, and would currently render as an ever-growing, comma-less `12345.6k` instead of rolling to `12.3m`.

Separately, the exact/full-precision contexts (tooltips) are inconsistent: `AgentComposerStrip.tsx`'s `contextTitle()` already does this correctly (`tokens.toLocaleString()` → real thousands commas), but nothing enforces the other 6 sites follow the same pattern if a future full-precision display is added.

## 2. Proposed shared utility

New file: `frontend/util/format-count.ts` (generic — context-compaction and swarm displays aren't really "token usage store" concerns, so this doesn't belong in `store/token-usage.ts`).

```ts
/** Compact k/m/b abbreviation for a non-negative integer count.
 *  Precision rule (per magnitude tier, matches the existing
 *  TokenUsageIndicator/TokenBreakdownPopover convention): one decimal
 *  place below 10x the tier, integer at 10x and above — this keeps the
 *  abbreviated mantissa to at most 2 significant digits before the
 *  decimal (e.g. "9.9k", "10k", never "9999.9k"), which is what makes
 *  the k→m→b rollover below sufficient on its own without ever needing
 *  a comma INSIDE the abbreviated form.
 */
export function formatCompactNumber(n: number): string {
    const abs = Math.abs(n);
    const sign = n < 0 ? "-" : "";
    if (abs < 1_000) return `${n}`;
    if (abs < 1_000_000) return sign + tier(abs, 1_000, "k");
    if (abs < 1_000_000_000) return sign + tier(abs, 1_000_000, "m");
    return sign + tier(abs, 1_000_000_000, "b");
}

function tier(abs: number, divisor: number, suffix: string): string {
    const scaled = abs / divisor;
    const text = scaled < 10 ? scaled.toFixed(1) : String(Math.round(scaled));
    return `${text}${suffix}`;
}

/** Exact, comma-grouped form for tooltips / full-precision text —
 *  thin wrapper so every call site is visibly using the same
 *  formatting decision, not a bare `.toLocaleString()` sprinkled ad hoc. */
export function formatExactNumber(n: number): string {
    return n.toLocaleString();
}
```

### Worked examples (the rollover the user asked for)

| Input | Old (`fmtK`/`fmtCtx`/#1-3 style) | New (`formatCompactNumber`) |
|---|---|---|
| 850 | `850` | `850` |
| 1,200 | `1.2k` | `1.2k` |
| 9,960 | `10.0k` | `10.0k` *(pre-existing rounding-at-boundary quirk — `9960/1000=9.96`, `toFixed(1)` rounds to "10.0" while still under the 10,000 k→next-precision-tier line — unchanged, not a regression)* |
| 45,000 | `45.0k` | `45k` |
| 999,000 | `999.0k` | `999k` |
| 1,200,000 | `1200.0k` ⚠️ | `1.2m` |
| 12,345,678 | `12345.7k` ⚠️ | `12m` *(≥10x the m tier, so integer precision — same rule as 45,000 above)* |
| 1,200,000,000 | `1200000.0k` ⚠️ | `1.2b` |

The ⚠️ rows are today's actual bug — this is exactly what motivated the request.

## 3. Migration (replace all 7 duplicates)

Each site keeps its own display shape (the `↑↓` glyphs, the `X / Y` context format, etc.) — only the per-number formatting call changes to `formatCompactNumber`:

1. `AgentComposerStrip.tsx` — `fmtTokens` and `fmtK` both call `formatCompactNumber` internally; drop the local `fmt`/`fmtK` bodies.
2. `AgentFooter.tsx` — same `fmtTokens`, now imports instead of re-declaring the identical function.
3. `TokenUsageIndicator.tsx` — `formatTokenCount` becomes a direct re-export/call of `formatCompactNumber` (identical tiering already, just extended with m/b).
4. `TokenBreakdownPopover.tsx` — same as #3, drop the inline duplicate.
5. `swarm-view.tsx` — `fmtCtx` calls `formatCompactNumber`.
6. `DocumentRow.tsx` — inline `fmt` calls `formatCompactNumber` (gains 1-decimal precision under 10k as a side effect — currently integer-only; worth confirming this reads fine in the compaction transcript node before/after numbers).

No call site's public shape changes (all still take a plain `number`, return a `string`) — this is a pure internal consolidation plus the m/b extension, not a UI redesign.

## 4. Exact-number (comma) contexts

Audited every tooltip/full-precision spot already showing a raw token/context number:

- `AgentComposerStrip.tsx`'s `contextTitle()` — already correct (`tokens.toLocaleString()`), no change needed.
- No other exact-number token display currently exists outside the 7 compact-format sites above. If a future tooltip needs one (e.g. hovering the composer strip's `↑↓` stats, per the earlier discussion about adding a tooltip there), it should call `formatExactNumber()` from the same new module rather than an ad hoc `.toLocaleString()`.

## 5. Tests

New `frontend/util/format-count.test.ts` covering:
- Boundary values at each tier edge: 999/1000, 9999/10000, 999999/1000000, 999999999/1000000000.
- The rounding-at-boundary quirk (9950 → "10.0k", still inside the k tier) — documented as expected, not asserted away.
- Negative numbers (defensive — none of today's 7 call sites can produce negatives, but the function should not throw/mis-render if one ever does).
- Zero.

Then one updated test per migrated call site (where tests already exist — `TokenUsageIndicator`/`TokenBreakdownPopover` likely have existing coverage to update; `AgentComposerStrip`/`AgentFooter`/`swarm-view`/`DocumentRow` may not have direct formatter tests today, worth checking during implementation).

## 6. Non-goals

- Does not touch unrelated `toLocaleString()` usages elsewhere in the app (dates, line counts, etc. — e.g. `AgentControlBar.tsx`'s session-line-count text) — out of scope, those aren't token/count displays this request is about.
- Does not add a tooltip to the composer strip's stats zone (a separate, already-discussed follow-up) — this plan only fixes the number formatting itself.
- Does not change `contextTitle()`'s existing correct exact-number formatting.

## 7. Broader sweep: other duplicated display-formatting utilities

Same audit method applied to other common "format a primitive for display" shapes across `frontend/app`. Three more genuine, worth-fixing duplicates found; three checked-and-fine (already centralized, no action needed).

### 7.1 Elapsed-time formatting — 5 copies, **two different conventions in use**

| File | Function | Output shape |
|---|---|---|
| `AgentComposerStrip.tsx:38-41` | `fmtElapsed` | `"42s"` / `"3m 5s"` |
| `AgentFooter.tsx:31-34` | `fmtElapsed` | **byte-identical to the above** |
| `ActivityRow.tsx:60-65` | `formatElapsed` | `"3:05"` (mm:ss clock) |
| `PersistentShellBlock.tsx:36-41` | `formatElapsed` | `"3:05"`, missing the `Math.max(0, …)` floor the other two mm:ss copies have |
| `swarm-view.tsx:382-387` | `formatElapsed` | `"3:05"`, same as ActivityRow's |

This one isn't purely mechanical — two visually distinct conventions are already live in the product (prose-style "3m 5s" in the composer strip/footer vs. clock-style "3:05" in dock/activity rows). Proposed: `frontend/util/format-time.ts` exporting **both** as separately named functions — `formatElapsedCompact` (prose form) and `formatElapsedClock` (mm:ss form, with the missing floor guard added to the one copy that lacks it) — rather than forcing one convention on every call site. Still closes the real bug in this group: `PersistentShellBlock.tsx`'s copy can format a negative duration (e.g. a clock-skew edge case) into something like `"-1:05"`; the other two clock-style copies already guard against it.

### 7.2 String truncate/abbreviate — 3 copies, inconsistent ellipsis + one has real smarts worth keeping

| File | Function | Behavior |
|---|---|---|
| `block/autotitle.ts:362-368` | `truncate` | Right-truncate, appends literal `"..."` (three ASCII dots) |
| `view/drone/drone-view.tsx:587-589` | `truncate` | Right-truncate, appends real `"…"` ellipsis char, default `max=40` |
| `view/agent/components/AgentFooter.tsx:48-55` | `abbreviateArg` | Right-truncate for plain strings, but **left-truncates path-like strings** (containing `/`/`\`) to preserve the filename — the most capable of the three |

The literal `"..."` vs. real `"…"` split is a small but visible inconsistency (three characters vs. one — different string length, different look in a monospace UI). Proposed: `frontend/util/format-text.ts` exporting `abbreviateText(s, max)` — `AgentFooter.tsx`'s path-aware logic generalized as the one canonical implementation (its behavior is a strict superset: plain strings truncate the same way the other two already do, paths additionally get the smarter tail-preserving treatment) — and migrate all three call sites to it, standardizing on the real `"…"` character.

### 7.3 `sleep(ms)` — no shared helper, 12 inline copies of the same promise

`grep -rn "new Promise<void>((r) => setTimeout(r"` across `frontend/app` returns **12 hits** (poll loops in `useAgentControllerStatus.ts`'s login-recovery flows, retry loops elsewhere) — the identical `new Promise<void>((r) => setTimeout(r, ms))` expression, inlined every time instead of extracted once. Proposed: `frontend/util/async.ts` exporting `export const sleep = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms));`, migrate all 12 call sites. Purely mechanical, no behavior question (unlike §7.1) — every inline copy is already identical.

### 7.4 Checked and already fine — no action needed

- **Debounce** — already a single external dependency (`throttle-debounce` npm package, imported in `hook/useDimensions.tsx`), not reimplemented anywhere. No duplication.
- **Clipboard writes** — already centralized at `frontend/util/clipboard.ts` (`writeText`), imported consistently by every call site checked (`markdown-codeblock.tsx`, `InstancePanel.tsx`, others). No duplication.
- **ID generation** — every site calls `crypto.randomUUID()` directly (`contextmenu.ts`, `flash-notifications.ts`, etc.) with no wrapper function to duplicate; this is idiomatic use of a built-in, not a DRY violation.
- **`formatBytes`-style byte-size formatting** — only one implementation exists (`ToolOverlayLog.tsx`), not reimplemented elsewhere.

### 7.5 Revised scope if this becomes one PR

Four new files under the existing `frontend/util/` convention, each with its own focused test file:

| New file | Exports | Replaces |
|---|---|---|
| `format-count.ts` | `formatCompactNumber`, `formatExactNumber` | 7 copies (§2/§3) |
| `format-time.ts` | `formatElapsedCompact`, `formatElapsedClock` | 5 copies (§7.1) |
| `format-text.ts` | `abbreviateText` | 3 copies (§7.2) |
| `async.ts` | `sleep` | 12 inline copies (§7.3) |

Recommend sequencing as separate, small PRs in roughly this order (count-formatting first, since it's the one already fully scoped in §1-§6) rather than one large mixed-concern PR — each is independently reviewable and none depend on the others.
