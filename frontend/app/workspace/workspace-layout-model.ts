// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { globalStore } from "@/app/store/jotaiStore";
import * as WOS from "@/app/store/wos";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { atoms, getApi, getTabMetaKeyAtom, recordTEvent } from "@/app/store/global";
import * as jotai from "jotai";
import { debounce } from "lodash-es";
import { ImperativePanelGroupHandle } from "react-resizable-panels";

const AIPANEL_DEFAULTWIDTH = 300;
const AIPANEL_DEFAULTWIDTHRATIO = 0.33;
const AIPANEL_MINWIDTH = 300;
const AIPANEL_MAXWIDTHRATIO = 0.66;

/**
 * Simple state holder for the AI panel.
 * Does NOT hold React refs or call imperative panel APIs.
 * Expand/collapse is handled by useEffect in the component.
 */
class WorkspaceLayoutModel {
    private static instance: WorkspaceLayoutModel | null = null;

    panelVisibleAtom: jotai.PrimitiveAtom<boolean>;
    private aiPanelWidth: number | null = null;
    private initialized = false;
    private _inResize = false;
    private panelGroupRef: ImperativePanelGroupHandle | null = null;
    private debouncedPersistWidth: (width: number) => void;

    private constructor() {
        this.panelVisibleAtom = jotai.atom(false);
        this.debouncedPersistWidth = debounce((width: number) => {
            try {
                RpcApi.SetMetaCommand(TabRpcClient, {
                    oref: WOS.makeORef("tab", this.getTabId()),
                    meta: { "waveai:panelwidth": width },
                });
            } catch (e) {
                console.warn("Failed to persist panel width:", e);
            }
        }, 300);
    }

    static getInstance(): WorkspaceLayoutModel {
        if (!WorkspaceLayoutModel.instance) {
            WorkspaceLayoutModel.instance = new WorkspaceLayoutModel();
        }
        return WorkspaceLayoutModel.instance;
    }

    private getTabId(): string {
        return globalStore.get(atoms.activeTabId);
    }

    private initializeFromTabMeta(): void {
        if (this.initialized) return;
        this.initialized = true;
        try {
            const savedVisible = globalStore.get(
                getTabMetaKeyAtom(this.getTabId(), "waveai:panelopen")
            );
            const savedWidth = globalStore.get(
                getTabMetaKeyAtom(this.getTabId(), "waveai:panelwidth")
            );
            if (savedVisible != null) {
                globalStore.set(this.panelVisibleAtom, savedVisible);
            }
            if (savedWidth != null) {
                this.aiPanelWidth = savedWidth;
            }
        } catch (e) {
            console.warn("Failed to initialize from tab meta:", e);
        }
    }

    private getStoredWidth(): number {
        this.initializeFromTabMeta();
        if (this.aiPanelWidth == null) {
            this.aiPanelWidth = Math.max(
                AIPANEL_DEFAULTWIDTH,
                window.innerWidth * AIPANEL_DEFAULTWIDTHRATIO
            );
        }
        return this.aiPanelWidth;
    }

    private clampWidth(width: number, windowWidth: number): number {
        const maxWidth = Math.floor(windowWidth * AIPANEL_MAXWIDTHRATIO);
        if (AIPANEL_MINWIDTH > maxWidth) return AIPANEL_MINWIDTH;
        return Math.max(AIPANEL_MINWIDTH, Math.min(width, maxWidth));
    }

    // --- Ref management (only panelGroupRef for window resize) ---

    setPanelGroupRef(ref: ImperativePanelGroupHandle | null): void {
        this.panelGroupRef = ref;
    }

    get inResize(): boolean {
        return this._inResize;
    }

    // --- Public API ---

    getDefaultSize(): number {
        this.initializeFromTabMeta();
        const isVisible = globalStore.get(this.panelVisibleAtom);
        if (!isVisible) return 0;
        const width = this.getStoredWidth();
        const clamped = this.clampWidth(width, window.innerWidth);
        const pct = (clamped / window.innerWidth) * 100;
        return Math.max(0, Math.min(pct, AIPANEL_MAXWIDTHRATIO * 100));
    }

    getAIPanelVisible(): boolean {
        this.initializeFromTabMeta();
        return globalStore.get(this.panelVisibleAtom);
    }

    setAIPanelVisible(visible: boolean, opts?: { nofocus?: boolean }): void {
        const wasVisible = globalStore.get(this.panelVisibleAtom);
        if (visible === wasVisible) return;
        if (visible) recordTEvent("action:openwaveai");
        globalStore.set(this.panelVisibleAtom, visible);
        getApi().setWaveAIOpen(visible);
        RpcApi.SetMetaCommand(TabRpcClient, {
            oref: WOS.makeORef("tab", this.getTabId()),
            meta: { "waveai:panelopen": visible },
        });
        // nofocus flag is read by the component's useEffect
        this._lastSetVisibleOpts = opts ?? null;
    }

    // Expose last opts so component can check nofocus
    _lastSetVisibleOpts: { nofocus?: boolean } | null = null;

    // --- Panel callbacks (called from Panel props, never re-enter library) ---

    captureResize(sizePct: number): void {
        if (this._inResize) return;
        const pixelWidth = (sizePct / 100) * window.innerWidth;
        if (pixelWidth >= AIPANEL_MINWIDTH) {
            this.aiPanelWidth = pixelWidth;
            this.debouncedPersistWidth(pixelWidth);
        }
    }

    onCollapsed(): void {
        const wasVisible = globalStore.get(this.panelVisibleAtom);
        if (wasVisible) {
            globalStore.set(this.panelVisibleAtom, false);
            getApi().setWaveAIOpen(false);
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("tab", this.getTabId()),
                meta: { "waveai:panelopen": false },
            });
        }
    }

    onExpanded(): void {
        const wasVisible = globalStore.get(this.panelVisibleAtom);
        if (!wasVisible) {
            recordTEvent("action:openwaveai");
            globalStore.set(this.panelVisibleAtom, true);
            getApi().setWaveAIOpen(true);
            RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("tab", this.getTabId()),
                meta: { "waveai:panelopen": true },
            });
        }
    }

    // --- Window resize (only place setLayout is called) ---

    handleWindowResize = (): void => {
        const isVisible = globalStore.get(this.panelVisibleAtom);
        if (!isVisible || !this.panelGroupRef) return;

        const width = this.getStoredWidth();
        const windowWidth = window.innerWidth;
        const clamped = this.clampWidth(width, windowWidth);
        const pct = (clamped / windowWidth) * 100;

        this._inResize = true;
        this.panelGroupRef.setLayout([pct, 100 - pct]);
        this._inResize = false;
    };
}

export { WorkspaceLayoutModel };
