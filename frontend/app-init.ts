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
import { getLayoutModelForStaticTab } from "@/layout/index";
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
import * as WOS from "@/app/store/wos";
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
import { isWindows } from "@/util/platformutil";
import { render } from "solid-js/web";
import { benchMark, benchDump } from "@/util/startup-bench";
import { ContextMenuModel } from "@/app/store/contextmenu";
import { isHostApp } from "@/app/init/host-detect";
import { showStartupError } from "@/app/init/error-display";
import { withTimeout } from "@/app/init/timeout";
import { fireAndForget } from "@/util/util";
import { setProviderModels } from "@/app/view/agent/providers";
import { scheduleRevealLift } from "@/store/tab-reveal";
import { installLauncherEventBridge } from "@/util/launcher-events";
import { installSrvEventBridge } from "@/util/srv-events";
import { REDOCK_DWELL_MS } from "@/app/workspace/floating-pane-constants";
import {
    seedKnownEntriesFromSnapshot,
    startLauncherEventReducer,
} from "@/app/store/launcher-event-reducer";
import { startSingletonCrashRelease } from "@/app/store/singleton-modal";

// Deferred — assigned inside initApp() after window.api is ready.
// Do NOT call getApi() at module level: this file is statically imported by
// bootstrap.ts before setupCefApi() runs, so window.api does not exist yet.
let platform: NodeJS.Platform;
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
 * Subscribe to the host's `floating-redock:hover-state` event so this
 * window paints a drop-slot preview when a floater is being dragged
 * over it. The host computes the target window via the same Z-order
 * walk `resolve_window_at_cursor` uses (with the dragged floater
 * excluded) and emits this event on every mousemove update; payload
 * includes the cursor position in physical screen px.
 *
 * Each window's renderer runs this in its own JS context. When the
 * event targets our window:
 *   1. Convert cursor screen-physical-px → client CSS px (DPR +
 *      `window.screenX/Y`, same pattern as `tabbar.tsx`).
 *   2. `document.elementFromPoint(clientX, clientY)` → walk to the
 *      nearest `[data-blockid]` ancestor.
 *   3. Run `determineDropDirection` (from `@/layout/lib/utils`) on
 *      the cursor's position within the leaf — same helper as
 *      within-window pane drag — to classify the drop as
 *      Center / Top / Right / Bottom / Left (and Outer variants).
 *   4. Render a singleton overlay div positioned over the
 *      half/quadrant/full-leaf that maps to that direction (e.g. Top
 *      → top half of leaf; Center → full leaf; OuterRight → right
 *      fifth of leaf).
 *
 * The block STILL lands as a sibling in the target tab's layout for
 * MVP (backend ignores the direction). Phase 4b will wire the
 * direction through `RedockFloatingPane` so the block lands in the
 * exact slot the user previewed.
 */
function installFloatingRedockHoverListener(): void {
    const myLabel =
        new URLSearchParams(window.location.search).get("windowLabel") || "main";
    let placeholderEl: HTMLDivElement | null = null;

    const ensurePlaceholder = (): HTMLDivElement => {
        if (!placeholderEl) {
            placeholderEl = document.createElement("div");
            placeholderEl.className = "floating-redock-drop-placeholder";
            document.body.appendChild(placeholderEl);
        }
        return placeholderEl;
    };
    const clearPlaceholder = () => {
        if (placeholderEl) {
            placeholderEl.remove();
            placeholderEl = null;
        }
        // Phase 4b — clear the stored ghost state for this window so a stale
        // direction cannot bleed into the next drop event.
        fireAndForget(async () => {
            const { invokeCommand } = await import("@/app/platform/ipc");
            await invokeCommand("set_floating_redock_target", {
                window_label: myLabel,
                block_id: null,
                dir: null,
            });
        });
    };

    // Map a DropDirection to a sub-rect (top, left, width, height in
    // client CSS px) within the leaf rect. Mirrors the visual the
    // within-window pane drag's `.placeholder` provides:
    //   Center      → full leaf
    //   Top/Bottom  → half along that axis
    //   Left/Right  → half along that axis
    //   Outer*      → thin (1/5) band against that edge
    const rectForDirection = (leaf: DOMRect, dir: number): { top: number; left: number; width: number; height: number } => {
        // DropDirection enum: Top=0, Right=1, Bottom=2, Left=3,
        //   OuterTop=4, OuterRight=5, OuterBottom=6, OuterLeft=7,
        //   Center=8.
        switch (dir) {
            case 0: // Top
                return { top: leaf.top, left: leaf.left, width: leaf.width, height: leaf.height / 2 };
            case 1: // Right
                return { top: leaf.top, left: leaf.left + leaf.width / 2, width: leaf.width / 2, height: leaf.height };
            case 2: // Bottom
                return { top: leaf.top + leaf.height / 2, left: leaf.left, width: leaf.width, height: leaf.height / 2 };
            case 3: // Left
                return { top: leaf.top, left: leaf.left, width: leaf.width / 2, height: leaf.height };
            case 4: // OuterTop
                return { top: leaf.top, left: leaf.left, width: leaf.width, height: leaf.height / 5 };
            case 5: // OuterRight
                return { top: leaf.top, left: leaf.left + (4 * leaf.width) / 5, width: leaf.width / 5, height: leaf.height };
            case 6: // OuterBottom
                return { top: leaf.top + (4 * leaf.height) / 5, left: leaf.left, width: leaf.width, height: leaf.height / 5 };
            case 7: // OuterLeft
                return { top: leaf.top, left: leaf.left, width: leaf.width / 5, height: leaf.height };
            case 8: // Center
            default:
                return { top: leaf.top, left: leaf.left, width: leaf.width, height: leaf.height };
        }
    };

    // Dwell gate for the redock ghost.
    // On Windows, Win32BeginMoveTask emits floating-redock:hover-state every 50ms
    // with no dwell awareness — we must gate the ghost ourselves to 180ms.
    // On non-Windows, the floater's onMouseMove path already waits REDOCK_DWELL_MS
    // before calling update_floating_redock_hover, so the first event arrives at
    // ~T=180ms; applying our own 180ms clock on top would delay to ~T=360ms and
    // would never fire if the cursor is held still (no further mousemoves = no
    // further events). Set ghostDwellMs=0 on non-Windows so the ghost appears on
    // the first event (already pre-gated by the floater).
    const ghostDwellMs = isWindows() ? REDOCK_DWELL_MS : 0;
    let dwellTarget: string | null = null;
    let dwellSince = 0;

    fireAndForget(async () => {
        const { listenEvent, invokeCommand } = await import("@/app/platform/ipc");
        const { determineDropDirection } = await import("@/layout/lib/utils");
        await listenEvent<{
            target_label: string | null;
            source_label?: string;
            cursor_x?: number;
            cursor_y?: number;
        }>("floating-redock:hover-state", (payload) => {
            const newTarget = payload?.target_label ?? null;
            if (!payload || newTarget !== myLabel) {
                // Target changed away from us or cleared — reset dwell and hide ghost.
                if (dwellTarget !== null) {
                    dwellTarget = null;
                    dwellSince = 0;
                }
                clearPlaceholder();
                return;
            }
            // Cursor is over our window. Start or continue dwell clock.
            const now = performance.now();
            if (dwellTarget !== myLabel) {
                dwellTarget = myLabel;
                dwellSince = now;
            }
            // Ghost only appears after the dwell has elapsed (0 on non-Windows).
            if (now - dwellSince < ghostDwellMs) return;

            const cursorX = payload.cursor_x;
            const cursorY = payload.cursor_y;
            if (typeof cursorX !== "number" || typeof cursorY !== "number") {
                clearPlaceholder();
                return;
            }
            // The broadcast cursor is in the host's coordinate space: physical
            // px on Windows (divide by DPR to get CSS px), DIP on macOS/Linux
            // (already CSS px — no divide). Inverse of the sender's posScale()
            // in floating-pane-workspace.tsx. Without this the drop-zone
            // highlight lands wrong on a Retina display.
            const invScale = isWindows() ? window.devicePixelRatio || 1 : 1;
            const clientX = cursorX / invScale - window.screenX;
            const clientY = cursorY / invScale - window.screenY;
            const el = document.elementFromPoint(clientX, clientY) as HTMLElement | null;
            const leafEl = el?.closest("[data-blockid]") as HTMLElement | null;
            if (!leafEl) {
                clearPlaceholder();
                return;
            }
            const leafRect = leafEl.getBoundingClientRect();
            const dir = determineDropDirection(
                {
                    width: leafRect.width,
                    height: leafRect.height,
                    left: leafRect.left,
                    top: leafRect.top,
                },
                { x: clientX, y: clientY },
            );
            if (dir === undefined) {
                clearPlaceholder();
                return;
            }
            const slot = rectForDirection(leafRect, dir);
            const ph = ensurePlaceholder();
            ph.style.top = `${slot.top}px`;
            ph.style.left = `${slot.left}px`;
            ph.style.width = `${slot.width}px`;
            ph.style.height = `${slot.height}px`;

            // Phase 4b — store the computed direction and target block so
            // the floater can pass them to RedockFloatingPane at drop time.
            // Fire-and-forget: the set is best-effort and must not stall the
            // event handler (ghost rendering happens synchronously above).
            const targetBlockId = leafEl.dataset.blockid;
            if (targetBlockId) {
                void invokeCommand("set_floating_redock_target", {
                    window_label: myLabel,
                    block_id: targetBlockId,
                    dir,
                });
            }
        });
    });
}

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
        let windowData = await withTimeout(WindowService.GetWindow(windowId), RPC_TIMEOUT, "GetWindow");
        tlog("GetWindow", t);

        if (!windowData) {
            t = performance.now();
            windowData = await withTimeout(WindowService.CreateWindow(null, "", currentWindowLabel()), RPC_TIMEOUT, "CreateWindow");
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
        await initWaveWrap(initOpts);
        tlog("initWaveWrap", t);
        tlog("TOTAL initTauriWave", t0);

        // Apply dev window title — task dev TITLE="agentx: PR #1780"
        // Only runs in Vite dev mode; VITE_DEV_TITLE is empty string in prod builds.
        const devTitle = import.meta.env.VITE_DEV_TITLE;
        if (import.meta.env.DEV && devTitle) {
            void ObjectService.UpdateObjectMeta(
                WOS.makeORef("window", initOpts.windowId),
                { [DISPLAY_NAME_META_KEY]: devTitle.slice(0, DISPLAY_NAME_MAX_LEN) } as MetaType,
            );
        }

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
        const newWindow = await withTimeout(WindowService.CreateWindow(null, tearOffWsId, currentWindowLabel()), RPC_TIMEOUT, "CreateWindow");
        tlog("CreateWindow", t);

        // Register label→window_id with the host NOW, not at the end of
        // initWave (which also registers — idempotently — after render):
        // the srv window row exists as of this line, and every host close
        // path (on_before_close, demote_srv_cleanup) resolves WHICH srv row
        // to close through this registration. A window closed in the
        // seconds between CreateWindow and initWave's late registration —
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

// initApp has two callers — bootstrap.ts and this module's self-start below —
// and in CEF both fire (isHostApp() reads window.__AGENTMUX_IPC_PORT__, which
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
    // until installWindowTitleEffect() runs at the end of initWave(). The
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
                await initHostNewWindow();
                if (initialView) {
                    await TabRpcClient.rpcCall("pane.open", {
                        view: initialView,
                        ...(initialMeta ? { meta: initialMeta } : {}),
                        floating: false,
                    }, {});
                }
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
                    await initHostWave();
                } else {
                    // New window: create new backend window objects
                    const label = await getApi().getWindowLabel();
                    getApi().sendLog(`Initializing as new window: ${label}`);
                    await initHostNewWindow();
                    const coldSearchParams = new URL(window.location.href).searchParams;
                    const coldInitialView = coldSearchParams.get("initialView");
                    if (coldInitialView) {
                        const coldMetaRaw = coldSearchParams.get("initialMeta");
                        let coldMeta: Record<string, unknown> | undefined;
                        try { coldMeta = coldMetaRaw ? JSON.parse(coldMetaRaw) : undefined; } catch { /* ignore */ }
                        await TabRpcClient.rpcCall("pane.open", {
                            view: coldInitialView,
                            ...(coldMeta ? { meta: coldMeta } : {}),
                            floating: false,
                        }, {});
                    }
                }
            }
        } catch (error) {
            console.error("[initApp] Host initialization failed:", error);
            getApi().sendLog(`Host init error: ${error}`);
            showStartupError(String(error));
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
        // Phase B.7.3.1 — start the launcher-event reducer effect now
        // that global state is wired. Idempotent: subsequent calls
        // (e.g. via reinitWave path) are no-ops.
        startLauncherEventReducer();
        // Bundle-management PR 3 — wire singleton-modal crash release.
        // Subscribes to the launcher window-exit signal so a dead
        // holder's singleton claim is auto-released. Idempotent.
        startSingletonCrashRelease();
    } catch (e) {
        getApi().sendLog("Error in initWave " + e.message + "\n" + e.stack);
        console.error("Error in initWave", e);
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
        //     own bootstrap → initWaveWrap)
        //   - Any future window-spawning path that reuses initWave
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
    // Title is driven by installWindowTitleEffect() (set up in initWave)
    // and reacts to atom changes — reinitWave's reloads update the atoms,
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
                ? WOS.getObjectValue<Tab>(WOS.makeORef("tab", activeTabId))
                : undefined;
            const win = WOS.getObjectValue<WaveWindow>(WOS.makeORef("window", windowId));
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
                    "Speech recognition isn't available in this build yet. " +
                    "Server-side transcription is in progress.";
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
    installFloatingRedockHoverListener();
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

    t = performance.now();
    const elem = document.getElementById("main");
    render(App, elem);
    tlog("SolidJS render", t);

    // Start the auto pane-overlay clip service. Any DOM element tagged
    // `data-pane-overlay` automatically participates in browser-pane
    // clipping. See docs/specs/SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md.
    const { startPaneOverlayAutoService } = await import("@/app/platform/pane-overlay-auto");
    startPaneOverlayAutoService();

    // Sound notifications. Subscribes to agent-pane reducer events and
    // plays a polite SFX on turn-complete (and other configured signals).
    // See docs/specs/SPEC_SOUND_NOTIFICATIONS_2026_06_05.md.
    const { installSoundService } = await import("@/app/notification/sound");
    installSoundService();

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

    tlog("TOTAL initWave", t0);

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
