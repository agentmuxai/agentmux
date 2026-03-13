// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0
//
// Global app state — migrated from Jotai atoms to SolidJS signals.

import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
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
import { deepCompareReturnPrev, fireAndForget, getPrefixedSettings, isBlank } from "@/util/util";
import { createMemo, createRoot, createSignal } from "solid-js";
import { modalsModel } from "./modalmodel";
import { ClientService, ObjectService, WorkspaceService } from "./services";
import * as WOS from "./wos";
import { getFileSubject, waveEventSubscribe } from "./wps";

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

export const [fullConfigAtom, setFullConfigAtom] = createSignal<FullConfigType>(null);

export const settingsAtom = createMemo<SettingsType>(() => fullConfigAtom()?.settings ?? ({} as SettingsType));

export const hasCustomAIPresetsAtom = createMemo<boolean>(() => {
    const fullConfig = fullConfigAtom();
    if (!fullConfig?.presets) return false;
    for (const presetId in fullConfig.presets) {
        if (presetId.startsWith("ai@") && presetId !== "ai@global" && presetId !== "ai@wave") {
            return true;
        }
    }
    return false;
});

export const [isFullScreen, setIsFullScreen] = createSignal(false);
export const [controlShiftDelayAtom, setControlShiftDelayAtom] = createSignal(false);
export const [updaterStatusAtom, setUpdaterStatusAtom] = createSignal<UpdaterStatus>("up-to-date");

export const reducedMotionSetting = createMemo(() => settingsAtom()?.["window:reducedmotion"]);
export const [reducedMotionSystemPreference, setReducedMotionSystemPreference] = createSignal(false);

export const prefersReducedMotionAtom = createMemo(() => reducedMotionSetting() || reducedMotionSystemPreference());

type BackendStatusState = "connecting" | "running" | "crashed";
export const [backendStatusAtom, setBackendStatusAtom] = createSignal<BackendStatusState>("running");

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
};

// ---------------------------------------------------------------------------
// globalStore shim — used by code not yet migrated to direct signal calls.
// globalStore.get(accessor) → accessor()
// globalStore.set(setter, value) → setter(value)
// ---------------------------------------------------------------------------

export const globalStore = {
    get<T>(accessor: (() => T) | any): T {
        if (typeof accessor === "function") return (accessor as () => T)();
        console.warn("[globalStore.get] non-function:", accessor);
        return undefined as unknown as T;
    },
    set<T>(setter: ((v: T | ((prev: T) => T)) => void) | any, value: T | ((prev: T) => T)) {
        if (typeof setter === "function") (setter as any)(value);
        else console.warn("[globalStore.set] non-function setter:", setter);
    },
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
        getApi().onMenuItemAbout(() => modalsModel.pushModal("AboutModal"));
    } catch (_) {}

    try {
        setUpdaterStatusAtom(getApi().getUpdaterStatus());
        getApi().onUpdaterStatusChange((status) => setUpdaterStatusAtom(status));
    } catch (_) {}

    if (globalThis.window != null) {
        const reducedMotionQuery = window.matchMedia("(prefers-reduced-motion: reduce)");
        setReducedMotionSystemPreference(!reducedMotionQuery || reducedMotionQuery.matches);
        reducedMotionQuery?.addEventListener("change", () => {
            setReducedMotionSystemPreference(reducedMotionQuery.matches);
        });
    }

    try {
        getApi().listen("backend-terminated", () => setBackendStatusAtom("crashed"));
        getApi().listen("backend-ready", () => setBackendStatusAtom("running"));
    } catch (_) {}

    // Expose atoms on window for wos.ts callBackendService
    (window as any).globalAtoms = atoms;
}

export function initGlobalEventSubs(initOpts: AgentMuxInitOpts) {
    waveEventSubscribe(
        {
            eventType: "waveobj:update",
            handler: (event) => {
                const update: WaveObjUpdate = event.data;
                WOS.updateWaveObject(update);
            },
        },
        {
            eventType: "config",
            handler: (event) => {
                const fullConfig = (event.data as WatcherUpdate).fullconfig;
                setFullConfigAtom(fullConfig);
            },
        },
        {
            eventType: "userinput",
            handler: (event) => {
                const data: UserInputRequest = event.data;
                modalsModel.pushModal("UserInputModal", { ...data });
            },
            scope: initOpts.windowId,
        },
        {
            eventType: "blockfile",
            handler: (event) => {
                const fileData: WSFileEventData = event.data;
                const fileSubject = getFileSubject(fileData.zoneid, fileData.filename);
                if (fileSubject != null) fileSubject.next(fileData);
            },
        },
    );
}

// ---------------------------------------------------------------------------
// Block / tab atom caches (used by per-block derived memos)
// ---------------------------------------------------------------------------

const blockCache = new Map<string, Map<string, any>>();

export function useBlockCache<T>(blockId: string, name: string, makeFn: () => T): T {
    let blockMap = blockCache.get(blockId);
    if (blockMap == null) {
        blockMap = new Map<string, any>();
        blockCache.set(blockId, blockMap);
    }
    let value = blockMap.get(name);
    if (value == null) {
        value = makeFn();
        blockMap.set(name, value);
    }
    return value as T;
}

const blockAtomCache = new Map<string, Map<string, () => any>>();
const tabAtomCache = new Map<string, Map<string, () => any>>();

function getSingleBlockAtomCache(blockId: string): Map<string, () => any> {
    let bc = blockAtomCache.get(blockId);
    if (bc == null) {
        bc = new Map();
        blockAtomCache.set(blockId, bc);
    }
    return bc;
}

function getSingleConnAtomCache(connName: string): Map<string, () => any> {
    return getSingleBlockAtomCache(connName);
}

function getSingleTabAtomCache(tabId: string): Map<string, () => any> {
    let tc = tabAtomCache.get(tabId);
    if (tc == null) {
        tc = new Map();
        tabAtomCache.set(tabId, tc);
    }
    return tc;
}

export function getBlockMetaKeyAtom<T extends keyof MetaType>(blockId: string, key: T): () => MetaType[T] {
    const bc = getSingleBlockAtomCache(blockId);
    const name = "#meta-" + key;
    let memo = bc.get(name);
    if (memo == null) {
        memo = createRoot(() => createMemo(() => {
            const blockAccessor = WOS.getWaveObjectAtom(WOS.makeORef("block", blockId));
            const blockData = blockAccessor();
            return blockData?.meta?.[key];
        }));
        bc.set(name, memo);
    }
    return memo as () => MetaType[T];
}

export function useBlockMetaKeyAtom<T extends keyof MetaType>(blockId: string, key: T): MetaType[T] {
    return getBlockMetaKeyAtom(blockId, key)();
}

export function getTabMetaKeyAtom<T extends keyof MetaType>(tabId: string, key: T): () => MetaType[T] {
    const tc = getSingleTabAtomCache(tabId);
    const name = "#meta-" + key;
    let memo = tc.get(name);
    if (memo == null) {
        memo = createRoot(() => createMemo(() => {
            const tabAccessor = WOS.getWaveObjectAtom(WOS.makeORef("tab", tabId));
            const tabData = tabAccessor();
            return tabData?.meta?.[key];
        }));
        tc.set(name, memo);
    }
    return memo as () => MetaType[T];
}

export function useTabMetaKeyAtom<T extends keyof MetaType>(tabId: string, key: T): MetaType[T] {
    return getTabMetaKeyAtom(tabId, key)();
}

// ---------------------------------------------------------------------------
// Connection config
// ---------------------------------------------------------------------------

function getConnConfigKeyAtom<T extends keyof ConnKeywords>(connName: string, key: T): () => ConnKeywords[T] {
    const cc = getSingleConnAtomCache(connName);
    const name = "#conn-" + key;
    let memo = cc.get(name);
    if (memo == null) {
        memo = createRoot(() => createMemo(() => fullConfigAtom()?.connections?.[connName]?.[key]));
        cc.set(name, memo);
    }
    return memo as () => ConnKeywords[T];
}

// ---------------------------------------------------------------------------
// Settings atoms
// ---------------------------------------------------------------------------

const settingsAtomCache = new Map<string, () => any>();

export function getSettingsKeyAtom<T extends keyof SettingsType>(key: T): () => SettingsType[T] {
    let memo = settingsAtomCache.get(key) as () => SettingsType[T];
    if (memo == null) {
        memo = createRoot(() => createMemo(() => {
            const settings = settingsAtom();
            if (settings == null) return null;
            return settings[key];
        }));
        settingsAtomCache.set(key, memo);
    }
    return memo;
}

export function useSettingsKeyAtom<T extends keyof SettingsType>(key: T): SettingsType[T] {
    return getSettingsKeyAtom(key)();
}

export function getOverrideConfigAtom<T extends keyof SettingsType>(blockId: string, key: T): () => SettingsType[T] {
    const bc = getSingleBlockAtomCache(blockId);
    const name = "#settingsoverride-" + key;
    let memo = bc.get(name);
    if (memo == null) {
        memo = createRoot(() => createMemo(() => {
            const metaKeyMemo = getBlockMetaKeyAtom(blockId, key as any);
            const metaKeyVal = metaKeyMemo();
            if (metaKeyVal != null) return metaKeyVal as SettingsType[T];

            const connNameMemo = getBlockMetaKeyAtom(blockId, "connection");
            const connName = connNameMemo();
            const connConfigKeyMemo = getConnConfigKeyAtom(connName, key as any);
            const connConfigKeyVal = connConfigKeyMemo();
            if (connConfigKeyVal != null) return connConfigKeyVal as SettingsType[T];

            const settingsKeyMemo = getSettingsKeyAtom(key);
            const settingsVal = settingsKeyMemo();
            if (settingsVal != null) return settingsVal;

            return null;
        }));
        bc.set(name, memo);
    }
    return memo as () => SettingsType[T];
}

export function useOverrideConfigAtom<T extends keyof SettingsType>(blockId: string, key: T): SettingsType[T] {
    return getOverrideConfigAtom(blockId, key)();
}

const settingsPrefixCache = new Map<string, () => SettingsType>();

export function getSettingsPrefixAtom(prefix: string): () => SettingsType {
    let memo = settingsPrefixCache.get(prefix + ":");
    if (memo == null) {
        const cacheKey = {};
        memo = createRoot(() => createMemo(() => {
            const settings = settingsAtom();
            const newValue = getPrefixedSettings(settings, prefix);
            return deepCompareReturnPrev(cacheKey, newValue);
        }));
        settingsPrefixCache.set(prefix + ":", memo);
    }
    return memo;
}

// ---------------------------------------------------------------------------
// Block atom cache (used by block components to store per-block memos)
// ---------------------------------------------------------------------------

export function useBlockAtom<T>(blockId: string, name: string, makeFn: () => () => T): () => T {
    const bc = getSingleBlockAtomCache(blockId);
    let memo = bc.get(name);
    if (memo == null) {
        memo = createRoot(makeFn);
        bc.set(name, memo);
        console.log("New BlockAtom", blockId, name);
    }
    return memo as () => T;
}

export function useBlockDataLoaded(blockId: string): boolean {
    const loadedMemo = useBlockAtom<boolean>(blockId, "block-loaded", () => {
        return WOS.getWaveObjectLoadingAtom(WOS.makeORef("block", blockId));
    });
    return loadedMemo();
}

// ---------------------------------------------------------------------------
// API
// ---------------------------------------------------------------------------

export function getApi(): AppApi {
    return (window as any).api;
}

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
    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) usp.set("authkey", authKey);
    }
    const resp = await fetch(getWebServerEndpoint() + "/wave/file?" + usp.toString());
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
        eventType: "connchange",
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
        wshenabled: false,
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

export function createTab() {
    const ws = workspace();
    if (ws == null) return;
    WorkspaceService.CreateTab(ws.oid, "", true, false).catch((e) => {
        console.error("[createTab] failed:", e);
    });
}

export async function setActiveTab(tabId: string): Promise<void> {
    const ws = workspace();
    if (ws == null) return;
    await WorkspaceService.SetActiveTab(ws.oid, tabId);
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

// Re-export WOS for call-sites that import it from here
export { WOS, setPlatform };
