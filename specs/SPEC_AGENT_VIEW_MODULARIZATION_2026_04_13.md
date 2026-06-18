# Spec: Modularize `frontend/app/view/agent/agent-view.tsx`

**Date:** 2026-04-13
**Status:** Draft — ready to execute in small steps
**Target:** `frontend/app/view/agent/agent-view.tsx` (1,182 lines) → ≤ 300 lines

---

## 1. Why this matters

### Current size audit

| File | Lines |
|---|---:|
| **`agent-view.tsx`** | **1,182** |
| `components/SetupWizard.tsx` | 437 |
| `agent-model.ts` | 423 |
| `types.ts` | 389 |
| `stream-parser.ts` | 357 |
| `components/AgentDocumentView.tsx` | 357 |
| `components/AgentControlBar.tsx` | 339 |
| `useAgentStream.ts` | 271 |
| `init-monitor.ts` | 262 |
| (19 other components) | 30–200 each |
| **Total `frontend/app/view/agent/`** | **6,235** |

### What's in `agent-view.tsx` (the elephant)

One file holds four distinct top-level components plus ~15 concerns inside `AgentPresentationView`:

```
lines  1-28   imports
       30-61  useForgeAgents hook
       63-80  AgentViewWrapper — picker/presentation dispatch
       82-173 AgentPicker component
      175-379 Launch flow (runLaunchFlow) — 204 lines, inline
      381-1179 AgentPresentationView — 798 lines
            ├ block / provider / agentAtoms                (3-4 lines)
            ├ historyOffset / historyTotal / loadingOlder  (5 lines)
            ├ documentVersion / bumpDocumentVersion        (3 lines)
            ├ digestSummary / generatedAt / loading / dismissed (4 signals)
            ├ logLines + log() helper
            ├ authUrl / canRetry / flowRunning / agentReady / loginWaiting / isLoading
            ├ loginCancelled flag (mutable let)
            ├ buildAuthEnv helper
            ├ loadOlder handler
            ├ fetchDigest handler
            ├ dismissDigest
            ├ startLaunchFlow wrapper
            ├ cancelLogin handler
            ├ onCleanup — closes WS / subscriptions
            ├ onMount (~220 lines) — subscriptions, event bindings, digest auto-trigger, reactive init
            ├ handleSendMessage (~80 lines)
            ├ handleBack
            ├ sessionStartTsMs / sessionLastActivityMs memos
            ├ bookmarks / bookmarkedNodeIds / showBookmarks (state)
            ├ scrollToNodeFn (mutable ref for bookmark+search)
            ├ searchVisible / searchMatches / searchCurrentIndex (state)
            ├ nodeSearchText helper
            ├ performSearch / searchNext / searchPrev / searchClose
            ├ searchHighlightId memo
            ├ saveBookmarks helper
            ├ nodePreview helper
            ├ handleBookmark / Delete / Rename / Jump (4 handlers)
            ├ zoomFactor memo
            ├ handleSubagentClick
            ├ handleContextMenu
            └ JSX rendering (~90 lines)
```

**70 `const`/`let` declarations** inside `AgentPresentationView` alone. That's the core tell — this function has too many local variables to reason about as a single unit.

### Problems that size creates

1. **Cross-concern coupling gets silently introduced.** The `scrollIntoView` ancestor-leakage bug reported in `docs/analysis/agent-pane-rich-features-structure-2026-04-13.md` is a direct consequence: bookmarks and search had to reach into `AgentDocumentView` via a mutable `let scrollToNodeFn` ref, because the two features were declared 60 lines apart in the same function but conceptually should never have shared mutable state in the first place. In a modular layout, bookmarks and search would each hold their own reference to the same public API and there'd be no shared `let`.
2. **Hooks and effects get grouped by "when I wrote them" instead of "what they do".** `onMount` is ~220 lines of subscriptions, event bindings, and reactive initializers. Any new feature adds another block of code to `onMount`, and unrelated blocks are N lines apart. Cleanup is in a separate `onCleanup` block far away.
3. **Testing is impractical.** No hook or helper in this file can be unit-tested in isolation. `nodeSearchText`, `nodePreview`, `formatTimestamp`, etc. are all locked inside closures. Only the whole `AgentPresentationView` can be mounted, and mounting it requires the full Jotai/Atoms/RPC stack.
4. **SolidJS reactivity becomes hard to audit.** With 70 declarations sharing one scope, it's not visible which signals depend on which. A future engineer (or me, next month) has to trace every `createEffect` + `createMemo` + closure to know whether a change to one signal triggers a re-render somewhere unexpected.
5. **Merge conflicts multiply.** Every PR that touches the agent pane lands changes in the same 1,182-line file. PR #340, #341, #345, #346 all modified `agent-view.tsx`. Every one risked conflicts with the others. A modular structure scopes those edits.
6. **It's hard to see what's there.** I forgot I'd added bookmarks until you reminded me. I forgot the Ctrl+B keyboard handler existed. A file that's too big to read end-to-end hides its own features from future sessions.

---

## 2. Target structure

```
frontend/app/view/agent/
├── agent-view.tsx                           ~250 lines — composition + JSX only
│
├── agent-model.ts                           (unchanged, 423)
├── state.ts                                 (unchanged, 88)
├── types.ts                                 (unchanged, 389)
├── useAgentStream.ts                        (unchanged, 271)
├── stream-parser.ts                         (unchanged, 357)
├── parseHistoryLines.ts                     (unchanged, 61)
├── init-monitor.ts                          (unchanged, 262)
├── buildRuntimeArgs.ts                      (unchanged, 99)
│
├── flows/
│   └── launch-flow.ts                       NEW — extracted from agent-view.tsx:175-379
│                                            Pure-ish async function runLaunchFlow(ctx)
│                                            Takes { blockId, provider, log, onAuthUrl, … }
│                                            Returns Promise<"success" | "needs-login" | "fatal">
│
├── hooks/
│   ├── useLaunchLogs.ts                     NEW — logLines signal + log() helper
│   ├── useAgentControllerStatus.ts          NEW — authUrl, canRetry, flowRunning,
│   │                                              agentReady, loginWaiting, isLoading
│   │                                              Subscribes to controllerstatus events
│   ├── useHistoryPagination.ts              NEW — historyOffset, historyTotal,
│   │                                              loadingOlder, loadOlder, sessionStartTsMs,
│   │                                              sessionLastActivityMs. Owns the
│   │                                              blockfile:line_count / read_range calls.
│   ├── useSessionDigest.ts                  NEW — digestSummary, generatedAt, loading,
│   │                                              dismissed, fetchDigest, dismissDigest.
│   │                                              Owns the session:digest RPC + auto-trigger.
│   ├── useBookmarks.ts                      NEW — bookmarks, bookmarkedNodeIds,
│   │                                              showBookmarks, saveBookmarks,
│   │                                              add/delete/rename/jump handlers,
│   │                                              nodePreview helper.
│   ├── useInSessionSearch.ts                NEW — searchVisible, searchMatches,
│   │                                              searchCurrentIndex, nodeSearchText,
│   │                                              performSearch, next, prev, close,
│   │                                              searchHighlightId memo.
│   ├── useAgentKeyboard.ts                  NEW — Ctrl+B / Ctrl+F listener, pane-scoped.
│   │                                              Takes { blockId, onToggleBookmarks,
│   │                                              onToggleSearch }.
│   └── useScrollToNode.ts                   NEW — signal-based jump command that
│                                                  AgentDocumentView reacts to. Replaces
│                                                  the mutable scrollToNodeFn ref. Hooks
│                                                  that need to jump (bookmarks, search)
│                                                  call `jumpTo(nodeId)`.
│
├── components/
│   ├── AgentPicker.tsx                      NEW — extracted from agent-view.tsx:82-173
│   │                                              Top-level picker UI + useForgeAgents
│   │                                              (moved inside the component file).
│   ├── AgentPresentationHeader.tsx          NEW — the .agent-pres-header JSX block
│   │                                              (icon + name + close button).
│   │
│   ├── AgentDocumentView.tsx                REFACTOR — consume useScrollToNode signal
│   │                                                  instead of exposing scrollToNodeRef.
│   │                                                  Replace scrollIntoView with
│   │                                                  scrollRef.scrollTo({ top: computed }).
│   ├── AgentControlBar.tsx                  REFACTOR — banners move to NotificationStack.
│   │                                                  AgentControlBar keeps only the
│   │                                                  mode/model/effort controls + the
│   │                                                  session management buttons.
│   ├── AgentNotificationStack.tsx           NEW — single flex child that hosts all
│   │                                              conditional banners (interrupted,
│   │                                              large-session, archived, digest).
│   │                                              When no banner is active, renders
│   │                                              as a zero-height fragment so the
│   │                                              document flex size is stable
│   │                                              regardless of banner state.
│   │                                              (Separate from this spec — just
│   │                                              referenced as the target structure;
│   │                                              see docs/analysis/agent-pane-rich-
│   │                                              features-structure-2026-04-13.md §4.1.)
│   │
│   └── (20 existing components unchanged)
│
└── agent-view.scss                          (unchanged, ~1500)
```

### `agent-view.tsx` after the refactor

What's left: composition glue. Roughly:

```tsx
export const AgentViewWrapper = ({ model }: { model: AgentViewModel }): JSX.Element => {
    const block = model.blockAtom;
    const agentId = () => block()?.meta?.["agentId"];
    return (
        <Show when={agentId()} fallback={<AgentPicker model={model} />}>
            <AgentPresentationView model={model} agentId={agentId()} />
        </Show>
    );
};

const AgentPresentationView = ({ model, agentId }: Props): JSX.Element => {
    const block = model.blockAtom;
    const provider = createMemo(() => getProvider(block()?.meta?.["agentProvider"] ?? agentId));
    const agentAtoms = createMemo(() => createAgentAtoms(model.blockId));

    // Hooks — each owns one concern, exposes its own read/write API
    const logs = useLaunchLogs();
    const status = useAgentControllerStatus({ model, provider, logs });
    const history = useHistoryPagination({ blockId: model.blockId, agentAtoms });
    const digest = useSessionDigest({ blockId: model.blockId, logs });
    const bookmarks = useBookmarks({ blockId: model.blockId, block });
    const search = useInSessionSearch({ document: agentAtoms().documentAtom[0] });
    const scroll = useScrollToNode();

    useAgentKeyboard({
        blockId: model.blockId,
        onToggleBookmarks: bookmarks.toggle,
        onToggleSearch: search.toggle,
    });

    // Stream subscription (existing)
    useAgentStream({
        blockId: model.blockId,
        outputFormat: block()?.meta?.["agentOutputFormat"] ?? "claude-stream-json",
        documentAtom: agentAtoms().documentAtom,
        streamingStateAtom: agentAtoms().streamingStateAtom,
        enabled: true,
        documentVersion: history.documentVersion,
    });

    // Top-level handlers — small, stay here
    const handleSendMessage = async (message: string) => { /* … */ };
    const handleBack = async () => { /* … */ };
    const handleSubagentClick = (node: SubagentLinkNode) => { /* … */ };
    const handleContextMenu = (e: MouseEvent) => { /* … */ };
    const zoomFactor = createMemo(() => { /* … */ });

    onCleanup(() => {
        // Aggregate cleanup: each hook has its own onCleanup internally
    });

    return (
        <div class="agent-pane" style={{ zoom: zoomFactor() }} onContextMenu={handleContextMenu}>
            <AgentPresentationHeader block={block} onBack={handleBack} />
            <AgentNotificationStack
                block={block}
                digest={digest}
                onDismissDigest={digest.dismiss}
            />
            <AgentControlBar blockId={model.blockId} blockAtom={block} providerId={provider()?.id ?? ""} />
            <Show when={bookmarks.visible()}>
                <BookmarksPanel {...bookmarks.panelProps} />
            </Show>
            <AgentSearchBar {...search.barProps} />
            <AgentDocumentView
                documentAtom={agentAtoms().documentAtom}
                documentStateAtom={agentAtoms().documentStateAtom}
                logLines={logs.lines}
                authUrl={status.authUrl}
                onSubagentClick={handleSubagentClick}
                onLoadOlder={history.loadOlder}
                loadingOlder={history.loadingOlder}
                startTsMs={history.sessionStartTsMs}
                endTsMs={history.sessionLastActivityMs}
                bookmarkedNodeIds={bookmarks.bookmarkedNodeIds}
                onBookmark={bookmarks.add}
                scrollCommand={scroll.command}
                highlightNodeId={search.highlightId}
            />
            <Show when={status.loginWaiting()}>
                <div class="agent-retry-bar">
                    <button class="agent-retry-btn--cancel" onClick={status.cancelLogin}>Cancel Login</button>
                </div>
            </Show>
            <Show when={status.canRetry()}>
                <div class="agent-retry-bar">
                    <button onClick={status.retry}>Retry Login</button>
                </div>
            </Show>
            <AgentFooter agentId={agentId} onSendMessage={handleSendMessage} loading={status.isLoading()} />
        </div>
    );
};
```

Target size: **~250 lines** vs current 1,182. That's a 4× reduction, mostly from moving state management into hooks.

---

## 3. Principles the refactor enforces

1. **One concern per module.** Each hook owns a single feature (history pagination, digest, bookmarks, search, etc.). Cross-hook references go through return values, not shared mutable lets.
2. **Pure logic out of the component.** Helpers like `nodeSearchText`, `nodePreview`, `formatTimestamp`, `buildAuthEnv` are exported functions in their owning hook file. They're unit-testable in isolation.
3. **No mutable refs crossing component boundaries.** The `let scrollToNodeFn: ((id: string) => void) | null = null` pattern is removed. `useScrollToNode` exposes a `command` signal; `AgentDocumentView` reads it in a `createEffect`. Callers just call `jumpTo(nodeId)` — they don't touch the document view directly.
4. **Pane-scoped side effects stay pane-scoped.** The Ctrl+B/Ctrl+F keyboard hook explicitly takes `blockId` and early-exits if `focusedBlockId() !== blockId`. No global listeners that leak across panes.
5. **Cleanup co-located with setup.** Each hook's `onCleanup` lives inside the hook, next to the `onMount` or subscription that created the resource. No more "look at the other end of the 220-line onMount for the matching onCleanup."
6. **Hooks return a stable API shape.** Every hook returns `{ state accessors, action callbacks }` as a single object. Components destructure what they need. No prop-drilling of 8 separate signals.
7. **Banners don't affect flex layout.** The `AgentNotificationStack` (referenced, not built by this spec) takes a separate PR. This spec just anticipates it in the `agent-view.tsx` target layout.

---

## 4. Extraction order

**Important:** this is NOT a big-bang refactor. Each step is a self-contained extraction that leaves the code in a compilable, shipping state. Order matters because later steps depend on earlier ones.

### Step 1 — Extract `AgentPicker` (~90 lines out)

**Scope:** Move `useForgeAgents` hook and `AgentPicker` component from `agent-view.tsx` into `components/AgentPicker.tsx`. The launch-flow-start handler inside `AgentPicker` still references `runLaunchFlow` from the original file — leave that import for now.

**Verification:** TypeScript clean, agent picker still renders, clicking an agent still calls into the launch flow.

**Risk:** Low. Purely moving code. No logic changes.

### Step 2 — Extract launch flow (~210 lines out)

**Scope:** Move `runLaunchFlow` and its helpers (`buildAuthEnv`, the log helper signature it expects) into `flows/launch-flow.ts`. Export as:

```ts
export interface LaunchFlowContext {
    blockId: string;
    provider: ProviderDefinition;
    log: (tag: string, msg: string, level?: "info" | "error" | "warn") => void;
    onAuthUrl: (url: string) => void;
    onNeedsLogin: () => void;
    onComplete: () => void;
}
export async function runLaunchFlow(ctx: LaunchFlowContext): Promise<"success" | "needs-login" | "fatal"> { /* … */ }
```

`AgentPicker` and `AgentPresentationView` both import from here.

**Verification:** Launch a fresh agent, watch the init-monitor logs flow through. Auth flow still opens the browser. Success still transitions to the presentation view.

**Risk:** Medium. The launch flow has ~15 subscriptions and state transitions. Changing how it's called is invasive. Test against a real agent before shipping.

### Step 3 — Extract `useLaunchLogs` (~15 lines out, enables later steps)

**Scope:** `logLines` signal + `log` helper → `hooks/useLaunchLogs.ts`. Returns `{ lines, append }` as accessors and a callable.

**Verification:** Agent launch logs still appear in the init-monitor panel.

**Risk:** Very low. This is ~15 lines of straightforward state.

### Step 4 — Extract `useAgentControllerStatus` (~40 lines out)

**Scope:** `authUrl`, `canRetry`, `flowRunning`, `agentReady`, `loginWaiting`, `loginCancelled`, `isLoading`, `buildAuthEnv`, `cancelLogin`, `startLaunchFlow` — all into `hooks/useAgentControllerStatus.ts`. Hook returns signal accessors and `{ retry, cancelLogin }` callbacks. `startLaunchFlow` wraps the `flows/launch-flow.ts` function from Step 2 and wires the `on*` callbacks to local state.

**Verification:** Login flow UX unchanged. Auth URL opens browser. Cancel/retry buttons work.

**Risk:** Medium. Auth state is touched by several places and loginCancelled is a mutable `let` that needs to become a ref or signal.

### Step 5 — Extract `useHistoryPagination` (~50 lines out)

**Scope:** `historyOffset`, `historyTotal`, `loadingOlder`, `loadOlder`, `sessionStartTsMs`, `sessionLastActivityMs`, `documentVersion`, `bumpDocumentVersion`, and the initial history-load RPC inside `onMount`. Hook takes `{ blockId, agentAtoms }` and returns the API surface.

**Verification:** Open a historical session, confirm history paginates on scroll-to-top, confirm `session:line_count` and `session:start_ts_ms` are read on mount.

**Risk:** Medium. The initial load is inside the giant `onMount` block and will need to be cleanly separated from the other subscription setups in that same `onMount`.

### Step 6 — Extract `useSessionDigest` (~80 lines out)

**Scope:** `digestSummary`, `digestGeneratedAt`, `digestLoading`, `digestDismissed`, `fetchDigest`, `dismissDigest`, and the auto-trigger `createEffect`. Hook returns `{ summary, generatedAt, loading, dismissed, fetch, dismiss }`.

**Verification:** Session digest still generates on first session completion, banner still renders, regenerate button still works.

**Risk:** Low.

### Step 7 — Extract `useBookmarks` (~70 lines out)

**Scope:** `bookmarks`, `bookmarkedNodeIds`, `showBookmarks`, `saveBookmarks`, `nodePreview`, `handleBookmark`, `handleBookmarkDelete`, `handleBookmarkRename`, `handleBookmarkJump`. Hook takes `{ blockId, block, jumpTo }` — `jumpTo` comes from `useScrollToNode` (Step 9).

**Verification:** Bookmark create/delete/rename/jump all still work. The fix for the `scrollIntoView` ancestor-leakage bug lands as part of Step 9.

**Risk:** Low once Step 9 is in place.

### Step 8 — Extract `useInSessionSearch` (~80 lines out)

**Scope:** `searchVisible`, `searchMatches`, `searchCurrentIndex`, `performSearch`, `searchNext`, `searchPrev`, `searchClose`, `searchHighlightId`, `nodeSearchText`. Hook takes `{ document, jumpTo }` and returns `{ visible, matchCount, currentIndex, highlightId, open, close, next, prev, performSearch }`.

**Verification:** Ctrl+F opens search bar, typing a query highlights matches, Enter/Shift+Enter cycles, Esc closes.

**Risk:** Low.

### Step 9 — Introduce `useScrollToNode` + FIX the scrollIntoView bug

**Scope:** New hook `hooks/useScrollToNode.ts`. Replaces the mutable `scrollToNodeFn` ref with a signal-based command:

```ts
export function useScrollToNode() {
    const [command, setCommand] = createSignal<{ nodeId: string; seq: number } | null>(null);
    let seq = 0;
    const jumpTo = (nodeId: string) => setCommand({ nodeId, seq: ++seq });
    return { command, jumpTo };
}
```

Consumers (`AgentDocumentView`) read `command()` in a `createEffect`, look up the target element inside their own scroll container, and do:

```ts
createEffect(() => {
    const cmd = props.scrollCommand.command();
    if (!cmd || !scrollRef) return;
    const el = scrollRef.querySelector(`[data-node-id="${cmd.nodeId}"]`) as HTMLElement | null;
    if (!el) return;
    const elRect = el.getBoundingClientRect();
    const containerRect = scrollRef.getBoundingClientRect();
    const offsetWithinContainer = elRect.top - containerRect.top + scrollRef.scrollTop;
    const centerOffset = offsetWithinContainer - (scrollRef.clientHeight / 2) + (el.clientHeight / 2);
    scrollRef.scrollTo({ top: centerOffset, behavior: "smooth" });
    // scroll only scrollRef; never walks ancestors.
});
```

`useBookmarks` and `useInSessionSearch` both accept the `jumpTo` callback from this hook and call it instead of holding a ref.

**Verification:** Set a bookmark, click it — pane titles do NOT disappear across the app. Ctrl+F + Enter to next match — pane titles do NOT disappear. Test with multiple agent panes open.

**Risk:** Low — this is strictly removing the scrollIntoView side effect.

**Importance:** This step is the one the user actually reported the bug against. It could ship before the rest of the refactor if we want the fix yesterday. The rest is housekeeping; this is a real bug fix.

### Step 10 — Extract `useAgentKeyboard`

**Scope:** The `window.addEventListener("keydown", …)` for Ctrl+B/Ctrl+F → `hooks/useAgentKeyboard.ts`. Hook takes `{ blockId, onToggleBookmarks, onToggleSearch }` and owns the pane-scoped early-exit via `focusedBlockId()`.

**Verification:** Ctrl+B still toggles bookmarks in the focused pane and does nothing elsewhere. Same for Ctrl+F.

**Risk:** Very low.

### Step 11 — Extract `AgentPresentationHeader` component

**Scope:** The `.agent-pres-header` JSX block → `components/AgentPresentationHeader.tsx`. Takes `{ block, onBack }`.

**Verification:** Visual equivalence.

**Risk:** Very low.

### Step 12 — Collapse the remaining agent-view.tsx

**Scope:** With all the hooks in place, agent-view.tsx should be ~250 lines. Remove any dead imports, dead helper vars, collect the cleanup into a single top-level `onCleanup`. Final TypeScript + lint pass.

**Verification:** Portable build + smoke test + manual agent launch.

**Risk:** Low — this is cleanup after the surgery.

---

## 5. Out of scope for this refactor

- **Deleting features.** Bookmarks, search, digest, timeline minimap, etc. stay as they are — this is a structural refactor, not a feature cull. A separate conversation covers whether the user wants them at all.
- **`AgentNotificationStack` component.** Referenced in the target structure but deferred to a separate PR (see `docs/analysis/agent-pane-rich-features-structure-2026-04-13.md` §4.1).
- **Changes to `AgentDocumentView.tsx` beyond the `scrollIntoView` fix in Step 9.** That file has its own issues (357 lines, several concerns) but it's in decent shape and can be refactored later.
- **Changes to `agent-model.ts`, `stream-parser.ts`, `init-monitor.ts`, `types.ts`.** All are within reasonable size.
- **CSS reorganization.** `agent-view.scss` is ~1500 lines. Worth splitting but separate effort.
- **Any backend changes.** Pure frontend refactor.

---

## 6. How to validate each step

For each PR in the extraction sequence:

1. **TypeScript clean:** `npx tsc --noEmit` with no new errors in agent/ directory.
2. **Smoke test:** Launch a freshly built portable, open an agent pane, run a full launch flow, confirm no visual regressions (`smoke-test-portable.cjs` was removed).
3. **Manual agent launch:** open the portable, pick an agent, run a full launch flow (CLI resolve → auth → spawn → first message → response). Every refactor step must not break this end-to-end path.
4. **Reagent review:** every PR goes through reagent. Expect ≥1 round of fixes per PR; the extraction touches cross-cutting state and review will catch things like missing cleanup or misplaced effects.
5. **Size budget:** after Step 12, `wc -l frontend/app/view/agent/agent-view.tsx` must be ≤ 300.

---

## 7. Estimated cost

| Step | Est. active time | Review rounds |
|---|---|---|
| 1. AgentPicker extract | 20 min | 0–1 |
| 2. Launch flow extract | 60 min | 1–2 |
| 3. useLaunchLogs | 10 min | 0 |
| 4. useAgentControllerStatus | 60 min | 1–2 |
| 5. useHistoryPagination | 45 min | 1 |
| 6. useSessionDigest | 30 min | 0–1 |
| 7. useBookmarks | 30 min | 0–1 |
| 8. useInSessionSearch | 30 min | 0–1 |
| 9. useScrollToNode + **scrollIntoView bug fix** | 30 min | 0–1 |
| 10. useAgentKeyboard | 15 min | 0 |
| 11. AgentPresentationHeader | 10 min | 0 |
| 12. Cleanup pass | 20 min | 0 |
| **Total** | **~6 hours** | **4–10** |

Realistic wall-clock: 2–3 days if we serialize through reagent reviews. Step 9 is the critical-path bug fix and can ship out-of-order on its own branch if we want the fix immediately.

---

## 8. Why this specific ordering

- **AgentPicker first** (Step 1) because it has the fewest cross-component dependencies — once extracted, the rest of the file doesn't care about the picker at all.
- **Launch flow second** (Step 2) because it's the largest blob, unblocking Steps 3-4 which depend on removing the launch-flow functions from `AgentPresentationView`.
- **useLaunchLogs → useAgentControllerStatus → useHistoryPagination** because each subsequent hook depends on the log interface of the previous one.
- **useSessionDigest** separate from useHistoryPagination because digest only needs `blockId` and `log`, not the history cursor state.
- **useScrollToNode** before **useBookmarks** and **useInSessionSearch** so those hooks don't need a mutable ref.
- **useAgentKeyboard** last among the hooks because it depends on bookmarks and search being extracted (to have `toggleBookmarks` and `toggleSearch` callables).
- **AgentPresentationHeader** and **cleanup pass** at the end — pure cosmetics.

---

## 9. What success looks like

After all 12 steps:

- `agent-view.tsx` is ≤ 300 lines, entirely composition + JSX + 4-5 local handlers.
- Every feature lives in its own file. Changing bookmarks does not touch the search file.
- The `scrollIntoView` ancestor-leakage bug is gone (Step 9).
- Each hook is unit-testable: `useBookmarks`, `useInSessionSearch`, `useSessionDigest`, `useHistoryPagination` can all be mounted in isolation with a mock `blockId` and tested for state transitions.
- Adding a new feature (e.g. a new banner, a new keyboard shortcut) means writing a new hook file — not adding another 50 lines to `agent-view.tsx`.
- Merge conflicts on the agent pane become rare: two PRs can modify `useBookmarks.ts` and `useInSessionSearch.ts` simultaneously with zero conflict.

---

## 10. Non-goals / explicit pushback on scope creep

- **We don't rewrite `AgentPresentationView` from scratch.** We extract in place. Big-bang rewrites are high-risk; the incremental sequence is boring on purpose.
- **We don't add features during the refactor.** Every step is "move this code" or "replace this pattern with a cleaner equivalent." No new UI, no new behavior, no new signals that didn't already exist. Only Step 9 changes behavior (fixing the scroll bug).
- **We don't rename files or directories randomly.** New files are added; existing files stay where they are. That keeps `git blame` useful.
- **We don't touch tests that still pass.** `state.test.ts`, `stream-parser.test.ts` continue to work as-is.
- **We don't address every code-quality issue in the directory.** `SetupWizard.tsx` is 437 lines and could be split too; `agent-model.ts` is 423 and has its own issues. Both are stable and working. Leave them.
