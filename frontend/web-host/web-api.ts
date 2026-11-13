// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// AppApi for AgentMux in a browser tab: the web host. A browser has none of
// the desktop host's capabilities (HostCaps all off), so the UI hides what
// needs them; what remains maps onto browser APIs. srv is reached on the
// page's own origin, through a proxy that adds srv's key, so the page holds none.
// `satisfies AppApi` at the end of buildWebApi: a member added to AppApi fails
// the type check here until this host answers it.

import { jsContextMenuApi } from "@/app/host/js-context-menu";
import { NO_HOST_CAPS } from "@/util/host-caps";

const notInBrowser = (what: string) => () => Promise.reject(new Error(`${what} isn't available in the browser`));
const resolved =
    <T>(value: T) =>
    () =>
        Promise.resolve(value);
const noop = (): void => {};

/** The OS the browser runs on, as `process.platform` names it. */
function browserPlatform(): NodeJS.Platform {
    const ua = navigator.userAgent;
    if (/Windows/i.test(ua)) return "win32";
    if (/Mac OS X|Macintosh|iPhone|iPad/i.test(ua)) return "darwin";
    return "linux";
}

/** The keys of `T` that are not optional: the ones every host answers. */
type RequiredKeys<T> = { [K in keyof T]-?: object extends Pick<T, K> ? never : K }[keyof T];

/**
 * A host sub-API none of whose capabilities the browser has: each required
 * member refuses its call (the UI doesn't make them, its capability checks
 * hide the UI). The optional members are left out, so the UI's "does this host
 * have it" checks (`typeof api.decideDesktop === "function"`) find none.
 * `required` names exactly `T`'s required members: a member added to `T` fails
 * the type check until it is listed.
 */
function unavailableHostApi<T extends object>(name: string, required: Record<RequiredKeys<T>, true>): T {
    return Object.fromEntries(Object.keys(required).map((key) => [key, notInBrowser(`${name}.${key}`)])) as T;
}

const refused = <K extends string>(...keys: K[]) => Object.fromEntries(keys.map((k) => [k, true])) as Record<K, true>;

let lastPointer = { x: 0, y: 0 };

/** `url`, if it's http or https; anything else (javascript:, file:, …) throws rather than opens. */
function webUrl(url: string): string {
    const u = new URL(url);
    if (u.protocol !== "https:" && u.protocol !== "http:") throw new Error(`not a web address: ${u.protocol}`);
    return u.href;
}

/**
 * A provider login's window, opened during the click that started the login
 * (app/view/agent/flows/login-window.ts): its URL comes seconds later, and a
 * browser blocks a window opened then. It shows a short waiting page until
 * `navigate`, which cuts it off from this page (no opener, no referrer) and
 * accepts only http(s). Null when the browser refused the window: the login
 * then shows its link to click instead.
 */
function reserveExternalWindow(): ExternalWindow | null {
    // Without "noopener": the handle is what lets us send it to the URL later.
    // `navigate` drops the opener before the provider's page loads in it.
    const w = window.open("", "_blank");
    if (!w) return null;
    try {
        const doc = w.document;
        doc.title = "Signing in…";
        const meta = doc.createElement("meta");
        meta.name = "referrer";
        meta.content = "no-referrer";
        doc.head.append(meta);
        doc.body.style.cssText = "margin:48px;font:15px/1.5 system-ui,sans-serif;color:GrayText";
        doc.body.textContent = "Opening the sign-in page…";
    } catch {
        // Only the waiting page; the window itself is what matters.
    }
    return {
        navigate(url) {
            const href = webUrl(url);
            w.opener = null;
            w.location.replace(href);
        },
        close() {
            w.close();
        },
    };
}

/** Desktop-only capabilities: refused or absent. */
function desktopOnlyApi() {
    return {
        getHostCaps: () => NO_HOST_CAPS,
        getAutostartStatus: notInBrowser("Start at login"),
        openSettingsFileInEditor: notInBrowser("Opening the settings file"),
        ensureSettingsFile: notInBrowser("The settings file"),
        browserPanes: unavailableHostApi<BrowserPaneHostApi>(
            "browserPanes",
            refused(
                "create",
                "resize",
                "setRects",
                "close",
                "navigate",
                "showPopup",
                "closePopup",
                "goBack",
                "goForward",
                "reload",
                "stop",
                "cancelDownload",
                "startDownload",
                "focus",
                "cut",
                "copy",
                "paste",
                "print",
                "viewSource",
                "inspectElement",
                "screenshot",
                "authSubmit",
                "authCancel",
                "authSave",
                "setOverlayClip",
                "respondMediaPermission",
                "revokeMedia"
            )
        ),
        windows: unavailableHostApi<WindowHostApi>(
            "windows",
            refused(
                "startDrag",
                "maximize",
                "getPosition",
                "setPosition",
                "getRect",
                "setRect",
                "getCursorScreenPoint",
                "getPaneDebugState",
                "captureViewport",
                "recycleBegin",
                "recycleDeclined",
                "recycleCancel",
                "takeReloadSnapshot",
                "openFloatingPane",
                "isMaximized",
                "toggleFloatingMaximize",
                "setFloatingAlwaysOnTop",
                "getFloatingRedockTarget",
                "updateFloatingRedockHover",
                "clearFloatingRedockHover",
                "setFloatingRedockTarget",
                "poolWindowReady",
                "panePoolWindowReady",
                "resolveWindowAtCursor"
            )
        ),
        approvals: unavailableHostApi<ApprovalHostApi>(
            "approvals",
            refused("decideCredential", "decideSsh", "decideBrowserAttention", "decideWidget")
        ),
    } satisfies Partial<AppApi>;
}

/** Links, the clipboard and dropped files. */
function linkAndClipboardApi() {
    return {
        openExternalChecked: async (url) => {
            window.open(webUrl(url), "_blank", "noopener,noreferrer");
        },
        openExternal: (url) => {
            // Fire-and-forget: a non-web address (a `vscode://` file link, say)
            // means nothing to this browser, so it's logged, not opened.
            try {
                window.open(webUrl(url), "_blank", "noopener,noreferrer");
            } catch (e) {
                console.warn(`[web-api] not opening ${String(e)}`);
            }
        },
        reserveExternalWindow,
        readClipboardText: () => navigator.clipboard.readText(),
        readClipboardAttachments: async () => ({ text: await navigator.clipboard.readText(), paths: [] }),
        writeClipboardText: (text) => navigator.clipboard.writeText(text),
        consumeDroppedFilePaths: resolved([]),
        peekDroppedFilePaths: resolved([]),
        copyFileToDir: notInBrowser("Copying a dropped file"),
        openDataDirInFileManager: notInBrowser("Opening the data folder"),
        getHostInfo: resolved({}),
        setTaskbarAttention: resolved(undefined),
        takeBackgroundAudit: resolved(null),
        reclaimWindowFocus: resolved(undefined),
        inputOwnerState: resolved(null),
    } satisfies Partial<AppApi>;
}

/** Who and where this host is, with no srv key. */
function identityAndEnvApi(platform: NodeJS.Platform) {
    return {
        // The proxy in front of srv adds the key; the page never holds it.
        getAuthKey: () => "",
        // The tab's session, set by srv's sign-in page after the person confirmed it in the terminal.
        // `sessionStorage` is per tab and per origin (port included); a hosted page has none.
        getSessionToken: () => {
            try {
                return window.sessionStorage.getItem("amx-session") ?? undefined;
            } catch {
                return undefined;
            }
        },
        reloadWindow: () => window.location.reload(),
        getIsDev: () => false,
        getCursorPoint: () => lastPointer,
        getPlatform: () => platform,
        getEnv: () => "",
        getAboutModalDetails: () => ({ version: __AGENTMUX_VERSION__, buildTime: 0, platform }),
        getBackendInfo: async () => ({ version: __AGENTMUX_VERSION__ }),
        restartBackend: notInBrowser("Restarting the backend"),
        getHealthSignals: resolved([]),
        getZoomFactor: () => 1,
    } satisfies Partial<AppApi>;
}

/** Host hooks the UI calls that a tab has nothing to do for: menus, zoom, updates. */
function uiHooksApi() {
    return {
        ...jsContextMenuApi(),
        onNavigate: noop,
        onIframeNavigate: noop,
        downloadFile: noop,
        onFullScreenChange: noop,
        onZoomFactorChange: noop,
        setZoomFactor: noop,
        onUpdaterStatusChange: noop,
        getUpdaterStatus: () => "up-to-date",
        getUpdaterVersion: () => null,
        getUpdaterChannel: () => "",
        installAppUpdate: noop,
        updateWindowControlsOverlay: noop,
    } satisfies Partial<AppApi>;
}

/** One window: the tab. */
function windowApi() {
    return {
        openNewWindow: notInBrowser("A new window"),
        openNewWindowWithView: notInBrowser("A new window"),
        openRestoredWindow: notInBrowser("A new window"),
        openWorkspaceInNewWindow: notInBrowser("A new window"),
        closeWindow: async () => window.close(),
        // A tab gets no OS close request to hold back.
        closeRequestSeen: resolved(undefined),
        minimizeWindow: noop,
        maximizeWindow: noop,
        toggleDevtools: noop,
        inspectElementAt: noop,
        setWindowOpacity: resolved(undefined),
        getWindowLabel: resolved("main"),
        isMainWindow: resolved(true),
        registerBackendWindow: noop,
        listWindows: resolved(["main"]),
        listWindowInstances: resolved([{ label: "main", windowId: null }]),
        getDoubleClickTime: resolved(500),
        focusWindow: resolved(undefined),
        getInstanceNumber: resolved(1),
        createWorkspace: noop,
        switchWorkspace: noop,
        deleteWorkspace: noop,
        setActiveTab: noop,
        createTab: noop,
        closeTab: noop,
        setWindowInitStatus: noop,
    } satisfies Partial<AppApi>;
}

/** Logging, native dialogs and a few host actions. */
function logAndDialogApi() {
    return {
        sendLog: (log) => console.debug("[agentmux]", log),
        sendLogStructured: (level, module, message, data) =>
            console.debug(`[agentmux] ${level} ${module}: ${message}`, data ?? ""),
        onQuicklook: noop,
        revealInFileExplorer: resolved(undefined),
        showOpenFileDialog: resolved(null),
        showOpenBundleDialog: resolved(null),
        showExportWorkspaceDialog: resolved(null),
        showImportWorkspaceDialog: resolved(null),
        captureScreenshot: notInBrowser("A screenshot"),
        openAgent: notInBrowser("Opening an agent from the host"),
    } satisfies Partial<AppApi>;
}

/** Provider sign-in and the host event channel. */
function loginApi() {
    return {
        // Provider sign-in runs through srv (docs/specs/SPEC_PROVIDER_LOGIN_THROUGH_SRV_2026_10_09.md);
        // the host-side login commands don't exist here.
        setProviderAuth: notInBrowser("Host sign-in"),
        runCliLogin: notInBrowser("Host sign-in"),
        cancelCliLogin: resolved(undefined),
        getCliLoginStatus: resolved({ active: false, credential_changed: false, generation: 0 }),
        openLoginTerminal: notInBrowser("A login terminal"),

        // No host events.
        listen: async () => noop,
    } satisfies Partial<AppApi>;
}

/** Dragging between windows, and the cursor and keys that follow it. */
function dragApi() {
    return {
        startCrossDrag: notInBrowser("Dragging between windows"),
        updateCrossDrag: resolved(null),
        completeCrossDrag: resolved(undefined),
        cancelCrossDrag: resolved(undefined),
        openWindowAtPosition: notInBrowser("A new window"),
        tearOffPoolPromote: notInBrowser("Tear-off"),
        tearOffSCMoveHandshake: notInBrowser("Tear-off"),
        closeWindowByLabel: resolved(undefined),
        startTabDragTracking: resolved(undefined),
        stopTabDragTracking: resolved(undefined),
        setDragCursor: resolved(undefined),
        restoreDragCursor: resolved(undefined),
        releaseDragCapture: resolved(undefined),
        getMouseButtonState: resolved(false),
        // Refused, which window-system.ts reads as "not native Wayland".
        getWindowSystem: notInBrowser("The window system"),
        setJsDragActive: resolved(undefined),
        // No window picture follows the cursor in a tab, and a browser has no host keys to forward.
        liveFollowBegin: resolved(undefined),
        liveFollowEnd: resolved(undefined),
        liveFollowResize: resolved(undefined),
        liveFollowCapture: resolved({ staged: false }),
        setHostKeys: noop,
    } satisfies Partial<AppApi>;
}

export function buildWebApi(): AppApi {
    window.addEventListener("pointermove", (e) => (lastPointer = { x: e.screenX, y: e.screenY }), { passive: true });
    const platform = browserPlatform();

    return {
        ...desktopOnlyApi(),
        ...linkAndClipboardApi(),
        ...identityAndEnvApi(platform),
        ...uiHooksApi(),
        ...windowApi(),
        ...logAndDialogApi(),
        ...loginApi(),
        ...dragApi(),
    } satisfies AppApi;
}
