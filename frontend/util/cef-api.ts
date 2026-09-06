// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CEF API shim — provides the same window.api (AppApi) interface
// using the platform-agnostic invokeCommand()/listenEvent() from ipc.ts.
//
// This is the CEF equivalent of tauri-api.ts. Must be loaded before
// the React app bootstraps.

import { invokeCommand, listenEvent } from "@/app/platform/ipc";
import {
    assertMenuInPaintableArea,
    computeMenuPosition,
    type MenuPositionResult,
} from "@/app/util/menu-position";
import { createSubmenuHover, type SubmenuHoverController } from "@/app/util/submenu-hover";
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
    userHomeDir: string;
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
    window.__WAVE_SERVER_WS_ENDPOINT__ = backendEndpoints.ws;
    window.__WAVE_SERVER_WEB_ENDPOINT__ = backendEndpoints.web;

    benchMark("invoke-batch-start");
    const [
        authKey,
        isDev,
        platform,
        userName,
        hostName,
        dataDir,
        configDir,
        userHomeDir,
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
        invokeCommand<string>("get_user_home_dir"),
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
        userHomeDir,
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
 * Apply a computed MenuPositionResult to a menu element: fixed left/top plus
 * the size() max-height cap so a menu taller than the free space scrolls
 * internally instead of being placed partly outside. max-width is deliberately
 * NOT applied — an inline max-width would override (and can loosen) the .menu
 * 400px CSS cap, and horizontal fit is already guaranteed by flip+shift for
 * menus at or under that cap. justify-content is forced to flex-start because
 * the .menu rule's flex-end makes overflow unreachable in a scroll container
 * (it is a no-op when the menu fits, so this only matters when capped).
 */
function applyMenuPosition(el: HTMLElement, pos: MenuPositionResult) {
    el.style.position = "fixed";
    if (pos.style.left != null) el.style.left = String(pos.style.left);
    if (pos.style.top != null) el.style.top = String(pos.style.top);
    el.style.maxHeight = `${pos.maxHeight}px`;
    el.style.overflowY = "auto";
    el.style.justifyContent = "flex-start";
}

/**
 * Render a context menu as a positioned HTML overlay.
 * Fires the callback with the clicked item's id, then removes the overlay.
 * Exported for tests only — production callers go through showContextMenu.
 */
export function showJsContextMenu(
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
    menuEl.className = "menu";
    menuEl.setAttribute("data-pane-overlay", "");
    menuEl.style.left = `${position.x}px`;
    menuEl.style.top = `${position.y}px`;
    // Hidden until computeMenuPosition places it — avoids a one-frame flash
    // at the cursor and stops the pane-overlay clip from firing for a
    // not-yet-positioned menu (visibility:hidden de-registers the rect).
    menuEl.style.visibility = "hidden";

    function renderItems(container: HTMLElement, itemList: NativeContextMenuItem[]) {
        // Peers (same-level submenu-bearing rows) close each other instantly
        // on entry — matches the old zero-delay mouseleave's implicit
        // sibling-closing side effect, which createSubmenuHover's open-delay/
        // safe-triangle close otherwise silently drops (reagent P1 on
        // PR #2525; termSettingsMenu.ts's Themes/Font Size/Terminal Zoom/
        // Transparency submenus are the concrete affected case). One list
        // per renderItems call — each call is exactly one menu level, so
        // nested submenus never reach across levels to close an ancestor.
        const peers: SubmenuHoverController[] = [];
        for (const item of itemList) {
            if (item.type === "separator") {
                const sep = document.createElement("div");
                sep.className = "menu-divider";
                container.appendChild(sep);
                continue;
            }
            if (item.visible === false) continue;

            const row = document.createElement("div");
            row.className = "menu-item";
            if (item.enabled === false) {
                row.style.cursor = "default";
                row.style.opacity = "0.4";
                row.style.pointerEvents = "none";
            }

            // Radio/checkbox indicator slot — same FA-icon shape the
            // FlyoutMenu uses. When checked, render fa-check in accent
            // color; when unchecked but in a check-capable group,
            // render a blank-width spacer so labels stay aligned.
            const isCheckableType = item.type === "radio" || item.type === "checkbox";
            if (isCheckableType || item.checked !== undefined) {
                const icon = document.createElement("i");
                icon.className = item.checked
                    ? "fa-solid fa-fw fa-check menu-item-icon menu-item-check"
                    : "fa-solid fa-fw menu-item-icon menu-item-check";
                row.appendChild(icon);
            }

            // Inline color swatch (exact hue) rendered before the label — used
            // by the pane "Pane Color" submenu. This is a real DOM square, not an
            // emoji, so it matches our palette colors precisely.
            if (item.swatchColor) {
                const swatch = document.createElement("span");
                swatch.className = "menu-item-swatch";
                swatch.style.backgroundColor = item.swatchColor;
                row.appendChild(swatch);
            }

            const label = document.createElement("span");
            label.className = "label";
            label.textContent = item.label ?? "";
            row.appendChild(label);

            if (item.submenu && item.submenu.length > 0) {
                // Static CSS fallback (the pre-framework behavior): anchor at
                // the row's right edge, which needs the row as positioned
                // ancestor. Kept so a computeMenuPosition rejection still
                // yields a positioned submenu; the framework placement below
                // overrides it with fixed viewport coords on success.
                row.style.position = "relative";

                const arrow = document.createElement("i");
                arrow.className = "fa-sharp fa-solid fa-chevron-right";
                row.appendChild(arrow);

                const sub = document.createElement("div");
                sub.className = "menu sub-menu";
                sub.setAttribute("data-pane-overlay", "");
                sub.style.display = "none";
                sub.style.left = "100%";
                sub.style.top = "0";
                renderItems(sub, item.submenu);
                row.appendChild(sub);
                // Positioned through the shared framework, like the top-level
                // menu: anchored to the row, preferring right-start — flip()
                // sends it left of the parent near the right edge, shift()
                // pulls it up near the bottom, size() caps it when taller than
                // the free space. The computed coords are position:fixed, so
                // staying nested inside the row (which the hover logic needs)
                // doesn't affect placement. Held visibility:hidden until
                // placed for the same reason as the parent menu: the
                // pane-overlay clip must register the final rect only.
                //
                // Open/close timing goes through the shared hover-intent core
                // (SPEC_SUBMENU_POSITIONING_AND_HOVER_TIMING_2026_08_10) instead
                // of firing instantly on mouseenter/mouseleave: a short open
                // delay avoids flashing a submenu while the cursor is just
                // sweeping across sibling rows, and a safe-triangle close lets
                // the user travel diagonally into the submenu without it
                // vanishing out from under the cursor.
                const hover = createSubmenuHover({
                    onOpen: () => {
                        sub.style.visibility = "hidden";
                        sub.style.display = "";
                        // Deferred one rAF (mirrors flyoutmenu.tsx's SubMenu /
                        // registerSubMenu) so the display:none→"" reflow has
                        // settled before anything gets measured.
                        requestAnimationFrame(() => {
                            if (!sub.isConnected || sub.style.display === "none") return;
                            // The actual bug (root-caused live 2026-08-13,
                            // reproduced deterministically regardless of anchor
                            // position — left:121px, top:-393px every time,
                            // ruling out a mere layout-timing race): `sub`
                            // starts `position:absolute` (inherited from the
                            // `.menu` class) as a deliberate row-relative
                            // fallback for the .catch() below, but
                            // computeMenuPosition computes coordinates for
                            // `strategy:"fixed"`. floating-ui resolves the
                            // floating element's offset parent from its
                            // CURRENT position at measurement time, so calling
                            // it while `sub` is still `position:absolute`
                            // (nested inside `row`, which has
                            // `position:relative`) resolves coordinates
                            // against the wrong containing block entirely.
                            // FlyoutMenu's Solid sibling never hits this
                            // because its placeholder style is already
                            // `position:fixed;left:0px;top:0px` before it ever
                            // calls computeMenuPosition — match that here:
                            // switch to fixed strategy BEFORE measuring, and
                            // restore the absolute row-relative fallback
                            // explicitly if placement itself fails.
                            sub.style.position = "fixed";
                            void computeMenuPosition(
                                {
                                    anchor: row.getBoundingClientRect(),
                                    placement: "right-start",
                                    avoidNativePanes: false,
                                },
                                sub,
                            ).then((pos) => {
                                // Hover may have left (or the whole menu closed)
                                // before the async placement resolved.
                                if (!sub.isConnected || sub.style.display === "none") return;
                                applyMenuPosition(sub, pos);
                                sub.style.visibility = "";
                                assertMenuInPaintableArea(sub, "context-submenu");
                            }).catch(() => {
                                if (!sub.isConnected) return;
                                // Restore the row-relative absolute fallback —
                                // position:fixed with left:100% would otherwise
                                // pin it just off the right edge of the screen.
                                sub.style.position = "absolute";
                                sub.style.left = "100%";
                                sub.style.top = "0";
                                sub.style.visibility = "";
                            });
                        });
                    },
                    onClose: () => {
                        sub.style.display = "none";
                    },
                });
                // `sub` reports a zero rect via getBoundingClientRect() while
                // display:none, which the controller treats as "no geometry
                // yet" — safe to register once, up front.
                hover.setSubmenuEl(sub);
                peers.push(hover);
                row.addEventListener("mouseenter", () => {
                    for (const peer of peers) {
                        if (peer !== hover) peer.close();
                    }
                    hover.onTriggerEnter();
                });
                row.addEventListener("mouseleave", (e) => hover.onTriggerLeave(e as MouseEvent));
                sub.addEventListener("mouseenter", () => hover.onSubmenuEnter());
                sub.addEventListener("mouseleave", (e) => hover.onSubmenuLeave(e as MouseEvent));
            } else if (item.enabled !== false) {
                // A plain (no-submenu) row is still an explicit new selection —
                // entering it closes any open peer submenu immediately too.
                row.addEventListener("mouseenter", () => {
                    for (const peer of peers) peer.close();
                });
                row.addEventListener("click", () => {
                    overlay.remove();
                    if (item.id && onClick) onClick(item.id);
                });
            }

            container.appendChild(row);
        }
    }

    renderItems(menuEl, items);

    overlay.appendChild(menuEl);
    document.body.appendChild(overlay);

    // Position at the cursor via the shared framework — flip/shift/size keep
    // the menu on-screen near window edges. Unlike FlyoutMenu/Popover, a
    // right-click context menu MUST appear where the user clicked, so
    // `avoidNativePanes` is OFF: it is *expected* to land over a browser
    // pane. The `data-pane-overlay` clip reveals it through the native pane;
    // holding it visibility:hidden until placed means the clip rect registers
    // once, at the final position — no flapping, no stale-rect black artifact.
    void computeMenuPosition(
        {
            anchor: { x: position.x, y: position.y },
            placement: "bottom-start",
            avoidNativePanes: false,
        },
        menuEl,
    ).then((pos) => {
        if (!menuEl.isConnected) return;
        applyMenuPosition(menuEl, pos);
        menuEl.style.visibility = "";
        assertMenuInPaintableArea(menuEl, "context-menu");
    }).catch(() => {
        // computeMenuPosition should not throw; if it does, fall back to the
        // raw cursor position rather than leaving the menu invisible.
        if (menuEl.isConnected) menuEl.style.visibility = "";
    });
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
        getUserHomeDir: () => cachedValues!.userHomeDir,
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
            return await invokeCommand<{ pid?: number; started_at?: string; web_endpoint?: string; version: string; pending_migrations?: number }>(
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
        openAgent: (agentId: string) => {
            return invokeCommand("open_agent", { agent_id: agentId });
        },
        openNativePath: (filePath: string) => {
            invokeCommand("open_native_path", { filePath }).catch(console.error);
        },
        revealInFileExplorer: (filePath: string) => {
            invokeCommand("reveal_in_file_explorer", { filePath }).catch(console.error);
        },
        showOpenFileDialog: () => {
            return invokeCommand<string | null>("show_open_file_dialog");
        },
        showOpenBundleDialog: () => {
            return invokeCommand<string | null>("show_open_bundle_dialog");
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
        openNewWindowWithView: async (view: string, meta?: Record<string, unknown>) => {
            return await invokeCommand<string>("open_new_window", {
                initial_view: view,
                initial_meta: meta ? JSON.stringify(meta) : undefined,
            });
        },
        closeWindow: async (label?: string) => {
            // Callers like the close button or `Cmd+W` invoke this without an
            // arg meaning "close the window I'm in." Resolve to the current
            // page's windowLabel so the Rust handler routes to the right CEF
            // window (its server-side default of "main" would close the wrong
            // window from any non-main window).
            const resolved = label ?? new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
            await invokeCommand("close_window", { label: resolved });
        },
        minimizeWindow: () => {
            const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
            invokeCommand("minimize_window", { label }).catch(console.error);
        },
        maximizeWindow: () => {
            const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
            invokeCommand("maximize_window", { label }).catch(console.error);
        },
        setWindowTransparency: (transparent: boolean, blur: boolean, opacity: number) => {
            const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
            invokeCommand("set_window_transparency", { transparent, blur, opacity, label }).catch(console.error);
        },
        setWindowOpacity: async (label: string, opacity: number): Promise<void> => {
            await invokeCommand("set_window_opacity", { label, opacity });
        },
        getWindowOpacity: async (label: string): Promise<number> => {
            const result = await invokeCommand("get_window_opacity", { label });
            return typeof result === "number" ? result : 1.0;
        },
        toggleDevtools: () => {
            const params = new URLSearchParams(window.location.search);
            const label = params.get("windowLabel") ?? "main";
            invokeCommand("toggle_devtools", { label }).catch(console.error);
        },
        inspectElementAt: (x: number, y: number) => {
            const params = new URLSearchParams(window.location.search);
            const label = params.get("windowLabel") ?? "main";
            // CEF's show_dev_tools(..., inspect_element_at) expects DIP
            // (device-independent pixels in view coords). `MouseEvent.clientX/Y`
            // is in CSS pixels — these match DIP when no zoom applies in the
            // event-target's ancestor chain. AgentMux today only zooms
            // `.window-header` and `.status-bar` (via the per-pane chrome
            // zoom in `zoom.ts`), so pane content events ARE in
            // view DIP. Defensive against future page-zoom changes:
            // divide by any inherited CSS `zoom` on documentElement.
            const rootZoomStr = getComputedStyle(document.documentElement).getPropertyValue("zoom").trim();
            const zoom = rootZoomStr ? parseFloat(rootZoomStr) || 1 : 1;
            invokeCommand("inspect_element_at", {
                label,
                x: Math.round(x / zoom),
                y: Math.round(y / zoom),
            }).catch((err) => {
                // No fallback. The only candidate (toggle_devtools) is
                // stateful — it would CLOSE DevTools when already open,
                // the opposite of what the user asked for by clicking
                // "Inspect Element". Codex flagged this twice on #1043,
                // and the right answer is to surface the failure rather
                // than paper over it with the wrong action. If the host
                // is mismatched (frontend has the new IPC, host doesn't
                // yet — e.g. mid-dev-rebuild), the user can fall back
                // to the hamburger menu's "Dev Tools" toggle manually.
                const errStr = typeof err === "string" ? err : (err?.message ?? String(err));
                if (errStr.startsWith("Unknown command")) {
                    console.warn(
                        "[cef-api] inspect_element_at not supported by current host. " +
                        "Rebuild the host binary (task build:backend) to enable Inspect Element; " +
                        "until then use the hamburger menu's Dev Tools entry."
                    );
                } else {
                    console.error("[cef-api] inspect_element_at failed:", err);
                }
            });
        },
        getWindowLabel: async () => {
            const params = new URLSearchParams(window.location.search);
            return params.get("windowLabel") ?? "main";
        },
        registerBackendWindow: (label: string, windowId: string) => {
            console.log(`[cef-api] registerBackendWindow: label=${label} windowId=${windowId}`);
            invokeCommand("register_backend_window", { label, window_id: windowId }).catch((e: unknown) => {
                console.error(`[cef-api] registerBackendWindow IPC failed: ${e}`);
            });
        },
        isMainWindow: async () => {
            const params = new URLSearchParams(window.location.search);
            return !params.has("windowLabel");
        },
        listWindows: async () => {
            return await invokeCommand<string[]>("list_windows");
        },
        listWindowInstances: async () => {
            return await invokeCommand<Array<{ label: string; windowId: string | null }>>(
                "list_window_instances",
            );
        },
        getDoubleClickTime: async () => {
            return await invokeCommand<number>("get_double_click_time");
        },
        focusWindow: async (label: string) => {
            await invokeCommand("focus_window", { label });
        },
        getInstanceNumber: async () => {
            const params = new URLSearchParams(window.location.search);
            const label = params.get("windowLabel") ?? "main";
            return await invokeCommand<number>("get_instance_number", { label });
        },

        setJsDragActive: async (active: boolean) => {
            await invokeCommand("set_js_drag_active", { active });
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
            const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "main";
            invokeCommand("set_window_init_status", { status, label }).catch(console.error);
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
        runCliLogin: async (
            cliPath: string,
            loginArgs: string[],
            authEnv: Record<string, string>,
            requiresTty?: boolean,
            authConfigDirEnvVar?: string,
        ) => {
            const result = await invokeCommand<{ auth_url: string | null } | string | null>("run_cli_login", {
                cliPath,
                loginArgs,
                authEnv,
                requiresTty: requiresTty ?? false,
                authConfigDirEnvVar,
            });
            // Backend now returns { auth_url } — extract for callers expecting just a URL
            if (result && typeof result === "object" && "auth_url" in result) {
                return result.auth_url;
            }
            return result as string | null;
        },
        cancelCliLogin: async () => {
            await invokeCommand("cancel_cli_login");
        },
        // Whether a runCliLogin child is still alive host-side. The child-exit
        // half of the in-app login session's completion check
        // (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.1) — see
        // cli_login.rs's get_cli_login_status doc for why credential probing
        // alone isn't enough (present-but-expired tokens false-positive it).
        getCliLoginStatus: async () => {
            return await invokeCommand<{ active: boolean; credential_changed: boolean; generation: number }>(
                "get_cli_login_status",
            );
        },
        openLoginTerminal: async (cliPath: string, loginArgs: string[], authEnv: Record<string, string>) => {
            return await invokeCommand<{ opened: boolean }>("open_login_terminal", { cliPath, loginArgs, authEnv });
        },

        listen: async (event: string, callback: (event: any) => void) => {
            const unlisten = await listenEvent(event, callback);
            return unlisten;
        },

        // --- Maintenance panel ---
        runMigrations: async () => {
            return await invokeCommand<{ started: boolean }>("run_migrations");
        },
        runSagaVacuum: async () => {
            return await invokeCommand<{ rows_deleted: number }>("run_saga_vacuum");
        },

        // --- Cross-window drag ---
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
        openWindowAtPosition: async (
            screenX: number,
            screenY: number,
            workspaceId?: string,
            width?: number,
            height?: number,
            tabAnchorX?: number,
            tabAnchorY?: number,
        ) => {
            return await invokeCommand<string>("open_window_at_position", {
                screenX,
                screenY,
                workspaceId: workspaceId ?? "",
                width,
                height,
                tabAnchorX,
                tabAnchorY,
            });
        },
        tearOffPoolPromote: async (
            workspaceId: string,
            screenX: number,
            screenY: number,
            width?: number,
            height?: number,
            tabAnchorX?: number,
            tabAnchorY?: number,
        ) => {
            return await invokeCommand<string>("tear_off_pool_promote", {
                workspaceId,
                screenX,
                screenY,
                width,
                height,
                tabAnchorX,
                tabAnchorY,
            });
        },
        tearOffSCMoveHandshake: async (args) => {
            return await invokeCommand<{ handshakeMs: number; totalMs: number }>(
                "tear_off_sc_move_handshake",
                args,
            );
        },
        closeWindowByLabel: async (label: string) => {
            await invokeCommand("close_window_by_label", { label });
        },
        startTabDragTracking: async (args) => {
            await invokeCommand("start_tab_drag_tracking", args);
        },
        stopTabDragTracking: async () => {
            await invokeCommand("stop_tab_drag_tracking");
        },

        // --- Drag cursor & helpers ---
        setDragCursor: async () => {
            await invokeCommand("set_drag_cursor");
        },
        restoreDragCursor: async () => {
            await invokeCommand("restore_drag_cursor");
        },
        releaseDragCapture: async () => {
            await invokeCommand("release_drag_capture");
        },
        getMouseButtonState: async () => {
            return await invokeCommand<boolean>("get_mouse_button_state");
        },
    };

    return api;
}

/**
 * Detect whether we're running inside a CEF host.
 *
 * Checks, in order: the `ipc_port` URL param (present only on first load),
 * the injected window global, then a sticky `sessionStorage` flag.
 *
 * The sessionStorage flag closes the #52 reload lock-out: `setupCefApi`
 * strips `ipc_port`/`ipc_token` from the URL after first read (token-leak
 * fix), so after ANY reload (Vite HMR, WebGL context-loss reload, bridge
 * auto-recover) neither the URL param nor the not-yet-re-injected global is
 * present — plain `isCef()` would return false and `setupCefApi` would bail
 * BEFORE `waitForIpcCreds`, so the host's `on_load_end` re-injection is
 * never awaited and `window.api` never rebuilds. Remembering "we are CEF"
 * in sessionStorage (set the moment we first see `ipc_port`) survives the
 * strip and any `history.replaceState` (e.g. pool promote), and is
 * per-browsing-context so it never leaks between windows or to a plain
 * browser tab (which never sees `ipc_port`).
 */
export function isCef(): boolean {
    if (new URLSearchParams(window.location.search).has("ipc_port")) {
        try { sessionStorage.setItem("amux:isCef", "1"); } catch { /* storage disabled */ }
        return true;
    }
    if (typeof window.__AGENTMUX_IPC_PORT__ !== "undefined") {
        return true;
    }
    try { return sessionStorage.getItem("amux:isCef") === "1"; } catch { return false; }
}
