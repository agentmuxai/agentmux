// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// BrowserViewModel — thin saga shell over the pure browser-pane reducer.
// All state transitions live in `frontend/app/store/browser-pane-state`;
// this file translates IPC events → commands, runs the reducer, and
// fans emitted events out to side effects (IPC calls, meta persist,
// focus). See docs/specs/browser-pane-reducer.md for the design.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { refocusNode } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import {
    initialState,
    update,
    type BrowserPaneCommand,
    type BrowserPaneEvent,
    type BrowserPaneState,
} from "@/app/store/browser-pane-state/reducer";
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

    // Single state signal driven by the reducer. All other "atom"
    // accessors below are memos derived from this — keeps reactivity
    // fine-grained while preserving the previous public API shape.
    private _state: ReturnType<typeof createSignal<BrowserPaneState>>;
    private get state(): BrowserPaneState { return this._state[0](); }

    urlAtom: Accessor<string>;
    titleAtom: Accessor<string>;
    faviconUrlAtom: Accessor<string>;
    loadingAtom: Accessor<boolean>;
    canGoBackAtom: Accessor<boolean>;
    canGoForwardAtom: Accessor<boolean>;
    errorAtom: Accessor<string | null>;

    /** Unsubscribers from listenEvent registrations. Released on
     *  `shutdown` event (Disposed command). */
    private _navUnsub: (() => void) | null = null;
    private _clickUnsub: (() => void) | null = null;
    private _titleUnsub: (() => void) | null = null;

    blockAtom: Accessor<Block | undefined>;

    /** Mirrors `state.closed` for backwards-compat with callers that
     *  reach for `model.closed` directly. */
    get closed(): boolean { return this.state.closed; }

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;

        this.blockAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));

        // Read the initial URL up-front; the reducer is seeded with
        // an empty url and we dispatch NavigateRequested with the real
        // value once subscriptions are registered (race fix).
        const meta = this.blockAtom()?.meta;
        const initialUrl = ((meta?.["url"] as string | undefined) ?? "").trim() || DEFAULT_BROWSER_URL;
        this._state = createSignal<BrowserPaneState>(initialState(blockId));

        this.urlAtom = createMemo(() => this.state.url);
        this.titleAtom = createMemo(() => this.state.title);
        this.faviconUrlAtom = createMemo(() => this.state.faviconUrl);
        this.loadingAtom = createMemo(() => this.state.loading);
        this.canGoBackAtom = createMemo(() => this.state.canGoBack);
        this.canGoForwardAtom = createMemo(() => this.state.canGoForward);
        this.errorAtom = createMemo(() => this.state.error);

        this.viewName = createMemo(() => this.state.title || "Browser");
        this.viewIcon = createMemo<string | IconButtonDecl>(() => {
            const fav = this.state.faviconUrl;
            if (fav) return buildBrowserHeaderIcon(fav, this.state.title);
            return "globe";
        });

        // Subscribe to host IPC events. Each subscription's promise
        // resolves once the host has acked the registration; we gate
        // the construction-time NavigateRequested on all three to
        // avoid the registration race where a fast on_load_end fires
        // before the renderer is listening.
        const navSubP = listenEvent<{
            block_id: string;
            url: string;
            can_go_back?: boolean;
            can_go_forward?: boolean;
            url_only?: boolean;
        }>("browser-pane-nav-state", (payload) => {
            if (payload.block_id !== this.blockId) return;
            this.dispatch({
                type: "NavStateReceived",
                url: payload.url,
                canGoBack: payload.can_go_back,
                canGoForward: payload.can_go_forward,
                urlOnly: payload.url_only ?? false,
            });
        }).then((unsub) => {
            if (this.state.closed) unsub();
            else this._navUnsub = unsub;
            return unsub;
        });

        const clickSubP = listenEvent<{ block_id: string }>("browser-pane-clicked", (payload) => {
            if (payload.block_id !== this.blockId) return;
            this.dispatch({ type: "Clicked" });
        }).then((unsub) => {
            if (this.state.closed) unsub();
            else this._clickUnsub = unsub;
            return unsub;
        });

        const titleSubP = listenEvent<{ block_id: string; title: string }>("browser-pane-title-change", (payload) => {
            if (payload.block_id !== this.blockId) return;
            this.dispatch({ type: "TitleChangeReceived", title: payload.title });
        }).then((unsub) => {
            if (this.state.closed) unsub();
            else this._titleUnsub = unsub;
            return unsub;
        });

        Promise.allSettled([navSubP, clickSubP, titleSubP]).then(() => {
            if (this.state.closed) return;
            this.dispatch({ type: "NavigateRequested", url: initialUrl });
        });
    }

    private dispatch(cmd: BrowserPaneCommand): void {
        const result = update(this.state, cmd);
        if (result.state !== this.state) this._state[1](result.state);
        for (const ev of result.events) this.handleEvent(ev);
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
                if (this._navUnsub) { this._navUnsub(); this._navUnsub = null; }
                if (this._clickUnsub) { this._clickUnsub(); this._clickUnsub = null; }
                if (this._titleUnsub) { this._titleUnsub(); this._titleUnsub = null; }
                break;
        }
    }

    // -------- public API (unchanged shape — just routes to reducer) --------

    navigate(url: string): void {
        this.dispatch({ type: "NavigateRequested", url });
    }

    goBack(): void {
        this.dispatch({ type: "BackRequested" });
    }

    goForward(): void {
        this.dispatch({ type: "ForwardRequested" });
    }

    /**
     * Reload the current page. The reducer emits ipc-navigate with
     * the existing URL; CEF re-loads. The previous "clear url then
     * rAF restore" gymnastic was needed for an iframe-based render
     * path that no longer exists in pane mode.
     */
    reload(): void {
        this.dispatch({ type: "ReloadRequested" });
    }

    onLoad(): void {
        // CEF's load completion arrives via browser-pane-nav-state;
        // this handler is kept for the legacy iframe path's onLoad
        // callback wiring in browser-view.tsx, where it drove
        // setLoading(false). The reducer now handles this via
        // NavStateReceived.
    }

    onError(msg: string): void {
        this.dispatch({ type: "LoadError", message: msg });
    }

    giveFocus(): boolean {
        if (this.state.closed) return false;
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
        invokeCommand("browser_pane_focus", { block_id: this.blockId }).catch(() => {});
        return true;
    }

    dispose(): void {
        this.dispatch({ type: "Disposed" });
    }
}
