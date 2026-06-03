# Phase 3 impl plan — slice owns positions, retire TanStack from the agent pane

Issue #1235 · spec `SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md` §6 Phase 3 / §4.1-B / §11.1
Branch builds on Phase 2 (`659f5600`). Render from the slice's pure selectors; delete TanStack.

## Selectors to render from (already built + tested, Phase 0)
`computeLayoutView(state): {rows: RowPosition[], totalSize, window:{startIndex,endIndex}}` — one O(n) pass.
`RowPosition = {nodeId,index,start,height,end}`. **`start` INCLUDES `scrollMarginPx`; `totalSize` EXCLUDES it.**

## Current wiring (truth)
- `NodesChanged`: dispatched in `agent-view.tsx:215` but with the **FULL doc** (`nodes.map(n=>n.id)`). 🔴 must become `partition().virtualizedNodes` ids.
- Expansion (`ExpansionResolved`) + `EstimateSet`: dispatched `agent-view.tsx:228-254` (Phase 1 done) — iterates full doc; scope to virtualized set.
- `RowMeasured`: `AgentDocumentVirtualList.tsx:234-243` inside TanStack `measureElement` (keyed by current state). Re-home to a standalone measure RO.
- `Scrolled` / `ScrollMarginChanged` / `ZoomChanged`: **MISSING** — wire in Phase 3.
- Store registered with **no-op** projections (`agent-view.tsx:204`) — register real ones.

## TanStack touchpoints (all in `AgentDocumentVirtualList.tsx` unless noted)
L31 import · L182-248 `createVirtualizer` (estimateSize / scrollMargin getter / measureElement) · L337 `scrollToIndex` · L388 `getVirtualItems` (anchor capture) · L435 `getOffsetForIndex` · L471 `getTotalSize` · L476 `<For getVirtualItems>` · L488 undefined guard · L514-517 data-index dance · L524 translateY. `DocumentRow.tsx` L55/L160 `dataIndex`/`data-index`. (No `shouldMeasureDuringScroll` exists; `animateEnabled` is the streaming-buffer enter anim — orthogonal, don't touch.)

## Steps (internal order; verify live via `__agentLayout()` each)
0. **Signals/projections.** `agent-view.tsx`: replace no-op `registerLayoutPane` projection with a real `layout: setLayoutView` signal; thread `layoutView` accessor → `AgentDocumentView` → `AgentDocumentVirtualList` (mirror `zoomFactor`/`blockId`). `zoom` projection stays no-op (INV-2; CSS zoom applied at agent-view.tsx:797).
1. **🔴 partition-scoped `NodesChanged` (critical).** MOVE the slice-feeding effect from `agent-view.tsx:206-256` INTO `AgentDocumentVirtualList` (where `partition()` lives). Dispatch `NodesChanged{orderedIds: partition().virtualizedNodes.map(n=>n.id)}` + the scoped `EstimateSet`/`ExpansionResolved` loop over `virtualizedNodes`. Keep `registerLayoutPane`/`unregisterLayoutPane` + the `__agentLayout` hook in agent-view. Gate on `props.blockId != null`; use `dispatchLayoutIfRegistered`.
2. **Wire viewport inputs.** `handleScroll` → `Scrolled{scrollTop: scrollRef.scrollTop/zoom, viewportPx: scrollRef.clientHeight/zoom}` (÷zoom: positions are unzoomed — CONFIRM via CDP at zoom 0.5/2). `ScrollMarginChanged{px: virtualContainerRef.offsetTop}` via a RO on the container. `ZoomChanged{zoom: props.zoomFactor()}` via createEffect. **Extend the PR#1257 reactivate-RO callback to also dispatch `Scrolled` with fresh clientHeight** (viewport tracking on resize).
3. **Render from the slice.** `const windowedRows = createMemo(() => { const v=view(); return v.window.endIndex<v.window.startIndex ? [] : v.rows.slice(v.window.startIndex, v.window.endIndex+1) })`. Container height = `view().totalSize`px. Iterate **keyed by stable `nodeId`** (`<Key by="nodeId">` / `mapArray` — NOT `<Index>` [position churn], NOT raw `<For>` [remount-every-recompute]). Row style: `position:absolute; transform: translateY(${row.start - scrollMarginValue()}px)` where `scrollMarginValue = virtualContainerRef?.offsetTop ?? 0` (first row → translateY 0). 
4. **Measure RO (no data-index).** `measureRow(el, nodeId)`: a `ResizeObserver` (continuous re-measure — markdown/image/tool growth) → `RowMeasured{nodeId, state: inFlowState(snapshot.expansion.get(nodeId)), cssPx: gbcr.height/zoom}`. Preserve the ÷zoom + `agentPerfStore.recordEstimatorMeasurement` (dev). Store ROs in a `Map<nodeId,RO>`, disconnect on leave/unmount. `DocumentRow.tsx`: drop `dataIndex`/`data-index`; `ref` carries `measureRow`.
5. **scrollToNode + older-history off slice.** `scrollToNode` virtualized branch → `const row = view().rows[idx]; center = row.start - clientHeight/2 + row.height/2`. Older-history capture → `view().rows[window.startIndex]` (`.start` includes margin, same basis as TanStack v3). Restore → read `computeLayoutView(snapshotLayout(blockId)!).rows[newIdx].start` (synchronous snapshot — no projection-timing dep). Keep streaming-buffer branches unchanged.
6. **Delete TanStack.** Remove `createVirtualizer`, import, undefined guard, data-index. Drop `@tanstack/solid-virtual` from package.json + lockfile (only importer). 

## MUST PRESERVE (do not break)
Streaming buffer (`<Index>`, STREAMING_BUFFER_SIZE, sticky-frontier `partition()` memo). Sticky-bottom `createEffect` + `jumpToBottom`/`scrollToBottomRef`. **PR#1257 reactivate ResizeObserver** (hidden→visible re-scroll). `animateEnabled` gate.

## Risks
- scrollTop unit under CSS zoom (÷zoom both scrollTop+clientHeight; **CONFIRM via CDP at zoom≠1**).
- No scroll feedback loop: rows are absolute in a fixed-height container; `Scrolled` doesn't move scrollTop. `viewsEqual` store gate + overscan=5 suppress micro-scroll re-renders.
- onCleanup/RO scoping → use the Map<nodeId,RO> approach.

## Tests / verification
Slice tests (Phase 0) unchanged. NEW: (1) partition→slice test (`orderedIds == virtualizedNodes ids`, not full doc); (2) translateY-origin (first row → 0); (3) window slicing == rows[start..end]; (4) measure→reflow no-overlap (`start[i+1]===end[i]`); (5) ZoomChanged no-relayout. **§7 CDP churn test** (new `tools/tests/agent-layout-drift.mjs`, model on `bench-agent-keystroke.mjs`, port 9223, `__agentLayout()`): fresh reload → N expand/collapse cycles → assert **0 overlap + 0 drift** (baseline: 5 mismatches/1 overlap after 9 cycles) at zoom 0.5/1/2.
