// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tauri API shim — provides the same window.api (AppApi) interface
// using Tauri's invoke() and listen() APIs.
//
// Provides the AppApi interface via Tauri invoke/listen.
// Must be loaded before the React app bootstraps.

import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { openUrl, openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import { benchMark } from "@/util/startup-bench";

// Tauri injects this global at build time via TAURI_ENV_APP_VERSION
declare const __TAURI_APP_VERSION__: string | undefined;

// Cache for "synchronous" values that are fetched once at startup.
// All IPC is async, so we pre-fetch and cache.
let cachedValues: {
    authKey: string;
    isDev: boolean;
    platform: string;
    userName: string;
    hostName: string;
    dataDir: string;
    configDir: string;
    docsiteUrl: string;
    zoomFactor: number;
    updaterStatus: UpdaterStatus;
    updaterChannel: string;
    aboutDetails: AboutModalDetails;
} | null = null;

/**
 * Initialize the Tauri API shim by pre-fetching all cached values.
 * Must be called before the React app renders.
 */
export async function initTauriApi(): Promise<void> {
    benchMark("initTauriApi-start");
    // Try fetching backend endpoints first (in case backend is already ready)
    // If it fails, wait for the backend-ready event
    console.log("[tauri-api] Checking if backend is ready...");
    let backendEndpoints: { ws: string; web: string };

    try {
        backendEndpoints = await invoke<{ ws: string; web: string }>("get_backend_endpoints");
        console.log("[tauri-api] Backend already ready:", backendEndpoints);
        benchMark("backend-endpoints-cached");
    } catch (e) {
        benchMark("backend-wait-start");
        console.log("[tauri-api] Backend not ready yet, waiting for backend-ready event...");
        backendEndpoints = await new Promise<{ ws: string; web: string }>((resolve) => {
            listen<{ ws: string; web: string }>("backend-ready", (event) => {
                console.log("[tauri-api] Backend ready:", event.payload);
                resolve(event.payload);
            });
        });
        benchMark("backend-ready-received");
    }
    console.log("[tauri-api] Using backend endpoints:", backendEndpoints);

    // Set endpoints as window globals for getEnv() to find
    (window as any).__WAVE_SERVER_WS_ENDPOINT__ = backendEndpoints.ws;
    (window as any).__WAVE_SERVER_WEB_ENDPOINT__ = backendEndpoints.web;

    benchMark("invoke-batch-start");
    const [
        authKey,
        isDev,
        platform,
        userName,
        hostName,
        dataDir,
        configDir,
        docsiteUrl,
        zoomFactor,
        aboutDetails,
    ] = await Promise.all([
        invoke<string>("get_auth_key"),
        invoke<boolean>("get_is_dev"),
        invoke<string>("get_platform"),
        invoke<string>("get_user_name"),
        invoke<string>("get_host_name"),
        invoke<string>("get_data_dir"),
        invoke<string>("get_config_dir"),
        invoke<string>("get_docsite_url"),
        invoke<number>("get_zoom_factor"),
        invoke<AboutModalDetails>("get_about_modal_details"),
    ]);
    benchMark("invoke-batch-done");

    cachedValues = {
        authKey,
        isDev,
        platform,
        userName,
        hostName,
        dataDir,
        configDir,
        docsiteUrl,
        zoomFactor,
        aboutDetails,
        updaterStatus: "up-to-date" as UpdaterStatus,
        updaterChannel: "latest",
    };
}

/**
 * Build the AppApi-compatible shim backed by Tauri invoke/listen.
 */
export function buildTauriApi(): AppApi {
    if (!cachedValues) {
        throw new Error("initTauriApi() must be called before buildTauriApi()");
    }

    const api: AppApi = {
        // --- Synchronous getters (return cached values) ---
        getAuthKey: () => cachedValues!.authKey,
        getIsDev: () => cachedValues!.isDev,
        getPlatform: () => cachedValues!.platform as NodeJS.Platform,
        getUserName: () => cachedValues!.userName,
        getHostName: () => cachedValues!.hostName,
        getDataDir: () => cachedValues!.dataDir,
        getConfigDir: () => cachedValues!.configDir,
        getDocsiteUrl: () => cachedValues!.docsiteUrl,
        getZoomFactor: () => cachedValues!.zoomFactor,
        getEnv: (varName: string) => {
            // In Tauri, we can't synchronously get env vars.
            // Fire-and-forget the invoke and return empty for now.
            // Most env var usage should be migrated to async.
            return "";
        },

        // --- Cursor ---
        getCursorPoint: () => {
            // Returns a default; the actual async version should be used where possible.
            return { x: 0, y: 0 };
        },

        // --- About ---
        getAboutModalDetails: () => {
            return cachedValues!.aboutDetails;
        },
        getBackendInfo: async () => {
            return await invoke<{ pid?: number; started_at?: string; web_endpoint?: string; version: string }>(
                "get_backend_info"
            );
        },

        // --- Context menu ---
        showContextMenu: (workspaceId: string, menu?: NativeContextMenuItem[], position?: { x: number; y: number }) => {
            invoke("show_context_menu", { workspaceId, menu, position }).catch(console.error);
        },
        onContextMenuClick: (callback: (id: string) => void) => {
            listen<string>("context-menu-click", (event) => {
                callback(event.payload);
            });
        },

        // --- Navigation (no-ops in Tauri, handled differently) ---
        onNavigate: (_callback: (url: string) => void) => {
            // Navigation interception handled via Tauri's URL scheme filtering
        },
        onIframeNavigate: (_callback: (url: string) => void) => {
            // No iframe navigation interception needed in Tauri
        },

        // --- File operations ---
        downloadFile: (path: string) => {
            invoke("download_file", { path }).catch(console.error);
        },
        openExternal: (url: string) => {
            openUrl(url).catch(console.error);
        },
        openNativePath: (filePath: string) => {
            openPath(filePath).catch(console.error);
        },
        revealInFileExplorer: (filePath: string) => {
            revealItemInDir(filePath).catch(console.error);
        },
        onQuicklook: (filePath: string) => {
            invoke("quicklook", { filePath }).catch(console.error);
        },

        // --- Window events ---
        onFullScreenChange: (callback: (isFullScreen: boolean) => void) => {
            listen<boolean>("fullscreen-change", (event) => {
                callback(event.payload);
            });
        },
        onZoomFactorChange: (callback: (zoomFactor: number) => void) => {
            listen<number>("zoom-factor-change", (event) => {
                cachedValues!.zoomFactor = event.payload;
                callback(event.payload);
            });
        },
        setZoomFactor: (zoomFactor: number) => {
            invoke("set_zoom_factor", { factor: zoomFactor }).catch(console.error);
        },

        // --- Updater ---
        getUpdaterStatus: () => cachedValues!.updaterStatus,
        getUpdaterChannel: () => cachedValues!.updaterChannel,
        onUpdaterStatusChange: (callback: (status: UpdaterStatus) => void) => {
            listen<UpdaterStatus>("app-update-status", (event) => {
                cachedValues!.updaterStatus = event.payload;
                callback(event.payload);
            });
        },
        installAppUpdate: () => {
            invoke("install_update").catch(console.error);
        },

        // --- Menu ---
        onMenuItemAbout: (callback: () => void) => {
            listen("menu-item-about", () => callback());
        },

        // --- Window controls ---
        updateWindowControlsOverlay: (rect: Dimensions) => {
            invoke("update_wco", { rect }).catch(console.error);
        },

        // --- Keyboard ---
        onReinjectKey: (callback: (waveEvent: WaveKeyboardEvent) => void) => {
            listen<WaveKeyboardEvent>("reinject-key", (event) => {
                callback(event.payload);
            });
        },
        setKeyboardChordMode: () => {
            invoke("set_keyboard_chord_mode").catch(console.error);
        },
        onControlShiftStateUpdate: (callback: (state: boolean) => void) => {
            listen<boolean>("control-shift-state-update", (event) => {
                callback(event.payload);
            });
        },

        // --- Window Management (Multi-Window Support) ---
        openNewWindow: async () => {
            return await invoke<string>("open_new_window");
        },
        closeWindow: async (label?: string) => {
            await invoke("close_window", { label: label ?? null });
        },
        minimizeWindow: () => {
            invoke("minimize_window").catch(console.error);
        },
        maximizeWindow: () => {
            invoke("maximize_window").catch(console.error);
        },
        setWindowTransparency: (transparent: boolean, blur: boolean, opacity: number) => {
            invoke("set_window_transparency", { transparent, blur, opacity }).catch(console.error);
        },
        toggleDevtools: () => {
            invoke("toggle_devtools").catch(console.error);
        },
        getWindowLabel: async () => {
            return await invoke<string>("get_window_label");
        },
        isMainWindow: async () => {
            return await invoke<boolean>("is_main_window");
        },
        listWindows: async () => {
            return await invoke<string[]>("list_windows");
        },
        focusWindow: async (label: string) => {
            await invoke("focus_window", { label });
        },
        getInstanceNumber: async () => {
            return await invoke<number>("get_instance_number");
        },
        getWindowCount: async () => {
            return await invoke<number>("get_window_count");
        },

        // --- Workspace & Tabs ---
        // In Tauri, tabs are managed in the frontend (React state).
        // These still invoke backend operations via the Go backend's WebSocket API.
        createWorkspace: () => {
            invoke("create_workspace").catch(console.error);
        },
        switchWorkspace: (workspaceId: string) => {
            invoke("switch_workspace", { workspaceId }).catch(console.error);
        },
        deleteWorkspace: (workspaceId: string) => {
            invoke("delete_workspace", { workspaceId }).catch(console.error);
        },
        setActiveTab: (tabId: string) => {
            // Tab switching is frontend-only in Tauri (no WebContentsView to move)
            // Still notify the backend for state persistence
            invoke("set_active_tab", { tabId }).catch(console.error);
        },
        createTab: () => {
            invoke("create_tab").catch(console.error);
        },
        closeTab: (workspaceId: string, tabId: string) => {
            invoke("close_tab", { workspaceId, tabId }).catch(console.error);
        },

        // --- Init ---
        setWindowInitStatus: (status: "ready" | "wave-ready") => {
            invoke("set_window_init_status", { status }).catch(console.error);
        },
        onAgentMuxInit: (callback: (initOpts: AgentMuxInitOpts) => void) => {
            listen<AgentMuxInitOpts>("agentmux-init", (event) => {
                callback(event.payload);
            });
        },

        // --- Logging ---
        sendLog: (log: string) => {
            invoke("fe_log", { msg: log }).catch(console.error);
        },
        sendLogStructured: (level: string, module: string, message: string, data: Record<string, any> | null) => {
            invoke("fe_log_structured", { level, module, message, data }).catch(() => {});
        },

        // --- Screenshot ---
        captureScreenshot: async (_rect: { x: number; y: number; width: number; height: number }): Promise<string> => {
            // No equivalent in Tauri — return empty string
            // If needed, can be implemented with html-to-image library
            return "";
        },

        // --- Claude Code Auth ---
        openClaudeCodeAuth: async () => {
            console.trace("[LEGACY] openClaudeCodeAuth called — who called this?");
            await invoke("open_claude_code_auth");
        },
        getClaudeCodeAuth: async () => {
            return await invoke<{ connected: boolean; email?: string; expires_at?: number }>(
                "get_claude_code_auth"
            );
        },
        disconnectClaudeCode: async () => {
            await invoke("disconnect_claude_code");
        },

        // --- Provider Commands ---
        detectInstalledClis: async () => {
            return await invoke<CliDetectionResult[]>("detect_installed_clis");
        },
        getProviderConfig: async () => {
            return await invoke<ProviderConfig>("get_provider_config");
        },
        saveProviderConfig: async (config: ProviderConfig) => {
            await invoke("save_provider_config", { config });
        },
        getProviderInstallInfo: async (provider: string) => {
            return await invoke<ProviderInstallInfo>("get_provider_install_info", { provider });
        },
        setProviderAuth: async (provider: string, token: string) => {
            await invoke("set_provider_auth", { provider, token });
        },
        clearProviderAuth: async (provider: string) => {
            await invoke("clear_provider_auth", { provider });
        },
        getProviderAuthStatus: async (provider: string) => {
            return await invoke<ProviderAuthStatus>("get_provider_auth_status", { provider });
        },
        checkCliAuthStatus: async (provider: string, cliPath?: string) => {
            return await invoke<CliAuthStatus>("check_cli_auth_status", { provider, cliPath: cliPath ?? null });
        },
        installCli: async (provider: string) => {
            return await invoke<CliInstallResult>("install_cli", { provider });
        },
        getCliPath: async (provider: string) => {
            return await invoke<string | null>("get_cli_path", { provider });
        },
        checkNodejsAvailable: async () => {
            return await invoke<NodejsStatus>("check_nodejs_available");
        },
        ensureAuthDir: async (providerId: string) => {
            return await invoke<string>("ensure_auth_dir", { providerId });
        },
        runCliLogin: async (cliPath: string, loginArgs: string[], authEnv: Record<string, string>) => {
            return await invoke<string | null>("run_cli_login", { cliPath, loginArgs, authEnv });
        },
        cancelCliLogin: async () => {
            await invoke("cancel_cli_login");
        },

        listen: async (event: string, callback: (event: any) => void) => {
            const unlisten = await listen(event, callback);
            return unlisten;
        },

        // --- Cross-window drag ---
        startCrossDrag: async (
            dragType: "pane" | "tab",
            sourceWindow: string,
            sourceWorkspaceId: string,
            sourceTabId: string,
            payload: { blockId?: string; tabId?: string }
        ) => {
            return await invoke<string>("start_cross_drag", {
                dragType,
                sourceWindow,
                sourceWorkspaceId,
                sourceTabId,
                payload,
            });
        },
        updateCrossDrag: async (dragId: string, screenX: number, screenY: number) => {
            return await invoke<string | null>("update_cross_drag", { dragId, screenX, screenY });
        },
        completeCrossDrag: async (
            dragId: string,
            targetWindow: string | null,
            screenX: number,
            screenY: number
        ) => {
            await invoke("complete_cross_drag", { dragId, targetWindow, screenX, screenY });
        },
        cancelCrossDrag: async (dragId: string) => {
            await invoke("cancel_cross_drag", { dragId });
        },
        openWindowAtPosition: async (screenX: number, screenY: number, workspaceId?: string) => {
            return await invoke<string>("open_window_at_position", { screenX, screenY, workspaceId: workspaceId ?? "" });
        },

        // --- Drag cursor ---
        setDragCursor: async () => {
            await invoke("set_drag_cursor");
        },
        restoreDragCursor: async () => {
            await invoke("restore_drag_cursor");
        },
        releaseDragCapture: async () => {
            await invoke("release_drag_capture");
        },
        setJsDragActive: async (active: boolean) => {
            await invoke("set_js_drag_active", { active });
        },
    };

    return api;
}

/**
 * Detect whether we're running inside Tauri.
 */
export function isTauri(): boolean {
    return typeof (window as any).__TAURI_INTERNALS__ !== "undefined";
}
