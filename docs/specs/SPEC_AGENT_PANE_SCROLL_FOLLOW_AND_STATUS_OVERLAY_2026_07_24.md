# SPEC: Agent pane — fix silent auto-scroll-follow drops, extend the message-list scrollbar past the Working/Host status rows

**Date:** 2026-07-24
**Status:** Implemented — except **§3.2**, superseded 2026-09-01 by
`SPEC_AGENT_WORKING_ROW_ABOVE_COMPOSER_2026_09_01.md`.

> **§3.2 only.** The floating-overlay arrangement described there
> (AgentWorkingRow absolutely positioned over `.agent-document-scroll-region`'s
> bottom edge, with `.agent-document` reserving matching padding) is gone: the
> row is now a normal-flow sibling between the ActivityDock and the composer.
> **Everything else in this spec stands and is still load-bearing** — in
> particular §2's scroll-follow analysis and both re-pin observers. The 09-01
> change actually leans on §3.3's normal-flow-sibling handling (the working row
> simply joined that family) rather than contradicting it.
**Scope:** `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`, `frontend/app/view/agent/agent-view.tsx`, `frontend/app/view/agent/components/AgentFooter.tsx`, `frontend/app/view/agent/components/AgentComposerStrip.tsx`, `frontend/app/view/agent/styles/_document.scss`, `frontend/app/view/agent/styles/_control-bar.scss`, `frontend/app/view/agent/styles/_composer-strip.scss`
**Author:** Agent3

---

## 1. Intent

Two live-QA reports from the agent pane, stated directly by the user, refined over the course of the conversation:

1. **Auto-scroll-follow silently stops.** While an agent is actively streaming, the message list is supposed to stay pinned to the latest content. Intermittently — no identifiable trigger, the user never touches the scrollbar — it just stops following, and new content keeps arriving below the visible viewport unnoticed. Happens "a lot."
2. **The message list's scrollbar doesn't reach the bottom of the pane.** Original ask: extend the scrollbar past both the `AgentWorkingRow` ("Working…"/"✓ Worked") and the Host/Container status row. Final, simpler shape after two rounds of clarification: **remove the Host/Container row entirely** (confirmed inert — clicking it does nothing) and replace it with a compact "HOST" (red) / "SANDBOX" (white) tag next to the model selector in the composer strip, to reclaim that vertical space outright. That leaves only `AgentWorkingRow` between the message list and the composer, which the scrollbar should now extend past.

## 2. Issue 1 — auto-scroll-follow silently stops

### 2.1 Current mechanism

`stickToBottom` (`frontend/app/view/agent/virtualization/state.ts:38,111`) is a plain boolean signal, `true` by default, flipped to `false` by `disengageStickToBottom()` (called from the native `scroll` handler's near-bottom heuristic, `AgentDocumentVirtualList.tsx:502-538` / `isNearBottom()` in `anchor.ts:68-85` — a user scrolling away from the bottom).

The actual "scroll to bottom while sticky" effect, `AgentDocumentVirtualList.tsx:376-393`:

```ts
createEffect(() => {
    // Track length changes — Solid will re-run when nodes() emits.
    const _len = props.viewState.nodes().length;
    if (props.viewState.stickToBottom() && scrollRef) {
        queueMicrotask(() => {
            if (!scrollRef) return;
            if (!props.viewState.stickToBottom()) return;
            scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "auto" });
            collapseScrolledOffTools();
        });
    }
});
```

Its **only** reactive dependency is `props.viewState.nodes().length` — it re-runs when a node is added or removed, not when an *existing* node's rendered height changes.

Row heights are measured independently, via a `ResizeObserver` (`measureRO`, `AgentDocumentVirtualList.tsx:681-712`) that dispatches a `RowMeasured` action into a separate layout slice, updating `layoutView().totalSize` (the virtualizer's computed total content height, consumed at `AgentDocumentVirtualList.tsx:738` to size the scroll container's inner spacer). This is a **completely separate reactive chain** from the stick-to-bottom effect above.

`_document.scss:28-35` sets `overflow-anchor: auto` on `.agent-document` specifically to cover "the cases CSS can't" per its own comment — intended as a belt-and-suspenders backstop. But the virtualized rows are `position: absolute` / translateY-positioned (`AgentDocumentVirtualList.tsx:763-769`), and Chromium's scroll-anchoring candidate selection does not reliably anchor out-of-flow (absolutely positioned) boxes. So neither layer compensates for this case.

### 2.2 Root cause

Whenever a row's measured height changes **without a corresponding node-count change** — a tool-call panel expanding/collapsing, syntax highlighting re-flowing a code block, a lazily-loaded image resolving, markdown re-render — `scrollHeight` grows or shrinks, but:
- The stick-to-bottom effect doesn't re-run (its only dependency, `nodes().length`, didn't change).
- CSS `overflow-anchor` doesn't reliably compensate (virtualized rows are out-of-flow).

`stickToBottom` itself never flips to `false` in this scenario — the bug is invisible in state inspection, which matches the user's report exactly: intermittent, no identifiable trigger, no user scroll input, the view just silently falls behind "true" bottom while `stickToBottom` would still read `true` if inspected.

### 2.3 Fix

Add `layoutView().totalSize` as a second tracked dependency in the same effect, so *any* height change — not just a node-count change — re-triggers the pin-to-bottom scroll while sticky:

```ts
createEffect(() => {
    const _len = props.viewState.nodes().length;
    const _totalSize = props.layoutView?.()?.totalSize;
    if (props.viewState.stickToBottom() && scrollRef) {
        queueMicrotask(() => {
            if (!scrollRef) return;
            if (!props.viewState.stickToBottom()) return;
            scrollRef.scrollTo({ top: Number.MAX_SAFE_INTEGER, behavior: "auto" });
            collapseScrolledOffTools();
        });
    }
});
```

`layoutView` is already threaded into this component as a prop (`AgentDocumentVirtualList.tsx:113`, `Accessor<LayoutView | null>`) and already drives the scroll container's total height (line 738) — this fix reuses an existing, already-correct signal rather than introducing a new one. No change needed to `measureRO`/`RowMeasured` — they already fire on the right events; the gap was purely that the pin-to-bottom effect wasn't listening.

**Risk check:** this effect already re-checks `stickToBottom()` inside the microtask before scrolling (line 385), so a user who scrolls away between a height-change event and the microtask firing is still respected — no regression to the "user scrolled away, stay put" behavior. The added dependency only makes the effect re-run *more often* when sticky; it doesn't change what it does once triggered.

## 3. Issue 2 — scrollbar stops short of the composer

### 3.1 Current layout

`.agent-document` (the scrollable message list) is a `flex: 1; overflow-y: auto;` sibling inside `.agent-view` (`display: flex; flex-direction: column;`, `_document.scss:8-10`). Its rendered height — and therefore its native scrollbar's length — is whatever remains after every *other* flex sibling below it has claimed its own space. In `agent-view.tsx`, in order, below `AgentDocumentView`:

1. `AgentWorkingRow` (`agent-view.tsx:1330-1358`) — "Working… · Ns" / "✓ Worked · Ns", conditional on `isLoading()`/turn phase/session stats.
2. Retry bar (conditional on `status.canRetry()`).
3. `AgentDecisionPanel` (conditional — pending tool-approval).
4. `AgentQuestionPanel` (conditional — pending `AskUserQuestion`).
5. `PendingMessagesPanel` (the queued-message list).
6. **Host/Container `PaneRow`** (`agent-view.tsx:1417-1428`) — "Host — full system access" / "Container — isolated Docker sandbox". Shown whenever `agentMode` is `"host"` or `"container"`, i.e. essentially always (host is the launch-time default). **Confirmed inert**: no `onActivate`/`actions` prop is passed to `PaneRow`, so its click handling never engages — this row does nothing when clicked.
7. `AgentCredentialsRevokedChip` (conditional).
8. Failure-recovery `PaneRow` (conditional).
9. `AgentDisconnectedBanner` (conditional).
10. `AgentComposerStrip` (composer + model selector, `AgentRuntimeDropup`).

In the common/steady-state case (no pending decision, no pending question, no queued messages, no retry available, no credentials/failure/disconnect state), rows 2–5 and 7–9 all render nothing, so `AgentWorkingRow` and the Host/Container `PaneRow` end up visually adjacent — which is what led to the first follow-up clarification ("I see the Host actually changes to Working…"): they're two separate, independent rows, not one row whose text changes, but in the common case they sit back-to-back with nothing between them.

### 3.2 Design — remove the Host/Container row, overlay only `AgentWorkingRow`

Second follow-up simplified this further: since the Host/Container row is confirmed inert, remove it outright rather than working around its height.

**3.2.1 Delete the Host/Container `PaneRow`.** Remove `agent-view.tsx:1412-1429` (the `<Show when={agentMode === "host" || agentMode === "container"}>` block and its comment) entirely.

**3.2.2 Add a compact HOST/SANDBOX tag next to the model selector.** In `AgentComposerStrip.tsx`'s left `.agent-composer-strip-controls` zone (which today holds only `AgentRuntimeDropup`, gated on `providerId === "claude"` — §3.1's line 139/`showControls()`), add a new tag rendered *unconditionally* of `showControls()` (agent mode applies to every provider, not just Claude) immediately after the dropup:

```tsx
<span class="agent-composer-strip-controls">
    <Show when={showControls()}>
        <AgentRuntimeDropup ... />
    </Show>
    <Show when={props.agentMode === "host" || props.agentMode === "container"}>
        <span
            class="agent-composer-strip-mode-tag"
            classList={{
                "agent-composer-strip-mode-tag--host": props.agentMode === "host",
                "agent-composer-strip-mode-tag--sandbox": props.agentMode === "container",
            }}
            title={props.agentMode === "container" ? "Container — isolated Docker sandbox" : "Host — full system access"}
        >
            {props.agentMode === "container" ? "SANDBOX" : "HOST"}
        </span>
    </Show>
</span>
```

New `agentMode?: string` prop on `AgentComposerStripProps`, wired from `agent-view.tsx`'s existing `<AgentComposerStrip>` call site: `agentMode={block()?.meta?.["agentMode"] as string | undefined}` — the exact same read the deleted `PaneRow` used, so behavior (which mode shows which label) is unchanged, just relocated. `title` carries the two full sentences the old row's title used to show, as a tooltip, so that context isn't lost even though the always-visible label shrinks to one word.

Styling (new rules in `_composer-strip.scss`, small uppercase pill matching the strip's existing 10-11px chrome):
```scss
.agent-composer-strip-mode-tag {
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.02em;
    padding: 1px 5px;
    border-radius: 3px;

    &--host {
        color: #f85149; // red — matches the "full system access" severity the old row's icon/accent implied
        background: color-mix(in srgb, #f85149 10%, transparent);
    }
    &--sandbox {
        color: var(--main-text-color); // plain/white
        background: color-mix(in srgb, var(--main-text-color) 8%, transparent);
    }
}
```
(Exact red hex/tokens: use whatever this codebase's existing danger/error color variable is, if one exists — check `--error-color`/`--danger-color` in the theme files before hardcoding `#f85149`, to stay consistent with the rest of the app's palette rather than introducing a one-off value.)

**Post-review update (reagent P2 on PR #2292):** the bespoke `.agent-composer-strip-mode-tag` above duplicated the existing `RuntimeBadge` component (`frontend/app/view/agent/components/RuntimeBadge.tsx`), already used in `MyAgentsList.tsx`/`HostPopover.tsx` for the same host/container distinction with `sm`/`md` size variants. Implemented as a new `size="tag"` variant on `RuntimeBadge` instead (icon-less, no border, red/white text per this section's spec) — `<RuntimeBadge runtime={props.agentMode} size="tag" />` in `AgentComposerStrip.tsx`. The `HOST`/`SANDBOX` all-caps wording still deliberately differs from the `sm`/`md` variants' `Host`/`Container` labels (an explicit, intentional design choice for this specific compact spot, documented inline in `RuntimeBadge.tsx`), even though both now share one component.

**3.2.3 Overlay `AgentWorkingRow` over the tail of the message list.** With the Host/Container row gone, only `AgentWorkingRow` needs to move. Same wrapper/overlay technique as originally proposed, now for a single row:

- `.agent-document`'s wrapper (`.agent-document-scroll-region`, `position: relative; flex: 1; min-height: 0; overflow: hidden;`) takes over the flex-1 slot; `.agent-document` becomes `position: absolute; inset: 0;` inside it, still `overflow-y: auto`, still the real scrollbar owner.
- `AgentWorkingRow` moves inside that same wrapper as `position: absolute; left: 0; right: 0; bottom: 0; z-index: 2;` — no separate stacking container needed now that it's the only row (§3.2 of the original draft's `.agent-document-status-overlay` collapses to just being `AgentWorkingRow`'s own positioning). `pointer-events: none` on it if (per current code) it has no click handling — confirm before shipping; if a future change adds interaction to this row, scope `pointer-events: auto` to just that clickable element.
- Bottom padding on `.agent-document` still needed, sized to `AgentWorkingRow`'s rendered height via `ResizeObserver` (same pattern as before, one fewer row to sum), so scrolling to true bottom shows the last message above the row, not hidden under it. Simpler now: `AgentWorkingRow` either renders (fixed, known height) or renders nothing (`display: none` per its own `<Show>` guard in `agent-view.tsx:1332-1358`) — the padding only needs to track those two states, not a variable multi-row stack.

**Post-review update (reagent P1 on PR #2292):** the `ResizeObserver`-driven height (`workingRowHeight` signal in `agent-view.tsx`) sizes `.agent-document`'s padding-bottom via a CSS custom property, but §2's stick-to-bottom effect in `AgentDocumentVirtualList.tsx` didn't know about it — only `nodes().length` and `layoutView().totalSize` were tracked. When the row appeared or grew while pinned to bottom (a turn starting, tool-name text widening it), `.agent-document`'s effective content height changed with no corresponding re-pin, so the newly-taller overlay could cover the previously-visible tail of the message list — the exact bug class §2's fix was meant to close, reintroduced by §3's own change. Fixed by threading `workingRowHeight` through as a new prop (`agent-view.tsx` → `AgentDocumentView.tsx` → `AgentDocumentVirtualList.tsx`) and adding it as a third tracked dependency in the same effect, rather than force-scrolling unconditionally (which would fight a user who'd manually scrolled away).

### 3.3 Trade-off carried over from the original draft

Rows 2–5 and 7–9 (retry bar, decision/question panels, pending messages, credentials chip, failure row, disconnected banner) are still normal-flow, unchanged, between `.agent-document-scroll-region` and `AgentComposerStrip`. If any of them is visible at the same time as `AgentWorkingRow`, the overlay (anchored to the bottom of the scroll region) won't sit flush above the composer in that combined state — same rare, non-breaking visual-layering quirk as before, now with one row instead of two. Deferred for the same reason: interposing states are comparatively rare/transient.

## 4. Verification plan

- **Issue 1**: reproduce by triggering a height change on an off-screen (scrolled-past) row while `stickToBottom` is engaged and the agent is actively streaming — e.g. expand/collapse a tool-call panel that's above the current viewport, or trigger syntax re-highlighting on a large code block, while new messages keep arriving below. Confirm the view stays pinned to true bottom throughout, both before and after the triggering event. Regression-check: user manually scrolls up mid-stream → confirm `stickToBottom` still disengages and stays disengaged (the fix doesn't touch that path).
- **Issue 2**: confirm the Host/Container row is gone and a "HOST" (red) or "SANDBOX" (white) tag renders next to the model selector for host/container agents respectively, with the old row's full sentence now in its tooltip. Confirm the message list's scrollbar track now extends to the top of `AgentComposerStrip` in the steady state. Confirm `AgentWorkingRow` still renders with identical text/styling, just floating over the tail of the message list instead of pushing it up. Confirm scrolling to true bottom shows the last real message fully above the row (not clipped/hidden under it), both while `AgentWorkingRow` is visible and while it's hidden (padding should shrink to 0). Manually trigger one of the interposing panels (e.g. queue a message) while working, and confirm the trade-off in §3.3 is visually tolerable.
- `npx vitest run app/view/agent` + `npx tsc --noEmit` — no existing test directly covers `AgentDocumentVirtualList`'s stick-to-bottom effect or this overlay CSS; add a unit test for the effect's new `layoutView().totalSize` dependency if the existing test harness for this file supports driving a layout-slice update without a full render (check `AgentDocumentVirtualList.test.tsx` if one exists before deciding whether to add coverage here or rely on manual verification alone). Add/update `AgentComposerStrip` tests (if a test file exists for it) for the new `agentMode` prop → tag rendering.
