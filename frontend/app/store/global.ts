// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Global app state — migrated from Jotai atoms to SolidJS signals.

import { WpsEvent } from "@/app/store/wps-events";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { setPlatform } from "@/util/platformutil";
import { createMemo, createSignal } from "solid-js";
import { reconnectWS } from "./ws";
import {
    backendStatusAtom,
    backendDeathInfoAtom,
    initBackendStatusListeners,
    setBackendStatusAtom,
} from "./backendStatus";
import { openModal } from "./modalmodel";
import { AboutModal } from "@/app/modals/about";
import { UserInputModal } from "@/app/modals/userinputmodal";
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
    getBlockMetaKeyAtom,
    getSettingsKeyAtom,
    getOverrideConfigAtom,
    getSettingsPrefixAtom,
    useBlockAtom,
} from "./block-atom-cache";
import {
    setWindowId,
    clientId,
    setClientId,
    staticTabId,
    setStaticTabId,
    client,
    waveWindow,
    workspace,
    tabAtom,
    activeTabId,
    uiContext,
} from "./window-identity";
import { allConnStatus } from "./conn-status";
import { flashErrors, notifications, notificationPopoverMode } from "./flash-notifications";

// ---------------------------------------------------------------------------
// Global signals (replace Jotai atoms)
// ---------------------------------------------------------------------------

// Window identity — moved to window-identity.ts (see below for re-export);
// imported above for use in the `atoms` back-compat object and initGlobalSignals.

export { fullConfigAtom, setFullConfigAtom, settingsAtom };

const [isFullScreen, setIsFullScreen] = createSignal(false);
export { isFullScreen };
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

// Reduced-motion respect is removed app-wide (product decision,
// 2026-07-11): OS-level "reduce motion"/disabled-animations settings were
// silently killing functional motion cues (drag strobes, drop pulses,
// insertion indicators) that carry meaning rather than decoration. The
// setting plumbing above is kept so the decision is easy to revisit, but
// the atom every consumer reads is hard false.
export const prefersReducedMotionAtom = createMemo(() => false);

export { backendStatusAtom, setBackendStatusAtom, backendDeathInfoAtom };

const [typeAheadModalAtom] = createSignal<Record<string, unknown>>({});
export const [modalOpen, setModalOpen] = createSignal(false);

const [reinitVersion, setReinitVersion] = createSignal(0);
export { setReinitVersion };
export const [isTermMultiInput, setIsTermMultiInput] = createSignal(false);

export const [, setWindowInstanceNumAtom] = createSignal(0);
export const [windowCountAtom, setWindowCountAtom] = createSignal(1);
const [lanInstancesAtom, setLanInstancesAtom] = createSignal<LanInstance[]>([]);
export { lanInstancesAtom };
// Last error message from the LAN discovery daemon (e.g. firewall block).
// Cleared on successful enable. See docs/specs/lan-discovery-toggle.md.
export const [lanDiscoveryErrorAtom, setLanDiscoveryErrorAtom] = createSignal<string | null>(null);

// List of all open AgentMux window labels in this process. Updated by
// app-init's window-instances-changed listener whenever a window opens
// or closes. Consumed by the version-click instance panel.
// See SPEC_VERSION_INSTANCE_PANEL_2026_04_25.md.
export const [, setOpenWindowLabelsAtom] = createSignal<string[]>([]);

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

let globalPrimaryTabStartup = false;

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
            eventType: WpsEvent.WaveObjBatchedUpdates,
            handler: (event) => {
                // All updates from one atomic backend transition, applied in
                // one batch() flush (updateWaveObjects) so the UI can't paint
                // a half-applied state — e.g. CloseTab's tab delete blanking
                // the still-mounted tab before the workspace update unmounts
                // it. See SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md §7.
                const updates: WaveObjUpdate[] = event.data ?? [];
                WOS.updateWaveObjects(updates);
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
    getBlockMetaKeyAtom,
    getSettingsKeyAtom,
    getOverrideConfigAtom,
    getSettingsPrefixAtom,
    useBlockAtom,
} from "./block-atom-cache";

// Window identity — moved to window-identity.ts; re-exported below for
// backward-compat (97 files import from this module). Also imported above
// (see top of file) for use in the `atoms` object and initGlobalSignals.
export {
    staticTabId,
    client,
    waveWindow,
    workspace,
    tabAtom,
    activeTabId,
    uiContext,
};

// Block creation / layout actions — moved to block-layout-actions.ts;
// re-exported below for backward-compat (97 files import from this module).
export {
    createBlockSplitHorizontally,
    createBlockSplitVertically,
    createBlock,
    replaceBlock,
} from "./block-layout-actions";

// Block component model registry — moved to block-component-registry.ts;
// re-exported below for backward-compat (97 files import from this module).
export {
    registerBlockComponentModel,
    unregisterBlockComponentModel,
    getBlockComponentModel,
    getAllBlockComponentModels,
    getFocusedBlockId,
    refocusNode,
    openOrFocusPaneByView,
} from "./block-component-registry";

// Wave file fetching — moved to wave-file.ts; re-exported below for
// backward-compat (97 files import from this module).
export { fetchWaveFile } from "./wave-file";

// Connection status — moved to conn-status.ts; re-exported below for
// backward-compat (97 files import from this module). Also imported above
// (see top of file) for use in the `atoms` object.
export {
    allConnStatus,
    loadConnStatus,
    subscribeToConnEvents,
    getConnStatusAtom,
} from "./conn-status";

// Flash errors / notifications — moved to flash-notifications.ts;
// re-exported below for backward-compat (97 files import from this module).
// Also imported above (see top of file) for use in the `atoms` object.
export {
    flashErrors,
    notifications,
    setNotifications,
    notificationPopoverMode,
    setNotificationPopoverMode,
    pushFlashError,
    pushNotification,
    removeNotificationById,
    removeFlashError,
} from "./flash-notifications";

// Tab management — moved to tab-actions.ts; re-exported below for
// backward-compat (97 files import from this module).
export { createTab, setActiveTab } from "./tab-actions";

// ---------------------------------------------------------------------------
// Telemetry
// ---------------------------------------------------------------------------

export function recordTEvent(event: string, props?: TEventProps) {
    if (props == null) props = {};
    RpcApi.RecordTEventCommand(TabRpcClient, { event, props }, { noresponse: true });
}

// Counters (dev tooling) — moved to dev-counters.ts; re-exported below for
// backward-compat (97 files import from this module).
export { countersClear, counterInc, countersPrint } from "./dev-counters";

// Misc utilities — moved to misc-utils.ts; re-exported below for
// backward-compat (97 files import from this module).
export { isDev, getUserName, getHostName, openLink } from "./misc-utils";

// Re-export WOS and getApi for call-sites that import them from here
export { WOS, getApi };
