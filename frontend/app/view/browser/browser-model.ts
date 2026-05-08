// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// BrowserViewModel — view-model + saga shell over the
// browser-pane-state slice (#9 in the frontend reducer roadmap).
//
// The slice owns state in `frontend/app/store/browser-pane-state-store.ts`
// keyed by blockId. The model:
//   - holds the per-pane projection setters (createSignal pairs whose
//     accessors are exposed as urlAtom, titleAtom, etc.),
//   - registers projections + initial state on construction,
//   - dispatches commands through the slice store and fans the returned
//     events out to side effects (IPC calls, meta persist, focus),
//   - unregisters on dispose.
//
// See docs/specs/browser-pane-reducer.md.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { refocusNode } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import {
    dispatch as bpDispatch,
    registerPane as bpRegister,
    unregisterPane as bpUnregister,
    type BrowserPaneEvent,
    type BrowserPaneProjections,
} from "@/app/store/browser-pane-state-store";
import { buildBrowserHeaderIcon } from "@/app/view/browser/components/BrowserHeaderIcon";
import { createMemo, createSignal, type Accessor } from "solid-js";

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

    // Projection signals. Setters are passed to `registerPane` so the
    // slice writes through them on every state transition; readers
    // continue to subscribe via these accessors.
    urlAtom: Accessor<string>;
    titleAtom: Accessor<string>;
    faviconUrlAtom: Accessor<string>;
    loadingAtom: Accessor<boolean>;
    canGoBackAtom: Accessor<boolean>;
    canGoForwardAtom: Accessor<boolean>;
    errorAtom: Accessor<string | null>;

    private _setUrl: (v: string) => void;
    private _setTitle: (v: string) => void;
    private _setFavicon: (v: string) => void;
    private _setLoading: (v: boolean) => void;
    private _setCanGoBack: (v: boolean) => void;
    private _setCanGoForward: (v: boolean) => void;
    private _setError: (v: string | null) => void;

    /** Mirrors `slot.state.closed` via projection. */
    private _closed: Accessor<boolean>;
    private _setClosed: (v: boolean) => void;
    get closed(): boolean { return this._closed(); }

    private _navUnsub: (() => void) | null = null;
    private _clickUnsub: (() => void) | null = null;
    private _titleUnsub: (() => void) | null = null;

    blockAtom: Accessor<Block | undefined>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;
        this.blockAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));

        // 1. Per-pane signals — projection write targets.
        const [url, setUrl] = createSignal<string>("");
        const [title, setTitle] = createSignal<string>("Browser");
        const [favicon, setFavicon] = createSignal<string>("");
        const [loading, setLoading] = createSignal<boolean>(false);
        const [canGoBack, setCanGoBack] = createSignal<boolean>(false);
        const [canGoForward, setCanGoForward] = createSignal<boolean>(false);
        const [error, setError] = createSignal<string | null>(null);
        const [closed, setClosed] = createSignal<boolean>(false);

        this.urlAtom = url;
        this.titleAtom = title;
        this.faviconUrlAtom = favicon;
        this.loadingAtom = loading;
        this.canGoBackAtom = canGoBack;
        this.canGoForwardAtom = canGoForward;
        this.errorAtom = error;
        this._closed = closed;

        this._setUrl = setUrl;
        this._setTitle = setTitle;
        this._setFavicon = setFavicon;
        this._setLoading = setLoading;
        this._setCanGoBack = setCanGoBack;
        this._setCanGoForward = setCanGoForward;
        this._setError = setError;
        this._setClosed = setClosed;

        this.viewName = createMemo(() => this.titleAtom() || "Browser");
        this.viewIcon = createMemo<string | IconButtonDecl>(() => {
            const fav = this.faviconUrlAtom();
            if (fav) return buildBrowserHeaderIcon(fav, this.titleAtom());
            return "globe";
        });

        // 2. Register the projection slot SYNCHRONOUSLY before anything
        //    can dispatch. Subsequent IPC events translate to commands
        //    that flow through the slice store.
        const projections: BrowserPaneProjections = {
            url: this._setUrl,
            title: this._setTitle,
            faviconUrl: this._setFavicon,
            loading: this._setLoading,
            canGoBack: this._setCanGoBack,
            canGoForward: this._setCanGoForward,
            error: this._setError,
            closed: this._setClosed,
        };
        bpRegister(blockId, projections);

        // 3. Subscribe to host IPC events. Registration race fix:
        //    listenEvent's promise resolves only after the host has
        //    acked the registration. Defer the construction-time
        //    NavigateRequested until all three are live.
        const navSubP = listenEvent<{
            block_id: string;
            url: string;
            can_go_back?: boolean;
            can_go_forward?: boolean;
            url_only?: boolean;
        }>("browser-pane-nav-state", (payload) => {
            if (payload.block_id !== this.blockId) return;
            this.dispatchCmd({
                type: "NavStateReceived",
                url: payload.url,
                canGoBack: payload.can_go_back,
                canGoForward: payload.can_go_forward,
                urlOnly: payload.url_only ?? false,
            });
        }).then((unsub) => {
            if (this.closed) unsub();
            else this._navUnsub = unsub;
            return unsub;
        });

        const clickSubP = listenEvent<{ block_id: string }>("browser-pane-clicked", (payload) => {
            if (payload.block_id !== this.blockId) return;
            this.dispatchCmd({ type: "Clicked" });
        }).then((unsub) => {
            if (this.closed) unsub();
            else this._clickUnsub = unsub;
            return unsub;
        });

        const titleSubP = listenEvent<{ block_id: string; title: string }>("browser-pane-title-change", (payload) => {
            if (payload.block_id !== this.blockId) return;
            this.dispatchCmd({ type: "TitleChangeReceived", title: payload.title });
        }).then((unsub) => {
            if (this.closed) unsub();
            else this._titleUnsub = unsub;
            return unsub;
        });

        // 4. Once all subs are confirmed registered, fire the initial
        //    navigate. Reading meta.url synchronously is fine — the
        //    slice's state cell is empty until this command runs.
        const meta = this.blockAtom()?.meta;
        const initialUrl = ((meta?.["url"] as string | undefined) ?? "").trim() || DEFAULT_BROWSER_URL;
        Promise.allSettled([navSubP, clickSubP, titleSubP]).then(() => {
            if (this.closed) return;
            this.dispatchCmd({ type: "NavigateRequested", url: initialUrl });
        });
    }

    /**
     * Dispatch a command through the slice store and process its events.
     * Splitting this from the public API methods keeps the model's
     * navigate/goBack/etc. as one-line dispatches.
     */
    private dispatchCmd(cmd: import("@/app/store/browser-pane-state-store").BrowserPaneCommand): void {
        const events = bpDispatch(this.blockId, cmd, "user");
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
                if (this._navUnsub) { this._navUnsub(); this._navUnsub = null; }
                if (this._clickUnsub) { this._clickUnsub(); this._clickUnsub = null; }
                if (this._titleUnsub) { this._titleUnsub(); this._titleUnsub = null; }
                bpUnregister(this.blockId);
                break;
        }
    }

    // -------- public API --------

    navigate(url: string): void {
        if (this.closed) return;
        this.dispatchCmd({ type: "NavigateRequested", url });
    }

    goBack(): void {
        if (this.closed) return;
        this.dispatchCmd({ type: "BackRequested" });
    }

    goForward(): void {
        if (this.closed) return;
        this.dispatchCmd({ type: "ForwardRequested" });
    }

    reload(): void {
        if (this.closed) return;
        this.dispatchCmd({ type: "ReloadRequested" });
    }

    onLoad(): void {
        // CEF's load completion arrives via browser-pane-nav-state;
        // legacy iframe path's onLoad callback is a no-op now.
    }

    onError(msg: string): void {
        if (this.closed) return;
        this.dispatchCmd({ type: "LoadError", message: msg });
    }

    giveFocus(): boolean {
        if (this.closed) return false;
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
        if (this.closed) return;
        // Disposed flips closed via projection AND emits the shutdown
        // event which unsubs IPC + unregisters the slot.
        this.dispatchCmd({ type: "Disposed" });
    }
}
