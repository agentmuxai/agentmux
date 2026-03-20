// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { App } from "@/app/app";
import {
    globalRefocus,
    registerControlShiftStateUpdateHandler,
    registerGlobalKeys,
} from "@/app/store/keymodel";
import { modalsModel } from "@/app/store/modalmodel";
import { ClientService, WindowService, WorkspaceService } from "@/app/store/services";
import { RpcApi } from "@/app/store/wshclientapi";
import { initWshrpc, TabRpcClient } from "@/app/store/wshrpcutil";
import { getLayoutModelForStaticTab } from "@/layout/index";
import {
    atoms,
    countersClear,
    countersPrint,
    getApi,
    globalStore,
    initGlobal,
    initGlobalEventSubs,
    loadConnStatus,
    pushFlashError,
    pushNotification,
    removeNotificationById,
    subscribeToConnEvents,
    windowCountAtom,
    windowInstanceNumAtom,
    setWindowInstanceNumAtom,
    setWindowCountAtom,
    setReinitVersion,
    setUpdaterStatusAtom,
    setFullConfigAtom,
} from "@/app/store/global";
import * as WOS from "@/app/store/wos";
import { loadFonts } from "@/util/fontutil";
import { setKeyUtilPlatform } from "@/util/keyutil";
import { render } from "solid-js/web";
import { benchMark, benchDump } from "@/util/startup-bench";
import { ContextMenuModel } from "@/app/store/contextmenu";
// Static import — avoids dynamic import() hang in WebKitGTK over tauri:// protocol.
import { getCurrentWindow } from "@tauri-apps/api/window";

// Deferred — assigned inside initBare() after window.api is ready.
// Do NOT call getApi() at module level: this file is statically imported by
// tauri-bootstrap.ts before setupTauriApi() runs, so window.api does not exist yet.
let platform: string;
let appVersion: string;
let savedInitOpts: AgentMuxInitOpts = null;

// Update window title with instance ID if running in multi-instance mode
async function updateWindowTitleWithInstanceID() {
    try {
        // Only in Tauri
        if (!(window as any).__TAURI_INTERNALS__) {
            return;
        }

        const { invoke } = await import("@tauri-apps/api/core");
        const { readTextFile, exists } = await import("@tauri-apps/plugin-fs");

        const dataDir = await invoke<string>("get_data_dir");
        // Use platform-agnostic path joining
        const instanceIDPath = dataDir.endsWith("/") || dataDir.endsWith("\\")
            ? `${dataDir}instance-id.txt`
            : `${dataDir}/instance-id.txt`;

        const fileExists = await exists(instanceIDPath);
        if (fileExists) {
            const instanceID = await readTextFile(instanceIDPath);
            if (instanceID && instanceID.trim()) {
                const newTitle = `AgentMux ${appVersion} [${instanceID.trim()}]`;
                document.title = newTitle;

                // Also update Tauri window title
                const window = getCurrentWindow();
                await window.setTitle(newTitle);

                console.log(`[multi-instance] Running as: ${instanceID.trim()}`);
            }
        }
    } catch (e) {
        // Ignore errors - instance file may not exist for default instance
        console.log("[multi-instance] No instance ID file found (default instance)");
    }
}

// Call after a short delay to allow backend to write the file
setTimeout(updateWindowTitleWithInstanceID, 1000);


(window as any).WOS = WOS;
(window as any).globalStore = globalStore;
(window as any).globalAtoms = atoms;
(window as any).RpcApi = RpcApi;
(window as any).isFullScreen = false;
(window as any).countersPrint = countersPrint;
(window as any).countersClear = countersClear;
(window as any).getLayoutModelForStaticTab = getLayoutModelForStaticTab;
(window as any).pushFlashError = pushFlashError;
(window as any).pushNotification = pushNotification;
(window as any).removeNotificationById = removeNotificationById;
(window as any).modalsModel = modalsModel;


/** Wrap a promise with a timeout. Rejects with a descriptive error if it takes too long. */
function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
    return Promise.race([
        promise,
        new Promise<T>((_, reject) =>
            setTimeout(() => reject(new Error(`Timeout: ${label} did not respond within ${ms / 1000}s`)), ms)
        ),
    ]);
}

/** Make body visible and show an error message so the user never sees an infinite grey screen. */
function showStartupError(message: string) {
    document.body.style.visibility = "visible";
    document.body.style.opacity = "1";
    document.body.classList.remove("is-transparent");
    // Remove the "Starting AgentMux..." loader
    const loader = document.getElementById("startup-loading");
    if (loader) loader.remove();
    // Show error in the main div
    const main = document.getElementById("main");
    if (main) {
        main.innerHTML = "";
        const errorDiv = document.createElement("div");
        errorDiv.style.cssText = "padding: 40px; font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; color: #f7f7f7;";
        const title = document.createElement("h2");
        title.textContent = "AgentMux failed to start";
        title.style.cssText = "color: #ff6b6b; margin-bottom: 16px;";
        errorDiv.appendChild(title);
        const msg = document.createElement("pre");
        msg.textContent = message;
        msg.style.cssText = "background: #1a1a1a; padding: 16px; border-radius: 8px; overflow-x: auto; white-space: pre-wrap; font-size: 13px;";
        errorDiv.appendChild(msg);
        const hint = document.createElement("p");
        hint.textContent = "Press F12 for console details. Try closing and reopening the app.";
        hint.style.cssText = "margin-top: 16px; color: rgba(255,255,255,0.5); font-size: 13px;";
        errorDiv.appendChild(hint);
        main.appendChild(errorDiv);
    }
}

const RPC_TIMEOUT = 5_000; // 5 seconds for individual RPC calls

/**
 * Initialize the window instance number and count atoms, and subscribe to
 * the "window-instances-changed" Tauri event so the count stays reactive.
 * Called once per window after the wave UI is fully initialized.
 */
async function initInstanceTracking(): Promise<void> {
    try {
        const [instanceNum, windowCount] = await Promise.all([
            getApi().getInstanceNumber(),
            getApi().getWindowCount(),
        ]);
        setWindowInstanceNumAtom(instanceNum);
        setWindowCountAtom(windowCount);

        // Keep count in sync whenever any window opens or closes.
        const { listen } = await import("@tauri-apps/api/event");
        await listen<number>("window-instances-changed", (event) => {
            setWindowCountAtom(event.payload);
        });
    } catch (e) {
        console.warn("[initInstanceTracking] failed:", e);
    }
}

/**
 * Initialize AgentMux in Tauri mode by fetching client/window/workspace/tab data
 * from backend, verifying objects exist, and creating missing ones if needed.
 * Handles first-window and new-window creation.
 */
async function initTauriWave(): Promise<void> {
    const t0 = performance.now();
    const tlog = (label: string, since: number) => {
        const ms = (performance.now() - since).toFixed(1);
        const total = (performance.now() - t0).toFixed(1);
        console.log(`[startup-perf] ${label}: ${ms}ms (total: ${total}ms)`);
    };

    try {
        // Get client data
        let t = performance.now();
        const clientData = await withTimeout(ClientService.GetClientData(), RPC_TIMEOUT, "GetClientData");
        tlog("GetClientData", t);

        let windowId = clientData.windowids?.[0];

        // If no windows exist, create one
        if (!windowId) {
            t = performance.now();
            const newWindow = await withTimeout(WindowService.CreateWindow(null, ""), RPC_TIMEOUT, "CreateWindow");
            tlog("CreateWindow (no windows)", t);
            windowId = newWindow.oid;
        }

        // Verify window exists
        t = performance.now();
        let windowData = await withTimeout(WindowService.GetWindow(windowId), RPC_TIMEOUT, "GetWindow");
        tlog("GetWindow", t);

        if (!windowData) {
            t = performance.now();
            windowData = await withTimeout(WindowService.CreateWindow(null, ""), RPC_TIMEOUT, "CreateWindow");
            tlog("CreateWindow (fallback)", t);
            windowId = windowData.oid;
        }

        // Get workspace
        t = performance.now();
        let workspace = await withTimeout(WorkspaceService.GetWorkspace(windowData.workspaceid), RPC_TIMEOUT, "GetWorkspace");
        tlog("GetWorkspace", t);

        if (!workspace) {
            // Workspace missing → recreate entire window
            t = performance.now();
            await withTimeout(WindowService.CloseWindow(windowData.oid), RPC_TIMEOUT, "CloseWindow");
            windowData = await withTimeout(WindowService.CreateWindow(null, ""), RPC_TIMEOUT, "CreateWindow");
            workspace = await withTimeout(WorkspaceService.GetWorkspace(windowData.workspaceid), RPC_TIMEOUT, "GetWorkspace");
            tlog("Recreate window+workspace", t);
        }

        // Get active tab ID
        const tabId = workspace.activetabid ||
                     workspace.tabids?.[0] ||
                     workspace.pinnedtabids?.[0] ||
                     "";

        if (!tabId) {
            throw new Error("No tab found in workspace");
        }

        tlog("Phase 1 complete (discovery)", t0);

        // Create complete init options with ALL valid IDs
        const initOpts: AgentMuxInitOpts = {
            clientId: clientData.oid,
            windowId: windowData.oid,
            tabId: tabId,
            activate: true,
            primaryTabStartup: true,
        };

        // Initialize wave (this will render the UI)
        t = performance.now();
        await initWaveWrap(initOpts);
        tlog("initWaveWrap", t);
        tlog("TOTAL initTauriWave", t0);

        // Initialize instance tracking (must come after initWaveWrap so globalStore is ready)
        await initInstanceTracking();

        // Show the window now that it's fully initialized (Tauri starts hidden)
        try {
            const currentWindow = getCurrentWindow();
            benchMark("window-show");
            await currentWindow.show();
            if (platform === "linux") {
                await currentWindow.center();
            }
            await currentWindow.setFocus();
            benchDump(); // emit full startup timeline to log
        } catch (showError) {
            console.warn("[initTauriWave] Failed to show window:", showError);
        }

    } catch (error) {
        console.error("[initTauriWave] Initialization failed:", error);
        getApi().sendLog(`[initTauriWave] ERROR: ${error}`);
        showStartupError(String(error));
        // Show window even on error so user can see the error message
        try {
            await getCurrentWindow().show();
        } catch {}
    }
}

/**
 * Initialize a new (non-main) Tauri window by creating new backend objects.
 * Unlike initTauriWave() which reuses existing Window/Workspace/Tab,
 * this creates a fresh set for the new window.
 */
async function initTauriNewWindow(): Promise<void> {
    const t0 = performance.now();
    const tlog = (label: string, since: number) => {
        const ms = (performance.now() - since).toFixed(1);
        const total = (performance.now() - t0).toFixed(1);
        console.log(`[startup-perf] ${label}: ${ms}ms (total: ${total}ms)`);
        getApi().sendLog(`[startup-perf] ${label}: ${ms}ms (total: ${total}ms)`);
    };

    try {
        getApi().sendLog("[initTauriNewWindow] Creating new backend objects");

        // Get client data (reuse existing client)
        let t = performance.now();
        const clientData = await withTimeout(ClientService.GetClientData(), RPC_TIMEOUT, "GetClientData");
        tlog("GetClientData", t);

        // Create NEW window (not reuse)
        t = performance.now();
        const newWindow = await withTimeout(WindowService.CreateWindow(null, ""), RPC_TIMEOUT, "CreateWindow");
        tlog("CreateWindow", t);

        // Get the workspace that was auto-created with the window
        t = performance.now();
        const workspace = await withTimeout(WorkspaceService.GetWorkspace(newWindow.workspaceid), RPC_TIMEOUT, "GetWorkspace");
        tlog("GetWorkspace", t);
        if (!workspace) {
            throw new Error("Workspace not created with new window");
        }

        // Get the active tab ID from the workspace
        const tabId = workspace.activetabid ||
                     workspace.tabids?.[0] ||
                     workspace.pinnedtabids?.[0] ||
                     "";

        if (!tabId) {
            throw new Error("No tab found in new workspace");
        }

        tlog("Phase 1 complete (discovery)", t0);

        // Create complete init options with NEW IDs
        const initOpts: AgentMuxInitOpts = {
            clientId: clientData.oid,
            windowId: newWindow.oid,
            tabId: tabId,
            activate: true,
            primaryTabStartup: false, // Not primary (main window is primary)
        };

        // Initialize wave (this will render the UI)
        t = performance.now();
        await initWaveWrap(initOpts);
        tlog("initWaveWrap", t);
        tlog("TOTAL initTauriNewWindow", t0);

        // Initialize instance tracking (must come after initWaveWrap so globalStore is ready)
        await initInstanceTracking();

        // Show the window now that it's initialized
        try {
            const currentWindow = getCurrentWindow();
            await currentWindow.show();
            await currentWindow.setFocus();
            getApi().sendLog("[initTauriNewWindow] Window shown and focused");
        } catch (showError) {
            console.warn("[initTauriNewWindow] Failed to show window:", showError);
        }

    } catch (error) {
        console.error("[initTauriNewWindow] Initialization failed:", error);
        try { getApi().sendLog(`[initTauriNewWindow] ❌ Error: ${error}`); } catch {}
        showStartupError("New window: " + String(error));
        // Show Tauri window so user sees the error
        try {
            await getCurrentWindow().show();
        } catch {}
    }
}

export async function initBare() {
    // window.api is guaranteed to exist here — tauri-bootstrap.ts calls setupTauriApi()
    // before calling initBare(). Assign deferred module-level values now.
    platform = getApi().getPlatform();
    appVersion = getApi().getAboutModalDetails().version;
    document.title = `AgentMux ${appVersion}`;

    // Register context menu click handler now that window.api exists.
    ContextMenuModel.init();

    const bareStart = performance.now();
    (window as any).__startupPerfStart = bareStart;
    getApi().sendLog("Init Bare");
    document.body.style.visibility = "hidden";
    document.body.style.opacity = "0";
    document.body.classList.add("is-transparent");

    // Check if we're in Tauri mode
    const isTauri = typeof (window as any).__TAURI_INTERNALS__ !== "undefined";
    getApi().sendLog(`Init Bare - Tauri mode: ${isTauri}`);

    // Tauri uses onAgentMuxInit callback (backend emits wave-init event)
    // Tauri handles initialization in frontend after backend is ready
    if (!isTauri) {
        getApi().onAgentMuxInit(initWaveWrap);
    }
    setKeyUtilPlatform(platform);
    loadFonts();
    // Reset Tauri window zoom to 1.0 (per-pane zoom is handled via block metadata,
    // chrome zoom via CSS custom properties)
    const api = getApi();
    if (api && typeof api.setZoomFactor === "function") {
        api.setZoomFactor(1.0);
    }

    // Initialize chrome zoom CSS variables
    import("@/app/store/zoom.platform").then(({ initChromeZoom }) => {
        initChromeZoom();
    });

    // Use Promise.race to add a timeout fallback for fonts.ready
    // In Tauri, fonts.ready might not resolve promptly
    const fontsPromise = document.fonts.ready;
    const timeoutPromise = new Promise(resolve => setTimeout(resolve, 2000));

    Promise.race([fontsPromise, timeoutPromise]).then(async () => {
        benchMark("fonts-ready");
        const fontsMsg = `[startup-perf] initBare (fonts ready): ${(performance.now() - bareStart).toFixed(1)}ms`;
        console.log(fontsMsg);
        try { getApi().sendLog(fontsMsg); } catch {}
        getApi().sendLog("Init Bare Done");
        getApi().setWindowInitStatus("ready");

        // In Tauri mode, handle initialization in frontend
        if (isTauri) {
            getApi().sendLog("Starting Tauri initialization");
            try {
                // Check if this is a new window or the main window
                benchMark("isMainWindow-start");
                const isMain = await getApi().isMainWindow();
                getApi().sendLog(`Window type: ${isMain ? "main" : "new window"}`);

                benchMark("isMainWindow-done");
                if (isMain) {
                    // Main window with freshly spawned backend: standard initialization
                    await initTauriWave();
                } else {
                    // New window: create new backend window objects
                    const label = await getApi().getWindowLabel();
                    getApi().sendLog(`Initializing as new window: ${label}`);
                    await initTauriNewWindow();
                }
            } catch (error) {
                console.error("[initBare] Tauri initialization failed:", error);
                getApi().sendLog(`Tauri init error: ${error}`);
                showStartupError(String(error));
            }
        }

        // Safety net: if body is still hidden after 30s, force it visible
        setTimeout(() => {
            if (document.body.style.visibility === "hidden") {
                console.warn("[initBare] Safety timeout: forcing body visible after 30s");
                getApi().sendLog("[initBare] Safety timeout: forcing body visible after 30s");
                document.body.style.visibility = "visible";
                document.body.style.opacity = "1";
                document.body.classList.remove("is-transparent");
            }
        }, 30_000);
    });
}

// tauri-bootstrap.ts calls initBare() directly (static import).
// This self-start path is kept only for non-Tauri/dev environments where
// tauri-bootstrap is not the entry point.
if (typeof (window as any).__TAURI_INTERNALS__ === "undefined") {
    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", initBare);
    } else {
        initBare();
    }
}

async function initWaveWrap(initOpts: AgentMuxInitOpts) {
    try {
        if (savedInitOpts) {
            await reinitWave();
            return;
        }
        savedInitOpts = initOpts;
        await initWave(initOpts);
    } catch (e) {
        getApi().sendLog("Error in initWave " + e.message + "\n" + e.stack);
        console.error("Error in initWave", e);
    } finally {
        document.body.style.visibility = null;
        document.body.style.opacity = null;
        document.body.classList.remove("is-transparent");
    }
}

async function reinitWave() {
    console.log("Reinit Wave");
    getApi().sendLog("Reinit Wave");

    // We use this hack to prevent a flicker of the previously-hovered tab when this view was last active.
    document.body.classList.add("nohover");
    requestAnimationFrame(() =>
        setTimeout(() => {
            document.body.classList.remove("nohover");
        }, 100)
    );

    await WOS.reloadWaveObject<Client>(WOS.makeORef("client", savedInitOpts.clientId));
    const waveWindow = await WOS.reloadWaveObject<WaveWindow>(WOS.makeORef("window", savedInitOpts.windowId));
    const ws = await WOS.reloadWaveObject<Workspace>(WOS.makeORef("workspace", waveWindow.workspaceid));
    const initialTab = await WOS.reloadWaveObject<Tab>(WOS.makeORef("tab", savedInitOpts.tabId));
    await WOS.reloadWaveObject<LayoutState>(WOS.makeORef("layout", initialTab.layoutstate));
    reloadAllWorkspaceTabs(ws);
    document.title = `AgentMux ${appVersion} - ${initialTab.name}`; // TODO update with tab name change
    getApi().setWindowInitStatus("wave-ready");
    setReinitVersion((v) => v + 1);
    setUpdaterStatusAtom(getApi().getUpdaterStatus());
    setTimeout(() => {
        globalRefocus();
    }, 50);
}

function reloadAllWorkspaceTabs(ws: Workspace) {
    if (ws == null || (!ws.tabids?.length && !ws.pinnedtabids?.length)) {
        return;
    }
    ws.tabids?.forEach((tabid) => {
        WOS.reloadWaveObject<Tab>(WOS.makeORef("tab", tabid));
    });
    ws.pinnedtabids?.forEach((tabid) => {
        WOS.reloadWaveObject<Tab>(WOS.makeORef("tab", tabid));
    });
}

function loadAllWorkspaceTabs(ws: Workspace) {
    if (ws == null || (!ws.tabids?.length && !ws.pinnedtabids?.length)) {
        return;
    }
    ws.tabids?.forEach((tabid) => {
        WOS.getObjectValue<Tab>(WOS.makeORef("tab", tabid));
    });
    ws.pinnedtabids?.forEach((tabid) => {
        WOS.getObjectValue<Tab>(WOS.makeORef("tab", tabid));
    });
}

async function initWave(initOpts: AgentMuxInitOpts) {
    const t0 = performance.now();
    const tlog = (label: string, since: number) => {
        const ms = (performance.now() - since).toFixed(1);
        const total = (performance.now() - t0).toFixed(1);
        console.log(`[startup-perf] initWave ${label}: ${ms}ms (total: ${total}ms)`);
    };

    getApi().sendLog("Init Wave " + JSON.stringify(initOpts));
    console.log(
        "Wave Init",
        "tabid",
        initOpts.tabId,
        "clientid",
        initOpts.clientId,
        "windowid",
        initOpts.windowId,
        "platform",
        platform
    );
    let t = performance.now();
    initGlobal({
        tabId: initOpts.tabId,
        clientId: initOpts.clientId,
        windowId: initOpts.windowId,
        platform,
        primaryTabStartup: initOpts.primaryTabStartup,
    });
    (window as any).globalAtoms = atoms;
    tlog("initGlobal", t);

    // Init WPS event handlers
    t = performance.now();
    const globalWS = initWshrpc(initOpts.tabId);
    (window as any).globalWS = globalWS;
    (window as any).TabRpcClient = TabRpcClient;
    tlog("initWshrpc", t);

    t = performance.now();
    await withTimeout(loadConnStatus(), RPC_TIMEOUT, "loadConnStatus");
    tlog("loadConnStatus", t);

    t = performance.now();
    initGlobalEventSubs(initOpts);
    subscribeToConnEvents();
    tlog("initEventSubs", t);

    // ensures client/window/workspace are loaded into the cache before rendering
    t = performance.now();
    const [client, waveWindow, initialTab] = await withTimeout(
        Promise.all([
            WOS.loadAndPinWaveObject<Client>(WOS.makeORef("client", initOpts.clientId)),
            WOS.loadAndPinWaveObject<WaveWindow>(WOS.makeORef("window", initOpts.windowId)),
            WOS.loadAndPinWaveObject<Tab>(WOS.makeORef("tab", initOpts.tabId)),
        ]),
        RPC_TIMEOUT,
        "loadAndPin client/window/tab"
    );
    tlog("loadAndPin client/window/tab", t);

    t = performance.now();
    const [ws, layoutState] = await withTimeout(
        Promise.all([
            WOS.loadAndPinWaveObject<Workspace>(WOS.makeORef("workspace", waveWindow.workspaceid)),
            WOS.reloadWaveObject<LayoutState>(WOS.makeORef("layout", initialTab.layoutstate)),
        ]),
        RPC_TIMEOUT,
        "loadAndPin workspace/layout"
    );
    tlog("loadAndPin workspace/layout", t);

    t = performance.now();
    loadAllWorkspaceTabs(ws);
    WOS.wpsSubscribeToObject(WOS.makeORef("workspace", waveWindow.workspaceid));
    tlog("loadAllWorkspaceTabs", t);

    document.title = `AgentMux ${appVersion} - ${initialTab.name}`; // TODO update with tab name change

    t = performance.now();
    registerGlobalKeys();
    registerControlShiftStateUpdateHandler();
    tlog("registerKeys", t);

    t = performance.now();
    const fullConfig = await withTimeout(RpcApi.GetFullConfigCommand(TabRpcClient), RPC_TIMEOUT, "GetFullConfig");
    tlog("GetFullConfig", t);
    setFullConfigAtom(fullConfig);

    t = performance.now();
    console.log("Wave First Render");
    const elem = document.getElementById("main");
    render(App, elem);
    tlog("SolidJS render", t);
    tlog("TOTAL initWave", t0);

    // Hide startup loading message
    const startupLoading = document.getElementById("startup-loading");
    if (startupLoading) {
        startupLoading.remove();
    }

    getApi().setWindowInitStatus("wave-ready");
}
