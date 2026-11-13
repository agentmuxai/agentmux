// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * initHostMux (app-init.ts): a main window finds (or makes) its window,
 * workspace and tab in srv, hands them to initMux, then starts the notices
 * that must not hold startup up. It is not exported, so these tests drive it
 * through initApp() with every module the file imports replaced by a stub;
 * the stubs record what initHostMux asked for, in which order, and how long
 * each step took (fake clock).
 */

import { describeError, formatDescribedError } from "@/app/errors/error-report";
import { DISPLAY_NAME_MAX_LEN, DISPLAY_NAME_META_KEY } from "@/util/window-title";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const all: Array<ReturnType<typeof vi.fn>> = [];
    const fn = () => {
        const f = vi.fn();
        all.push(f);
        return f;
    };
    class StartupFailureHandled extends Error {}
    return {
        all,
        order: [] as string[],
        StartupFailureHandled,
        // services
        GetClientData: fn(),
        CreateWindow: fn(),
        GetWindow: fn(),
        CloseWindow: fn(),
        GetWorkspace: fn(),
        UpdateObjectMeta: fn(),
        // the host API
        api: {
            sendLog: fn(),
            getPlatform: fn(),
            isMainWindow: fn(),
            getWindowLabel: fn(),
            getInstanceNumber: fn(),
            listWindowInstances: fn(),
            listWindows: fn(),
            setWindowInitStatus: fn(),
            registerBackendWindow: fn(),
        },
        // what initHostMux and initMux call
        failStartup: fn(),
        showStartupError: fn(),
        reopenRestoredWindows: fn(),
        surfaceWorkspaceRestoreNotice: fn(),
        surfaceBackgroundAudit: fn(),
        surfaceIncidents: fn(),
        offerLayoutFromOtherBuild: fn(),
        offerForkSlugUpdate: fn(),
        benchDump: fn(),
        initGlobal: fn(),
        initGlobalEventSubs: fn(),
        initUiEventSubs: fn(),
        loadConnStatus: fn(),
        setWindowInstanceNumAtom: fn(),
        seedKnownEntriesFromSnapshot: fn(),
        makeORef: fn(),
        loadAndPinMuxObject: fn(),
        reloadMuxObject: fn(),
        isPoolMode: fn(),
        isPanePoolMode: fn(),
        stub: (...names: string[]): Record<string, ReturnType<typeof vi.fn>> =>
            Object.fromEntries(
                names.map((n) => {
                    const f = vi.fn();
                    all.push(f);
                    return [n, f];
                })
            ),
    };
});

vi.mock("@/app/app", () => ({ App: () => null }));
vi.mock("@/app/store/browser-pause-all", () => h.stub("startBrowserPauseAll"));
vi.mock("@/app/store/command-registry", () => h.stub("registerDefaultCommands"));
vi.mock("@/app/store/input-owner", () => h.stub("registerInputOwnerTracking"));
vi.mock("@/app/store/keymodel", () =>
    h.stub(
        "globalRefocus",
        "installShortcutApi",
        "registerChordCapture",
        "registerControlShiftTracking",
        "registerGlobalKeys",
        "registerHostShortcuts",
        "registerUserKeybindings"
    )
);
vi.mock("@/app/store/modalmodel", () => ({ modalsModel: {} }));
vi.mock("@/app/element/link-open", () => h.stub("registerLinkInPaneListener"));
vi.mock("@/app/element/ui/text-drag", () => h.stub("installTextDragGuard"));
vi.mock("@/app/store/float-modifier", () => h.stub("installFloatModifier"));
vi.mock("@/app/store/ui-event-subs", () => ({ initUiEventSubs: h.initUiEventSubs }));
vi.mock("@/app/modals/modal-layer-panels", () => ({}));
vi.mock("@/layout/lib/layout-model-factory", () => ({}));
vi.mock("@/app/store/view-actions", () => h.stub("registerViewActions"));
vi.mock("@/app/store/services", () => ({
    ClientService: { GetClientData: h.GetClientData },
    ObjectService: { UpdateObjectMeta: h.UpdateObjectMeta },
    WindowService: { CreateWindow: h.CreateWindow, GetWindow: h.GetWindow, CloseWindow: h.CloseWindow },
    WorkspaceService: { GetWorkspace: h.GetWorkspace },
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { GetFullConfigCommand: async () => ({}), ProvidersModelsCommand: async () => ({ models: [] }) },
}));
vi.mock("@/app/store/rpc-util", () => ({ initWshrpc: () => ({}), TabRpcClient: {} }));
vi.mock("@/layout/index", () =>
    h.stub("getLayoutModelForStaticTab", "installLayoutModelEviction", "installWindowEdgeResizeListener")
);
vi.mock("@/app/store/global", () => ({
    atoms: { activeTabId: () => "", workspace: () => undefined },
    countersClear: vi.fn(),
    countersPrint: vi.fn(),
    getApi: () => h.api,
    getSettingsKeyAtom: () => () => undefined,
    initGlobal: h.initGlobal,
    initGlobalEventSubs: h.initGlobalEventSubs,
    loadConnStatus: h.loadConnStatus,
    openWindowEntriesAtom: () => [],
    pushFlashError: vi.fn(),
    pushNotification: vi.fn(),
    removeNotificationById: vi.fn(),
    subscribeToConnEvents: vi.fn(),
    setWindowInstanceNumAtom: h.setWindowInstanceNumAtom,
    setReinitVersion: vi.fn(),
    syncUpdaterStatus: vi.fn(),
    setFullConfigAtom: vi.fn(),
}));
vi.mock("@/app/i18n/i18n", () => ({ APP_LOCALE_KEY: "app:locale", bindAppLocaleSetting: vi.fn() }));
vi.mock("@/app/store/dev-mode", () => ({ devModeOn: () => false }));
vi.mock("@/app/store/mos", () => ({
    makeORef: h.makeORef,
    loadAndPinMuxObject: h.loadAndPinMuxObject,
    reloadMuxObject: h.reloadMuxObject,
    getObjectValue: () => undefined,
    mpsSubscribeToObject: vi.fn(),
}));
vi.mock("solid-js/web", () => ({ render: vi.fn() }));
vi.mock("@/util/fontutil", () => h.stub("loadFonts"));
vi.mock("@/app/view/identity/identity-model", () => h.stub("primeAccountCache"));
vi.mock("@/util/keyutil", () => h.stub("setKeyUtilPlatform"));
vi.mock("@/util/startup-bench", () => ({ benchMark: vi.fn(), benchDump: h.benchDump }));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { init: vi.fn() } }));
vi.mock("@/app/init/reload-window-id", () =>
    h.stub("forgetWindowId", "previousWindowId", "reattachableWindow", "rememberWindowId")
);
vi.mock("@/app/init/host-detect", () => ({ isHostApp: () => true }));
vi.mock("@/app/init/error-display", () => ({
    failStartup: h.failStartup,
    showStartupError: h.showStartupError,
    StartupFailureHandled: h.StartupFailureHandled,
}));
vi.mock("@/app/view/agent/providers", () => h.stub("setProviderModels"));
vi.mock("@/app/view/agent/providers/codex-models-refresh", () => h.stub("refreshCodexModels"));
vi.mock("@/store/tab-reveal", () => h.stub("scheduleRevealLift"));
vi.mock("@/util/launcher-events", () => h.stub("installLauncherEventBridge"));
vi.mock("@/util/srv-events", () => h.stub("installSrvEventBridge"));
vi.mock("@/app/store/ambient-spend", () => h.stub("installAmbientSpendListener"));
vi.mock("@/app/workspace/redock-ghost", () => h.stub("installFloatingRedockHoverListener"));
vi.mock("@/app/store/launcher-event-store", () => ({
    seedKnownEntriesFromSnapshot: h.seedKnownEntriesFromSnapshot,
    startLauncherEventReducer: vi.fn(),
}));
vi.mock("@/app/store/singleton-modal", () => h.stub("startSingletonCrashRelease"));
vi.mock("@/app/drag/file-drop", () => h.stub("installFileDropController"));
vi.mock("@/app/drag/window-drag-events", () => h.stub("installWindowDragEvents"));
vi.mock("@/app/store/focusManager", () => h.stub("installFocusFollowsSelection"));
vi.mock("@/app/store/pane-history", () => h.stub("installPaneHistory"));
vi.mock("@/app/store/side-buttons", () => h.stub("installSideButtons"));
vi.mock("@/app/init/restored-windows", () => ({ reopenRestoredWindows: h.reopenRestoredWindows }));
vi.mock("@/app/init/last-window", () => h.stub("restoresLastWindow"));
vi.mock("@/app/store/user-workspace", () => ({
    installWorkspaceFlushOnBlur: vi.fn(),
    surfaceWorkspaceRestoreNotice: h.surfaceWorkspaceRestoreNotice,
}));
// Loaded with import() while initApp and initMux run.
vi.mock("@/app/init/pool", () => ({
    isPoolMode: h.isPoolMode,
    isPanePoolMode: h.isPanePoolMode,
    awaitPoolPromote: vi.fn(),
    awaitPanePoolPromote: vi.fn(),
    markPoolWaiting: vi.fn(),
}));
vi.mock("@/app/init/startup-splash", () => h.stub("onStartupReveal", "fadeOutStartupSplash"));
vi.mock("@/app/store/zoom", () => h.stub("initChromeZoom"));
vi.mock("@/app/store/recycle", () => h.stub("showReloadCover", "installRecycleRequests"));
vi.mock("@/app/window/os-close", () => h.stub("installOsCloseRequests"));
vi.mock("@/app/block/widget-loader", () => h.stub("startWidgetLoader"));
vi.mock("@/app/init/background-audit", () => ({ surfaceBackgroundAudit: h.surfaceBackgroundAudit }));
vi.mock("@/app/init/incidents", () => ({ surfaceIncidents: h.surfaceIncidents }));
vi.mock("@/app/window/restore-other-build", () => ({ offerLayoutFromOtherBuild: h.offerLayoutFromOtherBuild }));
vi.mock("@/app/view/settings/fork-slug-notice", () => ({ offerForkSlugUpdate: h.offerForkSlugUpdate }));
vi.mock("@/app/block/widget-install-requests", () => h.stub("startWidgetInstallRequests"));
vi.mock("@/app/block/upgrade-requests", () => h.stub("startUpgradeRequests"));
vi.mock("@/app/block/desktop-requests", () => h.stub("startDesktopRequests"));
vi.mock("@/app/block/agent-notices", () => h.stub("startAgentNotices"));
vi.mock("@/app/block/pane-confirms", () => h.stub("startPaneConfirms"));
vi.mock("@/app/view/agent/components/vault-request-card/vault-request-cards", () => h.stub("startVaultRequestCards"));
vi.mock("@/app/view/agent/endpoint-notice-cards", () => h.stub("startEndpointNoticeCards"));
vi.mock("@/app/notification/waiting-for-you", () => h.stub("installWaitingForYou"));
vi.mock("@/app/platform/pane-overlay-auto", () => h.stub("startPaneOverlayAutoService"));
vi.mock("@/app/notification/sound", () => h.stub("installSoundService"));
vi.mock("@/app/notification/os/os-notify-bridge", () => h.stub("installOsNotifyBridge"));
vi.mock("@/app/notification/turn-awareness-service", () => h.stub("installTurnAwarenessService"));
vi.mock("@/app/util/reveal-block-events", () => h.stub("installBlockRevealEvents"));
vi.mock("@/app/util/agent-opened-notice", () => h.stub("installAgentOpenedNotice"));

const realTimeout = globalThis.setTimeout;
const never = () => new Promise<never>(() => {});

interface Outcome {
    done: boolean;
    error?: unknown;
}

let logs: string[][] = [];
let errors: unknown[][] = [];
let warns: unknown[][] = [];

let runs = 0;

/** Everything the stubs and the console have seen so far; it stops growing when startup is waiting on the fake clock or is over. */
const activity = () =>
    h.all.reduce((n, f) => n + f.mock.calls.length, 0) + h.order.length + logs.length + errors.length + warns.length;

/**
 * Let everything that can run without the fake clock run: module imports take real
 * time, and the notices started after initApp() resolves are still going then.
 */
async function settle() {
    let last = -1;
    let quiet = 0;
    while (quiet < 15) {
        await new Promise<void>((r) => realTimeout(r, 2));
        const now = activity();
        if (now === last) quiet++;
        else {
            quiet = 0;
            last = now;
        }
    }
}

/** Wait, in real time, until a condition holds (a test whose condition never holds fails on what it asserts next). */
async function until(condition: () => boolean) {
    for (let i = 0; i < 5000 && !condition(); i++) await new Promise<void>((r) => realTimeout(r, 2));
}

/** Move the fake clock, then wait for what that should make happen. */
async function advance(ms: number, then: () => boolean = () => true) {
    await vi.advanceTimersByTimeAsync(ms);
    await until(then);
    await settle();
}

/** initHostMux got as far as starting its notices; they load their modules, so they take a moment. */
const noticesStarted = () =>
    h.surfaceBackgroundAudit.mock.calls.length > 0 &&
    h.surfaceIncidents.mock.calls.length > 0 &&
    h.offerLayoutFromOtherBuild.mock.calls.length > 0 &&
    h.offerForkSlugUpdate.mock.calls.length > 0 &&
    h.GetClientData.mock.calls.length >= 2;

/**
 * Boot the module fresh and run initApp() for a main window until `blocked` holds (the call it
 * is meant to wait on has been made) or, without one, until it finishes.
 */
async function run(blocked?: () => boolean): Promise<Outcome> {
    // The page has been up a while: initHostMux's own start is not the clock's zero.
    vi.advanceTimersByTime(777);
    // A fresh copy of app-init (its once-per-page state starts empty); what it imports stays loaded.
    const spec = `./app-init?run=${++runs}`;
    const { initApp } = (await import(/* @vite-ignore */ spec)) as typeof import("./app-init");
    const out: Outcome = { done: false };
    void initApp().then(
        () => {
            out.done = true;
        },
        (e) => {
            out.done = true;
            out.error = e;
        }
    );
    await until(blocked ?? (() => out.done));
    if (!blocked && h.benchDump.mock.calls.length > 0) await until(noticesStarted);
    await settle();
    return out;
}

const client = (over: Record<string, unknown> = {}) => ({ oid: "client-1", windowids: ["win-1"], ...over });
const win = (over: Record<string, unknown> = {}) => ({ oid: "win-1", workspaceid: "ws-1", ...over });
const workspace = (over: Record<string, unknown> = {}) => ({
    oid: "ws-1",
    activetabid: "tab-1",
    tabids: ["tab-1", "tab-2"],
    pinnedtabids: ["pin-1"],
    ...over,
});

/** The timing lines initHostMux prints (initMux's own start with "initMux "). */
const perfLines = () =>
    logs
        .map((l) => String(l[0]))
        .filter((s) => s.startsWith("[startup-perf] ") && !/^\[startup-perf\] (initMux|initApp) /.test(s));

const seen = (...names: string[]) => h.order.filter((o) => names.includes(o));

const sentLogs = () => h.api.sendLog.mock.calls.map((c) => String(c[0]));

/** The options initMux was started with. */
const initOpts = () => h.initGlobalEventSubs.mock.calls[0]?.[0];

beforeEach(() => {
    for (const f of h.all) f.mockReset();
    h.order.length = 0;
    logs = [];
    errors = [];
    warns = [];
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval", "Date", "performance"] });
    vi.spyOn(console, "log").mockImplementation((...a: unknown[]) => void logs.push(a.map(String)));
    vi.spyOn(console, "error").mockImplementation((...a: unknown[]) => void errors.push(a));
    vi.spyOn(console, "warn").mockImplementation((...a: unknown[]) => void warns.push(a));
    vi.spyOn(console, "debug").mockImplementation(() => {});
    Object.defineProperty(window, "api", { value: h.api, configurable: true, writable: true });
    Object.defineProperty(document, "fonts", { value: { ready: Promise.resolve() }, configurable: true });

    h.GetClientData.mockImplementation(async () => {
        h.order.push("GetClientData");
        return client();
    });
    h.GetWindow.mockImplementation(async () => {
        h.order.push("GetWindow");
        return win();
    });
    h.GetWorkspace.mockImplementation(async () => {
        h.order.push("GetWorkspace");
        return workspace();
    });
    h.CreateWindow.mockImplementation(async () => {
        h.order.push("CreateWindow");
        return win({ oid: "win-new", workspaceid: "ws-new" });
    });
    h.CloseWindow.mockImplementation(async () => {
        h.order.push("CloseWindow");
    });
    h.UpdateObjectMeta.mockImplementation(async () => {
        h.order.push("UpdateObjectMeta");
    });
    h.reopenRestoredWindows.mockImplementation(async () => {
        h.order.push("reopenRestoredWindows");
    });
    h.initGlobal.mockImplementation(() => {
        h.order.push("initGlobal");
    });
    h.surfaceBackgroundAudit.mockImplementation(async () => {
        h.order.push("surfaceBackgroundAudit");
    });
    h.surfaceIncidents.mockImplementation(() => {
        h.order.push("surfaceIncidents");
    });
    h.surfaceWorkspaceRestoreNotice.mockImplementation(() => {
        h.order.push("surfaceWorkspaceRestoreNotice");
    });
    h.offerLayoutFromOtherBuild.mockImplementation(() => {
        h.order.push("offerLayoutFromOtherBuild");
    });
    h.offerForkSlugUpdate.mockImplementation(() => {
        h.order.push("offerForkSlugUpdate");
    });
    h.benchDump.mockImplementation(() => {
        h.order.push("benchDump");
    });
    h.seedKnownEntriesFromSnapshot.mockImplementation(() => {
        h.order.push("seedKnownEntriesFromSnapshot");
    });
    h.api.setWindowInitStatus.mockImplementation((s: string) => {
        h.order.push(`setWindowInitStatus:${s}`);
    });
    h.api.getInstanceNumber.mockImplementation(async () => {
        h.order.push("getInstanceNumber");
        return 3;
    });
    h.api.getPlatform.mockReturnValue("darwin");
    h.api.isMainWindow.mockResolvedValue(true);
    h.api.getWindowLabel.mockResolvedValue("main");
    h.api.listWindowInstances.mockResolvedValue([{ label: "main", windowId: "win-1" }]);
    h.api.listWindows.mockResolvedValue(["main"]);
    h.failStartup.mockImplementation((msg: string) => {
        throw new h.StartupFailureHandled(msg);
    });
    h.makeORef.mockImplementation((otype: string, oid: string) => `${otype}:${oid}`);
    h.loadAndPinMuxObject.mockImplementation(async (oref: string) => ({
        oid: oref,
        workspaceid: "ws-1",
        layoutstate: "layout-1",
        tabids: [],
        pinnedtabids: [],
    }));
    h.reloadMuxObject.mockImplementation(async (oref: string) => ({ oid: oref }));
    h.isPoolMode.mockReturnValue(false);
    h.isPanePoolMode.mockReturnValue(false);
});

afterEach(async () => {
    await settle();
    vi.useRealTimers();
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
    window.history.replaceState({}, "", "/");
});

describe("initHostMux — the happy path", () => {
    it("reads the client, window and workspace, then starts initMux with their ids", async () => {
        const out = await run();
        expect(out).toEqual({ done: true });
        expect(h.GetClientData).toHaveBeenCalledTimes(2); // the second is the restore-notice read
        expect(h.GetWindow).toHaveBeenCalledWith("win-1");
        expect(h.GetWorkspace).toHaveBeenCalledWith("ws-1");
        expect(h.CreateWindow).not.toHaveBeenCalled();
        expect(h.CloseWindow).not.toHaveBeenCalled();
        expect(initOpts()).toEqual({
            clientId: "client-1",
            windowId: "win-1",
            tabId: "tab-1",
            activate: true,
            primaryTabStartup: true,
        });
        expect(h.initGlobal).toHaveBeenCalledTimes(1);
        expect(h.initGlobal.mock.calls[0][0]).toMatchObject({
            tabId: "tab-1",
            clientId: "client-1",
            windowId: "win-1",
            primaryTabStartup: true,
        });
    });

    it("uses the first of the client's windows, not another", async () => {
        h.GetClientData.mockResolvedValueOnce(client({ windowids: ["win-a", "win-b"] }));
        h.GetWindow.mockResolvedValue(win({ oid: "win-a" }));

        await run();

        expect(h.GetWindow).toHaveBeenCalledTimes(1);
        expect(h.GetWindow).toHaveBeenCalledWith("win-a");
        expect(initOpts().windowId).toBe("win-a");
    });

    it("does not reopen the last session's windows when this one already has a window", async () => {
        await run();

        expect(h.reopenRestoredWindows).not.toHaveBeenCalled();
    });

    it("takes the client id from the client and the window id from the window it read", async () => {
        h.GetClientData.mockResolvedValueOnce(client({ oid: "client-9" }));
        h.GetWindow.mockResolvedValue(win({ oid: "win-9", workspaceid: "ws-9" }));

        await run();

        expect(initOpts().clientId).toBe("client-9");
        expect(initOpts().windowId).toBe("win-9");
        expect(h.GetWorkspace).toHaveBeenCalledWith("ws-9");
    });

    it("runs the discovery reads in order, then initMux, then the notices", async () => {
        await run();

        const names = ["GetClientData", "GetWindow", "GetWorkspace", "initGlobal"];
        expect(h.order.filter((o) => names.includes(o)).slice(0, 4)).toEqual(names);
        expect(h.order.indexOf("initGlobal")).toBeLessThan(h.order.indexOf("getInstanceNumber"));
        expect(h.order.indexOf("getInstanceNumber")).toBeLessThan(h.order.indexOf("benchDump"));
    });
});

describe("initHostMux — no window yet (cold start)", () => {
    beforeEach(() => {
        h.GetClientData.mockResolvedValueOnce(client({ windowids: [] }));
    });

    it("creates the window, asking srv to restore the last session, labelled main", async () => {
        await run();

        expect(h.CreateWindow).toHaveBeenCalledTimes(1);
        expect(h.CreateWindow).toHaveBeenCalledWith(null, "", "main", true);
    });

    it("labels the window from the windowLabel in the URL", async () => {
        window.history.replaceState({}, "", "/?windowLabel=win-2");

        await run();

        expect(h.CreateWindow).toHaveBeenCalledWith(null, "", "win-2", true);
    });

    it("passes an empty windowLabel on as it is, rather than falling back to main", async () => {
        window.history.replaceState({}, "", "/?windowLabel=");

        await run();

        expect(h.CreateWindow).toHaveBeenCalledWith(null, "", "", true);
    });

    it("also creates the window when the client has no list of windows at all", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockResolvedValueOnce({ oid: "client-1" });

        await run();

        expect(h.CreateWindow).toHaveBeenCalledWith(null, "", "main", true);
    });

    it("reads back the window it made and carries on with it", async () => {
        h.GetWindow.mockResolvedValue(win({ oid: "win-new", workspaceid: "ws-new" }));

        await run();

        expect(h.GetWindow).toHaveBeenCalledWith("win-new");
        expect(initOpts().windowId).toBe("win-new");
    });

    it("starts reopening the last session's other windows once this one exists, without waiting for them", async () => {
        h.reopenRestoredWindows.mockImplementation(() => {
            h.order.push("reopenRestoredWindows");
            return never();
        });

        const out = await run();

        expect(out.done).toBe(true);
        expect(h.reopenRestoredWindows).toHaveBeenCalledTimes(1);
        expect(h.reopenRestoredWindows).toHaveBeenCalledWith();
        expect(h.order.indexOf("CreateWindow")).toBeLessThan(h.order.indexOf("reopenRestoredWindows"));
        expect(h.order.indexOf("reopenRestoredWindows")).toBeLessThan(h.order.indexOf("GetWindow"));
    });
});

describe("initHostMux — the window is gone", () => {
    beforeEach(() => {
        h.GetWindow.mockImplementationOnce(async () => {
            h.order.push("GetWindow");
            return null;
        });
    });

    it("makes a new window (without asking for a restore) and uses its workspace", async () => {
        await run();

        expect(h.CreateWindow).toHaveBeenCalledTimes(1);
        expect(h.CreateWindow).toHaveBeenCalledWith(null, "", "main");
        expect(h.GetWorkspace).toHaveBeenCalledWith("ws-new");
        expect(initOpts().windowId).toBe("win-new");
        expect(h.reopenRestoredWindows).not.toHaveBeenCalled();
    });

    it("labels the replacement from the URL too", async () => {
        window.history.replaceState({}, "", "/?windowLabel=win-3");

        await run();

        expect(h.CreateWindow).toHaveBeenCalledWith(null, "", "win-3");
    });

    it("reads the window before it creates the replacement", async () => {
        await run();

        expect(seen("GetClientData", "GetWindow", "CreateWindow", "GetWorkspace").slice(0, 4)).toEqual([
            "GetClientData",
            "GetWindow",
            "CreateWindow",
            "GetWorkspace",
        ]);
    });
});

describe("initHostMux — the workspace is gone", () => {
    beforeEach(() => {
        h.GetWorkspace.mockImplementationOnce(async () => {
            h.order.push("GetWorkspace");
            return null;
        });
        h.GetWorkspace.mockImplementationOnce(async () => {
            h.order.push("GetWorkspace");
            return workspace({ oid: "ws-new", activetabid: "tab-new" });
        });
    });

    it("closes the window, makes another and reads its workspace", async () => {
        await run();

        expect(h.CloseWindow).toHaveBeenCalledTimes(1);
        expect(h.CloseWindow).toHaveBeenCalledWith("win-1");
        expect(h.CreateWindow).toHaveBeenCalledTimes(1);
        expect(h.CreateWindow).toHaveBeenCalledWith(null, "", "main");
        expect(h.GetWorkspace).toHaveBeenCalledTimes(2);
        expect(h.GetWorkspace).toHaveBeenNthCalledWith(1, "ws-1");
        expect(h.GetWorkspace).toHaveBeenNthCalledWith(2, "ws-new");
    });

    it("starts initMux with the new window and its tab", async () => {
        await run();

        expect(initOpts().windowId).toBe("win-new");
        expect(initOpts().tabId).toBe("tab-new");
    });

    it("closes before it creates, and creates before it reads", async () => {
        await run();

        expect(seen("GetClientData", "GetWindow", "GetWorkspace", "CloseWindow", "CreateWindow").slice(0, 6)).toEqual([
            "GetClientData",
            "GetWindow",
            "GetWorkspace",
            "CloseWindow",
            "CreateWindow",
            "GetWorkspace",
        ]);
    });

    it("labels the replacement from the URL", async () => {
        window.history.replaceState({}, "", "/?windowLabel=win-4");

        await run();

        expect(h.CreateWindow).toHaveBeenCalledWith(null, "", "win-4");
    });

    it("does not reopen the last session's windows", async () => {
        await run();

        expect(h.reopenRestoredWindows).not.toHaveBeenCalled();
    });
});

describe("initHostMux — which tab it opens", () => {
    it("the workspace's active tab first", async () => {
        h.GetWorkspace.mockResolvedValue(workspace({ activetabid: "tab-active" }));

        await run();

        expect(initOpts().tabId).toBe("tab-active");
    });

    it("else its first tab", async () => {
        h.GetWorkspace.mockResolvedValue(workspace({ activetabid: "", tabids: ["tab-first", "tab-2"] }));

        await run();

        expect(initOpts().tabId).toBe("tab-first");
    });

    it("else its first pinned tab", async () => {
        h.GetWorkspace.mockResolvedValue(
            workspace({ activetabid: "", tabids: [], pinnedtabids: ["pin-first", "pin-2"] })
        );

        await run();

        expect(initOpts().tabId).toBe("pin-first");
    });

    it("reads a workspace with no tab list, only pinned tabs", async () => {
        h.GetWorkspace.mockResolvedValue({ oid: "ws-1", pinnedtabids: ["pin-only"] });

        await run();

        expect(initOpts().tabId).toBe("pin-only");
    });

    it("reads a workspace with a tab list but no pinned list", async () => {
        h.GetWorkspace.mockResolvedValue({ oid: "ws-1", tabids: ["tab-only"] });

        await run();

        expect(initOpts().tabId).toBe("tab-only");
    });

    it("fails startup when the workspace has no tab at all", async () => {
        h.GetWorkspace.mockResolvedValue({ oid: "ws-1", activetabid: "", tabids: [], pinnedtabids: [] });

        const out = await run();

        expect(h.failStartup).toHaveBeenCalledTimes(1);
        expect(String(h.failStartup.mock.calls[0][0])).toContain("No tab found in workspace");
        expect(out.error).toBeInstanceOf(h.StartupFailureHandled);
        expect(h.initGlobal).not.toHaveBeenCalled();
    });

    it("fails startup for a workspace with no tab lists at all", async () => {
        h.GetWorkspace.mockResolvedValue({ oid: "ws-1" });

        await run();

        expect(String(h.failStartup.mock.calls[0][0])).toContain("No tab found in workspace");
    });
});

describe("initHostMux — reads are retried, writes are not", () => {
    const flaky = () => new TypeError("Failed to fetch");

    it("retries GetClientData after a request that never got a response", async () => {
        h.GetClientData.mockRejectedValueOnce(flaky());

        const out = await run(() => h.GetClientData.mock.calls.length >= 1);
        expect(out.done).toBe(false);
        await advance(100, () => out.done);

        expect(out).toEqual({ done: true });
        expect(h.GetClientData).toHaveBeenCalledTimes(3); // 2 reads + the restore-notice read
    });

    it("retries GetWindow", async () => {
        h.GetWindow.mockRejectedValueOnce(flaky());

        const out = await run(() => h.GetWindow.mock.calls.length >= 1);
        await advance(100, () => out.done);

        expect(out).toEqual({ done: true });
        expect(h.GetWindow).toHaveBeenCalledTimes(2);
    });

    it("retries the first GetWorkspace", async () => {
        h.GetWorkspace.mockRejectedValueOnce(flaky());

        const out = await run(() => h.GetWorkspace.mock.calls.length >= 1);
        await advance(100, () => out.done);

        expect(out).toEqual({ done: true });
        expect(h.GetWorkspace).toHaveBeenCalledTimes(2);
    });

    it("does not retry a failure that is not a lost request", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockRejectedValue(new Error("srv said no"));

        const out = await run();

        expect(h.GetClientData).toHaveBeenCalledTimes(1);
        expect(out.error).toBeInstanceOf(h.StartupFailureHandled);
    });

    it("does not retry CreateWindow when no window exists", async () => {
        h.GetClientData.mockResolvedValueOnce(client({ windowids: [] }));
        h.CreateWindow.mockReset();
        h.CreateWindow.mockRejectedValue(flaky());

        const out = await run();
        await advance(1000);

        expect(h.CreateWindow).toHaveBeenCalledTimes(1);
        expect(out.error).toBeInstanceOf(h.StartupFailureHandled);
    });

    it("does not retry the replacement CreateWindow", async () => {
        h.GetWindow.mockResolvedValueOnce(null);
        h.CreateWindow.mockReset();
        h.CreateWindow.mockRejectedValue(flaky());

        const out = await run();
        await advance(1000);

        expect(h.CreateWindow).toHaveBeenCalledTimes(1);
        expect(out.error).toBeInstanceOf(h.StartupFailureHandled);
    });

    it("does not retry CloseWindow", async () => {
        h.GetWorkspace.mockResolvedValueOnce(null);
        h.CloseWindow.mockReset();
        h.CloseWindow.mockRejectedValue(flaky());

        const out = await run();
        await advance(1000);

        expect(h.CloseWindow).toHaveBeenCalledTimes(1);
        expect(out.error).toBeInstanceOf(h.StartupFailureHandled);
    });

    it("does not retry the CreateWindow after the window is closed", async () => {
        h.GetWorkspace.mockResolvedValueOnce(null);
        h.CreateWindow.mockReset();
        h.CreateWindow.mockRejectedValue(flaky());

        const out = await run();
        await advance(1000);

        expect(h.CloseWindow).toHaveBeenCalledTimes(1);
        expect(h.CreateWindow).toHaveBeenCalledTimes(1);
        expect(out.error).toBeInstanceOf(h.StartupFailureHandled);
    });

    it("does not retry the GetWorkspace after the window is recreated", async () => {
        h.GetWorkspace.mockReset();
        h.GetWorkspace.mockResolvedValueOnce(null);
        h.GetWorkspace.mockRejectedValue(flaky());

        const out = await run();
        await advance(1000);

        expect(h.GetWorkspace).toHaveBeenCalledTimes(2);
        expect(out.error).toBeInstanceOf(h.StartupFailureHandled);
    });
});

describe("initHostMux — every srv call has five seconds", () => {
    const timeoutMsg = (label: string) => `Timeout: ${label} did not respond within 5s`;

    async function expectTimeout(label: string, blocked: () => boolean) {
        const out = await run(blocked);
        await advance(4999);
        expect(out.done).toBe(false);
        await advance(1, () => out.done);

        expect(out.done).toBe(true);
        expect(out.error).toBeInstanceOf(h.StartupFailureHandled);
        expect(String(h.failStartup.mock.calls[0][0])).toContain(timeoutMsg(label));
        expect(h.initGlobal).not.toHaveBeenCalled();
    }

    it("GetClientData", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockReturnValue(never());

        await expectTimeout("GetClientData", () => h.GetClientData.mock.calls.length >= 1);
    });

    it("CreateWindow, on a cold start", async () => {
        h.GetClientData.mockResolvedValueOnce(client({ windowids: [] }));
        h.CreateWindow.mockReset();
        h.CreateWindow.mockReturnValue(never());

        await expectTimeout("CreateWindow", () => h.CreateWindow.mock.calls.length >= 1);
    });

    it("GetWindow", async () => {
        h.GetWindow.mockReset();
        h.GetWindow.mockReturnValue(never());

        await expectTimeout("GetWindow", () => h.GetWindow.mock.calls.length >= 1);
    });

    it("the replacement CreateWindow", async () => {
        h.GetWindow.mockResolvedValueOnce(null);
        h.CreateWindow.mockReset();
        h.CreateWindow.mockReturnValue(never());

        await expectTimeout("CreateWindow", () => h.CreateWindow.mock.calls.length >= 1);
    });

    it("GetWorkspace", async () => {
        h.GetWorkspace.mockReset();
        h.GetWorkspace.mockReturnValue(never());

        await expectTimeout("GetWorkspace", () => h.GetWorkspace.mock.calls.length >= 1);
    });

    it("CloseWindow", async () => {
        h.GetWorkspace.mockResolvedValueOnce(null);
        h.CloseWindow.mockReset();
        h.CloseWindow.mockReturnValue(never());

        await expectTimeout("CloseWindow", () => h.CloseWindow.mock.calls.length >= 1);
    });

    it("the replacement CreateWindow, after the close", async () => {
        h.GetWorkspace.mockResolvedValueOnce(null);
        h.CreateWindow.mockReset();
        h.CreateWindow.mockReturnValue(never());

        await expectTimeout("CreateWindow", () => h.CreateWindow.mock.calls.length >= 1);
    });

    it("the GetWorkspace after the window is recreated", async () => {
        h.GetWorkspace.mockReset();
        h.GetWorkspace.mockResolvedValueOnce(null);
        h.GetWorkspace.mockReturnValue(never());

        await expectTimeout("GetWorkspace", () => h.GetWorkspace.mock.calls.length >= 2);
    });
});

describe("initHostMux — failure", () => {
    it("logs the described error, tells the host, and hands the card to failStartup", async () => {
        const boom = new Error("srv exploded");
        h.GetClientData.mockReset();
        h.GetClientData.mockRejectedValue(boom);

        const out = await run();

        const described = describeError(boom);
        const text = formatDescribedError(described);
        expect(errors).toContainEqual(["[initHostMux] Initialization failed:", described]);
        expect(sentLogs()).toContain(`[initHostMux] ERROR: ${text}`);
        expect(h.failStartup).toHaveBeenCalledTimes(1);
        expect(h.failStartup).toHaveBeenCalledWith(text);
        expect(text).toContain("srv exploded");
        expect(out.error).toBeInstanceOf(h.StartupFailureHandled);
        expect((out.error as Error).message).toBe(text);
    });

    it("does not put up a second card for a failure it already handled", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockRejectedValue(new Error("srv exploded"));

        await run();

        expect(h.showStartupError).not.toHaveBeenCalled();
    });

    it("describes a thrown string too", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockRejectedValue("plain string");

        await run();

        expect(h.failStartup).toHaveBeenCalledWith(formatDescribedError(describeError("plain string")));
    });

    it("starts none of the notices after a failure", async () => {
        h.GetWindow.mockReset();
        h.GetWindow.mockRejectedValue(new Error("no"));

        await run();

        expect(h.benchDump).not.toHaveBeenCalled();
        expect(h.surfaceBackgroundAudit).not.toHaveBeenCalled();
        expect(h.surfaceIncidents).not.toHaveBeenCalled();
        expect(h.offerLayoutFromOtherBuild).not.toHaveBeenCalled();
        expect(h.offerForkSlugUpdate).not.toHaveBeenCalled();
        expect(h.api.getInstanceNumber).not.toHaveBeenCalled();
    });

    it("fails startup when initMux cannot find a core object it was told to load", async () => {
        h.loadAndPinMuxObject.mockImplementation(async (oref: string) =>
            oref.startsWith("client:") ? null : { oid: oref, workspaceid: "ws-1", layoutstate: "layout-1" }
        );

        const out = await run();

        expect(h.failStartup).toHaveBeenCalledTimes(1);
        expect(String(h.failStartup.mock.calls[0][0])).toContain("client");
        expect(out.error).toBeInstanceOf(h.StartupFailureHandled);
        expect(h.benchDump).not.toHaveBeenCalled();
        expect(h.api.getInstanceNumber).not.toHaveBeenCalled();
    });
});

describe("initHostMux — the dev window title", () => {
    it("names the window after VITE_DEV_TITLE in a dev build", async () => {
        vi.stubEnv("VITE_DEV_TITLE", "agentx: PR #1780");

        await run();

        expect(h.UpdateObjectMeta).toHaveBeenCalledTimes(1);
        expect(h.UpdateObjectMeta).toHaveBeenCalledWith("window:win-1", {
            [DISPLAY_NAME_META_KEY]: "agentx: PR #1780",
        });
    });

    it("names the window that initMux started with, not the one it found first", async () => {
        vi.stubEnv("VITE_DEV_TITLE", "t");
        h.GetWindow.mockResolvedValueOnce(null);

        await run();

        expect(h.UpdateObjectMeta).toHaveBeenCalledWith("window:win-new", { [DISPLAY_NAME_META_KEY]: "t" });
    });

    it("names the recreated window when the workspace had to be recreated", async () => {
        vi.stubEnv("VITE_DEV_TITLE", "t");
        h.GetWorkspace.mockResolvedValueOnce(null);

        await run();

        expect(h.UpdateObjectMeta).toHaveBeenCalledWith("window:win-new", { [DISPLAY_NAME_META_KEY]: "t" });
    });

    it("cuts a long title to the display-name limit", async () => {
        vi.stubEnv("VITE_DEV_TITLE", "x".repeat(DISPLAY_NAME_MAX_LEN + 20));

        await run();

        expect(h.UpdateObjectMeta).toHaveBeenCalledWith("window:win-1", {
            [DISPLAY_NAME_META_KEY]: "x".repeat(DISPLAY_NAME_MAX_LEN),
        });
    });

    it("keeps a title exactly at the limit whole", async () => {
        vi.stubEnv("VITE_DEV_TITLE", "y".repeat(DISPLAY_NAME_MAX_LEN));

        await run();

        expect(h.UpdateObjectMeta).toHaveBeenCalledWith("window:win-1", {
            [DISPLAY_NAME_META_KEY]: "y".repeat(DISPLAY_NAME_MAX_LEN),
        });
    });

    it("does nothing without a title", async () => {
        vi.stubEnv("VITE_DEV_TITLE", "");

        await run();

        expect(h.UpdateObjectMeta).not.toHaveBeenCalled();
    });

    it("does nothing outside a dev build", async () => {
        vi.stubEnv("VITE_DEV_TITLE", "agentx: PR #1780");
        vi.stubEnv("DEV", false);

        await run();

        expect(h.UpdateObjectMeta).not.toHaveBeenCalled();
    });

    it("sets it after initMux and before the instance number is read", async () => {
        vi.stubEnv("VITE_DEV_TITLE", "t");

        await run();

        expect(h.order.indexOf("initGlobal")).toBeLessThan(h.order.indexOf("UpdateObjectMeta"));
        expect(h.order.indexOf("UpdateObjectMeta")).toBeLessThan(h.order.indexOf("getInstanceNumber"));
    });

    it("does not wait for the rename", async () => {
        vi.stubEnv("VITE_DEV_TITLE", "t");
        h.UpdateObjectMeta.mockReset();
        h.UpdateObjectMeta.mockReturnValue(never());

        const out = await run();

        expect(out).toEqual({ done: true });
        expect(h.benchDump).toHaveBeenCalledTimes(1);
    });
});

describe("initHostMux — instance tracking", () => {
    it("hands the instance number and the window list to the stores", async () => {
        h.api.listWindowInstances.mockResolvedValue([
            { label: "main", windowId: "win-1" },
            { label: "w2", windowId: null },
        ]);

        await run();

        expect(h.setWindowInstanceNumAtom).toHaveBeenCalledWith(3);
        expect(h.seedKnownEntriesFromSnapshot).toHaveBeenCalledWith([
            { label: "main", windowId: "win-1" },
            { label: "w2", windowId: null },
        ]);
    });

    it("seeds an empty list when the window list is not a list", async () => {
        h.api.listWindowInstances.mockResolvedValue(undefined);

        await run();

        expect(h.seedKnownEntriesFromSnapshot).toHaveBeenCalledWith([]);
    });

    it("falls back to the plain window labels when the instance list fails", async () => {
        h.api.listWindowInstances.mockRejectedValue(new Error("old host"));
        h.api.listWindows.mockResolvedValue(["main", "w2"]);

        await run();

        expect(h.seedKnownEntriesFromSnapshot).toHaveBeenCalledWith([
            { label: "main", windowId: null },
            { label: "w2", windowId: null },
        ]);
    });

    it("seeds an empty list when the fallback is not a list either", async () => {
        h.api.listWindowInstances.mockRejectedValue(new Error("old host"));
        h.api.listWindows.mockResolvedValue(null);

        await run();

        expect(h.seedKnownEntriesFromSnapshot).toHaveBeenCalledWith([]);
    });

    it("warns and carries on when tracking fails", async () => {
        const boom = new Error("no instance number");
        h.api.getInstanceNumber.mockReset();
        h.api.getInstanceNumber.mockRejectedValue(boom);

        const out = await run();

        expect(out).toEqual({ done: true });
        expect(warns).toContainEqual(["[initInstanceTracking] failed:", boom]);
        expect(h.setWindowInstanceNumAtom).not.toHaveBeenCalled();
        expect(h.seedKnownEntriesFromSnapshot).not.toHaveBeenCalled();
        expect(h.benchDump).toHaveBeenCalledTimes(1);
        expect(h.surfaceIncidents).toHaveBeenCalledTimes(1);
    });

    it("waits for tracking before it starts the notices", async () => {
        let release: () => void = () => {};
        h.api.getInstanceNumber.mockReset();
        h.api.getInstanceNumber.mockReturnValue(new Promise<number>((r) => (release = () => r(2))));

        const out = await run(() => h.api.getInstanceNumber.mock.calls.length >= 1);

        expect(out.done).toBe(false);
        expect(h.benchDump).not.toHaveBeenCalled();
        expect(h.surfaceBackgroundAudit).not.toHaveBeenCalled();
        expect(h.surfaceIncidents).not.toHaveBeenCalled();
        expect(h.offerLayoutFromOtherBuild).not.toHaveBeenCalled();
        expect(h.offerForkSlugUpdate).not.toHaveBeenCalled();
        expect(h.GetClientData).toHaveBeenCalledTimes(1);

        release();
        await until(() => out.done && noticesStarted());
        await settle();

        expect(out.done).toBe(true);
        expect(h.benchDump).toHaveBeenCalledTimes(1);
        expect(h.surfaceBackgroundAudit).toHaveBeenCalledTimes(1);
    });
});

describe("initHostMux — the notices after startup", () => {
    it("starts each of them once", async () => {
        await run();

        expect(h.surfaceBackgroundAudit).toHaveBeenCalledTimes(1);
        expect(h.surfaceIncidents).toHaveBeenCalledTimes(1);
        expect(h.offerLayoutFromOtherBuild).toHaveBeenCalledTimes(1);
        expect(h.offerForkSlugUpdate).toHaveBeenCalledTimes(1);
        expect(h.benchDump).toHaveBeenCalledTimes(1);
    });

    it("reads the client again for the workspace-restored notice and hands it over", async () => {
        const second = client({ oid: "client-1", meta: { "workspace:restored_from": "2026-11-08" } });
        h.GetClientData.mockReset();
        h.GetClientData.mockResolvedValueOnce(client());
        h.GetClientData.mockResolvedValueOnce(second);

        await run();

        expect(h.GetClientData).toHaveBeenCalledTimes(2);
        expect(h.surfaceWorkspaceRestoreNotice).toHaveBeenCalledTimes(1);
        expect(h.surfaceWorkspaceRestoreNotice).toHaveBeenCalledWith(second);
    });

    it("does not let a failed second read hold startup up or surface an error", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockResolvedValueOnce(client());
        h.GetClientData.mockRejectedValueOnce(new Error("lost"));

        const out = await run();

        expect(out).toEqual({ done: true });
        expect(h.surfaceWorkspaceRestoreNotice).not.toHaveBeenCalled();
        expect(h.failStartup).not.toHaveBeenCalled();
    });

    it("does not wait for any of the notices", async () => {
        h.surfaceBackgroundAudit.mockReset();
        h.surfaceBackgroundAudit.mockReturnValue(never());
        h.surfaceIncidents.mockReset();
        h.surfaceIncidents.mockReturnValue(never());
        h.offerLayoutFromOtherBuild.mockReset();
        h.offerLayoutFromOtherBuild.mockReturnValue(never());
        h.offerForkSlugUpdate.mockReset();
        h.offerForkSlugUpdate.mockReturnValue(never());
        h.GetClientData.mockReset();
        h.GetClientData.mockResolvedValueOnce(client());
        h.GetClientData.mockReturnValueOnce(never());

        const out = await run();

        expect(out).toEqual({ done: true });
        expect(h.benchDump).toHaveBeenCalledTimes(1);
    });

    it("hands the restore notice the client after the cold start's CreateWindow", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockResolvedValueOnce(client({ windowids: [] }));
        h.GetClientData.mockResolvedValueOnce(client({ oid: "client-1", meta: { "workspace:restored_from": "x" } }));

        await run();

        expect(h.order.indexOf("CreateWindow")).toBeLessThan(h.order.indexOf("surfaceWorkspaceRestoreNotice"));
        expect(h.surfaceWorkspaceRestoreNotice).toHaveBeenCalledTimes(1);
    });
});

describe("initHostMux — the startup-perf lines", () => {
    // Stage lengths on the fake clock; every srv read takes a different time.
    const after = <T>(ms: number, value: T) => new Promise<T>((r) => setTimeout(() => r(value), ms));

    it("times each stage, and the total since it began", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockImplementationOnce(() => after(120, client()));
        h.GetClientData.mockResolvedValue(client());
        h.GetWindow.mockReset();
        h.GetWindow.mockImplementation(() => after(40, win()));
        h.GetWorkspace.mockReset();
        h.GetWorkspace.mockImplementation(() => after(25, workspace()));
        h.loadConnStatus.mockImplementation(() => after(300, undefined));

        const out = await run(() => h.GetClientData.mock.calls.length >= 1);
        await advance(120, () => h.GetWindow.mock.calls.length >= 1);
        await advance(40, () => h.GetWorkspace.mock.calls.length >= 1);
        await advance(25, () => h.loadConnStatus.mock.calls.length >= 1);
        await advance(300, () => out.done && noticesStarted());

        expect(out).toEqual({ done: true });
        expect(perfLines()).toEqual([
            "[startup-perf] GetClientData: 120.0ms (total: 120.0ms)",
            "[startup-perf] GetWindow: 40.0ms (total: 160.0ms)",
            "[startup-perf] GetWorkspace: 25.0ms (total: 185.0ms)",
            "[startup-perf] Phase 1 complete (discovery): 185.0ms (total: 185.0ms)",
            "[startup-perf] initMuxWrap: 300.0ms (total: 485.0ms)",
            "[startup-perf] TOTAL initCefMux: 485.0ms (total: 485.0ms)",
        ]);
    });

    it("times the cold-start CreateWindow on its own", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockImplementationOnce(() => after(20, client({ windowids: [] })));
        h.GetClientData.mockResolvedValue(client());
        h.CreateWindow.mockReset();
        h.CreateWindow.mockImplementation(() => after(70, win({ oid: "win-new", workspaceid: "ws-new" })));
        h.GetWindow.mockReset();
        h.GetWindow.mockImplementation(() => after(10, win()));

        const out = await run(() => h.GetClientData.mock.calls.length >= 1);
        await advance(20, () => h.CreateWindow.mock.calls.length >= 1);
        await advance(70, () => h.GetWindow.mock.calls.length >= 1);
        await advance(10, () => out.done && noticesStarted());

        expect(perfLines().slice(0, 3)).toEqual([
            "[startup-perf] GetClientData: 20.0ms (total: 20.0ms)",
            "[startup-perf] CreateWindow (no windows): 70.0ms (total: 90.0ms)",
            "[startup-perf] GetWindow: 10.0ms (total: 100.0ms)",
        ]);
    });

    it("times the replacement window", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockImplementationOnce(() => after(20, client()));
        h.GetClientData.mockResolvedValue(client());
        h.GetWindow.mockReset();
        h.GetWindow.mockImplementationOnce(() => after(35, null));
        h.CreateWindow.mockReset();
        h.CreateWindow.mockImplementation(() => after(55, win({ oid: "win-new", workspaceid: "ws-new" })));
        h.GetWorkspace.mockReset();
        h.GetWorkspace.mockImplementation(() => after(5, workspace()));

        const out = await run(() => h.GetClientData.mock.calls.length >= 1);
        await advance(20, () => h.GetWindow.mock.calls.length >= 1);
        await advance(35, () => h.CreateWindow.mock.calls.length >= 1);
        await advance(55, () => h.GetWorkspace.mock.calls.length >= 1);
        await advance(5, () => out.done && noticesStarted());

        expect(perfLines().slice(0, 4)).toEqual([
            "[startup-perf] GetClientData: 20.0ms (total: 20.0ms)",
            "[startup-perf] GetWindow: 35.0ms (total: 55.0ms)",
            "[startup-perf] CreateWindow (fallback): 55.0ms (total: 110.0ms)",
            "[startup-perf] GetWorkspace: 5.0ms (total: 115.0ms)",
        ]);
    });

    it("times closing and recreating the window as one stage", async () => {
        h.GetClientData.mockReset();
        h.GetClientData.mockImplementationOnce(() => after(20, client()));
        h.GetClientData.mockResolvedValue(client());
        h.GetWindow.mockReset();
        h.GetWindow.mockImplementation(() => after(35, win()));
        h.GetWorkspace.mockReset();
        h.GetWorkspace.mockImplementationOnce(() => after(12, null));
        h.GetWorkspace.mockImplementation(() => after(15, workspace()));
        h.CloseWindow.mockReset();
        h.CloseWindow.mockImplementation(() => after(30, undefined));
        h.CreateWindow.mockReset();
        h.CreateWindow.mockImplementation(() => after(45, win({ oid: "win-new", workspaceid: "ws-new" })));

        const out = await run(() => h.GetClientData.mock.calls.length >= 1);
        await advance(20, () => h.GetWindow.mock.calls.length >= 1);
        await advance(35, () => h.GetWorkspace.mock.calls.length >= 1);
        await advance(12, () => h.CloseWindow.mock.calls.length >= 1);
        await advance(30, () => h.CreateWindow.mock.calls.length >= 1);
        await advance(45, () => h.GetWorkspace.mock.calls.length >= 2);
        await advance(15, () => out.done && noticesStarted());

        expect(perfLines().slice(0, 5)).toEqual([
            "[startup-perf] GetClientData: 20.0ms (total: 20.0ms)",
            "[startup-perf] GetWindow: 35.0ms (total: 55.0ms)",
            "[startup-perf] GetWorkspace: 12.0ms (total: 67.0ms)",
            "[startup-perf] Recreate window+workspace: 90.0ms (total: 157.0ms)",
            "[startup-perf] Phase 1 complete (discovery): 157.0ms (total: 157.0ms)",
        ]);
    });

    it("prints the lines to the console only, not to the host log", async () => {
        await run();

        expect(sentLogs().filter((s) => s.includes("GetClientData") || s.includes("TOTAL initCefMux"))).toEqual([]);
    });
});
