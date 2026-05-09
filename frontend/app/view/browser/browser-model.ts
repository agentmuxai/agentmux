// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// NOTE: Browser is a pane-level view for embedded web browsing.
// Phase 1 uses an iframe (works for most sites). Phase 2 will add
// native CefBrowserView for sites that block iframes.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import {
    type BrowserPaneCommand,
    type BrowserPaneState,
    initialState as browserPaneInitialState,
    TITLE_FALLBACK,
    update as browserPaneUpdate,
} from "@/app/store/browser-pane-state";
import { refocusNode } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, createSignal, type Accessor } from "solid-js";

/**
 * Fallback URL for browser panes created without an explicit `meta.url`.
 * Keeps blank-spawned panes from landing on about:blank (no signposting,
 * no backlink). Callers that want a blank pane can pass `"about:blank"`
 * explicitly. See specs/SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21.md.
 */
const DEFAULT_BROWSER_URL = "https://agentmux.ai";

export class BrowserViewModel implements ViewModel {
    viewType = "browser";
    blockId: string;
    nodeModel: BlockNodeModel;

    viewIcon: Accessor<string> = () => "globe";
    viewName: Accessor<string>;
    viewText: Accessor<string | HeaderElem[]> = () => [];
    noPadding: Accessor<boolean> = () => true;

    get viewComponent(): ViewComponent {
        return null; // overridden by barrel via Object.defineProperty
    }

    private _url = createSignal<string>("");
    urlAtom: Accessor<string> = this._url[0];
    setUrl = this._url[1];

    private _title = createSignal<string>(TITLE_FALLBACK);
    titleAtom: Accessor<string> = this._title[0];
    setTitle = this._title[1];

    private _loading = createSignal<boolean>(false);
    loadingAtom: Accessor<boolean> = this._loading[0];
    setLoading = this._loading[1];

    private _canGoBack = createSignal<boolean>(false);
    canGoBackAtom: Accessor<boolean> = this._canGoBack[0];
    setCanGoBack = this._canGoBack[1];

    private _canGoForward = createSignal<boolean>(false);
    canGoForwardAtom: Accessor<boolean> = this._canGoForward[0];
    setCanGoForward = this._canGoForward[1];

    private _error = createSignal<string | null>(null);
    errorAtom: Accessor<string | null> = this._error[0];
    setError = this._error[1];

    /**
     * Unsubscribe from the backend's `browser-pane-nav-state` event.
     * Nulled in `dispose()` so we don't leak listeners when the pane
     * closes and the ViewModel is GC'd.
     */
    private _navUnsub: (() => void) | null = null;

    /**
     * Unsubscribe from the backend's `browser-pane-clicked` event.
     * Fired when the user clicks inside the pane content (which the
     * DOM never sees because the pane HWND intercepts the click at
     * Win32 level). We use it to drive `refocusNode` so the block
     * shows the blue focus border and keyboard shortcuts target it.
     */
    private _clickUnsub: (() => void) | null = null;

    blockAtom: Accessor<Block | undefined>;

    /**
     * Slice #9 (Phases 3a + 3b + 3c + 3e) reducer state — owns
     * `closed`, `loading`, `error`, `canGoBack`, `canGoForward`,
     * `title`. The signals above are projections of this state so the
     * SolidJS view layer keeps reactive parity. Per the roadmap at
     * `docs/specs/browser-pane-reducer-roadmap.md`, the remaining
     * cells (url, faviconUrl) migrate in follow-up PRs (faviconUrl
     * is derived from url, so it lands together with §3d); the slot
     * store + recordDispatch audit lands in Phase 4.
     */
    private _paneState: BrowserPaneState = browserPaneInitialState();
    /** Late callers (IPC handlers landing post-dispose, defensive guards
     *  in goBack/Forward/reload) read this to no-op instead of firing
     *  IPC against a Browser CEF is mid-destruction. See
     *  docs/specs/SPEC_BROWSER_PANE_LIFECYCLE.md §9 step 4. */
    get closed(): boolean { return this._paneState.closed; }

    /**
     * Single sync point between the reducer's pure transitions and the
     * SolidJS signals the view subscribes to. Projects every reducer
     * cell currently in scope (`loading`, `error`, `canGoBack`,
     * `canGoForward`, `title`) onto its signal, but only when the
     * value actually changed — avoiding spurious reactive churn that
     * could leak into the address-bar typing path that PR #737
     * regressed. Diag logs preserve the prior `state-write key=...`
     * shape so Phase-1 grep recipes still work.
     */
    private _dispatch(cmd: BrowserPaneCommand, src: string): void {
        const prev = this._paneState;
        const result = browserPaneUpdate(prev, cmd);
        this._paneState = result.state;
        if (result.state.loading !== prev.loading) {
            this.diag(
                `state-write key=loading value=${result.state.loading} src=${src}`,
            );
            this.setLoading(result.state.loading);
        }
        if (result.state.error !== prev.error) {
            this.diag(
                `state-write key=error value=${JSON.stringify(result.state.error)} src=${src}`,
            );
            this.setError(result.state.error);
        }
        if (result.state.canGoBack !== prev.canGoBack) {
            this.diag(
                `state-write key=canGoBack value=${result.state.canGoBack} src=${src}`,
            );
            this.setCanGoBack(result.state.canGoBack);
        }
        if (result.state.canGoForward !== prev.canGoForward) {
            this.diag(
                `state-write key=canGoForward value=${result.state.canGoForward} src=${src}`,
            );
            this.setCanGoForward(result.state.canGoForward);
        }
        if (result.state.title !== prev.title) {
            this.diag(
                `state-write key=title value=${JSON.stringify(result.state.title)} src=${src}`,
            );
            this.setTitle(result.state.title);
        }
        for (const e of result.events) {
            if (e.type === "post-close-command-dropped") {
                this.diag(
                    `post-close-command-dropped commandType=${e.commandType} src=${src}`,
                );
            }
        }
    }

    /** Tag every diag log with the block prefix so multi-pane sessions
     *  are greppable per pane. See docs/specs/browser-pane-reducer-roadmap.md
     *  Phase 1. */
    private get _diagTag(): string { return `[browser-pane:diag][${this.blockId.slice(0, 7)}]`; }
    private diag(msg: string): void { console.log(`${this._diagTag} ${msg}`); }

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;

        this.blockAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));

        const ctorMetaUrl = (this.blockAtom()?.meta?.["url"] as string | undefined) ?? "";
        console.log(`[browser-pane:diag][${blockId.slice(0, 7)}] ctor meta.url=${JSON.stringify(ctorMetaUrl)}`);

        this.viewName = createMemo(() => this.titleAtom());

        // Subscribe to nav-state updates fired by the backend on every
        // `on_load_end_pane`. This is the source of truth for address bar +
        // back/forward state: CEF knows the real history (including
        // in-pane link clicks and popup-intercept redirects), and the
        // local fake history array we used before diverged the moment
        // the user clicked any link inside the pane. See
        // specs/SPEC_BROWSER_PANE_Z_ORDER_2026_04_21.md (unrelated but
        // adjacent) and the nav-state wiring added alongside this PR.
        void listenEvent<{
            block_id: string;
            url: string;
            can_go_back?: boolean;
            can_go_forward?: boolean;
            url_only?: boolean;
        }>("browser-pane-nav-state", (payload) => {
            if (this.closed) {
                this.diag(`post-close-event-dropped name=browser-pane-nav-state url=${payload.url}`);
                return;
            }
            if (payload.block_id !== this.blockId) return;
            this.diag(
                `nav-state recv url=${JSON.stringify(payload.url)} url_only=${!!payload.url_only} can_back=${payload.can_go_back} can_forward=${payload.can_go_forward}`,
            );
            this.diag(`state-write key=url value=${JSON.stringify(payload.url)}`);
            this.setUrl(payload.url);
            // `url_only` events come from `on_load_end_pane` — they arrive
            // before the navigation controller has fully committed, so the
            // `can_go_back` / `can_go_forward` values from that hook would
            // be stale (kimi's investigation identified this race). The
            // authoritative values come from `on_loading_state_change_pane`
            // which CEF invokes with direct params. Skip touching the
            // back/forward atoms on `url_only` events.
            if (
                !payload.url_only &&
                (payload.can_go_back !== undefined ||
                    payload.can_go_forward !== undefined)
            ) {
                this._dispatch(
                    {
                        type: "HistoryUpdated",
                        canGoBack: payload.can_go_back,
                        canGoForward: payload.can_go_forward,
                    },
                    "nav-state",
                );
            }
            this._dispatch({ type: "LoadFinished" }, "nav-state");
            // Persist the real URL to block meta so pane restore lands
            // on the last page, not whatever was passed at create time.
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: makeORef("block", this.blockId),
                meta: { url: payload.url },
            }).catch(() => {});
        }).then((unsub) => {
            this.diag(`sub-registered name=browser-pane-nav-state`);
            if (this.closed) unsub();
            else this._navUnsub = unsub;
        });

        // Click-to-focus: the pane HWND captures clicks at the Win32 level,
        // so the DOM onMouseDown on `.browser-placeholder` never fires. The
        // backend emits this event directly from its WndProc's WM_LBUTTONDOWN
        // handler (see `pane/hwnd.rs`) using a HWND→block_id map registered
        // at pane creation. We drive refocusNode so the layout marks this
        // block as focused (blue border + keyboard shortcut target).
        void listenEvent<{ block_id: string }>("browser-pane-clicked", (payload) => {
            if (this.closed) {
                this.diag(`post-close-event-dropped name=browser-pane-clicked`);
                return;
            }
            if (payload.block_id !== this.blockId) return;
            this.diag(`clicked recv`);
            // The pane HWND captured this click at Win32 level so React never
            // saw it — `document.activeElement` is whatever it was before
            // (typically the address bar, since the user usually clicks
            // there first). If we leave that stale DOM focus, the
            // subsequent `giveFocus()` flow sees `isMainInput=true` and
            // tells the host to keep OS focus on the main window —
            // bouncing focus back from the pane HWND we just gave it.
            // The user's click on the pane is unambiguous "I want to
            // interact with the page, not chrome" intent; explicitly blur
            // whatever main-window input has DOM focus so giveFocus's
            // activeElement check resolves to "not a main input" and
            // OS focus stays on the pane HWND.
            const active = document.activeElement as HTMLElement | null;
            if (
                active != null &&
                (active.tagName === "INPUT" || active.tagName === "TEXTAREA") &&
                !active.classList.contains("dummy-focus")
            ) {
                this.diag(`pane-click blur active=${active.tagName.toLowerCase()}.${active.className}`);
                active.blur();
            }
            refocusNode(this.blockId);
        }).then((unsub) => {
            this.diag(`sub-registered name=browser-pane-clicked`);
            if (this.closed) unsub();
            else this._clickUnsub = unsub;
        });

        // Load URL from block meta on init. An empty/missing `url` falls
        // back to DEFAULT_BROWSER_URL so fresh panes aren't blank (the
        // widget definition in widgets.json also ships this URL, but the
        // fallback covers panes created through the API with no meta.url).
        const meta = this.blockAtom()?.meta;
        const initialUrl = ((meta?.["url"] as string | undefined) ?? "").trim() || DEFAULT_BROWSER_URL;
        this.navigate(initialUrl);
    }

    navigate(url: string): void {
        this.diag(`navigate(url=${JSON.stringify(url)}) closed=${this.closed}`);
        if (this.closed) return;
        // Normalize URL
        let normalized = url.trim();
        if (!normalized) return;
        if (!normalized.match(/^https?:\/\//i) && !normalized.startsWith("about:")) {
            if (normalized.includes(".") && !normalized.includes(" ")) {
                normalized = `https://${normalized}`;
            } else {
                // Treat as search query
                normalized = `https://www.google.com/search?q=${encodeURIComponent(normalized)}`;
            }
        }

        this.diag(`state-write key=url value=${JSON.stringify(normalized)} src=navigate`);
        this.setUrl(normalized);
        this._dispatch({ type: "Navigate", url: normalized }, "navigate");
        // can_go_back / can_go_forward are set by the `browser-pane-nav-state`
        // event subscription; we don't touch them here. CEF is the source of
        // truth for history state.

        // Persist the requested URL to block meta immediately so a quick
        // pane restore before load_end still has something. The nav-state
        // event will overwrite this with the post-redirect final URL.
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref: makeORef("block", this.blockId),
            meta: { url: normalized },
        }).catch(() => {});
    }

    goBack(): void {
        this.diag(`goBack closed=${this.closed}`);
        if (this.closed) return;
        // CEF owns the history — we just fire the IPC. The button's
        // enabled/disabled state came from `can_go_back` in the nav-state
        // event, so if we got here the browser has somewhere to go.
        this._dispatch({ type: "LoadStarted" }, "goBack");
        invokeCommand("browser_pane_go_back", { block_id: this.blockId }).catch(() => {});
    }

    goForward(): void {
        this.diag(`goForward closed=${this.closed}`);
        if (this.closed) return;
        this._dispatch({ type: "LoadStarted" }, "goForward");
        invokeCommand("browser_pane_go_forward", { block_id: this.blockId }).catch(() => {});
    }

    reload(): void {
        this.diag(`reload closed=${this.closed}`);
        if (this.closed) return;
        const url = this.urlAtom();
        if (url) {
            this.diag(`state-write key=url value="" src=reload-clear`);
            this.setUrl("");
            // Force iframe reload by briefly clearing then re-setting
            requestAnimationFrame(() => {
                this.diag(`state-write key=url value=${JSON.stringify(url)} src=reload-restore`);
                this.setUrl(url);
                this._dispatch({ type: "Navigate", url }, "reload-restore");
            });
        }
    }

    onLoad(): void {
        this.diag(`onLoad`);
        this._dispatch({ type: "LoadFinished" }, "onLoad");
    }

    onError(msg: string): void {
        this.diag(`onError msg=${JSON.stringify(msg)}`);
        this._dispatch({ type: "LoadFailed", reason: msg }, "onError");
    }

    giveFocus(): boolean {
        this.diag(`giveFocus closed=${this.closed}`);
        if (this.closed) return false;
        // If a main-window input inside this block (e.g. the URL bar) is
        // already focused, keep it — the user is interacting with the block's
        // chrome, not the embedded page. Also tell the host to move OS-level
        // keyboard focus back to the main window, in case a pane was holding
        // it (otherwise keystrokes still get routed to the pane's HWND).
        const active = document.activeElement as HTMLElement | null;
        const isMainInput =
            active != null &&
            (active.tagName === "INPUT" || active.tagName === "TEXTAREA") &&
            !active.classList.contains("dummy-focus");
        if (isMainInput) {
            invokeCommand("main_window_focus", {}).catch(() => {});
            return true;
        }
        // Otherwise the user wants to interact with the embedded page — tell
        // the host to move Windows-level keyboard focus to the pane's HWND.
        invokeCommand("browser_pane_focus", { block_id: this.blockId }).catch(() => {});
        return true;
    }

    dispose(): void {
        this.diag(`dispose`);
        this._dispatch({ type: "Disposed" }, "dispose");
        if (this._navUnsub) {
            this._navUnsub();
            this._navUnsub = null;
        }
        if (this._clickUnsub) {
            this._clickUnsub();
            this._clickUnsub = null;
        }
    }
}
