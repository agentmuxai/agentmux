// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CEF API shim — provides the same window.api (AppApi) interface
// using the platform-agnostic invokeCommand()/listenEvent() from ipc.ts.
//
// This is the CEF equivalent of tauri-api.ts. Must be loaded before
// the React app bootstraps.

import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import { benchMark } from "@/util/startup-bench";

// Cache for "synchronous" values that are fetched once at startup.
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
    updaterVersion: string | null;
    updaterChannel: string;
    aboutDetails: AboutModalDetails;
} | null = null;

/**
 * Initialize the CEF API shim by pre-fetching all cached values.
 * Must be called after __AGENTMUX_IPC_PORT__ and __AGENTMUX_IPC_TOKEN__
 * are set on window (from URL query params).
 */
export async function initCefApi(): Promise<void> {
    benchMark("initCefApi-start");

    // Wait for backend endpoints (backend may still be starting)
    console.log("[cef-api] Checking if backend is ready...");
    let backendEndpoints: { ws: string; web: string };

    try {
        backendEndpoints = await invokeCommand<{ ws: string; web: string }>("get_backend_endpoints");
        console.log("[cef-api] Backend already ready:", backendEndpoints);
        benchMark("backend-endpoints-cached");
    } catch (e) {
        benchMark("backend-wait-start");
        console.log("[cef-api] Backend not ready yet, waiting for backend-ready event...");
        backendEndpoints = await new Promise<{ ws: string; web: string }>((resolve, reject) => {
            const timeout = setTimeout(() => {
                reject(new Error("[cef-api] Backend failed to start within 30s"));
            }, 30_000);
            listenEvent<{ ws: string; web: string }>("backend-ready", (payload) => {
                clearTimeout(timeout);
                console.log("[cef-api] Backend ready:", payload);
                resolve(payload);
            });
            listenEvent<{ error: string }>("backend-spawn-error", (payload) => {
                clearTimeout(timeout);
                reject(new Error(`[cef-api] Backend spawn failed: ${payload.error}`));
            });
        });
        benchMark("backend-ready-received");
    }
    console.log("[cef-api] Using backend endpoints:", backendEndpoints);

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
        invokeCommand<string>("get_auth_key"),
        invokeCommand<boolean>("get_is_dev"),
        invokeCommand<string>("get_platform"),
        invokeCommand<string>("get_user_name"),
        invokeCommand<string>("get_host_name"),
        invokeCommand<string>("get_data_dir"),
        invokeCommand<string>("get_config_dir"),
        invokeCommand<string>("get_docsite_url"),
        invokeCommand<number>("get_zoom_factor"),
        invokeCommand<AboutModalDetails>("get_about_modal_details"),
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
        updaterVersion: null,
        updaterChannel: "latest",
    };
}

// Context menu click callback — registered by onContextMenuClick, called by showJsContextMenu.
let contextMenuClickCallback: ((id: string) => void) | null = null;

/**
 * Render a context menu as a positioned HTML overlay.
 * Fires the callback with the clicked item's id, then removes the overlay.
 */
function showJsContextMenu(
    items: NativeContextMenuItem[],
    position: { x: number; y: number },
    onClick: ((id: string) => void) | null
) {
    // Remove any existing menu
    document.getElementById("cef-context-menu-overlay")?.remove();

    const overlay = document.createElement("div");
    overlay.id = "cef-context-menu-overlay";
    Object.assign(overlay.style, {
        position: "fixed", inset: "0", zIndex: "99999",
    });
    overlay.addEventListener("mousedown", (e) => {
        if (e.target === overlay) { overlay.remove(); }
    });

    const menuEl = document.createElement("div");
    Object.assign(menuEl.style, {
        position: "absolute",
        left: `${position.x}px`,
        top: `${position.y}px`,
        background: "var(--main-bg-color, #222)",
        border: "1px solid var(--border-color, #444)",
        borderRadius: "6px",
        padding: "4px 0",
        minWidth: "160px",
        boxShadow: "0 4px 16px rgba(0,0,0,0.4)",
        fontFamily: "var(--termfontfamily, sans-serif)",
        fontSize: "13px",
        color: "var(--main-text-color, #ddd)",
    });

    function renderItems(container: HTMLElement, itemList: NativeContextMenuItem[]) {
        for (const item of itemList) {
            if (item.type === "separator") {
                const sep = document.createElement("div");
                Object.assign(sep.style, {
                    height: "1px", margin: "4px 8px",
                    background: "var(--border-color, #444)",
                });
                container.appendChild(sep);
                continue;
            }
            if (item.visible === false) continue;

            const row = document.createElement("div");
            Object.assign(row.style, {
                padding: "6px 24px 6px 12px",
                cursor: item.enabled === false ? "default" : "pointer",
                opacity: item.enabled === false ? "0.4" : "1",
                display: "flex",
                alignItems: "center",
                gap: "8px",
                whiteSpace: "nowrap",
                position: "relative",
            });
            if (item.enabled !== false) {
                row.addEventListener("mouseenter", () => { row.style.background = "var(--accent-color, #335)"; });
                row.addEventListener("mouseleave", () => { row.style.background = ""; });
            }

            // Radio/checkbox indicator
            if (item.type === "radio" || item.type === "checkbox") {
                const check = document.createElement("span");
                check.style.width = "14px";
                check.style.display = "inline-block";
                check.textContent = item.checked ? "●" : "";
                if (item.type === "checkbox" && item.checked) check.textContent = "✓";
                row.appendChild(check);
            }

            const label = document.createElement("span");
            label.textContent = item.label ?? "";
            row.appendChild(label);

            if (item.submenu && item.submenu.length > 0) {
                const arrow = document.createElement("span");
                arrow.textContent = "▸";
                arrow.style.marginLeft = "auto";
                row.appendChild(arrow);

                const sub = document.createElement("div");
                Object.assign(sub.style, {
                    display: "none", position: "absolute",
                    left: "100%", top: "0", zIndex: "100000",
                    background: "var(--main-bg-color, #222)",
                    border: "1px solid var(--border-color, #444)",
                    borderRadius: "6px", padding: "4px 0",
                    minWidth: "140px",
                    boxShadow: "0 4px 16px rgba(0,0,0,0.4)",
                });
                renderItems(sub, item.submenu);
                row.appendChild(sub);
                row.addEventListener("mouseenter", () => { sub.style.display = "block"; });
                row.addEventListener("mouseleave", () => { sub.style.display = "none"; });
            } else if (item.enabled !== false) {
                row.addEventListener("click", () => {
                    overlay.remove();
                    if (item.id && onClick) onClick(item.id);
                });
            }

            container.appendChild(row);
        }
    }

    renderItems(menuEl, items);

    // Clamp to viewport
    overlay.appendChild(menuEl);
    document.body.appendChild(overlay);
    const rect = menuEl.getBoundingClientRect();
    if (rect.right > window.innerWidth) {
        menuEl.style.left = `${Math.max(0, window.innerWidth - rect.width - 4)}px`;
    }
    if (rect.bottom > window.innerHeight) {
        menuEl.style.top = `${Math.max(0, window.innerHeight - rect.height - 4)}px`;
    }
}

/**
 * Build the AppApi-compatible shim backed by CEF IPC.
 */
export function buildCefApi(): AppApi {
    if (!cachedValues) {
        throw new Error("initCefApi() must be called before buildCefApi()");
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
        getEnv: (_varName: string) => {
            return "";
        },

        // --- Cursor ---
        getCursorPoint: () => {
            return { x: 0, y: 0 };
        },

        // --- About ---
        getAboutModalDetails: () => {
            return cachedValues!.aboutDetails;
        },
        getBackendInfo: async () => {
            return await invokeCommand<{ pid?: number; started_at?: string; web_endpoint?: string; version: string }>(
                "get_backend_info"
            );
        },
        restartBackend: async () => {
            await invokeCommand("restart_backend");
        },

        // --- Context menu (JS overlay for CEF — no native menu API) ---
        showContextMenu: (_workspaceId: string, menu?: NativeContextMenuItem[], position?: { x: number; y: number }) => {
            if (!menu || menu.length === 0) return;
            showJsContextMenu(menu, position ?? { x: 0, y: 0 }, contextMenuClickCallback);
        },
        onContextMenuClick: (callback: (id: string) => void) => {
            contextMenuClickCallback = callback;
        },

        // --- Navigation ---
        onNavigate: (_callback: (url: string) => void) => {
            // Navigation interception handled by CEF host
        },
        onIframeNavigate: (_callback: (url: string) => void) => {
            // No iframe navigation interception needed in CEF
        },

        // --- File operations ---
        downloadFile: (path: string) => {
            invokeCommand("download_file", { path }).catch(console.error);
        },
        openExternal: (url: string) => {
            // CEF: open in system browser
            invokeCommand("open_external", { url }).catch(console.error);
        },
        openNativePath: (filePath: string) => {
            invokeCommand("open_native_path", { filePath }).catch(console.error);
        },
        revealInFileExplorer: (filePath: string) => {
            invokeCommand("reveal_in_file_explorer", { filePath }).catch(console.error);
        },
        onQuicklook: (filePath: string) => {
            invokeCommand("quicklook", { filePath }).catch(console.error);
        },

        // --- Window events ---
        onFullScreenChange: (callback: (isFullScreen: boolean) => void) => {
            listenEvent<boolean>("fullscreen-change", (payload) => {
                callback(payload);
            });
        },
        onZoomFactorChange: (callback: (zoomFactor: number) => void) => {
            listenEvent<number>("zoom-factor-change", (payload) => {
                cachedValues!.zoomFactor = payload;
                callback(payload);
            });
        },
        setZoomFactor: (zoomFactor: number) => {
            invokeCommand("set_zoom_factor", { factor: zoomFactor }).catch(console.error);
        },

        // --- Updater ---
        getUpdaterStatus: () => cachedValues!.updaterStatus,
        getUpdaterVersion: () => cachedValues!.updaterVersion,
        getUpdaterChannel: () => cachedValues!.updaterChannel,
        onUpdaterStatusChange: (callback: (status: UpdaterStatus) => void) => {
            listenEvent<{ status: string; version?: string }>(
                "app-update-status",
                (payload) => {
                    const status = payload.status as UpdaterStatus;
                    cachedValues!.updaterStatus = status;
                    cachedValues!.updaterVersion = payload.version ?? null;
                    callback(status);
                }
            );
        },
        installAppUpdate: () => {
            invokeCommand("install_update").catch(console.error);
        },

        // --- Menu ---
        onMenuItemAbout: (callback: () => void) => {
            listenEvent("menu-item-about", () => callback());
        },

        // --- Window controls ---
        updateWindowControlsOverlay: (rect: Dimensions) => {
            invokeCommand("update_wco", { rect }).catch(console.error);
        },

        // --- Keyboard ---
        onReinjectKey: (callback: (waveEvent: WaveKeyboardEvent) => void) => {
            listenEvent<WaveKeyboardEvent>("reinject-key", (payload) => {
                callback(payload);
            });
        },
        setKeyboardChordMode: () => {
            invokeCommand("set_keyboard_chord_mode").catch(console.error);
        },
        onControlShiftStateUpdate: (callback: (state: boolean) => void) => {
            listenEvent<boolean>("control-shift-state-update", (payload) => {
                callback(payload);
            });
        },

        // --- Window Management ---
        openNewWindow: async () => {
            return await invokeCommand<string>("open_new_window");
        },
        closeWindow: async (label?: string) => {
            await invokeCommand("close_window", { label: label ?? null });
        },
        minimizeWindow: () => {
            invokeCommand("minimize_window").catch(console.error);
        },
        maximizeWindow: () => {
            invokeCommand("maximize_window").catch(console.error);
        },
        setWindowTransparency: (transparent: boolean, blur: boolean, opacity: number) => {
            invokeCommand("set_window_transparency", { transparent, blur, opacity }).catch(console.error);
        },
        toggleDevtools: async () => {
            // CEF host.show_dev_tools() crashes from IPC thread (wrap_task! broken
            // in CEF Rust bindings v146). Open DevTools in the system browser via
            // the remote debugging protocol (port 9222).
            try {
                const resp = await fetch("http://127.0.0.1:9222/json");
                const targets = await resp.json();
                const page = targets.find((t: any) => t.type === "page");
                if (page?.id) {
                    // Use Chrome's built-in DevTools frontend with the remote WS target
                    const dtUrl = `devtools://devtools/bundled/inspector.html?ws=127.0.0.1:9222/devtools/page/${page.id}`;
                    invokeCommand("open_external", { url: dtUrl }).catch(console.error);
                } else {
                    invokeCommand("open_external", { url: "http://127.0.0.1:9222" }).catch(console.error);
                }
            } catch {
                invokeCommand("open_external", { url: "http://127.0.0.1:9222" }).catch(console.error);
            }
        },
        getWindowLabel: async () => {
            return await invokeCommand<string>("get_window_label");
        },
        isMainWindow: async () => {
            return await invokeCommand<boolean>("is_main_window");
        },
        listWindows: async () => {
            return await invokeCommand<string[]>("list_windows");
        },
        focusWindow: async (label: string) => {
            await invokeCommand("focus_window", { label });
        },
        getInstanceNumber: async () => {
            return await invokeCommand<number>("get_instance_number");
        },
        getWindowCount: async () => {
            return await invokeCommand<number>("get_window_count");
        },

        // --- Workspace & Tabs ---
        createWorkspace: () => {
            invokeCommand("create_workspace").catch(console.error);
        },
        switchWorkspace: (workspaceId: string) => {
            invokeCommand("switch_workspace", { workspaceId }).catch(console.error);
        },
        deleteWorkspace: (workspaceId: string) => {
            invokeCommand("delete_workspace", { workspaceId }).catch(console.error);
        },
        setActiveTab: (tabId: string) => {
            invokeCommand("set_active_tab", { tabId }).catch(console.error);
        },
        createTab: () => {
            invokeCommand("create_tab").catch(console.error);
        },
        closeTab: (workspaceId: string, tabId: string) => {
            invokeCommand("close_tab", { workspaceId, tabId }).catch(console.error);
        },

        // --- Init ---
        setWindowInitStatus: (status: "ready" | "wave-ready") => {
            invokeCommand("set_window_init_status", { status }).catch(console.error);
        },
        onAgentMuxInit: (callback: (initOpts: AgentMuxInitOpts) => void) => {
            listenEvent<AgentMuxInitOpts>("agentmux-init", (payload) => {
                callback(payload);
            });
        },

        // --- Logging ---
        sendLog: (log: string) => {
            invokeCommand("fe_log", { msg: log }).catch(() => {});
        },
        sendLogStructured: (level: string, module: string, message: string, data: Record<string, any> | null) => {
            invokeCommand("fe_log_structured", { level, module, message, data }).catch(() => {});
        },

        // --- Screenshot ---
        captureScreenshot: async (_rect: { x: number; y: number; width: number; height: number }): Promise<string> => {
            return "";
        },

        // --- Claude Code Auth (legacy stubs) ---
        openClaudeCodeAuth: async () => {
            await invokeCommand("open_claude_code_auth");
        },
        getClaudeCodeAuth: async () => {
            return await invokeCommand<{ connected: boolean; email?: string; expires_at?: number }>(
                "get_claude_code_auth"
            );
        },
        disconnectClaudeCode: async () => {
            await invokeCommand("disconnect_claude_code");
        },

        // --- Provider Commands ---
        detectInstalledClis: async () => {
            return await invokeCommand<CliDetectionResult[]>("detect_installed_clis");
        },
        getProviderConfig: async () => {
            return await invokeCommand<ProviderConfig>("get_provider_config");
        },
        saveProviderConfig: async (config: ProviderConfig) => {
            await invokeCommand("save_provider_config", { config });
        },
        getProviderInstallInfo: async (provider: string) => {
            return await invokeCommand<ProviderInstallInfo>("get_provider_install_info", { provider });
        },
        setProviderAuth: async (provider: string, token: string) => {
            await invokeCommand("set_provider_auth", { provider, token });
        },
        clearProviderAuth: async (provider: string) => {
            await invokeCommand("clear_provider_auth", { provider });
        },
        getProviderAuthStatus: async (provider: string) => {
            return await invokeCommand<ProviderAuthStatus>("get_provider_auth_status", { provider });
        },
        checkCliAuthStatus: async (provider: string, cliPath?: string) => {
            return await invokeCommand<CliAuthStatus>("check_cli_auth_status", { provider, cliPath: cliPath ?? null });
        },
        installCli: async (provider: string) => {
            return await invokeCommand<CliInstallResult>("install_cli", { provider });
        },
        getCliPath: async (provider: string) => {
            return await invokeCommand<string | null>("get_cli_path", { provider });
        },
        checkNodejsAvailable: async () => {
            return await invokeCommand<NodejsStatus>("check_nodejs_available");
        },
        ensureAuthDir: async (providerId: string) => {
            return await invokeCommand<string>("ensure_auth_dir", { providerId });
        },
        runCliLogin: async (cliPath: string, loginArgs: string[], authEnv: Record<string, string>) => {
            return await invokeCommand<string | null>("run_cli_login", { cliPath, loginArgs, authEnv });
        },
        cancelCliLogin: async () => {
            await invokeCommand("cancel_cli_login");
        },

        listen: async (event: string, callback: (event: any) => void) => {
            const unlisten = await listenEvent(event, callback);
            return unlisten;
        },

        // --- Cross-window drag (stubbed — Phase 3) ---
        startCrossDrag: async (
            dragType: "pane" | "tab",
            sourceWindow: string,
            sourceWorkspaceId: string,
            sourceTabId: string,
            payload: { blockId?: string; tabId?: string }
        ) => {
            return await invokeCommand<string>("start_cross_drag", {
                dragType, sourceWindow, sourceWorkspaceId, sourceTabId, payload,
            });
        },
        updateCrossDrag: async (dragId: string, screenX: number, screenY: number) => {
            return await invokeCommand<string | null>("update_cross_drag", { dragId, screenX, screenY });
        },
        completeCrossDrag: async (
            dragId: string,
            targetWindow: string | null,
            screenX: number,
            screenY: number
        ) => {
            await invokeCommand("complete_cross_drag", { dragId, targetWindow, screenX, screenY });
        },
        cancelCrossDrag: async (dragId: string) => {
            await invokeCommand("cancel_cross_drag", { dragId });
        },
        openWindowAtPosition: async (screenX: number, screenY: number, workspaceId?: string) => {
            return await invokeCommand<string>("open_window_at_position", { screenX, screenY, workspaceId: workspaceId ?? "" });
        },

        // --- Drag cursor (stubbed — Phase 3) ---
        setDragCursor: async () => {
            await invokeCommand("set_drag_cursor");
        },
        restoreDragCursor: async () => {
            await invokeCommand("restore_drag_cursor");
        },
        releaseDragCapture: async () => {
            await invokeCommand("release_drag_capture");
        },
    };

    return api;
}

/**
 * Detect whether we're running inside a CEF host.
 * Checks URL query params first (available immediately), then window globals.
 */
export function isCef(): boolean {
    return new URLSearchParams(window.location.search).has("ipc_port")
        || typeof (window as any).__AGENTMUX_IPC_PORT__ !== "undefined";
}
