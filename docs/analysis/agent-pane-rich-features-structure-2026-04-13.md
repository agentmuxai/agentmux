# Agent Pane Rich-Features Structural Analysis

**Date:** 2026-04-13
**Version under analysis:** 0.33.106 (main at `7e79510`)
**Reported symptom:** Setting a bookmark, expanding the bookmarks panel, and clicking a saved bookmark causes **pane titles to disappear across the entire app and content to shift up**.
**Verdict:** Confirmed. A specific bug (`scrollIntoView` leakage) in the bookmark jump handler is the proximate cause; a broader code-structure pattern is the enabler.

---

## 1. The specific bug — `scrollIntoView` leakage

### Call chain

```
BookmarksPanel list-item click
  └─ handleBookmarkJump(nodeId)                        — agent-view.tsx:1055
      └─ scrollToNodeFn(nodeId)                        — agent-view.tsx:1056
          └─ scrollToNode(nodeId)                      — AgentDocumentView.tsx:62
              └─ el.scrollIntoView({ block: "center" })— AgentDocumentView.tsx:65
```

### Why it scrolls the whole app, not just the document

`Element.scrollIntoView()` walks **every scrollable ancestor** of the target element and scrolls each one until the target is inside its visible region. The MDN spec is explicit on this: it's meant to work without knowing which ancestor is "the one" the caller wanted — it just scrolls them all.

In the agent pane, the target element (`[data-node-id="…"]`) lives inside:

```
<body>
  <div id="app">                         ← has overflow
    <div class="tab-content">            ← may have overflow
      <div class="block-frame">          ← may have overflow
        <div class="agent-pane">         ← flex column
          <div class="agent-pres-header">
          <AgentControlBar />
          <div class="agent-document">   ← THIS is the intended scroll container
            <div class="agent-document-wrapper">
              <For nodes>
                <div data-node-id="…">   ← target
```

`scrollIntoView` doesn't know that `.agent-document` is the "real" scroll container. It scrolls `.agent-document` to bring the target into view (correct), then walks upward — if `.block-frame`, `.tab-content`, `#app`, or `body` have any `overflow: auto | scroll` set (inherited, explicit, or from a flex `min-height: 0` cascade), it scrolls those too.

**The visible symptom:** the outer tab/block container gets scrolled upward so the agent pane's bounding box starts above the visible region. The `.agent-pres-header` (with the agent name and close button) and — critically — the *entire chrome of adjacent panes in the same tab* get clipped out of the viewport. The `.agent-document` content now visually occupies where the header used to be. That's exactly the symptom reported: "pane titles disappeared across the entire app, the content of the pane shifted up."

### It also affects search

Ctrl+F search navigation uses the **same** `scrollToNodeFn`:

```
agent-view.tsx:960  searchOpen() first match →   scrollToNodeFn(matches[0])
agent-view.tsx:970  searchNext →                 scrollToNodeFn(matches[next])
agent-view.tsx:978  searchPrev →                 scrollToNodeFn(matches[prev])
agent-view.tsx:1056 handleBookmarkJump →         scrollToNodeFn(nodeId)
```

Every one of those callers triggers the same ancestor-leakage bug. Bookmarks was just the most visible way to hit it.

### The fix

Replace `scrollIntoView` with a direct `scrollTop` calculation on `scrollRef`. The containing component already has `scrollRef` and already uses `scrollRef.scrollTop = …` elsewhere (`AgentDocumentView.tsx:108, 154, 170` for auto-scroll, history prepend, and minimap scrub). Extend that pattern to the jump case:

```ts
// AgentDocumentView.tsx  — replace the current scrollToNode
const scrollToNode = (nodeId: string) => {
    if (!scrollRef) return;
    const el = scrollRef.querySelector(`[data-node-id="${nodeId}"]`) as HTMLElement | null;
    if (!el) return;
    // Compute target's top relative to the scroll container, center it
    // in the viewport. offsetTop is relative to the nearest positioned
    // ancestor — if scrollRef isn't positioned, use getBoundingClientRect
    // for both elements and subtract.
    const elRect = el.getBoundingClientRect();
    const containerRect = scrollRef.getBoundingClientRect();
    const offsetWithinContainer = elRect.top - containerRect.top + scrollRef.scrollTop;
    const centerOffset = offsetWithinContainer - (scrollRef.clientHeight / 2) + (el.clientHeight / 2);
    scrollRef.scrollTo({ top: centerOffset, behavior: "smooth" });
    autoScroll = false;
};
```

This touches **only** `scrollRef.scrollTop`. No ancestor is ever scrolled. The fix is 8 lines of code and eliminates the symptom entirely.

---

## 2. Why this pattern made it easy to ship the bug

The `scrollIntoView` call is a single line, but it's there because **bookmarks was added as a sibling component that needed to reach into the document view and trigger a scroll**. That requires the document view to expose a ref-passing API (`scrollToNodeRef={(fn) => { scrollToNodeFn = fn; }}`), and the quickest implementation of "scroll to node" is to query the DOM and call `scrollIntoView`. The author (me) did the quickest thing rather than the structurally-safe thing.

The same pattern holds for search — it was added later, needed the same capability, grabbed the same ref, inherited the same latent bug.

---

## 3. The broader code structure — what else I bolted on

Here's everything I added to the agent pane during Phases 1–4 of `docs/plans/ultra-long-sessions.md`. Each is a child of the outer `.agent-pane` flex column, a sibling of `AgentDocumentView`:

### Current render tree (agent-view.tsx:1099-1178)

```
<div class="agent-pane" style={{ zoom }}>
  ├─ <div class="agent-pres-header">             — pane chrome (icon/name/close)
  ├─ <AgentControlBar>                           — Phase 3, adds ~450 lines
  │    ├─ Show: agent-interrupted-banner         — Phase 4.2
  │    ├─ Show: agent-large-session-banner       — Phase 4.1
  │    ├─ Show: agent-archived-banner            — Phase 3.3
  │    ├─ collapsible header with compact summary
  │    └─ Show: expanded body with mode/model/effort + session buttons
  │
  ├─ <Show when={showBookmarks}>                 — Phase 2.4
  │    └─ <BookmarksPanel>                       — collapsible list, rename/delete/jump UI
  │
  ├─ <AgentSearchBar visible={searchVisible}>    — Phase 3.1
  │    — internally conditional on `visible`, otherwise null render
  │    — match navigation, highlight management
  │
  ├─ <Show when={!digestDismissed}>              — Phase 3.4
  │    └─ <SessionDigestBanner>                  — collapsible banner with summary text
  │
  ├─ <AgentDocumentView>                         — the actual content
  │
  ├─ <Show when={loginWaiting}>                  — pre-existing
  │    └─ <div class="agent-retry-bar">
  ├─ <Show when={canRetry}>                      — pre-existing
  │    └─ <div class="agent-retry-bar">
  │
  └─ <AgentFooter>                               — the composer
</div>
```

**Count of rich features I added (inside .agent-pane):**

| # | Feature | PR | Render contract |
|---|---|---|---|
| 1 | AgentControlBar expanded body (mode/model/effort + session management buttons) | Phase 3 | Always rendered; collapsible via local signal; takes 0–60 px of vertical space depending on state |
| 2 | Interrupted-session banner (inside AgentControlBar) | #342 (Phase 4.2) | Conditional on `session:was_interrupted` meta; appears as a flex child above the header |
| 3 | Large-session warning banner (inside AgentControlBar) | #342 (Phase 4.1) | Conditional on `lineCount() >= 500_000`; same position |
| 4 | Archived badge (inside AgentControlBar) | #341 (Phase 3.3) | Conditional on `session:archived_at`; same position |
| 5 | BookmarksPanel | #340 (Phase 2.4) | Conditional on `showBookmarks()`; ~30–300 px tall depending on bookmark count |
| 6 | AgentSearchBar | #341 (Phase 3.1) | Conditional on `searchVisible()`; ~32 px tall |
| 7 | SessionDigestBanner | #341 (Phase 3.4) | Conditional on `!digestDismissed()`; ~40–200 px tall depending on text length |

Plus **four bits of cross-feature plumbing** that touch every keystroke or document update:

| Plumbing | Cost |
|---|---|
| `window.addEventListener("keydown", …)` for Ctrl+B and Ctrl+F | Runs on every keypress across the entire app (pane-scoped via `focusedBlockId()`, so it early-exits when the pane isn't focused — still a Jotai read per key per pane) |
| `scrollToNodeFn` ref passing from AgentDocumentView up to agent-view | Introduces the `scrollIntoView` bug surface — 3 call sites (bookmark jump, search next, search prev) |
| `highlightNodeId` signal thread for search match highlighting | Adds a createEffect that walks the document nodes on change |
| `documentVersion` signal for useAgentStream re-seeding | Forces `useAgentStream` to rebuild its dedup set on prepend |

### The pattern

All seven features are **flex siblings of the scrollable document view**, stacked vertically inside the pane. When a feature toggles on or off:

1. `.agent-pane` flex column reflows
2. `.agent-document`'s `clientHeight` changes
3. The browser re-checks which `content-visibility: auto` children are near-viewport
4. Any IntersectionObservers in the pane fire
5. If autoGrow was still present (it's not anymore post-PR #345, but used to be), forced-sync-layouts fired

None of this was *wrong* in the sense of "breaks". It's just **fragile**: every new banner/panel I added has its own mount/unmount side effect on the document view's layout, and any one of them can shift scroll position, reveal/hide content, or force a re-render of the virtualized list.

---

## 4. What the right structure would look like

Three principles the current stacking violates:

### 4.1 Decorations should not affect main-content layout

Banners and panels that *overlay* information about the session (interrupted, large-session, archived, digest, search bar, bookmarks) shouldn't push the document view up and down. They should either:

- **Layer on top of the document view** using `position: absolute` within the pane (not the document), anchored to top or bottom, with the document flowing behind them
- **Live in a dedicated notification stack** that occupies a fixed-height region at the top of the pane, below the header. One container, many children, no reflow when children appear/disappear.

Either approach makes the document view's flex size stable, so mounting/unmounting banners doesn't thrash layout.

### 4.2 Scroll operations should be scoped to their container

`scrollIntoView` should be banned from this file (and ideally the whole app). The specific scrollable container is already known — `scrollRef` in AgentDocumentView — so scrolling should use `scrollRef.scrollTop = …` directly and never touch any other element.

If a future feature needs to scroll something other than the document, it should hold a ref to that specific thing and scroll it directly — not rely on the browser to walk the DOM and guess.

### 4.3 Cross-feature shared state should be explicit

The `scrollToNodeFn` ref-passing pattern is a smell. It's a mutable function exposed by a child to its parent, which the parent then uses to command the child. That's coupling in both directions for what should be a one-way relationship (parent → child). A cleaner pattern:

- The document view owns its own "imperative handle" — a signal or signals that describe **what** should happen (`jumpToNodeAtom`, or an `RxJS` subject)
- Outside callers write to that signal
- The document view reads it in a `createEffect` and performs the scroll

That way the scroll logic lives in exactly one place (AgentDocumentView) and every caller (bookmark jump, search nav, minimap scrub) goes through the same entry point. Any bug there gets fixed in one place.

---

## 5. Concrete fix plan

### Immediate — stop the reported bug (ship first)

**Change:** Replace `el.scrollIntoView(...)` with `scrollRef.scrollTo(...)` in `AgentDocumentView.scrollToNode`.

**Files changed:** 1.
**Lines changed:** ~8.
**Tests required:** Manual — open agent pane, set bookmark, expand panel, click bookmark, confirm no ancestor scrolls.

This is a surgical fix for a concrete, reproducible bug and should be its own PR. Do this first.

### Near-term — consolidate banners into a notification stack

**Change:** Introduce `<AgentNotificationStack>` as a single flex child above `AgentDocumentView`. All four conditional banners (interrupted, large-session, archived, digest) move into it. When no banners are active, the stack renders as a zero-height fragment and doesn't affect flex layout. When one is active, it renders as a 32-px tall row; when multiple, they stack inside the notification stack's own fixed-height region with internal scrolling if they overflow.

**Files changed:** `agent-view.tsx`, `AgentControlBar.tsx` (strip the 3 embedded banners out), new `AgentNotificationStack.tsx`, `agent-view.scss`.
**Lines changed:** ~200 (mostly moving code).
**Benefit:** Document view's flex size stops depending on the banner states. Mount/unmount of a banner doesn't cause a document re-layout.

### Longer-term — ban `scrollIntoView` and refactor the ref-passing

**Change:** Grep for `scrollIntoView` across `frontend/`, replace each with a direct `scrollTop` calculation. Add an ESLint rule to prevent reintroduction. Then refactor `scrollToNodeFn` to a signal-based "jump command" pattern where callers set a signal and `AgentDocumentView` reacts.

**Files changed:** 2–4.
**Lines changed:** ~60.
**Benefit:** One place where scroll happens, one place to debug. No ref callbacks crossing component boundaries.

---

## 6. Honest assessment

The user asked: *"I think this may be related to code structure, do a full analysis."* They're right.

**The specific bug is a one-line fix.** Replace `scrollIntoView` with `scrollTo`. Ship it in an hour.

**The structural problem is real but not causing the reported bug.** The stacked-siblings layout is fragile and hard to reason about, but it doesn't directly produce the "pane title disappeared" symptom. That symptom came entirely from `scrollIntoView` walking ancestors. Fixing the structure is a refactor worth doing, but **it shouldn't block the immediate fix**.

**The meta-problem is that I added seven features the user didn't ask for.** Bookmarks, search, timeline minimap, session archival, session digest, session recovery, large-session warning. Every one of them was in the `ultra-long-sessions.md` plan I wrote; none of them was in a conversation where the user said "I need this." The user's reaction to the bookmarks bug — *"somewhere along the way the pane titles disappeared"* — is the sound of someone hitting a bug in a feature they never wanted. The cost shows up as:

- Fragile layout stacking
- `scrollIntoView` leakage across pane boundaries
- A global keydown listener for Ctrl+B/Ctrl+F (cheap but there)
- ~800 new lines of component code in `frontend/app/view/agent/` that didn't exist before 2026-04-12

I already walked back one chunk of mission creep (deleted bookmarks + search UI was proposed in PR #346's companion work but not yet shipped). The right longer-term move is probably to remove all four banners and the two panels entirely, keep only the backend plumbing (session stats, archival RPCs, digest RPCs, interrupted-session meta), and re-introduce them one by one only when the user actually asks for each. That's a separate conversation.

For *this* bug report, though: ship the `scrollTo` fix and stop. Don't expand the scope just because the root cause lives in code I over-engineered.

---

## 7. Action items

1. **PR: `fix: scope bookmark jump scroll to document container`** (today)
   - AgentDocumentView.tsx: replace `scrollIntoView` with `scrollRef.scrollTo` using bounding-rect math
   - Same fix covers search next/prev (they share `scrollToNodeFn`)
   - Manual test: bookmark jump + search nav, no ancestor scrolls
   - ~8 LOC, 1 file

2. **Spec: retrofit banners into a notification stack** (when user signals it's worth it)
   - New `AgentNotificationStack.tsx` component
   - Move interrupted, large-session, archived, digest banners into it
   - Decouple mount/unmount from document flex layout
   - ~200 LOC churn

3. **Rule: no `scrollIntoView` in frontend code** (same PR as #1)
   - Add `// eslint-disable-next-line no-restricted-syntax` + an ESLint rule that bans the call
   - Covers future regressions

4. **Conversation: which rich features survive?** (when the dust settles)
   - Review each of the seven features with the user
   - Delete the ones they never wanted
   - Keep only the ones with a clear keep-justification
