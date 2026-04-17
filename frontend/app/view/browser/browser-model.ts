// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// NOTE: Browser is a pane-level view for embedded web browsing.
// Phase 1 uses an iframe (works for most sites). Phase 2 will add
// native CefBrowserView for sites that block iframes.

import { BlockNodeModel } from "@/app/block/blocktypes";
import { invokeCommand } from "@/app/platform/ipc";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { getWaveObjectAtom, makeORef } from "@/app/store/wos";
import { createMemo, createSignal, type Accessor } from "solid-js";

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

    private _title = createSignal<string>("Browser");
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

    // Navigation history (iframe doesn't expose browser history)
    private history: string[] = [];
    private historyIndex = -1;

    blockAtom: Accessor<Block | undefined>;

    constructor(blockId: string, nodeModel: BlockNodeModel) {
        this.blockId = blockId;
        this.nodeModel = nodeModel;

        this.blockAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));

        this.viewName = createMemo(() => {
            const title = this.titleAtom();
            return title || "Browser";
        });

        // Load URL from block meta on init
        const meta = this.blockAtom()?.meta;
        if (meta?.["url"]) {
            this.navigate(meta["url"] as string);
        }
    }

    navigate(url: string): void {
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

        // Update history
        if (this.historyIndex < this.history.length - 1) {
            this.history = this.history.slice(0, this.historyIndex + 1);
        }
        this.history.push(normalized);
        this.historyIndex = this.history.length - 1;
        this.setCanGoBack(this.historyIndex > 0);
        this.setCanGoForward(false);

        // Persist URL to block meta
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref: makeORef("block", this.blockId),
            meta: { url: normalized },
        }).catch(() => {});
    }

    goBack(): void {
        if (this.historyIndex > 0) {
            this.historyIndex--;
            this.setUrl(this.history[this.historyIndex]);
            this.setCanGoBack(this.historyIndex > 0);
            this.setCanGoForward(true);
            this.setLoading(true);
        }
    }

    goForward(): void {
        if (this.historyIndex < this.history.length - 1) {
            this.historyIndex++;
            this.setUrl(this.history[this.historyIndex]);
            this.setCanGoBack(true);
            this.setCanGoForward(this.historyIndex < this.history.length - 1);
            this.setLoading(true);
        }
    }

    reload(): void {
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
        // Tell the host to move Windows-level keyboard focus to this pane's
        // HWND. Without this, FocusManager falls back to focusing a hidden
        // "dummy-focus" input in the main window and keystrokes never reach
        // the embedded page.
        invokeCommand("browser_pane_focus", { block_id: this.blockId }).catch(() => {});
        return true;
    }

    dispose(): void {}
}
