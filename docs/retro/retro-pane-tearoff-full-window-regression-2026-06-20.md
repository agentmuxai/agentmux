# Retro: Pane Tear-Off Produces Full Window With Tabs

**Date**: 2026-06-20  
**Severity**: P1 regression — core floating-pane UX broken  
**Introduced by**: PR #1610 — Pane tear-off pool (macOS/Linux), merged ~2026-06-19  
**Fixed in**: `frontend/app/app.tsx` — move `IS_FLOATING_PANE` evaluation from module load to component mount  
**Reported by**: User (manual testing immediately after pool work)

---

## Symptom

After PR #1610 landed, dragging a pane out of the main window (pane tear-off) produced a **full AgentMux window** — tab bar, widget bar, status bar — instead of the expected **chromeless floating pane** (just the pane content, no chrome). The floating-pane UX was completely broken on the fast (pool) path on macOS and Linux.

---

## Root Cause

### Background

Floating-pane mode is rendered via a branch in `AppInner` (app.tsx):

```tsx
<Show when={IS_FLOATING_PANE} fallback={<Workspace />}>
    <FloatingPaneWorkspace />
</Show>
```

`IS_FLOATING_PANE` was a **module-level IIFE constant**:

```typescript
const IS_FLOATING_PANE: boolean = (() => {
    try {
        return new URLSearchParams(window.location.search).has("floatingPaneId");
    } catch {
        return false;
    }
})();
```

The comment above it read: *"the URL never changes in-flight for an AgentMux renderer"*.

### Before PR #1610 (cold path)

`open_floating_pane_window` created a **new** CEF window with `?floatingPaneId=...` already in the URL from the start. Module load → IIFE fires → URL already has `floatingPaneId` → `IS_FLOATING_PANE = true`. Correct.

### After PR #1610 (pane pool path)

PR #1610 introduced a pre-warmed pane pool window for fast tear-off. The pool window is spawned with `?pane-pool=1` in the URL — **no `floatingPaneId`** at spawn time, since the pane being torn off isn't known yet.

The bootstrap sequence for a pool window is:

```
bootstrap → initApp → isPanePoolMode() → awaitPanePoolPromote()
    → [waits for pool:pane-promote event]
    → replaceState: adds floatingPaneId + workspaceId, removes pane-pool=1
    → initHostNewWindow() → initWaveWrap() → initWave() → render(App, elem)
```

The problem: `app.tsx` is imported at **program start** (it's a static `import` in `app-init.ts`). When `app.tsx` is first parsed, the IIFE fires immediately — the URL still has `?pane-pool=1`, not `?floatingPaneId`. So `IS_FLOATING_PANE = false`.

By the time `render(App, elem)` is called (after `awaitPanePoolPromote` updated the URL), `IS_FLOATING_PANE` is already frozen as `false`. The App renders `<Workspace>` — full chrome — forever, no matter what the URL now says.

### Why the assumption was valid before

The comment was correct for all pre-pool paths: new CEF windows have their final URL from the host at creation time and never navigate. The module-level IIFE was a reasonable micro-optimization.

PR #1610 violated the assumption by making the URL mutable (via `replaceState`) as part of the pool promote handshake. The IIFE fires at module parse time, which is always before any async pool coordination.

---

## Fix

Remove the module-level IIFE. Move the URL check inside `AppInner`, which is a SolidJS component function called at render time — after `awaitPanePoolPromote()` has updated the URL and after `render(App, elem)` is called.

**`frontend/app/app.tsx`**:

```typescript
// BEFORE (module level, fires at parse time):
const IS_FLOATING_PANE: boolean = (() => {
    try {
        return new URLSearchParams(window.location.search).has("floatingPaneId");
    } catch {
        return false;
    }
})();

const AppInner = () => {
    ...
```

```typescript
// AFTER (inside component, fires at render time):
const AppInner = () => {
    // Evaluated at component-mount time so the pane pool fast path works:
    // awaitPanePoolPromote() calls replaceState() to inject floatingPaneId
    // into the URL BEFORE initHostNewWindow() calls render(App) — but a
    // module-level IIFE fires before any of that and always sees ?pane-pool=1.
    const IS_FLOATING_PANE = new URLSearchParams(window.location.search).has("floatingPaneId");
    ...
```

This is safe because SolidJS component functions run once at mount, not on every render cycle — the URL is read exactly once per window lifetime, as before.

---

## Timeline

| Step | What happened |
|------|---------------|
| Pre-PR #1610 | Cold `open_floating_pane_window` creates window with `?floatingPaneId=` in URL from the start. IIFE fires at module load with the final URL. Works. |
| PR #1610 merges | Pane pool added. Pool windows start with `?pane-pool=1`. `awaitPanePoolPromote` updates URL via `replaceState` before `render()`. But IIFE already fired with wrong URL. |
| User tests | Every pane tear-off produces a full window instead of a floating pane. |
| Root cause found | Module-level IIFE vs. runtime URL mutation — the pool path mutates the URL between module parse and `render()`. |
| Fix | Move IIFE evaluation inside `AppInner` component body. |

---

## Contributing Factors

1. **Stale assumption in code comment**: "the URL never changes in-flight" was true for all pre-pool paths and reasonable as an optimization, but became a liability when PR #1610 introduced URL mutation via `replaceState` during the pool promote handshake.

2. **Implicit coupling across layers**: `pool.ts:awaitPanePoolPromote()` has a comment saying *"FloatingPaneWorkspace renders because floatingPaneId is in the URL — same code path as the cold-start"*. This comment was right about the intent but wrong about the mechanics — it didn't account for the module-level IIFE.

3. **No integration test for pool promote path**: The floating-pane render branch (`IS_FLOATING_PANE`) had no automated test that exercised the pool promote flow end-to-end. A test that verified `<FloatingPaneWorkspace>` renders after a `pool:pane-promote` event (with the URL mutation) would have caught this immediately.

---

## Lessons

- **Module-level IIFEs that read browser state (URL, DOM) are fragile** when async init sequences mutate that state. Prefer lazy evaluation at component mount time, or use a reactive signal that can be updated.
- **URL mutation via `replaceState` is not free**: callers need to audit what module-level code has already captured a snapshot of the URL.
- **Comments about invariants should list their assumptions explicitly**: "the URL never changes" is an invariant — any PR that introduces URL mutation in the bootstrap path should verify no module-level snapshot of the URL exists.
- **Pool promote paths need end-to-end render tests**: the pool fast path and cold path have subtly different initialization orders. Both should be covered by a test that asserts the correct component (`<FloatingPaneWorkspace>` vs `<Workspace>`) is rendered.
