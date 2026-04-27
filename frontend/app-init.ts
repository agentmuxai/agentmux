// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { App } from "@/app/app";
import { registerDefaultCommands } from "@/app/store/command-registry";
import {
    globalRefocus,
    registerControlShiftStateUpdateHandler,
    registerGlobalKeys,
} from "@/app/store/keymodel";
import { modalsModel } from "@/app/store/modalmodel";
import { ClientService, WindowService, WorkspaceService } from "@/app/store/services";
import { RpcApi } from "@/app/store/rpc-api";
import { initWshrpc, TabRpcClient } from "@/app/store/rpc-util";
import { getLayoutModelForStaticTab } from "@/layout/index";
import {
    atoms,
    countersClear,
    countersPrint,
    getApi,
    initGlobal,
    initGlobalEventSubs,
    loadConnStatus,
    pushFlashError,
    pushNotification,
    removeNotificationById,
    subscribeToConnEvents,
    windowInstanceNumAtom,
    setWindowInstanceNumAtom,
    setWindowCountAtom,
    setOpenWindowLabelsAtom,
    setOpenWindowEntriesAtom,
    setReinitVersion,
    setUpdaterStatusAtom,
    setUpdaterVersionAtom,
    setFullConfigAtom,
} from "@/app/store/global";
import * as WOS from "@/app/store/wos";
import { loadFonts } from "@/util/fontutil";
import { primeAccountCache } from "@/app/view/identity/identity-model";
import { setKeyUtilPlatform } from "@/util/keyutil";
import { render } from "solid-js/web";
import { benchMark, benchDump } from "@/util/startup-bench";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { isHostApp } from "@/app/init/host-detect";
import { showStartupError } from "@/app/init/error-display";
import { withTimeout } from "@/app/init/timeout";

// Deferred — assigned inside initApp() after window.api is ready.
// Do NOT call getApi() at module level: this file is statically imported by
// bootstrap.ts before setupCefApi() runs, so window.api does not exist yet.
let platform: NodeJS.Platform;
let appVersion: string;
let savedInitOpts: AgentMuxInitOpts = null;

window.WOS = WOS;
window.globalAtoms = atoms;
window.RpcApi = RpcApi;
window.isFullScreen = false;
window.countersPrint = countersPrint;
window.countersClear = countersClear;
window.getLayoutModelForStaticTab = getLayoutModelForStaticTab;
window.pushFlashError = pushFlashError;
window.pushNotification = pushNotification;
window.removeNotificationById = removeNotificationById;
window.modalsModel = modalsModel;

const RPC_TIMEOUT = 5_000; // 5 seconds for individual RPC calls

/**
 * Initialize the window instance number and count atoms, and subscribe to
 * the "window-instances-changed" Tauri event so the count stays reactive.
 * Called once per window after the wave UI is fully initialized.
 */
async function initInstanceTracking(): Promise<void> {
    // listWindows() returns ALL keys in state.browsers, which includes
    // browser-pane child browsers (`browser-pane-*`) and other internal
    // labels — not just top-level app windows. The instance panel only
    // wants the labels it can sensibly focus, so filter here. Top-level
    // window labels are either "main" or "window-<uuid>".
    const isInstanceLabel = (l: string): boolean =>
        l === "main" || /^window-/.test(l);

    // Stable ordering: "main" pinned first, others alphabetical. Without
    // this the panel rows can shuffle between refreshes (state.browsers
    // is a Rust HashMap with non-deterministic iteration), and two
    // windows could both display as "Window 1" because InstancePanel
    // uses the array index for naming and special-cases "main".
    const sortInstanceLabels = (labels: string[]): string[] =>
        [...labels].sort((a, b) => {
            if (a === "main") return -1;
            if (b === "main") return 1;
            return a.localeCompare(b);
        });

    // The host emits `window-instances-changed` *before* the new browser
    // is registered in state.browsers (the emit fires from window.rs:514
    // while registration happens later via on_after_created in client.rs).
    // The previous fix compared count against the event payload, but the
    // event count comes from window_instance_registry (different data
    // structure than state.browsers) so a count match is not reliable.
    // Robust fix: poll until the listWindows() result actually CHANGES
    // from the previously-seen set. Up to 6×50ms (~300ms total) covers
    // the empirical CEF on_after_created delay with margin.
    let lastLabelsKey = "";
    const refreshLabels = async (waitForChange = false, retriesLeft = 6): Promise<void> => {
        try {
            // listWindowInstances gives us [{label, windowId}] in one
            // RPC; we sort/filter on label and keep windowId aligned.
            // Falls back to listWindows if the new RPC isn't supported
            // (older host build) so the panel still works partially.
            let entries: Array<{ label: string; windowId: string | null }>;
            try {
                const all = await getApi().listWindowInstances();
                entries = Array.isArray(all) ? all : [];
            } catch {
                const all = await getApi().listWindows();
                entries = (Array.isArray(all) ? all : []).map((label) => ({ label, windowId: null }));
            }
            const filtered = entries.filter((e) => isInstanceLabel(e.label));
            filtered.sort((a, b) => {
                if (a.label === "main") return -1;
                if (b.label === "main") return 1;
                return a.label.localeCompare(b.label);
            });
            const labels = filtered.map((e) => e.label);
            const key = labels.join("|");
            if (waitForChange && key === lastLabelsKey && retriesLeft > 0) {
                setTimeout(() => refreshLabels(true, retriesLeft - 1), 50);
                return;
            }
            lastLabelsKey = key;
            setOpenWindowLabelsAtom(labels);
            setOpenWindowEntriesAtom(filtered);
        } catch {
            setOpenWindowLabelsAtom([]);
            setOpenWindowEntriesAtom([]);
        }
    };

    try {
        const [instanceNum, windowCount] = await Promise.all([
            getApi().getInstanceNumber(),
            getApi().getWindowCount(),
            refreshLabels(),
        ]);
        setWindowInstanceNumAtom(instanceNum);
        setWindowCountAtom(windowCount);

        // Keep count + label list in sync whenever any window opens or
        // closes. Pass `waitForChange=true` so refreshLabels polls until
        // listWindows() reports a different label set than last time —
        // robust against the event-vs-registration race documented above.
        await getApi().listen("window-instances-changed", (event: any) => {
            const payload = event?.payload ?? event;
            const count = typeof payload === "number" ? payload : 0;
            setWindowCountAtom(count);
            refreshLabels(true);
        });
    } catch (e) {
        console.warn("[initInstanceTracking] failed:", e);
    }
}

/**
 * Initialize AgentMux in host app mode by fetching
 * client/window/workspace/tab data from backend, verifying objects exist,
 * and creating missing ones if needed.
 */
async function initHostWave(): Promise<void> {
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

        // Initialize instance tracking (must come after initWaveWrap so global state is ready)
        await initInstanceTracking();

        benchDump(); // emit full startup timeline to log

    } catch (error) {
        console.error("[initHostWave] Initialization failed:", error);
        getApi().sendLog(`[initHostWave] ERROR: ${error}`);
        showStartupError(String(error));
    }
}

/**
 * Initialize a new (non-main) host window by creating new backend objects.
 * Unlike initHostWave() which reuses existing Window/Workspace/Tab,
 * this creates a fresh set for the new window.
 */
async function initHostNewWindow(): Promise<void> {
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

        // If this window was opened for a tear-off, the workspace ID is in the URL.
        // Pass it to CreateWindow so the backend reuses the existing workspace+tab
        // instead of creating a blank one.
        const tearOffWsId = new URLSearchParams(window.location.search).get("workspaceId") ?? "";
        if (tearOffWsId) {
            getApi().sendLog(`[initTauriNewWindow] tear-off workspaceId=${tearOffWsId}`);
        }

        t = performance.now();
        const newWindow = await withTimeout(WindowService.CreateWindow(null, tearOffWsId), RPC_TIMEOUT, "CreateWindow");
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

        // Initialize instance tracking (must come after initWaveWrap so global state is ready)
        await initInstanceTracking();

    } catch (error) {
        console.error("[initHostNewWindow] Initialization failed:", error);
        try { getApi().sendLog(`[initHostNewWindow] Error: ${error}`); } catch {}
        showStartupError("New window: " + String(error));
    }
}

export async function initApp() {
    // window.api is guaranteed to exist here — bootstrap.ts calls
    // setupCefApi() before calling initApp().
    // Defensive wait: if a race condition leaves window.api unset, poll briefly.
    if (!window.api) {
        console.error("[initApp] window.api not ready — polling (max 5s)");
        await new Promise<void>((resolve, reject) => {
            const check = setInterval(() => {
                if (window.api) { clearInterval(check); resolve(); }
            }, 50);
            setTimeout(() => {
                clearInterval(check);
                if (window.api) {
                    resolve();
                } else {
                    reject(new Error("[initApp] window.api still undefined after 5s — host API bridge failed to initialize"));
                }
            }, 5000);
        });
    }
    // Assign deferred module-level values now.
    platform = getApi().getPlatform();
    appVersion = getApi().getAboutModalDetails().version;
    document.title = `AgentMux ${appVersion}`;

    // Register context menu click handler now that window.api exists.
    ContextMenuModel.init();

    const bareStart = performance.now();
    window.__startupPerfStart = bareStart;
    getApi().sendLog("Init Bare");
    document.body.style.visibility = "hidden";
    document.body.style.opacity = "0";
    document.body.classList.add("is-transparent");

    // Check if we're in a host app (Tauri or CEF) that owns the backend sidecar.
    // Host apps query the backend for client/window/tab state.
    // Non-host mode waits for an agentmux-init event from the host.
    const hostApp = isHostApp();
    getApi().sendLog(`Init Bare - Host app mode: ${hostApp}`);

    if (!hostApp) {
        // Non-host: wait for the host to emit agentmux-init with IDs
        getApi().onAgentMuxInit(initWaveWrap);
    }
    setKeyUtilPlatform(platform);
    loadFonts();
    // Per-pane zoom is handled via block metadata. Chrome zoom via CSS custom
    // properties. Window-level zoom reset is not needed.

    // Initialize chrome zoom CSS variables
    import("@/app/store/zoom.platform").then(({ initChromeZoom }) => {
        initChromeZoom();
    });

    // Use Promise.race to add a timeout fallback for fonts.ready
    const fontsPromise = document.fonts.ready;
    const timeoutPromise = new Promise(resolve => setTimeout(resolve, 2000));

    try {
        await Promise.race([fontsPromise, timeoutPromise]);
    } catch (fontErr) {
        getApi().sendLog(`initApp: font wait error (non-fatal): ${fontErr}`);
    }
    benchMark("fonts-ready");
    const fontsMsg = `[startup-perf] initApp (fonts ready): ${(performance.now() - bareStart).toFixed(1)}ms`;
    try { getApi().sendLog(fontsMsg); } catch {}
    getApi().sendLog("Init Bare Done");
    getApi().setWindowInitStatus("ready");

    // In host app mode, handle initialization in frontend
    if (hostApp) {
        getApi().sendLog("Starting host app initialization");
        try {
            // Tear-off Phase 6 — pool-mode short-circuit. If the URL
            // carries `?pool=1`, this renderer was spawned by the
            // host's pre-warmed window pool. Skip the standard
            // workspace init and wait for `pool:promote` to arrive.
            // On promote, the same initHostNewWindow flow runs against
            // the workspace ID the promote event delivers (pushed into
            // the URL by awaitPoolPromote).
            const { isPoolMode, awaitPoolPromote } = await import("@/app/init/pool");
            if (isPoolMode()) {
                getApi().sendLog("[initApp] pool mode — deferring init until promote");
                await awaitPoolPromote();
                getApi().sendLog("[initApp] pool:promote received — bootstrapping workspace");
                await initHostNewWindow();
            } else {
                // Check if this is a new window or the main window
                benchMark("isMainWindow-start");
                const isMain = await getApi().isMainWindow();
                getApi().sendLog(`Window type: ${isMain ? "main" : "new window"}`);

                benchMark("isMainWindow-done");
                if (isMain) {
                    // Main window with freshly spawned backend: standard initialization
                    await initHostWave();
                } else {
                    // New window: create new backend window objects
                    const label = await getApi().getWindowLabel();
                    getApi().sendLog(`Initializing as new window: ${label}`);
                    await initHostNewWindow();
                }
            }
        } catch (error) {
            console.error("[initApp] Host initialization failed:", error);
            getApi().sendLog(`Host init error: ${error}`);
            showStartupError(String(error));
        }
    }

    // Safety net: if body is still hidden after 30s, force it visible
    setTimeout(() => {
        if (document.body.style.visibility === "hidden") {
            console.warn("[initApp] Safety timeout: forcing body visible after 30s");
            getApi().sendLog("[initApp] Safety timeout: forcing body visible after 30s");
            document.body.style.visibility = "visible";
            document.body.style.opacity = "1";
            document.body.classList.remove("is-transparent");
        }
    }, 30_000);
}

// bootstrap.ts calls initApp() directly (static import).
// This self-start path is kept only for dev environments where the
// bootstrap entry point is not used. Skip if running in Tauri or CEF
// since the bootstrap handles setup (window.api) before calling initApp().
if (!isHostApp()) {
    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", initApp);
    } else {
        initApp();
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
    setUpdaterVersionAtom(getApi().getUpdaterVersion());
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
    let t = performance.now();
    initGlobal({
        tabId: initOpts.tabId,
        clientId: initOpts.clientId,
        windowId: initOpts.windowId,
        platform,
        primaryTabStartup: initOpts.primaryTabStartup,
    });
    window.globalAtoms = atoms;
    tlog("initGlobal", t);

    // Init WPS event handlers
    t = performance.now();
    const globalWS = initWshrpc(initOpts.tabId);
    window.globalWS = globalWS;
    window.TabRpcClient = TabRpcClient;
    tlog("initWshrpc", t);

    t = performance.now();
    await withTimeout(loadConnStatus(), RPC_TIMEOUT, "loadConnStatus");
    tlog("loadConnStatus", t);

    t = performance.now();
    initGlobalEventSubs(initOpts);
    subscribeToConnEvents();
    tlog("initEventSubs", t);

    // Prime the identity-account cache from the DB so synchronous callers
    // (e.g. agent startup payload assembly via `loadAccounts()`) see real
    // data instead of an empty list. Fire-and-forget; the panel and
    // launch flow tolerate a momentarily-empty cache.
    primeAccountCache();

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
    registerDefaultCommands();
    registerControlShiftStateUpdateHandler();
    tlog("registerKeys", t);

    t = performance.now();
    const fullConfig = await withTimeout(RpcApi.GetFullConfigCommand(TabRpcClient), RPC_TIMEOUT, "GetFullConfig");
    tlog("GetFullConfig", t);
    setFullConfigAtom(fullConfig);

    t = performance.now();
    const elem = document.getElementById("main");
    render(App, elem);
    tlog("SolidJS render", t);
    tlog("TOTAL initWave", t0);

    // Register this window's backend ID with the CEF host so on_before_close
    // can call CloseWindow on the backend when this window is destroyed.
    // The CEF host handles cleanup at the right time (after the browser commits to
    // closing), keeping shells grouped under the CEF process in Task Manager.
    {
        const wlabel = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
        const wid = initOpts.windowId;
        console.log(`[wave] registerBackendWindow decision: wlabel=${wlabel} wid=${wid ?? "(falsy)"}`);
        if (wid) {
            getApi().registerBackendWindow(wlabel, wid);
        } else {
            console.error(`[wave] registerBackendWindow SKIPPED — windowId is falsy`);
        }
    }

    // Hide startup loading message
    const startupLoading = document.getElementById("startup-loading");
    if (startupLoading) {
        startupLoading.remove();
    }

    getApi().setWindowInitStatus("wave-ready");
}
