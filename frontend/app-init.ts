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
import { ClientService, ObjectService, WindowService, WorkspaceService } from "@/app/store/services";
import { RpcApi } from "@/app/store/rpc-api";
import { initWshrpc, TabRpcClient } from "@/app/store/rpc-util";
import { getLayoutModelForStaticTab, installLayoutModelEviction, installWindowEdgeResizeListener } from "@/layout/index";
import {
    atoms,
    countersClear,
    countersPrint,
    getApi,
    initGlobal,
    initGlobalEventSubs,
    loadConnStatus,
    openWindowEntriesAtom,
    pushFlashError,
    pushNotification,
    removeNotificationById,
    subscribeToConnEvents,
    setWindowInstanceNumAtom,
    setReinitVersion,
    setUpdaterStatusAtom,
    setUpdaterVersionAtom,
    setFullConfigAtom,
} from "@/app/store/global";
import * as MOS from "@/app/store/mos";
import { createEffect, createRoot, createSignal } from "solid-js";
import {
    DISPLAY_NAME_MAX_LEN,
    DISPLAY_NAME_META_KEY,
    formatWindowTitle,
    resolveWindowName,
} from "@/util/window-title";
import { loadFonts } from "@/util/fontutil";
import { primeAccountCache } from "@/app/view/identity/identity-model";
import { setKeyUtilPlatform } from "@/util/keyutil";
import { render } from "solid-js/web";
import { benchMark, benchDump } from "@/util/startup-bench";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { isHostApp } from "@/app/init/host-detect";
import { failStartup, showStartupError, StartupFailureHandled } from "@/app/init/error-display";
import { describeError, formatDescribedError } from "@/app/errors/error-report";
import { withTimeout } from "@/app/init/timeout";
import { retryTransient } from "@/util/transient-network";
import { fireAndForget } from "@/util/util";
import { setProviderModels } from "@/app/view/agent/providers";
import { scheduleRevealLift } from "@/store/tab-reveal";
import { installLauncherEventBridge } from "@/util/launcher-events";
import { installSrvEventBridge } from "@/util/srv-events";
import { installFloatingRedockHoverListener } from "@/app/workspace/redock-ghost";
import {
    seedKnownEntriesFromSnapshot,
    startLauncherEventReducer,
} from "@/app/store/launcher-event-reducer";
import { startSingletonCrashRelease } from "@/app/store/singleton-modal";
import { installFileDropController } from "@/app/drag/file-drop";
import { installWindowDragEvents } from "@/app/drag/window-drag-events";
import { MuxInitFatalError, requireLoaded } from "@/app/init/require-loaded";

// Deferred — assigned inside initApp() after window.api is ready.
// Do NOT call getApi() at module level: this file is statically imported by
// bootstrap.ts before setupCefApi() runs, so window.api does not exist yet.
let platform: NodeJS.Platform;
let savedInitOpts: AgentMuxInitOpts = null;

window.MOS = MOS;
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
 * Initialize the window-instance-number atom and seed the reducer
 * with the current snapshot. Subsequent updates to the panel atoms
 * come from typed launcher events via `launcher-event-reducer.ts`.
 *
 * Phase B.7.3.3 — bespoke `window-instances-changed` channel and
 * its retry/fallback paths are gone. The init RPC's only job is to
 * give the reducer a starting set of entries (so a renderer joining
 * mid-session sees existing windows before the first typed event
 * arrives for one of them). Pre-seed close events are tombstoned
 * inside the reducer and skipped at seed time. (codex P2 #603.)
 */
async function initInstanceTracking(): Promise<void> {
    try {
        // Each renderer's own instance number is stable per-run —
        // fetched once. The snapshot fetch primes the reducer for
        // labels that exist before this renderer joined; thereafter
        // typed events drive the panel.
        const [instanceNum, snapshotEntries] = await Promise.all([
            getApi().getInstanceNumber(),
            (async (): Promise<Array<{ label: string; windowId: string | null }>> => {
                try {
                    const all = await getApi().listWindowInstances();
                    return Array.isArray(all) ? all : [];
                } catch {
                    const all = await getApi().listWindows();
                    return (Array.isArray(all) ? all : []).map((label) => ({ label, windowId: null }));
                }
            })(),
        ]);
        setWindowInstanceNumAtom(instanceNum);
        seedKnownEntriesFromSnapshot(snapshotEntries);
    } catch (e) {
        console.warn("[initInstanceTracking] failed:", e);
    }
}

/**
 * Initialize AgentMux in host app mode by fetching
 * client/window/workspace/tab data from backend, verifying objects exist,
 * and creating missing ones if needed.
 */
/** This renderer's CEF window label — `windowLabel` from the boot URL, or
 *  "main" (the main window's URL carries no label param). */
function currentWindowLabel(): string {
    return new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
}

async function initHostMux(): Promise<void> {
    const t0 = performance.now();
    const tlog = (label: string, since: number) => {
        const ms = (performance.now() - since).toFixed(1);
        const total = (performance.now() - t0).toFixed(1);
        console.log(`[startup-perf] ${label}: ${ms}ms (total: ${total}ms)`);
    };

    try {
        // Get client data
        let t = performance.now();
        // Discovery reads retry a request that never got a response (#3868:
        // a refused loopback connect used to fail the whole window). Writes
        // below (CreateWindow/CloseWindow) are not retried.
        const clientData = await withTimeout(
            retryTransient(() => ClientService.GetClientData()),
            RPC_TIMEOUT,
            "GetClientData"
        );
        tlog("GetClientData", t);

        let windowId = clientData.windowids?.[0];

        // If no windows exist, create one. This is the genuine cold-start
        // case (SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13 Feature 1)
        // — `Client.windowids` is empty, which only happens right after a
        // graceful quit (the destroy-on-close cascade always empties it) or
        // on a truly first-ever launch. Pass `restoreIfAvailable: true` so
        // srv replays the last-session snapshot if one was saved on close,
        // instead of always seeding the hardcoded default 3-pane layout.
        if (!windowId) {
            t = performance.now();
            const newWindow = await withTimeout(WindowService.CreateWindow(null, "", currentWindowLabel(), true), RPC_TIMEOUT, "CreateWindow");
            tlog("CreateWindow (no windows)", t);
            windowId = newWindow.oid;
        }

        // Verify window exists
        t = performance.now();
        let windowData = await withTimeout(
            retryTransient(() => WindowService.GetWindow(windowId)),
            RPC_TIMEOUT,
            "GetWindow"
        );
        tlog("GetWindow", t);

        if (!windowData) {
            t = performance.now();
            windowData = await withTimeout(WindowService.CreateWindow(null, "", currentWindowLabel()), RPC_TIMEOUT, "CreateWindow");
            tlog("CreateWindow (fallback)", t);
            windowId = windowData.oid;
        }

        // Get workspace
        t = performance.now();
        let workspace = await withTimeout(
            retryTransient(() => WorkspaceService.GetWorkspace(windowData.workspaceid)),
            RPC_TIMEOUT,
            "GetWorkspace"
        );
        tlog("GetWorkspace", t);

        if (!workspace) {
            // Workspace missing → recreate entire window
            t = performance.now();
            await withTimeout(WindowService.CloseWindow(windowData.oid), RPC_TIMEOUT, "CloseWindow");
            windowData = await withTimeout(WindowService.CreateWindow(null, "", currentWindowLabel()), RPC_TIMEOUT, "CreateWindow");
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
        await initMuxWrap(initOpts);
        tlog("initMuxWrap", t);
        tlog("TOTAL initCefMux", t0);

        // Apply dev window title — task dev TITLE="agentx: PR #1780"
        // Only runs in Vite dev mode; VITE_DEV_TITLE is empty string in prod builds.
        const devTitle = import.meta.env.VITE_DEV_TITLE;
        if (import.meta.env.DEV && devTitle) {
            void ObjectService.UpdateObjectMeta(
                MOS.makeORef("window", initOpts.windowId),
                { [DISPLAY_NAME_META_KEY]: devTitle.slice(0, DISPLAY_NAME_MAX_LEN) } as MetaType,
            );
        }

        // Initialize instance tracking (must come after initMuxWrap so global state is ready)
        await initInstanceTracking();

        // Issue #2977 WS4 — show what the background service did while no
        // window was open. Fire-and-forget: it must never delay or break
        // startup, and it self-suppresses when there is nothing to report.
        void (async () => {
            const { surfaceBackgroundAudit } = await import("@/app/init/background-audit");
            await surfaceBackgroundAudit();
        })();

        benchDump(); // emit full startup timeline to log

    } catch (error) {
        // §6.1: describeError keeps the stack — String(error) serialized a
        // plain Error to "{}" or dropped it entirely.
        const described = describeError(error);
        console.error("[initHostMux] Initialization failed:", described);
        getApi().sendLog(`[initHostMux] ERROR: ${formatDescribedError(described)}`);
        // Bounded auto-reload, then the card — the same recovery a bootstrap
        // failure gets. Throws, so initApp's caller never sees success.
        failStartup(formatDescribedError(described));
    }
}

/**
 * Initialize a new (non-main) host window by creating new backend objects.
 * Unlike initHostMux() which reuses existing Window/Workspace/Tab,
 * this creates a fresh set for the new window.
 */
async function initHostNewWindow(seedView?: string | null, seedMeta?: Record<string, unknown> | null): Promise<void> {
    const t0 = performance.now();
    const tlog = (label: string, since: number) => {
        const ms = (performance.now() - since).toFixed(1);
        const total = (performance.now() - t0).toFixed(1);
        console.log(`[startup-perf] ${label}: ${ms}ms (total: ${total}ms)`);
        getApi().sendLog(`[startup-perf] ${label}: ${ms}ms (total: ${total}ms)`);
    };

    try {
        getApi().sendLog("[initCefNewWindow] Creating new backend objects");

        // Get client data (reuse existing client)
        let t = performance.now();
        // Reads retry a request that never got a response (#3868), as in
        // initHostMux; CreateWindow below is a write and does not.
        const clientData = await withTimeout(
            retryTransient(() => ClientService.GetClientData()),
            RPC_TIMEOUT,
            "GetClientData"
        );
        tlog("GetClientData", t);

        // If this window was opened for a tear-off, the workspace ID is in the URL.
        // Pass it to CreateWindow so the backend reuses the existing workspace+tab
        // instead of creating a blank one.
        const tearOffWsId = new URLSearchParams(window.location.search).get("workspaceId") ?? "";
        if (tearOffWsId) {
            getApi().sendLog(`[initCefNewWindow] tear-off workspaceId=${tearOffWsId}`);
        }

        t = performance.now();
        const newWindow = await withTimeout(
            WindowService.CreateWindow(
                null,
                tearOffWsId,
                currentWindowLabel(),
                false,
                seedView ?? undefined,
                seedMeta ?? undefined
            ),
            RPC_TIMEOUT,
            "CreateWindow"
        );
        tlog("CreateWindow", t);

        // Register label→window_id with the host NOW, not at the end of
        // initMux (which also registers — idempotently — after render):
        // the srv window row exists as of this line, and every host close
        // path (on_before_close, demote_srv_cleanup) resolves WHICH srv row
        // to close through this registration. A window closed in the
        // seconds between CreateWindow and initMux's late registration —
        // e.g. a tear-off merged straight back — used to orphan its srv
        // row forever: the close's demote reloads this renderer to the
        // pool boot URL, so the late registration never arrives, and the
        // host's bounded registration-race retry waits for an event that
        // can no longer happen (task #29 round 2, found by the
        // window-close-baseline E2E suite).
        {
            const wlabel = currentWindowLabel();
            getApi().registerBackendWindow(wlabel, newWindow.oid);
        }

        // Get the workspace that was auto-created with the window
        t = performance.now();
        const workspace = await withTimeout(
            retryTransient(() => WorkspaceService.GetWorkspace(newWindow.workspaceid)),
            RPC_TIMEOUT,
            "GetWorkspace"
        );
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
        await initMuxWrap(initOpts);
        tlog("initMuxWrap", t);
        tlog("TOTAL initCefNewWindow", t0);

        // Initialize instance tracking (must come after initMuxWrap so global state is ready)
        await initInstanceTracking();

        // Issue #2977 WS4 — show what the background service did while no
        // window was open. Fire-and-forget: it must never delay or break
        // startup, and it self-suppresses when there is nothing to report.
        void (async () => {
            const { surfaceBackgroundAudit } = await import("@/app/init/background-audit");
            await surfaceBackgroundAudit();
        })();

    } catch (error) {
        const described = describeError(error);
        console.error("[initHostNewWindow] Initialization failed:", described);
        try { getApi().sendLog(`[initHostNewWindow] Error: ${formatDescribedError(described)}`); } catch {}
        showStartupError("New window: " + formatDescribedError(described));
        // Card, not failStartup's auto-reload: pool and pane-pool renderers are
        // promoted in place by host events, and reloading one after promotion
        // is not verified to bring it back. But it is still a failed startup —
        // throw so bootstrap doesn't log success or reset the reload budget.
        throw new StartupFailureHandled("New window: " + formatDescribedError(described));
    }
}

// initApp has two callers — bootstrap.ts and this module's self-start below —
// and in CEF both fire (isHostApp() reads the host's injected IPC port global, which
// setupCefApi() only sets later, at bootstrap time). It must run exactly once
// per page: a second concurrent run reaches CreateWindow twice and strands an
// unregistered Window row in srv's Client.windowids.
let initAppOnce: Promise<void> | undefined;

export function initApp(): Promise<void> {
    return (initAppOnce ??= initAppInner());
}

async function initAppInner() {
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
    // Note: document.title is left at the index.html default ("AgentMux")
    // until installWindowTitleEffect() runs at the end of initMux(). The
    // body is `visibility: hidden` during init so users don't see the
    // bare "AgentMux" pre-init title.

    // Phase B.7.3.1 — install `window.__agentmux_launcher_event` BEFORE
    // any host-touching call. The host's `launcher_event_bridge` may
    // start dispatching as soon as the renderer's V8 context is ready;
    // registering early guarantees no events are dropped on the floor.
    installLauncherEventBridge();
    // Phase E.2c.5b — same discipline for srv events. The host's
    // `srv_event_bridge.rs` (PR #618) starts forwarding srv reducer
    // events as soon as the srv pipe is connected; install before
    // any host-touching call so early events aren't dropped.
    installSrvEventBridge();

    // Register context menu click handler now that window.api exists.
    ContextMenuModel.init();

    // Install before anything else can race a drop in: an unhandled file
    // drop anywhere outside a pane's own drop zone would otherwise navigate
    // the whole window away and destroy the app (window-drag-events.ts).
    // No dependency on window.api / host state, so there's no reason to
    // delay it.
    installWindowDragEvents();
    // Pane file-drop targets and their indicator (SPEC_DRAG_AND_DROP_CONSOLIDATION §5.3).
    installFileDropController();

    // Phase 3 voice input — surface permission errors via the existing
    // notification system. `useVoiceInput.ts` dispatches `voice-input-error`
    // on the only fatal SpeechRecognition error codes ("not-allowed" /
    // "service-not-allowed"); transient errors like "no-speech" and
    // "aborted" are silently auto-restarted by `recognition.onend` and
    // never reach this listener.
    installVoiceInputErrorListener();

    const bareStart = performance.now();
    window.__startupPerfStart = bareStart;
    getApi().sendLog("Init Bare");
    document.body.style.visibility = "hidden";
    document.body.style.opacity = "0";
    document.body.classList.add("is-transparent");

    // Check if we're in the host app (CEF) that owns the backend sidecar.
    // Host apps query the backend for client/window/tab state.
    // Non-host mode waits for an agentmux-init event from the host.
    const hostApp = isHostApp();
    getApi().sendLog(`Init Bare - Host app mode: ${hostApp}`);

    if (!hostApp) {
        // Non-host: wait for the host to emit agentmux-init with IDs.
        //
        // `onAgentMuxInit` invokes this fire-and-forget (`cef-api.ts` just
        // calls `callback(payload)` — no await, no catch), so unlike the host
        // paths there is no caller to receive a re-thrown fatal error. Without
        // this handler it would surface only as an unhandled rejection, which
        // the global forwarder logs but never turns into a startup card — and
        // `initMuxWrap`'s `finally` would still reveal the body, reproducing
        // the exact blank window this is meant to fix, just on a different
        // path. reagentx P1 on PR #3486.
        getApi().onAgentMuxInit((payload) => {
            void initMuxWrap(payload).catch((error) => {
                const described = describeError(error);
                console.error("[onAgentMuxInit] Initialization failed:", described);
                getApi().sendLog(`[onAgentMuxInit] ERROR: ${formatDescribedError(described)}`);
                showStartupError(formatDescribedError(described));
            });
        });
    }
    setKeyUtilPlatform(platform);
    loadFonts();
    // Per-pane zoom is handled via block metadata. Chrome zoom via CSS custom
    // properties. Window-level zoom reset is not needed.

    // Initialize chrome zoom CSS variables
    import("@/app/store/zoom").then(({ initChromeZoom }) => {
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
            // Pool-mode short-circuit. `?pool=1` means this renderer was
            // pre-spawned by the host's window pool. Defer workspace init and
            // wait for either `pool:promote` (tear-off, injects workspaceId) or
            // `pool:new-window` (Cmd+N, no workspaceId → fresh workspace).
            // initHostNewWindow branches on workspaceId presence automatically.
            const { isPoolMode, awaitPoolPromote, isPanePoolMode, awaitPanePoolPromote } = await import("@/app/init/pool");
            if (isPoolMode()) {
                getApi().sendLog("[initApp] pool mode — deferring init until pool:promote or pool:new-window");
                const { initialView, initialMeta } = await awaitPoolPromote();
                getApi().sendLog("[initApp] pool event received — bootstrapping workspace");
                // Pass the widget's view straight into CreateWindow's seed
                // (rather than seeding the default 3-pane layout and then
                // pane.open-ing a 4th pane alongside it) so "Open in New
                // Window" on a widget opens with ONLY that widget.
                await initHostNewWindow(initialView, initialMeta);
            } else if (isPanePoolMode()) {
                // Pane pool: wait for pool:pane-promote which injects floatingPaneId+workspaceId
                // into the URL, then initHostNewWindow reattaches and wave renders FloatingPaneWorkspace.
                getApi().sendLog("[initApp] pane-pool mode — deferring init until pool:pane-promote");
                await awaitPanePoolPromote();
                getApi().sendLog("[initApp] pool:pane-promote received — bootstrapping floating pane");
                await initHostNewWindow();
            } else {
                // Check if this is a new window or the main window
                benchMark("isMainWindow-start");
                const isMain = await getApi().isMainWindow();
                getApi().sendLog(`Window type: ${isMain ? "main" : "new window"}`);

                benchMark("isMainWindow-done");
                if (isMain) {
                    // Main window with freshly spawned backend: standard initialization
                    await initHostMux();
                } else {
                    // New window: create new backend window objects
                    const label = await getApi().getWindowLabel();
                    getApi().sendLog(`Initializing as new window: ${label}`);
                    const coldSearchParams = new URL(window.location.href).searchParams;
                    const coldInitialView = coldSearchParams.get("initialView");
                    // "credential-approval" is not a real pane view — it's
                    // rendered by app.tsx as this window's ENTIRE content,
                    // bypassing the normal Workspace/pane tree on purpose
                    // (see CredentialApprovalWindow's own doc comment for
                    // why: a pane opened via pane.open gets a
                    // [data-blockid] wrapper, which would make its Approve
                    // button reachable by any agent's UIQuery/UIClick).
                    // Seeding it as a real pane here would defeat that, so
                    // it's excluded from seedView and falls back to
                    // initHostNewWindow's default 3-pane seed underneath
                    // (irrelevant — app.tsx replaces this window's content
                    // entirely for credential-approval).
                    let coldMeta: Record<string, unknown> | undefined;
                    // The memory-adoption approval window is the same kind.
                    const approvalViews = ["credential-approval", "memory-adoption-approval", "ssh-approval"];
                    const seedView =
                        coldInitialView && !approvalViews.includes(coldInitialView) ? coldInitialView : undefined;
                    if (seedView) {
                        const coldMetaRaw = coldSearchParams.get("initialMeta");
                        try { coldMeta = coldMetaRaw ? JSON.parse(coldMetaRaw) : undefined; } catch { /* ignore */ }
                    }
                    await initHostNewWindow(seedView, coldMeta);
                }
            }
        } catch (error) {
            // Already handled below us (initHostMux) — don't show a second card.
            if (error instanceof StartupFailureHandled) throw error;
            const described = describeError(error);
            console.error("[initApp] Host initialization failed:", described);
            getApi().sendLog(`Host init error: ${formatDescribedError(described)}`);
            showStartupError(formatDescribedError(described));
            // The card is up; tell bootstrap this was not a successful startup.
            throw new StartupFailureHandled(formatDescribedError(described));
        }
    }

    // Safety net: if body is still hidden after 30s, force it visible and
    // drop the splash. The reveal gate (MAX_GATE_MS = 800ms) normally fades
    // the splash long before this; this only fires if init wedged.
    setTimeout(() => {
        if (document.body.style.visibility === "hidden") {
            console.warn("[initApp] Safety timeout: forcing body visible after 30s");
            getApi().sendLog("[initApp] Safety timeout: forcing body visible after 30s");
            document.body.style.visibility = "visible";
            document.body.style.opacity = "1";
            document.body.classList.remove("is-transparent");
        }
        import("@/app/init/startup-splash").then((m) => m.fadeOutStartupSplash());
    }, 30_000);
}

// bootstrap.ts calls initApp() directly (static import).
// This self-start path is kept only for dev environments where the
// bootstrap entry point is not used. Skip if running in the CEF host
// since the bootstrap handles setup (window.api) before calling initApp().
// A StartupFailureHandled rejection has already put up its own UI.
const selfStartApp = () =>
    initApp().catch((error) => {
        if (!(error instanceof StartupFailureHandled)) console.error("[initApp] self-start failed:", error);
    });
if (!isHostApp()) {
    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", selfStartApp);
    } else {
        selfStartApp();
    }
}

async function initMuxWrap(initOpts: AgentMuxInitOpts) {
    try {
        if (savedInitOpts) {
            await reinitMux();
            return;
        }
        savedInitOpts = initOpts;
        await initMux(initOpts);
        // Phase B.7.3.1 — start the launcher-event reducer effect now
        // that global state is wired. Idempotent: subsequent calls
        // (e.g. via reinitMux path) are no-ops.
        startLauncherEventReducer();
        // Bundle-management PR 3 — wire singleton-modal crash release.
        // Subscribes to the launcher window-exit signal so a dead
        // holder's singleton claim is auto-released. Idempotent.
        startSingletonCrashRelease();
    } catch (e) {
        getApi().sendLog("Error in initMux " + e.message + "\n" + e.stack);
        console.error("Error in initMux", e);
        // A fatal startup failure must not be swallowed here. This catch
        // deliberately tolerates late, non-essential failures (the reducers
        // and crash-release wiring at the end of this function) so one of
        // them cannot take the whole window down — but swallowing EVERYTHING
        // meant a missing core object left `initHostMux`'s `showStartupError`
        // card unreachable, while the `finally` below still revealed the
        // body. The user got a blank window and a console line. Re-throwing
        // only the fatal class keeps the tolerance for the former and gets
        // the latter in front of someone.
        if (e instanceof MuxInitFatalError) {
            throw e;
        }
    } finally {
        // First-paint + new-window reveal coordination — see issue
        // #774. The body was hidden at line 324 before any rendering;
        // we now have to lift it. Drive the lift through the same
        // frame-budget gate that handles tab open/switch so the
        // FIRST tab in the FIRST window also gets the "wait for the
        // mount cascade to settle, then reveal atomically" treatment.
        // This covers:
        //   - Cold app start (the first window in the user's session)
        //   - "New Window" from the hamburger menu (each opens its
        //     own bootstrap → initMuxWrap)
        //   - Any future window-spawning path that reuses initMux
        //
        // `scheduleRevealLift` already handles rapid Ctrl-Tab spam by
        // resetting its detector, so the prior call from createTab /
        // setActiveTab (if any) is just superseded.
        scheduleRevealLift();
        document.body.style.visibility = null;
        document.body.style.opacity = null;
        document.body.classList.remove("is-transparent");
    }
}

async function reinitMux() {
    console.log("Reinit AgentMux");
    getApi().sendLog("Reinit AgentMux");

    // We use this hack to prevent a flicker of the previously-hovered tab when this view was last active.
    document.body.classList.add("nohover");
    requestAnimationFrame(() =>
        setTimeout(() => {
            document.body.classList.remove("nohover");
        }, 100)
    );

    await MOS.reloadMuxObject<Client>(MOS.makeORef("client", savedInitOpts.clientId));
    const muxWindow = requireLoaded(
        await MOS.reloadMuxObject<MuxWindow>(MOS.makeORef("window", savedInitOpts.windowId)),
        "window",
        savedInitOpts.windowId
    );
    const ws = requireLoaded(
        await MOS.reloadMuxObject<Workspace>(MOS.makeORef("workspace", muxWindow.workspaceid)),
        "workspace",
        muxWindow.workspaceid
    );
    const initialTab = requireLoaded(
        await MOS.reloadMuxObject<Tab>(MOS.makeORef("tab", savedInitOpts.tabId)),
        "tab",
        savedInitOpts.tabId
    );
    await MOS.reloadMuxObject<LayoutState>(MOS.makeORef("layout", initialTab.layoutstate));
    reloadAllWorkspaceTabs(ws);
    // Title is driven by installWindowTitleEffect() (set up in initMux)
    // and reacts to atom changes — reinitMux's reloads update the atoms,
    // the effect re-runs, document.title updates. No imperative write needed.
    getApi().setWindowInitStatus("wave-ready");
    setReinitVersion((v) => v + 1);
    setUpdaterStatusAtom(getApi().getUpdaterStatus());
    setUpdaterVersionAtom(getApi().getUpdaterVersion());
    setTimeout(() => {
        globalRefocus();
    }, 50);
}

/**
 * Install the reactive document.title effect for this window. Title format:
 *
 *     {Window Name} - {Tab Name} - AgentMux
 *
 * Window Name resolves via the same three-tier rule the InstancePanel uses
 * (user display name → workspace name → "Window N"). The effect re-runs
 * automatically whenever any input atom changes — tab switches, window
 * rename, workspace re-assign, or the window's position in
 * `openWindowEntriesAtom` shifts.
 *
 * Spec: docs/specs/SPEC_WINDOW_TITLE_FORMAT_2026-05-13.md
 */
function installWindowTitleEffect(windowId: string): void {
    // This window's launcher label, fetched async. Used as a fallback
    // when entry.windowId-based findIndex returns -1 (which happens at
    // startup before registerBackendWindow has populated the entry's
    // windowId — see global.ts:145 comment). Without the label fallback,
    // a freshly-opened second window resolves to idx=0 and the title
    // shows "Window 1" while the InstancePanel correctly shows "Window 2"
    // (because the panel iterates entries with positional indices).
    // Same root cause as the InstancePanel resolveEntryWindowId fallback.
    const [myLabel, setMyLabel] = createSignal<string | null>(null);
    getApi().getWindowLabel().then((l) => setMyLabel(l)).catch(() => setMyLabel(null));

    // Capture dispose so the reactive root can be torn down explicitly.
    // In practice the CEF renderer is destroyed when the window closes,
    // taking the JS context (and the effect) with it — but routing
    // through `beforeunload` keeps the pattern correct if the renderer
    // ever outlives a single window load (e.g. in-place navigation,
    // future host-driven reload paths). Per ReAgent review on PR #841.
    const dispose = createRoot((disposeFn) => {
        createEffect(() => {
            const activeTabId = atoms.activeTabId();
            const tab = activeTabId
                ? MOS.getObjectValue<Tab>(MOS.makeORef("tab", activeTabId))
                : undefined;
            const win = MOS.getObjectValue<MuxWindow>(MOS.makeORef("window", windowId));
            const ws = atoms.workspace();
            const entries = openWindowEntriesAtom();

            // Find this window's entry. Prefer windowId match; fall back
            // to label when entry.windowId is null (registration race).
            // If both fail, use 0 — a wrong rank is better than no title.
            let idx = entries.findIndex((e) => e.windowId === windowId);
            let idxSource: "windowId" | "label" | "fallback" = "windowId";
            if (idx < 0) {
                const lbl = myLabel();
                if (lbl) {
                    idx = entries.findIndex((e) => e.label === lbl);
                    if (idx >= 0) idxSource = "label";
                }
            }
            if (idx < 0) {
                idx = 0;
                idxSource = "fallback";
            }

            const displayName = win?.meta?.[DISPLAY_NAME_META_KEY] as string | undefined;
            const workspaceName = ws?.name;
            const windowName = resolveWindowName({
                displayName,
                workspaceName,
                indexInOpenWindows: idx,
            });
            const title = formatWindowTitle(windowName, tab?.name);
            document.title = title;

            // Diagnostic log — cross-reference with [wave-panel] logs in
            // InstancePanel.tsx to spot inconsistencies. Same windowId/label
            // should produce the same windowName in both surfaces.
            // Goes through frontend's [fe] log pipe → host log; tail with
            // `muxlog host '\[fe\] \[wave-title\]'`.
            console.debug(
                "[wave-title]",
                "windowId=" + windowId,
                "label=" + (myLabel() ?? "<unknown>"),
                "idx=" + idx,
                "idxSource=" + idxSource,
                "displayName=" + (displayName ?? "<none>"),
                "workspaceName=" + (workspaceName ?? "<none>"),
                "tab=" + (tab?.name ?? "<none>"),
                "→ title=" + JSON.stringify(title),
            );
        });
        return disposeFn;
    });
    window.addEventListener("beforeunload", () => dispose(), { once: true });
}

/**
 * One-time listener for the `voice-input-error` CustomEvent dispatched by
 * `useVoiceInput.ts` when SpeechRecognition emits a fatal permission error.
 * Surfaces a notification toast via the app's existing notification system
 * (`pushNotification`, same path used by term.tsx for drop/copy failures).
 *
 * Spec: docs/specs/SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md §7 Phase 3.
 */
function installVoiceInputErrorListener(): void {
    window.addEventListener("voice-input-error", (e: Event) => {
        const detail = (e as CustomEvent<string>).detail;

        // Platform-specific path to the OS microphone privacy setting, so the
        // "blocked" guidance is actionable rather than generic.
        const isMac = /Mac|iP(hone|ad|od)/.test(navigator.platform || navigator.userAgent);
        const micSettingsPath = isMac
            ? "System Settings ▸ Privacy & Security ▸ Microphone"
            : "Settings ▸ Privacy & security ▸ Microphone";

        // Classify the recognition error into an actionable message. Distinct
        // causes need distinct guidance — a single "unavailable" toast left the
        // user with no idea whether to fix permissions, plug in a mic, or wait
        // for a feature. See SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md §Phase 4.
        let title = "Voice input unavailable";
        let message: string | null = null;
        switch (detail) {
            case "not-allowed":
                title = "Microphone access blocked";
                message =
                    `Enable microphone access for AgentMux in ${micSettingsPath}, ` +
                    `then click the mic again.`;
                break;
            case "audio-capture":
                title = "No microphone detected";
                message = "Connect a microphone and click the mic again.";
                break;
            case "service-not-allowed":
                title = "Voice transcription unavailable";
                message =
                    "Voice transcription isn't configured — open Settings ▸ Recording to set it up.";
                break;
            default:
                return; // non-fatal / unknown — no toast
        }

        pushNotification({
            icon: "fa-microphone-slash",
            title,
            message,
            timestamp: new Date().toISOString(),
            type: "error",
            expiration: Date.now() + 12000,
        });
    });
}

function reloadAllWorkspaceTabs(ws: Workspace) {
    if (ws == null || (!ws.tabids?.length && !ws.pinnedtabids?.length)) {
        return;
    }
    ws.tabids?.forEach((tabid) => {
        MOS.reloadMuxObject<Tab>(MOS.makeORef("tab", tabid));
    });
    ws.pinnedtabids?.forEach((tabid) => {
        MOS.reloadMuxObject<Tab>(MOS.makeORef("tab", tabid));
    });
}

function loadAllWorkspaceTabs(ws: Workspace) {
    if (ws == null || (!ws.tabids?.length && !ws.pinnedtabids?.length)) {
        return;
    }
    ws.tabids?.forEach((tabid) => {
        MOS.getObjectValue<Tab>(MOS.makeORef("tab", tabid));
    });
    ws.pinnedtabids?.forEach((tabid) => {
        MOS.getObjectValue<Tab>(MOS.makeORef("tab", tabid));
    });
}

async function initMux(initOpts: AgentMuxInitOpts) {
    const t0 = performance.now();
    const tlog = (label: string, since: number) => {
        const ms = (performance.now() - since).toFixed(1);
        const total = (performance.now() - t0).toFixed(1);
        console.log(`[startup-perf] initMux ${label}: ${ms}ms (total: ${total}ms)`);
    };

    getApi().sendLog("Init AgentMux " + JSON.stringify(initOpts));
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

    // Init MPS event handlers
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
    installFloatingRedockHoverListener();
    installLayoutModelEviction();
    // Shift + OS-window-edge resize (Windows host emits windowresize:*
    // during the native size loop; inert elsewhere). See
    // SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26.md §3.
    installWindowEdgeResizeListener();
    tlog("initEventSubs", t);

    // Prime the identity-account cache from the DB so synchronous callers
    // (e.g. agent startup payload assembly via `loadAccounts()`) see real
    // data instead of an empty list. Fire-and-forget; the panel and
    // launch flow tolerate a momentarily-empty cache.
    primeAccountCache();

    // ensures client/window/workspace are loaded into the cache before rendering
    t = performance.now();
    const [clientRaw, muxWindowRaw, initialTabRaw] = await withTimeout(
        Promise.all([
            MOS.loadAndPinMuxObject<Client>(MOS.makeORef("client", initOpts.clientId)),
            MOS.loadAndPinMuxObject<MuxWindow>(MOS.makeORef("window", initOpts.windowId)),
            MOS.loadAndPinMuxObject<Tab>(MOS.makeORef("tab", initOpts.tabId)),
        ]),
        RPC_TIMEOUT,
        "loadAndPin client/window/tab"
    );
    const client = requireLoaded(clientRaw, "client", initOpts.clientId);
    const muxWindow = requireLoaded(muxWindowRaw, "window", initOpts.windowId);
    const initialTab = requireLoaded(initialTabRaw, "tab", initOpts.tabId);
    tlog("loadAndPin client/window/tab", t);

    t = performance.now();
    const [wsRaw, layoutState] = await withTimeout(
        Promise.all([
            MOS.loadAndPinMuxObject<Workspace>(MOS.makeORef("workspace", muxWindow.workspaceid)),
            MOS.reloadMuxObject<LayoutState>(MOS.makeORef("layout", initialTab.layoutstate)),
        ]),
        RPC_TIMEOUT,
        "loadAndPin workspace/layout"
    );
    const ws = requireLoaded(wsRaw, "workspace", muxWindow.workspaceid);
    tlog("loadAndPin workspace/layout", t);

    t = performance.now();
    loadAllWorkspaceTabs(ws);
    MOS.mpsSubscribeToObject(MOS.makeORef("workspace", muxWindow.workspaceid));
    tlog("loadAllWorkspaceTabs", t);

    installWindowTitleEffect(initOpts.windowId);

    t = performance.now();
    registerGlobalKeys();
    registerDefaultCommands();
    registerControlShiftStateUpdateHandler();
    tlog("registerKeys", t);

    t = performance.now();
    const fullConfig = await withTimeout(RpcApi.GetFullConfigCommand(TabRpcClient), RPC_TIMEOUT, "GetFullConfig");
    tlog("GetFullConfig", t);
    setFullConfigAtom(fullConfig);

    // Third-party pane tabs from widgets.json (Pane Tab contract Phase 6),
    // loaded before the first render so a persisted `ext:` pane finds its
    // manifest. Local files, so this is quick; the timeout only guards a
    // stuck read — a widget that misses it still loads, for panes opened
    // later.
    t = performance.now();
    const { startWidgetLoader } = await import("@/app/block/widget-loader");
    await withTimeout(startWidgetLoader(), 2000, "LoadWidgets").catch((e) =>
        console.warn("[widget-loader] first pass still running at first render:", e)
    );
    tlog("LoadWidgets", t);

    // Window services that don't paint. They load alongside the render and
    // install when loaded; initMux doesn't wait for them, because its end is
    // what starts the content-reveal gate (initMuxWrap → scheduleRevealLift),
    // and on a tear-off every ms here is brain splash on screen.
    // SPEC_TEAROFF_PAINT_LATENCY_2026_09_30.md §2.3, phase 1.1.
    // Each installs on its own: one failed import (a stale chunk after an
    // update) must not take the others down, and is reported like any other
    // startup failure.
    const windowServices: [string, Promise<() => void>][] = [
        // Auto pane-overlay clip: any DOM element tagged `data-pane-overlay`
        // participates in browser-pane clipping.
        // docs/specs/SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md.
        ["pane-overlay-auto", import("@/app/platform/pane-overlay-auto").then((m) => m.startPaneOverlayAutoService)],
        // Sound notifications: subscribes to agent-pane reducer events and
        // plays a polite SFX on turn-complete (and other configured signals).
        // docs/specs/SPEC_SOUND_NOTIFICATIONS_2026_06_05.md.
        ["sound", import("@/app/notification/sound").then((m) => m.installSoundService)],
        // OS notifications (native toasts): forward pane events + focus to
        // the srv Router and handle toast-click activation.
        // docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md.
        ["os-notify-bridge", import("@/app/notification/os/os-notify-bridge").then((m) => m.installOsNotifyBridge)],
        // `block:reveal`: srv asks THIS window to reveal a block another
        // window's revealBlock couldn't reach.
        // SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md §4.3.
        ["reveal-block-events", import("@/app/util/reveal-block-events").then((m) => m.installBlockRevealEvents)],
    ];

    t = performance.now();
    const elem = document.getElementById("main");
    render(App, elem);
    tlog("SolidJS render", t);

    for (const [name, loaded] of windowServices) {
        loaded
            .then((install) => install())
            .catch((e) => {
                console.error(`[initMux] window service ${name} failed to start`, e);
                getApi().sendLog(`[initMux] window service ${name} failed to start: ${e}`);
            });
    }

    // Refresh the Claude model catalog from the authoritative /v1/models list
    // (backend `providers.models`, account OAuth token). Fire-and-forget: the
    // model drop-up shows the curated static list until this resolves, then
    // re-renders with fresh labels (Sonnet 5, …) + any new families (Fable).
    // Best-effort — returns [] with no token (logged out / macOS Keychain).
    fireAndForget(async () => {
        const res = await RpcApi.ProvidersModelsCommand(TabRpcClient, { provider_id: "claude" });
        setProviderModels(
            "claude",
            (res?.models ?? []).map((m) => ({ value: m.id, label: m.display_name })),
        );
    });

    tlog("TOTAL initMux", t0);

    // Register this window's backend ID with the CEF host so on_before_close
    // can call CloseWindow on the backend when this window is destroyed.
    // The CEF host handles cleanup at the right time (after the browser commits to
    // closing), keeping shells grouped under the CEF process in Task Manager.
    {
        const wlabel = currentWindowLabel();
        const wid = initOpts.windowId;
        console.log(`[wave] registerBackendWindow decision: wlabel=${wlabel} wid=${wid ?? "(falsy)"}`);
        if (wid) {
            getApi().registerBackendWindow(wlabel, wid);
        } else {
            console.error(`[wave] registerBackendWindow SKIPPED — windowId is falsy`);
        }
    }

    // NOTE: the startup splash (#startup-loading, the pulsing brain) is no
    // longer removed here. Removing it mid-mount exposed the bare chrome →
    // empty → piecemeal-mount cascade behind it (very visible on tear-off).
    // It is now cross-faded out by the content-reveal gate's "settled" moment
    // (tab-reveal.ts `liftGate` → startup-splash.ts `fadeOutStartupSplash`),
    // so the brain covers the whole bootstrap and the transition reads as
    // brain → content with nothing uncovered in between.

    getApi().setWindowInitStatus("wave-ready");
}
