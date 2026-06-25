// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Global app state — migrated from Jotai atoms to SolidJS signals.

import { WpsEvent } from "@/app/store/wps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { markEnd, markStart } from "@/perf";
import {
    getLayoutModelForStaticTab,
    LayoutTreeActionType,
    LayoutTreeInsertNodeAction,
    newLayoutNode,
} from "@/layout/index";
import {
    LayoutTreeReplaceNodeAction,
    LayoutTreeSplitHorizontalAction,
    LayoutTreeSplitVerticalAction,
} from "@/layout/lib/types";
import { getWebServerEndpoint } from "@/util/endpoints";
import { fetch } from "@/util/fetchutil";
import { setPlatform } from "@/util/platformutil";
import { fireAndForget, isBlank } from "@/util/util";
import { createMemo, createSignal } from "solid-js";
import { reconnectWS } from "./ws";
import {
    backendStatusAtom,
    backendDeathInfoAtom,
    initBackendStatusListeners,
    setBackendDeathInfoAtom,
    setBackendStatusAtom,
} from "./backendStatus";
import { openModal } from "./modalmodel";
import { AboutModal } from "@/app/modals/about";
import { UserInputModal } from "@/app/modals/userinputmodal";
import { TAB_COLORS } from "@/app/tab/tab";
import { ClientService, ObjectService, WorkspaceService } from "./services";
import { holdRevealGate, scheduleRevealLift } from "./tab-reveal";
import * as WOS from "./wos";
import { getFileSubject, waveEventSubscribe } from "./wps";
import { getApi } from "./app-api";
import {
    fullConfigAtom,
    setFullConfigAtom,
    settingsAtom,
    hasCustomAIPresetsAtom,
} from "./config-signals";
import {
    useBlockCache,
    cleanupBlockAtomCache,
    cleanupTabAtomCache,
    getBlockMetaKeyAtom,
    useBlockMetaKeyAtom,
    getTabMetaKeyAtom,
    useTabMetaKeyAtom,
    getSettingsKeyAtom,
    useSettingsKeyAtom,
    getOverrideConfigAtom,
    useOverrideConfigAtom,
    getSettingsPrefixAtom,
    useBlockAtom,
    useBlockDataLoaded,
} from "./block-atom-cache";

// ---------------------------------------------------------------------------
// Global signals (replace Jotai atoms)
// ---------------------------------------------------------------------------

// Window identity — set once at init, never change.
export const [windowId, setWindowId] = createSignal("");
export const [clientId, setClientId] = createSignal("");
export const [staticTabId, setStaticTabId] = createSignal("");

// Derived objects from WOS
export const client = createMemo<Client>(() => {
    const cid = clientId();
    if (!cid) return null;
    return WOS.getObjectValue(WOS.makeORef("client", cid));
});

export const waveWindow = createMemo<WaveWindow>(() => {
    const wid = windowId();
    if (!wid) return null;
    return WOS.getObjectValue<WaveWindow>(WOS.makeORef("window", wid));
});

export const workspace = createMemo<Workspace>(() => {
    const win = waveWindow();
    if (!win) return null;
    return WOS.getObjectValue(WOS.makeORef("workspace", win.workspaceid));
});

export const tabAtom = createMemo<Tab>(() => {
    return WOS.getObjectValue(WOS.makeORef("tab", staticTabId()));
});

export const activeTabId = createMemo<string>(() => {
    const ws = workspace();
    const tabId = staticTabId();
    if (!ws) return tabId;
    return ws.activetabid || ws.pinnedtabids?.[0] || ws.tabids?.[0] || tabId;
});

// NOTE: uiContext must use activeTabId (derived from workspace), NOT staticTabId.
// staticTabId is set once at init and never changes. activeTabId tracks the
// workspace's current active tab so backend service calls get the correct tab.
export const uiContext = createMemo<UIContext>(() => ({
    windowid: windowId(),
    activetabid: activeTabId(),
}));

export { fullConfigAtom, setFullConfigAtom, settingsAtom, hasCustomAIPresetsAtom };

export const [isFullScreen, setIsFullScreen] = createSignal(false);
export const [controlShiftDelayAtom, setControlShiftDelayAtom] = createSignal(false);
export const [updaterStatusAtom, setUpdaterStatusAtom] = createSignal<UpdaterStatus>("up-to-date");
export const [updaterVersionAtom, setUpdaterVersionAtom] = createSignal<string | null>(null);

// Which renderer the most recently-mounted terminal actually loaded: "webgl"
// (GPU-accelerated, xterm WebglAddon) or "dom" (software fallback when WebGL is
// unavailable). Set by TermWrap.loadRendererAddon; read by the status-bar GPU
// indicator. null until the first terminal mounts.
export const [termRendererAtom, setTermRendererAtom] = createSignal<"webgl" | "dom" | null>(null);

export const reducedMotionSetting = createMemo(() => settingsAtom()?.["window:reducedmotion"]);
export const [reducedMotionSystemPreference, setReducedMotionSystemPreference] = createSignal(false);

export const prefersReducedMotionAtom = createMemo(() => reducedMotionSetting() || reducedMotionSystemPreference());

export type { BackendStatusState, BackendDeathInfo } from "./backendStatus";
export { backendStatusAtom, setBackendStatusAtom, backendDeathInfoAtom, setBackendDeathInfoAtom };

export const [typeAheadModalAtom, setTypeAheadModalAtom] = createSignal<Record<string, unknown>>({});
export const [modalOpen, setModalOpen] = createSignal(false);

// Connection status map: connName → ConnStatus signal
const [connStatusMap, setConnStatusMap] = createSignal(new Map<string, [() => ConnStatus, (v: ConnStatus) => void]>());

export const allConnStatus = createMemo<ConnStatus[]>(() => {
    const map = connStatusMap();
    return Array.from(map.values()).map(([get]) => get());
});

export const [flashErrors, setFlashErrors] = createSignal<FlashErrorType[]>([]);
export const [notifications, setNotifications] = createSignal<NotificationType[]>([]);
export const [notificationPopoverMode, setNotificationPopoverMode] = createSignal(false);
export const [reinitVersion, setReinitVersion] = createSignal(0);
export const [isTermMultiInput, setIsTermMultiInput] = createSignal(false);

export const [windowInstanceNumAtom, setWindowInstanceNumAtom] = createSignal(0);
export const [windowCountAtom, setWindowCountAtom] = createSignal(1);
export const [lanInstancesAtom, setLanInstancesAtom] = createSignal<LanInstance[]>([]);
// Last error message from the LAN discovery daemon (e.g. firewall block).
// Cleared on successful enable. See specs/lan-discovery-toggle.md.
export const [lanDiscoveryErrorAtom, setLanDiscoveryErrorAtom] = createSignal<string | null>(null);

// List of all open AgentMux window labels in this process. Updated by
// app-init's window-instances-changed listener whenever a window opens
// or closes. Consumed by the version-click instance panel.
// See SPEC_VERSION_INSTANCE_PANEL_2026_04_25.md.
export const [openWindowLabelsAtom, setOpenWindowLabelsAtom] = createSignal<string[]>([]);

// Same list, but each entry carries the backend window id alongside the
// host label so the InstancePanel can resolve per-window Window records
// (display name in meta, workspace name fallback) without an extra RPC.
// `windowId` may be null for windows whose `registerBackendWindow`
// round-trip hasn't fired yet (typical during the first ~100ms of a
// new window). See SPEC_WINDOW_RENAME_2026_04_27.md.
export type WindowEntry = { label: string; windowId: string | null };
export const [openWindowEntriesAtom, setOpenWindowEntriesAtom] = createSignal<WindowEntry[]>([]);

// Open floating panes in this process. Floating panes are process-scoped
// (shared across all windows) so they live in a separate atom from the
// per-window `openWindowEntriesAtom`. Updated by the launcher event reducer
// and seeded from `listWindowInstances()` at boot.
// See docs/specs/SPEC_INSTANCE_PANEL_FLOATING_PANES_SECTION_2026_06_24.md.
export type FloatingPaneEntry = { label: string; windowId: string | null };
export const [openFloatingPaneEntriesAtom, setOpenFloatingPaneEntriesAtom] =
    createSignal<FloatingPaneEntry[]>([]);

// ---------------------------------------------------------------------------
// GlobalAtomsType-compatible export (used in wos.ts callBackendService)
// ---------------------------------------------------------------------------

export const atoms = {
    clientId: clientId,
    uiContext: uiContext,
    client: client,
    waveWindow: waveWindow,
    workspace: workspace,
    fullConfigAtom: fullConfigAtom,
    settingsAtom: settingsAtom,
    hasCustomAIPresetsAtom: hasCustomAIPresetsAtom,
    tabAtom: tabAtom,
    staticTabId: staticTabId,
    activeTabId: activeTabId,
    isFullScreen: isFullScreen,
    controlShiftDelayAtom: controlShiftDelayAtom,
    updaterStatusAtom: updaterStatusAtom,
    updaterVersionAtom: updaterVersionAtom,
    prefersReducedMotionAtom: prefersReducedMotionAtom,
    typeAheadModalAtom: typeAheadModalAtom,
    modalOpen: modalOpen,
    allConnStatus: allConnStatus,
    flashErrors: flashErrors,
    notifications: notifications,
    notificationPopoverMode: notificationPopoverMode,
    reinitVersion: reinitVersion,
    isTermMultiInput: isTermMultiInput,
    backendStatusAtom: backendStatusAtom,
    lanInstancesAtom: lanInstancesAtom,
};

// ---------------------------------------------------------------------------
// globalPrimaryTabStartup
// ---------------------------------------------------------------------------

export let globalPrimaryTabStartup = false;

// ---------------------------------------------------------------------------
// Init
// ---------------------------------------------------------------------------

type GlobalInitOptions = {
    tabId: string;
    platform: NodeJS.Platform;
    windowId: string;
    clientId: string;
    primaryTabStartup?: boolean;
};

export function initGlobal(initOpts: GlobalInitOptions) {
    globalPrimaryTabStartup = initOpts.primaryTabStartup ?? false;
    setPlatform(initOpts.platform);
    // Add platform CSS class to body for platform-specific styling
    document.body.classList.add(`platform-${initOpts.platform}`);
    initGlobalSignals(initOpts);
}

function initGlobalSignals(initOpts: GlobalInitOptions) {
    setWindowId(initOpts.windowId);
    setClientId(initOpts.clientId);
    setStaticTabId(initOpts.tabId);

    try {
        getApi().onFullScreenChange((isFS) => setIsFullScreen(isFS));
    } catch (_) {}

    try {
        getApi().onMenuItemAbout(() => openModal(AboutModal));
    } catch (_) {}

    try {
        setUpdaterStatusAtom(getApi().getUpdaterStatus());
        setUpdaterVersionAtom(getApi().getUpdaterVersion());
        getApi().onUpdaterStatusChange((status) => {
            setUpdaterStatusAtom(status);
            setUpdaterVersionAtom(getApi().getUpdaterVersion());
        });
    } catch (_) {}

    if (globalThis.window != null) {
        const reducedMotionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
        setReducedMotionSystemPreference(!reducedMotionQuery || reducedMotionQuery.matches);
        reducedMotionQuery?.addEventListener("change", () => {
            setReducedMotionSystemPreference(reducedMotionQuery.matches);
        });
    }

    try {
        initBackendStatusListeners(getApi(), reconnectWS);
    } catch (_) {}

    // Expose atoms on window for wos.ts callBackendService
    window.globalAtoms = atoms;
}

export function initGlobalEventSubs(initOpts: AgentMuxInitOpts) {
    waveEventSubscribe(
        {
            eventType: WpsEvent.WaveObjUpdate,
            handler: (event) => {
                const update: WaveObjUpdate = event.data;
                WOS.updateWaveObject(update);
            },
        },
        {
            eventType: WpsEvent.Config,
            handler: (event) => {
                const fullConfig = (event.data as WatcherUpdate).fullconfig;
                setFullConfigAtom(fullConfig);
            },
        },
        {
            eventType: WpsEvent.UserInput,
            handler: (event) => {
                const data: UserInputRequest = event.data;
                openModal(UserInputModal, { ...data });
            },
            scope: initOpts.windowId,
        },
        {
            eventType: WpsEvent.BlockFile,
            handler: (event) => {
                const fileData: WSFileEventData = event.data;
                const fileSubject = getFileSubject(fileData.zoneid, fileData.filename);
                if (fileSubject != null) fileSubject.next(fileData);
            },
        },
        {
            eventType: "laninstances",
            handler: (event) => {
                const instances: LanInstance[] = event.data ?? [];
                setLanInstancesAtom(instances);
                // A successful broadcast clears any prior error state.
                setLanDiscoveryErrorAtom(null);
            },
        },
        {
            eventType: "laninstances:error",
            handler: (event) => {
                const errMsg = event.data?.error ?? "unknown error";
                setLanDiscoveryErrorAtom(String(errMsg));
            },
        },
        {
            // Server-initiated floating pane (e.g. OpenEditor(floating:true)).
            // srv creates the block + tears it into a fresh floating workspace,
            // then broadcasts this directive scoped to the source window — srv
            // can't open OS windows, so the window's frontend calls the host
            // `open_floating_pane_window` command (same as a drag tear-off).
            // See docs/specs/SPEC_OPENEDITOR_FLOATING_AND_COLLAPSED_TREE_2026_06_16.md.
            eventType: "openfloatingpane",
            scope: initOpts.windowId,
            handler: (event) => {
                const data = event.data as { block_id?: string; workspace_id?: string };
                if (!data?.block_id || !data?.workspace_id) return;
                void (async () => {
                    try {
                        const { invokeCommand } = await import("@/app/platform/ipc");
                        // width/height are DIP on all platforms; x/y are physical
                        // px on Windows, DIP on macOS/Linux (see floating_pane.rs).
                        // Center a default-sized floater over the current window.
                        const dpr = window.devicePixelRatio || 1;
                        const isWindows = /Windows/i.test(navigator.userAgent);
                        const width = Math.max(600, Math.min(1200, Math.round(window.innerWidth * 0.5)));
                        const height = Math.max(400, Math.min(900, Math.round(window.innerHeight * 0.6)));
                        const cssX = window.screenX + Math.max(0, (window.outerWidth - width) / 2);
                        const cssY = window.screenY + Math.max(0, (window.outerHeight - height) / 2);
                        await invokeCommand("open_floating_pane_window", {
                            pane_id: data.block_id,
                            workspace_id: data.workspace_id,
                            x: isWindows ? Math.round(cssX * dpr) : Math.round(cssX),
                            y: isWindows ? Math.round(cssY * dpr) : Math.round(cssY),
                            width,
                            height,
                        });
                    } catch (e) {
                        console.error("[openfloatingpane] failed to open floating window", e);
                    }
                })();
            },
        },
    );
}

// Block / tab atom caches — moved to block-atom-cache.ts; re-exported below for
// backward-compat (97 files import from this module).
export {
    useBlockCache,
    cleanupBlockAtomCache,
    cleanupTabAtomCache,
    getBlockMetaKeyAtom,
    useBlockMetaKeyAtom,
    getTabMetaKeyAtom,
    useTabMetaKeyAtom,
    getSettingsKeyAtom,
    useSettingsKeyAtom,
    getOverrideConfigAtom,
    useOverrideConfigAtom,
    getSettingsPrefixAtom,
    useBlockAtom,
    useBlockDataLoaded,
} from "./block-atom-cache";

// ---------------------------------------------------------------------------
// Block creation / layout actions
// ---------------------------------------------------------------------------

export async function createBlockSplitHorizontally(
    blockDef: BlockDef,
    targetBlockId: string,
    position: "before" | "after"
): Promise<string> {
    const layoutModel = getLayoutModelForStaticTab();
    const rtOpts: RuntimeOpts = { termsize: { rows: 25, cols: 80 } };
    const newBlockId = await ObjectService.CreateBlock(blockDef, rtOpts);
    const targetNodeId = layoutModel.getNodeByBlockId(targetBlockId)?.id;
    if (targetNodeId == null) throw new Error(`targetNodeId not found for blockId: ${targetBlockId}`);
    const splitAction: LayoutTreeSplitHorizontalAction = {
        type: LayoutTreeActionType.SplitHorizontal,
        targetNodeId,
        newNode: newLayoutNode(undefined, undefined, undefined, { blockId: newBlockId }),
        position,
        focused: true,
    };
    layoutModel.treeReducer(splitAction);
    return newBlockId;
}

export async function createBlockSplitVertically(
    blockDef: BlockDef,
    targetBlockId: string,
    position: "before" | "after"
): Promise<string> {
    const layoutModel = getLayoutModelForStaticTab();
    const rtOpts: RuntimeOpts = { termsize: { rows: 25, cols: 80 } };
    const newBlockId = await ObjectService.CreateBlock(blockDef, rtOpts);
    const targetNodeId = layoutModel.getNodeByBlockId(targetBlockId)?.id;
    if (targetNodeId == null) throw new Error(`targetNodeId not found for blockId: ${targetBlockId}`);
    const splitAction: LayoutTreeSplitVerticalAction = {
        type: LayoutTreeActionType.SplitVertical,
        targetNodeId,
        newNode: newLayoutNode(undefined, undefined, undefined, { blockId: newBlockId }),
        position,
        focused: true,
    };
    layoutModel.treeReducer(splitAction);
    return newBlockId;
}

export async function createBlock(blockDef: BlockDef, magnified = false, ephemeral = false): Promise<string> {
    const layoutModel = getLayoutModelForStaticTab();
    const rtOpts: RuntimeOpts = { termsize: { rows: 25, cols: 80 } };
    const blockId = await ObjectService.CreateBlock(blockDef, rtOpts);
    if (ephemeral) {
        layoutModel.newEphemeralNode(blockId);
        return blockId;
    }
    const insertNodeAction: LayoutTreeInsertNodeAction = {
        type: LayoutTreeActionType.InsertNode,
        node: newLayoutNode(undefined, undefined, undefined, { blockId }),
        magnified,
        focused: true,
    };
    layoutModel.treeReducer(insertNodeAction);
    return blockId;
}

export async function replaceBlock(blockId: string, blockDef: BlockDef, focus: boolean): Promise<string> {
    const layoutModel = getLayoutModelForStaticTab();
    const rtOpts: RuntimeOpts = { termsize: { rows: 25, cols: 80 } };
    const newBlockId = await ObjectService.CreateBlock(blockDef, rtOpts);
    setTimeout(() => {
        fireAndForget(() => ObjectService.DeleteBlock(blockId));
    }, 300);
    const targetNodeId = layoutModel.getNodeByBlockId(blockId)?.id;
    if (targetNodeId == null) throw new Error(`targetNodeId not found for blockId: ${blockId}`);
    const replaceNodeAction: LayoutTreeReplaceNodeAction = {
        type: LayoutTreeActionType.ReplaceNode,
        targetNodeId,
        newNode: newLayoutNode(undefined, undefined, undefined, { blockId: newBlockId }),
        focused: focus,
    };
    layoutModel.treeReducer(replaceNodeAction);
    return newBlockId;
}

// ---------------------------------------------------------------------------
// Wave file fetching
// ---------------------------------------------------------------------------

export async function fetchWaveFile(
    zoneId: string,
    fileName: string,
    offset?: number
): Promise<{ data: Uint8Array; fileInfo: WaveFile }> {
    const usp = new URLSearchParams();
    usp.set("zoneid", zoneId);
    usp.set("name", fileName);
    if (offset != null) usp.set("offset", offset.toString());
    // Use X-AuthKey header instead of `?authkey=` query-string fallback.
    // The fallback was removed in the 2026-05-11 audit (C3) for everything
    // except the /ws upgrade route, where headers aren't possible.
    const headers: Record<string, string> = {};
    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) headers["X-AuthKey"] = authKey;
    }
    const resp = await fetch(getWebServerEndpoint() + "/agentmux/file?" + usp.toString(), { headers });
    if (!resp.ok) {
        if (resp.status === 404) return { data: null, fileInfo: null };
        throw new Error("error getting wave file: " + resp.statusText);
    }
    if (resp.status == 204) return { data: null, fileInfo: null };
    const fileInfo64 = resp.headers.get("X-ZoneFileInfo");
    if (fileInfo64 == null) throw new Error(`missing zone file info for ${zoneId}:${fileName}`);
    const fileInfo = JSON.parse(atob(fileInfo64));
    const data = await resp.arrayBuffer();
    return { data: new Uint8Array(data), fileInfo };
}

// ---------------------------------------------------------------------------
// Focus / node
// ---------------------------------------------------------------------------

export function setNodeFocus(nodeId: string) {
    getLayoutModelForStaticTab().focusNode(nodeId);
}

// ---------------------------------------------------------------------------
// Block component model registry
// ---------------------------------------------------------------------------

const blockComponentModelMap = new Map<string, BlockComponentModel>();

export function registerBlockComponentModel(blockId: string, bcm: BlockComponentModel) {
    blockComponentModelMap.set(blockId, bcm);
}

export function unregisterBlockComponentModel(blockId: string) {
    blockComponentModelMap.delete(blockId);
    cleanupBlockAtomCache(blockId);
}

export function getBlockComponentModel(blockId: string): BlockComponentModel {
    return blockComponentModelMap.get(blockId);
}

export function getAllBlockComponentModels(): BlockComponentModel[] {
    return Array.from(blockComponentModelMap.values());
}

export function getFocusedBlockId(): string {
    const layoutModel = getLayoutModelForStaticTab();
    const focusedLayoutNode = layoutModel.focusedNode();
    return focusedLayoutNode?.data?.blockId;
}

export function refocusNode(blockId: string) {
    if (blockId == null) {
        blockId = getFocusedBlockId();
        if (blockId == null) return;
    }
    const layoutModel = getLayoutModelForStaticTab();
    const layoutNodeId = layoutModel.getNodeByBlockId(blockId);
    if (layoutNodeId?.id == null) return;
    layoutModel.focusNode(layoutNodeId.id);
    const bcm = getBlockComponentModel(blockId);
    const ok = bcm?.viewModel?.giveFocus?.();
    if (!ok) {
        const inputElem = document.getElementById(`${blockId}-dummy-focus`);
        inputElem?.focus();
    }
}

/**
 * Open or focus a pane by view type.
 * If a block with the given viewType already exists in the current tab's layout,
 * focus it. Otherwise create a new block using blockDef (defaults to `{ meta: { view: viewType } }`).
 */
export async function openOrFocusPaneByView(viewType: string, blockDef?: BlockDef): Promise<void> {
    for (const bcm of blockComponentModelMap.values()) {
        if (bcm.viewModel?.viewType === viewType) {
            const blockId = (bcm.viewModel as any).blockId as string | undefined;
            if (blockId) {
                refocusNode(blockId);
                return;
            }
        }
    }
    await createBlock(blockDef ?? { meta: { view: viewType } });
}

// ---------------------------------------------------------------------------
// Counters (dev tooling)
// ---------------------------------------------------------------------------

const Counters = new Map<string, number>();

export function countersClear() {
    Counters.clear();
}

export function counterInc(name: string, incAmt = 1) {
    let count = Counters.get(name) ?? 0;
    count += incAmt;
    Counters.set(name, count);
}

export function countersPrint() {
    let outStr = "";
    for (const [name, count] of Counters.entries()) {
        outStr += `${name}: ${count}\n`;
    }
    console.log(outStr);
}

// ---------------------------------------------------------------------------
// Connection status
// ---------------------------------------------------------------------------

export async function loadConnStatus() {
    const connStatusArr = await ClientService.GetAllConnStatus();
    if (connStatusArr == null) return;
    for (const connStatus of connStatusArr) {
        const [, setter] = getOrCreateConnStatusPair(connStatus.connection);
        setter(connStatus);
    }
}

export function subscribeToConnEvents() {
    waveEventSubscribe({
        eventType: WpsEvent.ConnChange,
        handler: (event: WaveEvent) => {
            try {
                const connStatus = event.data as ConnStatus;
                if (connStatus == null || isBlank(connStatus.connection)) return;
                console.log("connstatus update", connStatus);
                const [, setter] = getOrCreateConnStatusPair(connStatus.connection);
                setter(connStatus);
            } catch (e) {
                console.log("connchange error", e);
            }
        },
    });
}

function makeDefaultConnStatus(conn: string, connected: boolean, hasconnected: boolean): ConnStatus {
    return {
        connection: conn,
        connected,
        error: null,
        status: connected ? "connected" : "disconnected",
        hasconnected,
        activeconnnum: 0,
    };
}

function getOrCreateConnStatusPair(conn: string): [() => ConnStatus, (v: ConnStatus) => void] {
    const map = connStatusMap();
    let pair = map.get(conn);
    if (pair == null) {
        const initial =
            isBlank(conn) || conn.startsWith("aws:")
                ? makeDefaultConnStatus(conn, true, true)
                : makeDefaultConnStatus(conn, false, false);
        const [get, set] = createSignal<ConnStatus>(initial);
        pair = [get, set];
        const newMap = new Map(map);
        newMap.set(conn, pair);
        setConnStatusMap(newMap);
    }
    return pair;
}

export function getConnStatusAtom(conn: string): () => ConnStatus {
    return getOrCreateConnStatusPair(conn)[0];
}

// ---------------------------------------------------------------------------
// Flash errors / notifications
// ---------------------------------------------------------------------------

export function pushFlashError(ferr: FlashErrorType) {
    if (ferr.expiration == null) ferr.expiration = Date.now() + 5000;
    ferr.id = crypto.randomUUID();
    setFlashErrors((prev) => [...prev, ferr]);
}

export function addOrUpdateNotification(notif: NotificationType) {
    setNotifications((prev) => {
        const withoutThis = prev.filter((n) => n.id !== notif.id);
        return [...withoutThis, notif];
    });
}

export function pushNotification(notif: NotificationType) {
    if (!notif.id && notif.persistent) return;
    notif.id = notif.id ?? crypto.randomUUID();
    addOrUpdateNotification(notif);
}

export function removeNotificationById(id: string) {
    setNotifications((prev) => prev.filter((n) => n.id !== id));
}

export function removeFlashError(id: string) {
    setFlashErrors((prev) => prev.filter((ferr) => ferr.id !== id));
}

export function removeNotification(id: string) {
    setNotifications((prev) => prev.filter((n) => n.id !== id));
}

// ---------------------------------------------------------------------------
// Tab management
// ---------------------------------------------------------------------------

/**
 * Default color for newly-created tabs. Matches the "Blue" entry in
 * TAB_COLORS and the startup-tab color applied by tabbar.tsx — every
 * new tab now starts the same way as the first one. Users can change
 * the colour per-tab via the right-click menu after creation.
 */
const DEFAULT_NEW_TAB_COLOR = "#3b82f6";

export function createTab() {
    const ws = workspace();
    if (ws == null) return;
    fireAndForget(async () => {
        // Pin the gate while CreateTab + UpdateObjectMeta + preset import
        // + applyTabPreset run. Calling scheduleRevealLift here would let
        // the 80ms SETTLE window elapse during the (longtask-free) RPCs
        // and layout-model polling inside applyTabPreset, so the gate
        // would lift before the agent/sysinfo/swarm blocks have mounted
        // and the user would still see the piecemeal cascade. The
        // detector is started in `finally` once the preset apply has
        // returned (or failed) — at that point SETTLE / MAX_GATE measure
        // the actual mount window. See issue #774 /
        // SPEC_TAB_CONTENT_REVEAL_GATE.md.
        holdRevealGate();
        try {
            const tabId = await WorkspaceService.CreateTab(ws.oid, "", true, false);
            await ObjectService.UpdateObjectMeta(
                WOS.makeORef("tab", tabId),
                { "tab:color": DEFAULT_NEW_TAB_COLOR } as MetaType,
            );
            // Default-layout preset (agent + sysinfo + swarm). Lives in
            // a single central module so any future tab-creation path
            // (duplicate, tear-off destination, startup-tab backfill)
            // can reuse the same panes layout. See
            // frontend/app/tab/tab-presets.ts.
            const { applyTabPreset, DEFAULT_TAB_PRESET } = await import("@/app/tab/tab-presets");
            await applyTabPreset(tabId, DEFAULT_TAB_PRESET);
        } catch (e) {
            console.error("[createTab] failed:", e);
        } finally {
            // Pair with holdRevealGate above — without this the gate
            // would stay pinned forever on the error path.
            scheduleRevealLift();
        }
    });
}

// Tracks an in-flight tab-switch measurement so rapid back-to-back
// switches (held Ctrl+Tab, programmatic bursts) don't collide on the
// shared `tab-switch:start` mark name. performance.mark throws on
// duplicates and the second call would silently drop its measurement.
// Sequence guard ensures the prior switch's pending double-rAF
// markEnd doesn't close the new switch's measurement instead.
let tabSwitchInFlight = false;
let tabSwitchSeq = 0;

export async function setActiveTab(tabId: string): Promise<void> {
    const ws = workspace();
    if (ws == null) return;
    const fromTabId = activeTabId();
    if (fromTabId === tabId) return;
    // Canonical chokepoint for tab-switch perf marks. Wraps every entry
    // path: click (tabbar), keyboard (Ctrl+Tab/1..9 in keymodel),
    // palette (command-registry), test app API (cef-api). markEnd lands
    // two rAFs after the IPC so the duration captures user-perceived
    // switch cost — IPC + Solid fan-out + layout + paint — not just IPC.
    // Backend-driven switches (tearoff merge, cross-drag) bypass this
    // function and are not measured here; they're rare and observable
    // via the long-task timeline.
    if (tabSwitchInFlight) {
        // Close prior measurement (truncated) so the new markStart
        // doesn't collide. The prior call's pending rAF markEnd will
        // see its sequence is stale and skip.
        markEnd("tab-switch", "interrupted");
    }
    const mySeq = ++tabSwitchSeq;
    tabSwitchInFlight = true;
    markStart("tab-switch", { from: fromTabId, to: tabId });
    // Pin the gate during the SetActiveTab RPC so the destination
    // tab can't paint piecemeal once the workspace update lands.
    // The auto-lift detector is started in `finally` (i.e. AFTER
    // the active-tab update lands) so SETTLE / MAX_GATE measure the
    // destination mount window, not the longtask-free RPC duration.
    // Honours rapid Ctrl-Tab spam — each call resets the detector.
    // See issue #774 / SPEC_TAB_CONTENT_REVEAL_GATE.md.
    holdRevealGate();
    try {
        await WorkspaceService.SetActiveTab(ws.oid, tabId);
    } finally {
        // Pair with holdRevealGate above. Also lifts the gate on
        // the RPC-throws path so the user isn't stuck on a hidden
        // source tab.
        scheduleRevealLift();
        requestAnimationFrame(() =>
            requestAnimationFrame(() => {
                if (mySeq === tabSwitchSeq) {
                    markEnd("tab-switch");
                    tabSwitchInFlight = false;
                }
            })
        );
    }
}

// ---------------------------------------------------------------------------
// Telemetry
// ---------------------------------------------------------------------------

export function recordTEvent(event: string, props?: TEventProps) {
    if (props == null) props = {};
    RpcApi.RecordTEventCommand(TabRpcClient, { event, props }, { noresponse: true });
}

// ---------------------------------------------------------------------------
// Misc utilities
// ---------------------------------------------------------------------------

const objectIdWeakMap = new WeakMap();
let objectIdCounter = 0;

export function getObjectId(obj: any): number {
    if (!objectIdWeakMap.has(obj)) objectIdWeakMap.set(obj, objectIdCounter++);
    return objectIdWeakMap.get(obj);
}

let cachedIsDev: boolean = null;
export function isDev() {
    if (cachedIsDev == null) cachedIsDev = getApi().getIsDev();
    return cachedIsDev;
}

let cachedUserName: string = null;
export function getUserName(): string {
    if (cachedUserName == null) cachedUserName = getApi().getUserName();
    return cachedUserName;
}

let cachedHostName: string = null;
export function getHostName(): string {
    if (cachedHostName == null) cachedHostName = getApi().getHostName();
    return cachedHostName;
}

export async function openLink(uri: string) {
    getApi().openExternal(uri);
}

// Re-export WOS, getApi, and setPlatform for call-sites that import them from here
export { WOS, setPlatform, getApi };
