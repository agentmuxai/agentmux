// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// NOTE: Browser is a pane-level view for embedded web browsing.
// Phase 1 uses an iframe (works for most sites). Phase 2 will add
// native CefBrowserView for sites that block iframes.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { refocusNode } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { buildBrowserHeaderIcon } from "@/app/view/browser/components/BrowserHeaderIcon";
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

    viewIcon: Accessor<string | IconButtonDecl>;
    viewName: Accessor<string>;
    viewText: Accessor<string | HeaderElem[]> = () => [];
    noPadding: Accessor<boolean> = () => true;

    get viewComponent(): ViewComponent {
        return null; // overridden by barrel via Object.defineProperty
    }

    private _url = createSignal<string>("");
    urlAtom: Accessor<string> = this._url[0];
    setUrl = this._url[1];

    private _title = createSignal<string>("Browser");
    titleAtom: Accessor<string> = this._title[0];
    setTitle = this._title[1];

    // Favicon URL — derived from the page URL's origin in the
    // `browser-pane-nav-state` handler (`${origin}/favicon.ico`), not
    // from a separate CEF callback. See docs/specs/browser-pane-title-favicon.md
    // (favicon tradeoff section). Empty string → the viewIcon memo
    // returns "globe" for the fallback. Cleared at navigate() start
    // so the loading state shows the globe instead of the prior page's
    // icon; the new derived URL is set when the nav-state event lands.
    private _faviconUrl = createSignal<string>("");
    faviconUrlAtom: Accessor<string> = this._faviconUrl[0];
    setFaviconUrl = this._faviconUrl[1];

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

    private _titleUnsub: (() => void) | null = null;

    blockAtom: Accessor<Block | undefined>;

    // Flipped in dispose() so late callers see the pane is gone and no-op
    // instead of firing IPC against a Browser that CEF is mid-destruction.
    // See docs/specs/SPEC_BROWSER_PANE_LIFECYCLE.md §9 step 4.
    private _closed = false;
    get closed(): boolean { return this._closed; }

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;

        this.blockAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));

        this.viewName = createMemo(() => {
            const title = this.titleAtom();
            return title || "Browser";
        });

        // Header icon: live favicon when set, otherwise the globe.
        // Wrapping the favicon in an IconButtonDecl with noAction=true
        // matches the agent pane pattern (see AgentPaneIcon) so the
        // blockframe renders our <img> instead of treating viewIcon as
        // a font-icon name.
        this.viewIcon = createMemo<string | IconButtonDecl>(() => {
            const fav = this.faviconUrlAtom();
            if (fav) return buildBrowserHeaderIcon(fav, this.titleAtom());
            return "globe";
        });

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
            if (this._closed) return;
            if (payload.block_id !== this.blockId) return;
            this.setUrl(payload.url);
            // Derive favicon from the URL origin. `${origin}/favicon.ico`
            // is the convention every browser falls back to, so it
            // covers most sites without needing the host to wire CEF's
            // OnFaviconURLChange. Sites that don't serve at this path
            // gracefully degrade to the globe via FaviconImg's onError.
            // about: / file: / chrome: URLs produce origin "null" → clear.
            try {
                const origin = new URL(payload.url).origin;
                if (origin && origin !== "null") {
                    this.setFaviconUrl(`${origin}/favicon.ico`);
                } else {
                    this.setFaviconUrl("");
                }
            } catch {
                this.setFaviconUrl("");
            }
            // `url_only` events come from `on_load_end_pane` — they arrive
            // before the navigation controller has fully committed, so the
            // `can_go_back` / `can_go_forward` values from that hook would
            // be stale (kimi's investigation identified this race). The
            // authoritative values come from `on_loading_state_change_pane`
            // which CEF invokes with direct params. Skip touching the
            // back/forward atoms on `url_only` events.
            if (!payload.url_only) {
                if (payload.can_go_back !== undefined) this.setCanGoBack(payload.can_go_back);
                if (payload.can_go_forward !== undefined) this.setCanGoForward(payload.can_go_forward);
            }
            this.setLoading(false);
            // Persist the real URL to block meta so pane restore lands
            // on the last page, not whatever was passed at create time.
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: makeORef("block", this.blockId),
                meta: { url: payload.url },
            }).catch(() => {});
        }).then((unsub) => {
            if (this._closed) unsub();
            else this._navUnsub = unsub;
        });

        // Click-to-focus: the pane HWND captures clicks at the Win32 level,
        // so the DOM onMouseDown on `.browser-placeholder` never fires. The
        // backend emits this event directly from its WndProc's WM_LBUTTONDOWN
        // handler (see `pane/hwnd.rs`) using a HWND→block_id map registered
        // at pane creation. We drive refocusNode so the layout marks this
        // block as focused (blue border + keyboard shortcut target).
        void listenEvent<{ block_id: string }>("browser-pane-clicked", (payload) => {
            if (this._closed) return;
            if (payload.block_id !== this.blockId) return;
            refocusNode(this.blockId);
        }).then((unsub) => {
            if (this._closed) unsub();
            else this._clickUnsub = unsub;
        });

        // Page <title> updates. CEF fires on_title_change every time the
        // top-level frame's document.title changes (initial load + later
        // mutations). Updating titleAtom drives the viewName memo that
        // renders the pane header label.
        void listenEvent<{ block_id: string; title: string }>("browser-pane-title-change", (payload) => {
            if (this._closed) return;
            if (payload.block_id !== this.blockId) return;
            this.setTitle(payload.title || "Browser");
        }).then((unsub) => {
            if (this._closed) unsub();
            else this._titleUnsub = unsub;
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
        if (this._closed) return;
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

        this.setUrl(normalized);
        this.setError(null);
        this.setLoading(true);
        // Clear the previous page's favicon so the loading state shows
        // the globe instead of the stale icon. Title is intentionally
        // NOT cleared — the new title arrives via on_title_change soon
        // after, and clearing here would briefly flash "Browser".
        this.setFaviconUrl("");
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
        if (this._closed) return;
        // CEF owns the history — we just fire the IPC. The button's
        // enabled/disabled state came from `can_go_back` in the nav-state
        // event, so if we got here the browser has somewhere to go.
        this.setLoading(true);
        invokeCommand("browser_pane_go_back", { block_id: this.blockId }).catch(() => {});
    }

    goForward(): void {
        if (this._closed) return;
        this.setLoading(true);
        invokeCommand("browser_pane_go_forward", { block_id: this.blockId }).catch(() => {});
    }

    reload(): void {
        if (this._closed) return;
        const url = this.urlAtom();
        if (url) {
            this.setUrl("");
            // Force iframe reload by briefly clearing then re-setting
            requestAnimationFrame(() => {
                this.setUrl(url);
                this.setLoading(true);
            });
        }
    }

    onLoad(): void {
        this.setLoading(false);
    }

    onError(msg: string): void {
        this.setLoading(false);
        this.setError(msg);
    }

    giveFocus(): boolean {
        if (this._closed) return false;
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
        this._closed = true;
        if (this._navUnsub) {
            this._navUnsub();
            this._navUnsub = null;
        }
        if (this._clickUnsub) {
            this._clickUnsub();
            this._clickUnsub = null;
        }
        if (this._titleUnsub) {
            this._titleUnsub();
            this._titleUnsub = null;
        }
    }
}
