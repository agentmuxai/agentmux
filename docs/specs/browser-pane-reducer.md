# Browser pane: migrate to reducer architecture

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-07
**Related:** [`browser-pane-title-favicon.md`](./browser-pane-title-favicon.md), [`reference_master_reducer_status.md`](../../) (memory pointer)
**Pattern reference:** `frontend/app/store/agent-pane-state/{reducer,types}.ts`

## Problem

The browser pane's state is currently scattered across six independent SolidJS signals on `BrowserViewModel` plus a parallel write to `block.meta`:

```ts
private _url       = createSignal<string>("");
private _title     = createSignal<string>("Browser");
private _faviconUrl= createSignal<string>("");
private _loading   = createSignal<boolean>(false);
private _canGoBack = createSignal<boolean>(false);
private _canGoForward = createSignal<boolean>(false);
private _error     = createSignal<string | null>(null);
```

Each IPC event handler (`browser-pane-nav-state`, `browser-pane-title-change`, `browser-pane-clicked`) directly imperatively mutates two-or-more of these signals. The constructor does the same on `navigate()`. Result:

- **No single source of truth.** State transitions are spread across the constructor, three event handlers, and the public `navigate/goBack/goForward/reload` methods.
- **No invariants.** Nothing prevents `loading=true` + `error="…"` simultaneously, or `canGoBack=true` while `url=""` (loading state).
- **Hard to test.** The current test file only verifies lifecycle gating (`closed` flag); the actual state-transition logic is untested because there's no pure function to invoke.
- **Fragile IPC ordering.** `url_only` events from `on_load_end_pane` deliberately skip back/forward updates because they'd be stale; this rule is enforced inline as a comment + branch instead of being a reducer invariant.
- **Race conditions.** The post-spec race fix (deferring `navigate()` until subscriptions register) is band-aid for a fundamental issue: the constructor's `navigate()` is itself a state transition that should flow through the same reducer.
- **Dual-write to meta.** `RpcApi.SetMetaCommand({url})` is called from the nav-state handler. Meta is then read again on next pane construction. The split means a quick reload could read stale meta while the reducer holds newer state.

The `agent-pane-state` slice is the in-tree exemplar of the reducer pattern. Migrating browser pane state to that pattern centralizes the rules, exposes them to unit testing, and removes the ad-hoc mutation paths.

## Goals

1. **Pure reducer** for all browser-pane state transitions: `update(state, command) → { state, events }`. Same shape as `agent-pane-state/reducer.ts`.
2. **Single state object** replacing the six signals. Wrapped in `createSignal<BrowserPaneState>` so consumers (URL bar, favicon, header) read derived values via `createMemo`.
3. **Saga (or thin orchestrator) handles side effects**: IPC subscriptions, IPC commands (`browser_pane_navigate` etc.), meta persistence (`SetMetaCommand`), focus ops. Saga turns external events into commands; reducer-emitted events are fed back to the saga to dispatch IPC calls.
4. **No regression in current behavior.** Race fix preserved. Title + favicon update on nav. URL bar reflects nav-state. Back/forward gated by CEF history.
5. **Unit tests** cover every transition in `reducer.test.ts` (no model construction needed — pure function).
6. **Smaller `BrowserViewModel`.** It becomes the saga's view-facing host: signals derived from reducer state, public methods that dispatch commands, IPC subscriptions deferred until ack.

## Non-goals

- No change to host-side IPC events or their payload shapes.
- No persistence model overhaul. Meta writes still happen for restore-on-reopen, just gated through the reducer's emitted events.
- No batching/throttling of state updates (defer to a later PR if needed).
- No cross-pane state coordination (each pane keeps its own reducer instance).

## State shape

```ts
// frontend/app/store/browser-pane-state/types.ts

export interface BrowserPaneState {
    /** Stable block id; immutable for the lifetime of the reducer. */
    readonly blockId: string;

    /** Current URL — may be the loading target, may be the post-redirect committed URL. */
    url: string;

    /** Page <title> from CEF; fallback "Browser" when empty. */
    title: string;

    /** Derived favicon URL; empty → header renders globe. */
    faviconUrl: string;

    /** Loading phase: any in-flight nav between NavigateRequested and NavStateReceived. */
    loading: boolean;

    /** Last error message; null when no error. Mutually exclusive with loading=true. */
    error: string | null;

    /**
     * History gates from CEF's `on_loading_state_change`. Note: nav-state
     * events with url_only=true (from `on_load_end`) deliberately do NOT
     * update these — see Invariant #4 below.
     */
    canGoBack: boolean;
    canGoForward: boolean;

    /** Once flipped true (Disposed command), all subsequent commands are no-ops. */
    closed: boolean;
}

export const initialState = (blockId: string, defaultUrl: string): BrowserPaneState => ({
    blockId,
    url: defaultUrl,
    title: "Browser",
    faviconUrl: "",
    loading: false,
    error: null,
    canGoBack: false,
    canGoForward: false,
    closed: false,
});
```

### Invariants enforced by the reducer

1. **Closed is terminal.** Any command other than `Disposed` is a no-op when `state.closed === true`.
2. **block_id filter at the boundary, not the reducer.** Saga drops events for the wrong block_id before calling `update()`. The reducer trusts that every command it sees is for its block.
3. **`error` and `loading` are mutually exclusive.** Setting one clears the other.
4. **`url_only` nav-state events do not update history gates.** They arrive from `on_load_end` before CEF commits the navigation controller, so `canGoBack`/`canGoForward` would be stale (kimi's race finding). The reducer ignores those fields when `url_only=true`.
5. **Favicon derivation is part of the reducer**, not the saga. `URL.origin || "null"` → `${origin}/favicon.ico` or `""`.
6. **Title falls back to "Browser"**, applied at the reducer (not at the view).
7. **NavigateRequested clears favicon and error, sets `loading=true`, preserves title.** Avoids "Browser" flash mid-load (per the title/favicon spec).

## Commands

```ts
export type BrowserPaneCommand =
    // Issued by the model's public methods (URL bar submit, history buttons).
    | { type: "NavigateRequested"; url: string }
    | { type: "BackRequested" }
    | { type: "ForwardRequested" }
    | { type: "ReloadRequested" }
    | { type: "Disposed" }

    // Issued by the saga in response to host IPC events.
    | { type: "NavStateReceived"; url: string; canGoBack?: boolean; canGoForward?: boolean; urlOnly: boolean }
    | { type: "TitleChangeReceived"; title: string }
    | { type: "LoadError"; message: string }
    | { type: "Clicked" };
```

## Events emitted

```ts
export type BrowserPaneEvent =
    /** Saga: invoke `browser_pane_navigate` IPC with the normalized URL. */
    | { type: "ipc-navigate"; url: string }
    /** Saga: invoke `browser_pane_go_back` IPC. */
    | { type: "ipc-back" }
    /** Saga: invoke `browser_pane_go_forward` IPC. */
    | { type: "ipc-forward" }
    /** Saga: persist URL to block.meta.url so pane restore lands on the latest. */
    | { type: "meta-persist-url"; url: string }
    /** Saga: refocus this block in the layout (click → focus). */
    | { type: "focus-block" }
    /** Saga: stop emitting / unsubscribe (paired with Disposed). */
    | { type: "shutdown" };
```

## Reducer skeleton

```ts
// frontend/app/store/browser-pane-state/reducer.ts
import type { BrowserPaneCommand, BrowserPaneEvent, BrowserPaneState } from "./types";

export interface ReducerResult { state: BrowserPaneState; events: BrowserPaneEvent[]; }

const ALLOWED_AFTER_CLOSED = new Set<BrowserPaneCommand["type"]>(["Disposed"]);

function deriveFavicon(url: string): string {
    try {
        const origin = new URL(url).origin;
        return origin && origin !== "null" ? `${origin}/favicon.ico` : "";
    } catch {
        return "";
    }
}

function normalize(url: string): string {
    let n = url.trim();
    if (!n) return n;
    if (!/^https?:\/\//i.test(n) && !n.startsWith("about:")) {
        n = n.includes(".") && !n.includes(" ")
            ? `https://${n}`
            : `https://www.google.com/search?q=${encodeURIComponent(n)}`;
    }
    return n;
}

export function update(state: BrowserPaneState, cmd: BrowserPaneCommand): ReducerResult {
    if (state.closed && !ALLOWED_AFTER_CLOSED.has(cmd.type)) {
        return { state, events: [] };
    }
    switch (cmd.type) {
        case "NavigateRequested": {
            const normalized = normalize(cmd.url);
            if (!normalized) return { state, events: [] };
            return {
                state: {
                    ...state,
                    url: normalized,
                    error: null,
                    loading: true,
                    faviconUrl: "",
                    // title intentionally preserved (avoids "Browser" flash)
                },
                events: [
                    { type: "ipc-navigate", url: normalized },
                    { type: "meta-persist-url", url: normalized },
                ],
            };
        }
        case "NavStateReceived": {
            const newState: BrowserPaneState = {
                ...state,
                url: cmd.url,
                faviconUrl: deriveFavicon(cmd.url),
                loading: false,
                error: null,
            };
            if (!cmd.urlOnly) {
                if (cmd.canGoBack !== undefined) newState.canGoBack = cmd.canGoBack;
                if (cmd.canGoForward !== undefined) newState.canGoForward = cmd.canGoForward;
            }
            return {
                state: newState,
                events: [{ type: "meta-persist-url", url: cmd.url }],
            };
        }
        case "TitleChangeReceived":
            return {
                state: { ...state, title: cmd.title || "Browser" },
                events: [],
            };
        case "BackRequested":
            return {
                state: { ...state, loading: true, error: null },
                events: [{ type: "ipc-back" }],
            };
        case "ForwardRequested":
            return {
                state: { ...state, loading: true, error: null },
                events: [{ type: "ipc-forward" }],
            };
        case "ReloadRequested":
            return {
                state: { ...state, loading: true, error: null },
                events: [{ type: "ipc-navigate", url: state.url }],
            };
        case "LoadError":
            return {
                state: { ...state, loading: false, error: cmd.message },
                events: [],
            };
        case "Clicked":
            return { state, events: [{ type: "focus-block" }] };
        case "Disposed":
            return {
                state: { ...state, closed: true },
                events: [{ type: "shutdown" }],
            };
    }
}
```

## Saga / view-model integration

The new `BrowserViewModel` becomes a thin shell that:

1. Holds `_state = createSignal<BrowserPaneState>(initialState(blockId, defaultUrl))`.
2. Exposes `urlAtom`, `titleAtom`, `faviconUrlAtom`, `loadingAtom`, `canGoBackAtom`, `canGoForwardAtom`, `errorAtom` as `createMemo`s reading from `_state[0]()`.
3. Implements `viewIcon` and `viewName` as memos over the same state.
4. Provides a single `dispatch(cmd)` that runs the reducer, replaces state, and processes emitted events.
5. Public methods `navigate`, `goBack`, `goForward`, `reload`, `dispose` simply dispatch the corresponding command.
6. IPC subscriptions translate host events → commands; subscription registration races are eliminated by the existing `Promise.allSettled([...subs]).then(() => dispatch(NavigateRequested))` pattern.

```ts
private dispatch(cmd: BrowserPaneCommand): void {
    const { state, events } = update(this._state[0](), cmd);
    this._state[1](state);
    for (const ev of events) this.handleEvent(ev);
}

private handleEvent(ev: BrowserPaneEvent): void {
    switch (ev.type) {
        case "ipc-navigate":
            invokeCommand("browser_pane_navigate", { block_id: this.blockId, url: ev.url }).catch(() => {});
            break;
        case "ipc-back":
            invokeCommand("browser_pane_go_back", { block_id: this.blockId }).catch(() => {});
            break;
        case "ipc-forward":
            invokeCommand("browser_pane_go_forward", { block_id: this.blockId }).catch(() => {});
            break;
        case "meta-persist-url":
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: makeORef("block", this.blockId),
                meta: { url: ev.url },
            }).catch(() => {});
            break;
        case "focus-block":
            refocusNode(this.blockId);
            break;
        case "shutdown":
            this._navUnsub?.(); this._clickUnsub?.(); this._titleUnsub?.();
            break;
    }
}
```

The IPC subscriptions become:

```ts
listenEvent("browser-pane-nav-state", (p) => {
    if (p.block_id !== this.blockId) return;
    this.dispatch({
        type: "NavStateReceived",
        url: p.url,
        canGoBack: p.can_go_back,
        canGoForward: p.can_go_forward,
        urlOnly: p.url_only ?? false,
    });
});
```

Same shape for `browser-pane-title-change` → `TitleChangeReceived` and `browser-pane-clicked` → `Clicked`.

## Migration plan

1. **New slice** at `frontend/app/store/browser-pane-state/{types,reducer,reducer.test}.ts`. Pure code, fully unit-tested before any wiring.
2. **Refactor `BrowserViewModel`** to hold a single `_state` signal and dispatch through the reducer. Public API unchanged: `urlAtom`, `setUrl`, `navigate`, `goBack`, `goForward`, `reload`, `dispose`. Existing call sites (browser-view.tsx URL bar, history buttons) keep working.
3. **Move IPC subscription registration** to use the same `Promise.allSettled([...]).then(...)` race fix the title/favicon PR introduced — but only one entry point (constructor → reducer Init).
4. **Delete the six individual `createSignal` calls** + all direct `setUrl/setTitle/...` setters. They become memos over `_state[0]()`.
5. **Existing `browser-model.test.ts`** stays — its lifecycle-gating tests still apply (now via `closed` invariant in the reducer). Add a new test file for the pure reducer.

## Test plan

`reducer.test.ts` invariants (one `it` per case):

- Closed terminal: any non-`Disposed` command after `Disposed` is a no-op.
- `NavigateRequested` with empty/whitespace url → no state change, no events.
- `NavigateRequested` normalizes URL: `"foo.com"` → `"https://foo.com"`, `"foo bar"` → `"https://www.google.com/search?q=foo%20bar"`.
- `NavigateRequested` clears favicon + error, sets loading, preserves title, emits `ipc-navigate` + `meta-persist-url`.
- `NavStateReceived` derives favicon from URL origin (https/http).
- `NavStateReceived` for `about:blank` clears favicon (origin `"null"`).
- `NavStateReceived` for malformed URL clears favicon, no throw.
- `NavStateReceived` with `urlOnly=true` does NOT touch `canGoBack` / `canGoForward`.
- `NavStateReceived` with `urlOnly=false` updates history gates from payload.
- `NavStateReceived` clears `loading` + `error`.
- `NavStateReceived` always emits `meta-persist-url`.
- `TitleChangeReceived` with empty string falls back to `"Browser"`.
- `BackRequested` / `ForwardRequested` / `ReloadRequested` set loading, clear error, emit appropriate IPC event.
- `LoadError` sets `error`, clears `loading`.
- `Clicked` emits `focus-block`, no state change.
- `Disposed` flips `closed=true`, emits `shutdown`.

Total ~16 tests, all pure-function with no mocks.

`browser-model.test.ts` stays focused on lifecycle gating + IPC-subscription race coverage.

## Edge cases handled by the reducer

- **Out-of-order IPC events**: e.g., `TitleChangeReceived` arrives before the first `NavStateReceived`. Title atom updates fine; URL stays at initial value until nav-state lands.
- **Two NavigateRequested in quick succession**: each clears favicon + error, sets loading, emits ipc-navigate. Saga fires both; the host arbitrates which one wins. Reducer state always reflects the latest dispatched command.
- **Race already fixed by title/favicon PR**: subscriptions register first, then `dispatch({type:"NavigateRequested", url:initialUrl})`.
- **Block_id filtering**: enforced at the saga boundary (`if (p.block_id !== this.blockId) return;`), reducer is block-agnostic.

## Out-of-scope follow-ups

- Persist `title` and `faviconUrl` in `block.meta` so pane restore on reopen has them immediately, not just URL.
- Move browser pane history (full back/forward stack) into the reducer if we want user-visible history navigation beyond the OS-level back/forward.
- Generalize the saga pattern: a `useReducer` hook for SolidJS + IPC routing helper, applicable to other panes (terminal, agent, forge) that share the same shape.
- Consider promoting `BrowserPaneState` into the host's reducer stack (Phase B style) if/when we want backend-driven state restoration.

## Files touched

| File | Change |
|---|---|
| `frontend/app/store/browser-pane-state/types.ts` | new — State, Command, Event types |
| `frontend/app/store/browser-pane-state/reducer.ts` | new — pure `update()` + helpers |
| `frontend/app/store/browser-pane-state/reducer.test.ts` | new — ~16 invariants |
| `frontend/app/view/browser/browser-model.ts` | refactor — single state signal, dispatch wrapper, saga |
| `frontend/app/view/browser/browser-model.test.ts` | trim — lifecycle gating only; transition logic moves to reducer.test.ts |
| `frontend/app/view/browser/components/{FaviconImg,BrowserHeaderIcon}.tsx` | unchanged |

## Rollout

Single PR off `agenta/browser-pane-reducer-migration`. Bumps patch. The migration is internal — no IPC contract changes, no public API changes on the model, no new commands or files in `agentmux-cef`. Existing browser pane tabs continue working through the reducer with no user-visible behavior change beyond the race fix already shipped.
