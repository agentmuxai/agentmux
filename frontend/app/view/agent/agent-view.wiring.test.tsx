// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How AgentPresentationView is wired, pinned: the real view is mounted with every hook and every
 * child component stood in for. A snapshot holds the hooks it calls, in order, with the option keys
 * each gets (hooks called in the same order keep their onMount order), and the tree it draws with the
 * props each child receives. Behaviour tests then call the callbacks the view hands down and check
 * where they go. The hooks and children have tests of their own; this is what keeps a split of the
 * view from rewiring them. Regenerate with `vitest -u` only for a change meant to differ.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { ModalLayerContext } from "@/app/element/modal-layer";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const calls: string[] = [];
    /** The arguments each hook was last called with. */
    const argsOf: Record<string, unknown[]> = {};
    /** What the callback tests watch happen, in order. */
    const trace: string[] = [];
    const fns: Record<string, any> = {};
    /** A value that answers anything: call it, read a field, call that. */
    const any = (): any => new Proxy(() => {}, { get: (_t, k) => (k === Symbol.toPrimitive ? () => "" : k === "then" ? undefined : any()), apply: () => any() });
    /** `base`, and the answer-anything value for every field it lacks. */
    const withAny = <T extends object>(base: T): any => new Proxy(base, { get: (t, k) => (k in t ? (t as any)[k] : k === "then" ? undefined : any()) });
    const describeArg = (a: unknown): string =>
        typeof a === "function" ? "fn" : a && typeof a === "object" && !Array.isArray(a) ? `{${Object.keys(a).sort().join(",")}}` : JSON.stringify(a) ?? String(a);
    const hook = (name: string, ret?: unknown) =>
        (...args: unknown[]) => {
            calls.push(`${name}(${args.map(describeArg).join(", ")})`);
            argsOf[name] = args;
            return typeof ret === "function" ? (ret as (...a: unknown[]) => unknown)(...args) : ret;
        };
    /** A module factory for one stand-in child component (agent-view.wiring.stub.tsx). */
    const comp = (name: string, withChildren = false) => async () => ({ [name]: (await import("./agent-view.wiring.stub")).stub(name, withChildren) });
    return { calls, argsOf, trace, fns, any, withAny, hook, comp };
});

// ---- the model and what its hooks return ----
vi.mock("@/app/block/pane-tab-visibility", () => ({ usePaneTabVisibility: h.hook("usePaneTabVisibility", () => () => h.fns.visibility()) }));
vi.mock("@/app/block/context-menu-region", () => ({ resolveContextMenuRegion: () => h.fns.region() }));
vi.mock("@/app/store/contextmenu", () => ({ showCopyContextMenu: (...a: unknown[]) => h.fns.copyMenu(...a) }));
vi.mock("@/app/store/global", () => ({ getBlockMetaKeyAtom: () => () => undefined }));
vi.mock("@/app/store/pane-zoom-local", () => ({ paneZoom: () => 1 }));
vi.mock("@/app/store/agent-pane-state/types", () => ({ isAuthFailure: (f: { auth?: boolean } | null) => !!f?.auth, isStopping: () => false }));
vi.mock("@/app/window/window-focus", () => ({ makeWindowFocusSignal: () => () => true }));
vi.mock("../../agent-model/live-feed", () => ({ LIVE_FEED_RESTORE_BYTES: 1000 }));
vi.mock("../../agent-model/state", () => ({ createAgentAtoms: h.hook("createAgentAtoms", () => ({ documentStateAtom: [() => ({}), (u: unknown) => h.fns.setDocState(u)] })) }));
vi.mock("./agent-view-commands", () => ({ registerAgentViewCommands: () => {} }));
vi.mock("./agent-view-controls", () => ({
    createAgentViewControls: h.hook("createAgentViewControls", () => h.fns.viewControls),
    registerAgentViewControls: h.hook("registerAgentViewControls", () => () => {}),
    viewChipLabel: () => "",
}));
vi.mock("./activity/promotion-clock", () => ({ createPromotionClock: h.hook("createPromotionClock", () => () => 0) }));
vi.mock("./activity/useAttachedTaskAxis", () => ({ useAttachedTaskAxis: h.hook("useAttachedTaskAxis") }));
vi.mock("./btw", () => ({ askSideQuestion: (...a: unknown[]) => h.fns.askSide(...a) }));
vi.mock("./quick-fork", () => ({ quickForkAgent: (...a: unknown[]) => h.fns.quickFork(...a) }));
vi.mock("./open-history-tab", () => ({ openOrFocusHistoryTab: (...a: unknown[]) => h.fns.openHistory(...a) }));
vi.mock("./providers", () => ({ getProvider: (key: string) => (key === "unknown" ? undefined : { id: "claude", displayName: "Claude", authCheckCommand: ["auth", "status"] }) }));
vi.mock("./render-dormant", () => ({ useRenderDormant: h.hook("useRenderDormant", () => () => false) }));
vi.mock("./sign-in-notice", () => ({ createSignInNotice: h.hook("createSignInNotice", () => ({ answered: () => {}, needed: () => {}, bound: () => {} })) }));
vi.mock("./system-notification", () => ({ createSystemNotifier: h.hook("createSystemNotifier", () => () => {}) }));
vi.mock("./startup/startupOnReady", () => ({ startupOnReady: h.hook("startupOnReady", () => h.fns.startup) }));
vi.mock("./transcript-cursor", () => ({ createTranscriptSettleLatch: h.hook("createTranscriptSettleLatch", () => h.fns.settle) }));
vi.mock("./inject-history-link", () => ({
    injectGapRows: (...a: unknown[]) => h.fns.pass("injectGapRows", ...a),
    injectHistoryLink: (...a: unknown[]) => h.fns.pass("injectHistoryLink", ...a),
}));
vi.mock("./inject-resume-preflight", () => ({
    buildResumePreflightNode: (...a: unknown[]) => (h.fns.pass("buildResumePreflightNode", ...a), null),
    injectResumePreflight: (...a: unknown[]) => h.fns.pass("injectResumePreflight", ...a),
}));
vi.mock("./virtualization/verbosity", () => ({ deriveDisplayNodes: (...a: unknown[]) => h.fns.pass("deriveDisplayNodes", ...a) }));
vi.mock("./failure/failure-writer", () => ({ clearFailure: (...a: unknown[]) => h.fns.clearFailure(...a) }));
vi.mock("./failure/resolved-auth-errors", () => ({ markResolvedAuthErrors: (...a: unknown[]) => h.fns.pass("markResolvedAuthErrors", ...a) }));
vi.mock("./failure/useAgentAccountBinding", () => ({
    useAgentAccountBinding: h.hook("useAgentAccountBinding", () => h.fns.account),
}));
vi.mock("./failure/useAuthHealth", () => ({
    useAuthHealth: h.hook("useAuthHealth", () => h.fns.authHealth),
    useSyntheticAuthRow: h.hook("useSyntheticAuthRow"),
}));
vi.mock("./failure/useFailureRow", () => ({ useFailureRow: h.hook("useFailureRow", () => h.fns.failureUI) }));
vi.mock("./offline/useOfflineInput", () => ({
    lastUserMessage: () => h.fns.lastUser(),
    useOfflineInput: h.hook("useOfflineInput", () => h.fns.offline),
}));
vi.mock("./hooks/turn-confirmation", () => ({
    createTurnConfirmation: h.hook("createTurnConfirmation", () => h.fns.turn),
}));
vi.mock("./hooks/useAgentActivitySummary", () => ({ useAgentActivitySummary: h.hook("useAgentActivitySummary") }));
vi.mock("./hooks/useAgentCommands", () => ({ useAgentCommands: h.hook("useAgentCommands", () => h.fns.commands) }));
vi.mock("./hooks/useAgentControllerStatus", () => ({
    useAgentControllerStatus: h.hook("useAgentControllerStatus", () => h.fns.launch),
}));
vi.mock("./hooks/useAgentDecisions", () => ({ useAgentDecisions: h.hook("useAgentDecisions", () => h.fns.decisions) }));
vi.mock("./hooks/useAgentDropAttach", () => ({ useAgentDropAttach: h.hook("useAgentDropAttach") }));
vi.mock("./hooks/useAgentKeyboard", () => ({ useAgentKeyboard: h.hook("useAgentKeyboard") }));
vi.mock("./hooks/useAgentQuestions", () => ({
    useAgentQuestions: h.hook("useAgentQuestions", () => h.fns.questions),
}));
vi.mock("./hooks/useAmbientNarration", () => ({ useAmbientNarration: h.hook("useAmbientNarration") }));
vi.mock("./hooks/useBackgroundHandoffNarration", () => ({ useBackgroundHandoffNarration: h.hook("useBackgroundHandoffNarration") }));
vi.mock("./hooks/useBlockActivity", () => ({ useBlockActivity: h.hook("useBlockActivity") }));
vi.mock("./hooks/useContextReading", () => ({
    useContextReading: h.hook("useContextReading", () => ({ reading: () => null, note: () => null, autoCompact: () => false })),
}));
vi.mock("./hooks/useControllerStatusEvents", () => ({ useControllerStatusEvents: h.hook("useControllerStatusEvents") }));
vi.mock("./hooks/useHistoryPagination", () => ({
    useHistoryPagination: h.hook("useHistoryPagination", () => h.fns.history),
}));
vi.mock("./hooks/useInSessionSearch", () => ({
    useInSessionSearch: h.hook("useInSessionSearch", () => h.fns.search),
}));
vi.mock("./hooks/useLiveFeedRollOff", () => ({ useLiveFeedRollOff: h.hook("useLiveFeedRollOff", () => h.fns.liveFeed) }));
vi.mock("./hooks/useNextPromptSuggestion", () => ({ useNextPromptSuggestion: h.hook("useNextPromptSuggestion") }));
vi.mock("./hooks/usePaneRegistration", () => ({ usePaneRegistration: h.hook("usePaneRegistration", () => ({ paneModel: h.fns.paneModel, layoutView: () => null })) }));
vi.mock("./hooks/usePaneReveal", () => ({
    usePaneReveal: h.hook("usePaneReveal", () => h.fns.reveal),
}));
vi.mock("./hooks/usePtyWidth", () => ({ usePtyWidth: h.hook("usePtyWidth"), computeTermSizeFromEl: (el: unknown) => h.fns.termSize(el) }));
vi.mock("./hooks/useResumePreflight", () => ({ useResumePreflight: h.hook("useResumePreflight", () => ({ result: () => null, showSteps: () => false })) }));
vi.mock("./hooks/useScrollToNode", () => ({ useScrollToNode: h.hook("useScrollToNode", () => h.fns.scroll) }));
vi.mock("./hooks/useShellDrawerLifecycle", () => ({ useShellDrawerLifecycle: h.hook("useShellDrawerLifecycle", () => h.fns.drawer) }));
vi.mock("./hooks/useShellLogBridge", () => ({
    useShellLogBridge: h.hook("useShellLogBridge", () => h.fns.bridge),
}));
vi.mock("./hooks/useSnapshotPersistence", () => ({ useSnapshotPersistence: h.hook("useSnapshotPersistence") }));
vi.mock("./hooks/useTurnEndSweep", () => ({ useTurnEndSweep: h.hook("useTurnEndSweep") }));
vi.mock("./hooks/useTurnLedger", () => ({ useTurnLedger: h.hook("useTurnLedger") }));
vi.mock("./hooks/useTurnReconciliation", () => ({ useFocusRepoll: h.hook("useFocusRepoll"), useHeldMessageDelivery: h.hook("useHeldMessageDelivery") }));
vi.mock("./hooks/useWorkingIndicator", () => ({ useWorkingIndicator: h.hook("useWorkingIndicator", () => h.fns.working) }));
vi.mock("./useAgentStream", () => ({ useAgentStream: h.hook("useAgentStream", () => h.fns.bgTasks) }));

// ---- the children: each draws its name and the props it was given ----
vi.mock("@/app/element/PaneLoadingCover", h.comp("PaneLoadingCover"));
vi.mock("./agent-media", h.comp("AgentPaneProviders", true));
vi.mock("./shutdown/ShutdownOverlay", h.comp("ShutdownOverlay"));
vi.mock("./components/AgentProgressBar", h.comp("AgentProgressBar"));
vi.mock("./components/BtwOverlay", h.comp("BtwOverlay"));
vi.mock("./components/AgentSearchBar", h.comp("AgentSearchBar"));
vi.mock("./components/HistoryLoadingRow", h.comp("HistoryLoadingRow"));
vi.mock("./components/AgentDocumentView", h.comp("AgentDocumentView"));
vi.mock("./components/NewActivityPill", h.comp("NewActivityPill"));
vi.mock("./components/AgentBottomPanels", h.comp("AgentBottomPanels"));
vi.mock("./components/AgentPreparingRow", h.comp("AgentPreparingRow"));
vi.mock("./components/AgentComposerStrip", h.comp("AgentComposerStrip"));
vi.mock("./components/AgentViewPopover", h.comp("AgentViewPopover"));
vi.mock("./components/SlashHelpPanel", h.comp("SlashHelpPanel"));
vi.mock("./components/SlashCommandPicker", h.comp("SlashCommandPicker"));
vi.mock("./components/AgentFooter", h.comp("AgentFooter"));
vi.mock("./components/AgentShellDrawer", h.comp("AgentShellDrawer"));

import { AgentPresentationView } from "./agent-view";
import { seen } from "./agent-view.wiring.stub";

const [helpVisible, setHelpVisible] = createSignal(false);
const [pickerSpec, setPickerSpec] = createSignal<unknown>(undefined);
const [btw, setBtw] = createSignal<unknown>(undefined);

/** One line of the trace: a call's name and its plain-data arguments. */
const fmt = (a: unknown): string =>
    typeof a === "function" ? "fn" : a === h.fns.paneModel ? "<paneModel>" : a === undefined ? "undefined" : a === Infinity ? "Infinity" : (JSON.stringify(a) ?? "undefined");
/** A spy that writes `name(args)` into the trace and answers `ret`. */
const spy = (name: string, ret?: unknown) =>
    vi.fn((...a: unknown[]) => {
        h.trace.push(`${name}(${a.map(fmt).join(", ")})`);
        return ret;
    });

/** What the live feed, the history hook and the launch flow report; the callback tests move these. */
const live = { on: false, following: true, canPage: false, turns: 0 };
const hist = { offset: 0, foreign: false };
const launchState = { loading: false, phase: null as string | null, waiting: false };
const searchState = { visible: false };
const [authSig, setAuthSig] = createSignal("unknown");
const doc = [{ id: "n1" }];
const corner = { visibility: "active", region: false, popoverOpen: false };
let lastUser: unknown = null;

function freshFns() {
    h.calls.length = 0;
    h.trace.length = 0;
    Object.assign(live, { on: false, following: true, canPage: false, turns: 0 });
    Object.assign(hist, { offset: 0, foreign: false });
    Object.assign(launchState, { loading: false, phase: null, waiting: false });
    searchState.visible = false;
    setAuthSig("unknown");
    Object.assign(corner, { visibility: "active", region: false, popoverOpen: false });
    lastUser = null;
    for (const k of Object.keys(seen)) delete seen[k];
    h.fns.copyMenu = vi.fn();
    h.fns.openHistory = vi.fn();
    h.fns.visibility = () => corner.visibility;
    h.fns.region = () => corner.region;
    h.fns.pass = (name: string, x: unknown, ...rest: unknown[]) => {
        h.trace.push(`${name}(${[x, ...rest].map(fmt).join(", ")})`);
        return x;
    };
    h.fns.setDocState = (u: (p: object) => object) => h.trace.push(`setDocState -> ${JSON.stringify(u({ keep: 1, a: 0 }))}`);
    h.fns.askSide = spy("askSideQuestion");
    h.fns.quickFork = spy("quickForkAgent");
    h.fns.clearFailure = spy("clearFailure");
    h.fns.lastUser = () => lastUser;
    h.fns.termSize = (el: unknown) => {
        h.trace.push(`computeTermSizeFromEl(${el instanceof Element ? "." + String((el as Element).className).split(" ")[0] : String(el)})`);
        return { cols: 80, rows: 24 };
    };
    h.fns.log = spy("log");
    h.fns.paneModel = h.withAny({
        state: { turnPhase: "idle", detailsOpen: false, compacting: false, pending: [], failure: null, sessionTotals: null, lastContextModel: null },
        document: () => doc,
        dispatchPane: spy("dispatchPane"),
        dispatchDoc: spy("dispatchDoc"),
    });
    h.fns.offline = h.withAny({ submit: vi.fn() });
    h.fns.viewControls = {
        resolved: () => ({ view: "full" }),
        popoverOpen: () => corner.popoverOpen,
        setPopoverOpen: spy("viewControls.setPopoverOpen"),
        cycle: vi.fn(),
        followDefault: vi.fn(),
    };
    h.fns.commands = h.withAny({
        btwOverlay: btw,
        helpVisible,
        pickerSpec,
        availableCommands: () => [],
        closeHelp: vi.fn(),
        closeBtw: vi.fn(),
        sendWithState: spy("commands.sendWithState", Promise.resolve()),
        stopAgent: vi.fn(),
        escapeOnEmptyComposer: vi.fn(),
        recallLatestHeld: vi.fn(),
        completions: vi.fn(),
        resolvePicker: vi.fn(),
        dismissPicker: vi.fn(),
        flushPendingControllerRefresh: spy("commands.flushPendingControllerRefresh"),
        flushHeldMessages: vi.fn(),
        hasHeldMessages: () => false,
    });
    h.fns.settle = {};
    h.fns.liveFeed = {
        liveFeedOn: () => live.on,
        gapsBefore: () => ["gap"],
        earlierHistoryAvailable: () => false,
        earlierTurnsKnown: () => 0,
        scheduleRollOff: spy("liveFeed.scheduleRollOff"),
        followingBottom: () => live.following,
        canPageOlder: () => live.canPage,
        setFollowingBottom: spy("liveFeed.setFollowingBottom"),
        loadToolResult: () => {},
        get liveFeedTurns() {
            return live.turns;
        },
    };
    h.fns.history = { loadOlder: () => {}, loadingOlder: () => false, historyOffset: () => hist.offset, snapshotIsForeignBlock: () => hist.foreign };
    h.fns.reveal = {
        readiness: { phase: () => "ready" },
        startPaintWait: spy("reveal.startPaintWait"),
        connectLaunchStatus: (s: unknown) => h.trace.push(`reveal.connectLaunchStatus(status is the launch flow: ${s === h.fns.launch})`),
    };
    h.fns.launch = h.withAny({
        isLoading: () => launchState.loading,
        launchPhase: () => launchState.phase,
        authStatus: authSig,
        loginWaiting: () => launchState.waiting,
        startLaunchFlow: spy("launch.startLaunchFlow"),
        relogin: spy("launch.relogin"),
        bindExistingAccount: vi.fn(),
        setAuthUrl: vi.fn(),
        canRetry: vi.fn(),
        setAuthNotice: vi.fn(),
        notifyControllerHealthy: vi.fn(),
        forceControllerRefresh: vi.fn(),
        beginRecoveryFlow: vi.fn(),
        endRecoveryFlow: vi.fn(),
        isCancelled: vi.fn(),
        resetCancelled: vi.fn(),
    });
    h.fns.startup = spy("startup");
    h.fns.turn = {
        turnJustEndedAtom: () => false,
        reconcileTurnActive: spy("reconcileTurnActive"),
        trackTurnJustEnded: spy("trackTurnJustEnded"),
        isBackendTurnActive: () => false,
        isBackendTurnConfirmedIdle: () => false,
    };
    h.fns.account = h.withAny({
        authEmail: () => null,
        canSwitchAccount: () => false,
        pendingAccountLabel: () => null,
        refreshLinkedAccountId: vi.fn(),
        onSwitchAccount: vi.fn(),
    });
    h.fns.authHealth = { declareAuthHealthy: spy("declareAuthHealthy") };
    h.fns.failureUI = { row: () => null };
    h.fns.decisions = { pendingDecisions: () => [], handleDecide: vi.fn() };
    h.fns.questions = { pendingQuestions: () => [], handleAnswer: vi.fn(), handleCancel: vi.fn() };
    h.fns.search = {
        visible: () => searchState.visible,
        setVisible: spy("search.setVisible"),
        performSearch: () => {},
        next: () => {},
        prev: () => {},
        close: spy("search.close"),
        currentIndex: () => 0,
        matchCount: () => 0,
        highlightId: () => null,
    };
    h.fns.scroll = { jumpTo: () => {}, command: () => null };
    h.fns.bridge = { log: h.fns.log, onTermReady: () => {}, onTermDispose: () => {}, clearTermWrite: () => {}, withShellFocus: (f: unknown) => f };
    h.fns.drawer = { handleShellExited: () => {} };
    h.fns.working = { paneBusy: () => false, workingRowVisible: () => false };
    h.fns.bgTasks = () => [];
}

function mount(metaOver: Record<string, unknown> = {}) {
    const block = () => ({
        meta: { agentProvider: "claude", agentName: "Korp", agentOutputFormat: "claude-stream-json", "cmd:cwd": "/work", agentMode: "host", ...metaOver },
    });
    const model = { blockId: "blk-12345678", blockAtom: block, _openModal: null, backToPicker: spy("model.backToPicker") } as any;
    const view = render(() => (
        <AgentPresentationView model={model} agentId="korp" agentDefinitions={() => [{ id: "korp" } as any]} progressBarMount={() => undefined} />
    ));
    return Object.assign(view, { model });
}

/** The drawn tree: one line per element, indented, with its class or stub props. */
function tree(el: Element, depth = 0): string {
    const stubName = el.getAttribute("data-stub");
    const attrs = [...el.attributes]
        .filter((a) => !["class", "data-stub", "data-props"].includes(a.name))
        .map((a) => ` ${a.name}="${a.value}"`)
        .join("");
    const label = stubName ? `<${stubName} ${el.getAttribute("data-props")}>` : `<${el.tagName.toLowerCase()}${el.className ? `.${String(el.className).split(" ").join(".")}` : ""}${attrs}>`;
    return ["  ".repeat(depth) + label, ...[...el.children].map((c) => tree(c, depth + 1))].join("\n");
}

beforeEach(() => {
    freshFns();
    setHelpVisible(false);
    setPickerSpec(undefined);
    setBtw(undefined);
});
afterEach(() => cleanup());

describe("AgentPresentationView wiring", () => {
    it("calls its hooks in this order, with these options", async () => {
        mount();
        await expect(h.calls.join("\n") + "\n").toMatchFileSnapshot("./__snapshots__/agent-view.wiring.hooks.txt");
    });

    it("draws the pane: the cover, shutdown log, progress bar, transcript, panels, strip, composer and shell drawer", async () => {
        const v = mount();
        await expect(tree(v.container.firstElementChild!) + "\n").toMatchFileSnapshot("./__snapshots__/agent-view.wiring.tree.txt");
    });

    it("shows the /btw overlay, the help panel and the picker only while they are asked for", async () => {
        const v = mount();
        expect(v.container.querySelector('[data-stub="BtwOverlay"]')).toBeNull();
        expect(v.container.querySelector('[data-stub="SlashHelpPanel"]')).toBeNull();
        expect(v.container.querySelector('[data-stub="SlashCommandPicker"]')).toBeNull();
        setBtw({ askId: "a1", question: "why?", requestId: "r1", error: undefined });
        setHelpVisible(true);
        setPickerSpec({ kind: "x" });
        await expect(tree(v.container.firstElementChild!) + "\n").toMatchFileSnapshot("./__snapshots__/agent-view.wiring.tree-overlays.txt");
    });

    it("hands the composer's callbacks to the commands and the offline queue", () => {
        mount();
        const f = seen.AgentFooter;
        expect(f.onSendMessage).toBe(h.fns.offline.submit);
        f.onStopAgent();
        expect(h.fns.commands.escapeOnEmptyComposer).toHaveBeenCalledTimes(1);
        f.onRecallLatestQueued();
        expect(h.fns.commands.recallLatestHeld).toHaveBeenCalledTimes(1);
        expect(f.getCompletions).toBe(h.fns.commands.completions);
    });

    it("typing in the composer scrolls the list to the bottom once the list has registered its scroller", () => {
        mount();
        const scroll = vi.fn();
        seen.AgentFooter.onTyping(); // before the list registers: nothing to call
        seen.AgentDocumentView.scrollToBottomRef(scroll);
        seen.AgentFooter.onTyping();
        expect(scroll).toHaveBeenCalledWith("typing");
        seen.NewActivityPill.onJump();
        expect(scroll).toHaveBeenCalledWith("new-activity-pill");
    });

    it("the strip's Compact sends /compact; the help panel sends the command it names and closes", () => {
        setHelpVisible(true);
        mount();
        seen.AgentComposerStrip.onCompact();
        expect(h.fns.commands.sendWithState).toHaveBeenCalledWith("/compact");
        seen.SlashHelpPanel.onInvoke({ name: "login" });
        expect(h.fns.commands.closeHelp).toHaveBeenCalledTimes(1);
        expect(h.fns.commands.sendWithState).toHaveBeenLastCalledWith("/login");
    });

    it("opens the history tab from the transcript, and wires the panels' Stop to the commands", () => {
        mount();
        seen.AgentDocumentView.onOpenHistory();
        expect(h.fns.openHistory).toHaveBeenCalledWith({ currentBlockId: "blk-12345678", agentId: "korp" });
        expect(seen.AgentBottomPanels.onStop).toBe(h.fns.commands.stopAgent);
        expect(seen.AgentBottomPanels.agentId).toBe("korp");
        expect(seen.AgentBottomPanels.agentName).toBe("Korp");
    });

    it("right-click copies the selected text, and leaves the menu alone with none", () => {
        const v = mount();
        const root = v.container.querySelector(".agent-view") as HTMLElement;
        fireEvent.contextMenu(root);
        expect(h.fns.copyMenu).not.toHaveBeenCalled();
        const sel = vi.spyOn(window, "getSelection").mockReturnValue({ toString: () => "picked words" } as unknown as Selection);
        fireEvent.contextMenu(root);
        expect(h.fns.copyMenu).toHaveBeenCalledWith([{ label: "Copy", value: "picked words" }], expect.anything());
        sel.mockRestore();
    });

    it("gives the modal layer to the model while mounted and takes it back on unmount", () => {
        const model = { blockId: "b", blockAtom: () => ({ meta: {} }), _openModal: "unset", backToPicker: () => {} } as any;
        const r = render(() => <AgentPresentationView model={model} agentId="korp" agentDefinitions={() => []} progressBarMount={() => undefined} />);
        expect(model._openModal).toBeTypeOf("function");
        r.unmount();
        expect(model._openModal).toBeNull();
    });
});

/** The first argument the hook was last called with. */
const opts = (name: string): any => h.argsOf[name][0];

describe("AgentPresentationView callbacks", () => {
    it("mounts: the document passes through its injections in order, then the pane logs its choice and starts the launch", async () => {
        mount();
        await expect(h.trace.join("\n") + "\n").toMatchFileSnapshot("./__snapshots__/agent-view.wiring.mount.txt");
    });

    it("reads the live feed and the history state when the history hook asks, and reports back where it finished", async () => {
        const T = h.trace;
        const note = (label: string, v: unknown) => T.push(`${label} = ${fmt(v)}`);
        mount();
        T.length = 0;
        const H = opts("useHistoryPagination");
        const read = (title: string) => {
            T.push(`> ${title}`);
            note("restoreTurns", H.restoreTurns());
            note("restoreBytes", H.restoreBytes());
            note("fillPaused", H.fillPaused());
        };
        read("feed off, following");
        live.on = true;
        live.turns = 3;
        read("feed on, 3 turns");
        live.turns = Infinity;
        read("feed on, no turn cap");
        live.following = false;
        read("feed on, no cap, reader scrolled up");
        T.push("> fixed options");
        note("agentName", H.agentName());
        note("outputFormat", H.outputFormat());
        note("definitionId", H.definitionId);
        note("blockId", H.blockId);
        note("model is the pane model", H.model === h.fns.paneModel);
        note("log is the bridge's", H.log === h.fns.log);
        note("settle latch is the stream's", H.transcriptSettle === opts("useAgentStream").transcriptSettle && H.transcriptSettle === h.fns.settle);
        T.push("> onHistoryReady before the list registers");
        H.onHistoryReady();
        T.push("> the list registers, onHistoryReady again");
        seen.AgentDocumentView.registerHistoryReadyCallback(spy("historyReady"));
        H.onHistoryReady();
        T.push("> onSnapshotOverlay");
        H.onSnapshotOverlay({ documentState: { a: 1 }, detailsOpen: true });
        H.onSnapshotOverlay({ documentState: { b: 2 }, detailsOpen: false });
        H.onSnapshotOverlay({ documentState: { c: 3 }, detailsOpen: undefined });
        cleanup();
        live.turns = 3;
        mount({ agentOutputFormat: "other-json" });
        T.push("> another output format, feed on, 3 turns");
        note("restoreTurns", opts("useHistoryPagination").restoreTurns());
        await expect(T.join("\n") + "\n").toMatchFileSnapshot("./__snapshots__/agent-view.wiring.history.txt");
    });

    it("hands every hook callbacks that reach the right place", async () => {
        const T = h.trace;
        const F = h.fns;
        const view = mount();
        const root = view.container.querySelector(".agent-view") as HTMLElement;
        Object.defineProperty(root, "offsetWidth", { value: 321 });
        T.length = 0;
        const now = vi.spyOn(Date, "now").mockReturnValue(1234);
        const step = (s: string) => T.push(`> ${s}`);
        const same = (label: string, a: unknown, b: unknown) => T.push(`${label}: ${a === b ? "same" : "DIFFERENT"}`);
        const note = (label: string, v: unknown) => T.push(`${label} = ${fmt(v)}`);
        const sameAll = (o: any, against: any, keys: string[]) => keys.forEach((k) => same(k, o[k], against[k]));

        step("questions, turn confirmation and the turn-end sweep");
        opts("useAgentQuestions").sendMessage("hello");
        opts("createTurnConfirmation").reconcile(true);
        opts("createTurnConfirmation").onTurnEnded();
        same("turnEnded", opts("useTurnEndSweep").turnEnded, F.turn.turnJustEndedAtom);
        opts("useTurnEndSweep").onTurnEnded();

        step("the launch flow's inner hooks");
        const U = opts("startupOnReady");
        U.send("opening");
        note("peerAgents", U.peerAgents());
        note("currentAgent", U.currentAgent());
        note("providerDisplayName", U.providerDisplayName());
        const S = opts("useAgentControllerStatus");
        same("onReady", S.onReady, F.startup);
        S.onRecovered();
        lastUser = { text: "again", attachments: [{ n: 1 }], system: false };
        step("recovered, a message to resend");
        S.onRecovered();
        launchState.waiting = true;
        step("recovered while another flow runs");
        S.onRecovered();
        launchState.waiting = false;
        S.getInitialTermSize();
        S.onControllerStatus({ turn_active: true });
        S.onControllerStatus({});

        step("controller events and refocus");
        const E = opts("useControllerStatusEvents");
        E.onTurnActive(true);
        E.onTurnActive(false);
        E.onActiveTurnConfirmed();
        const R = opts("useFocusRepoll");
        note("windowFocused", R.windowFocused());
        same("trackTurnJustEnded", R.trackTurnJustEnded, F.turn.trackTurnJustEnded);
        R.flushPendingControllerRefresh();

        step("activity summary and next-prompt suggestion");
        note("turnPhase", opts("useAgentActivitySummary").turnPhase());
        note("rootWidth", opts("useAgentActivitySummary").getRootWidth());
        const N = opts("useNextPromptSuggestion");
        note("composer empty before the footer registers", N.isComposerEmpty());
        seen.AgentFooter.isComposerEmptyRef(() => false);
        note("composer empty after", N.isComposerEmpty());
        note("turnPhase", N.turnPhase());
        same("turnJustEndedAtom", N.turnJustEndedAtom, F.turn.turnJustEndedAtom);
        same("document", N.document, F.paneModel.document);

        step("narration");
        (h.argsOf.useAmbientNarration[1] as any)({ id: "amb" });
        (h.argsOf.useBackgroundHandoffNarration[1] as any)([{ id: "bg" }]);
        same("handoff document", h.argsOf.useBackgroundHandoffNarration[0], F.paneModel.document);

        step("the stream, before and after the list registers its scroller");
        const ST = opts("useAgentStream");
        note("outputFormat", ST.outputFormat);
        note("provider", ST.provider);
        note("agentName", ST.agentName);
        note("enabled", ST.enabled);
        note("compacting", ST.compacting());
        note("turnPhase", ST.turnPhase());
        note("pendingMessages", ST.pendingMessages());
        same("documentNodes", ST.documentNodes, F.paneModel.document);
        ST.onTurnStartFromQueue();
        ST.onPendingLeft();
        seen.AgentDocumentView.scrollToBottomRef(spy("scrollToBottom"));
        ST.onTurnStartFromQueue();
        ST.onPendingLeft();

        step("the commands");
        const K = opts("useAgentCommands");
        note("showingLaunchActivity idle", K.showingLaunchActivity());
        launchState.phase = "relogin";
        note("showingLaunchActivity in relogin", K.showingLaunchActivity());
        launchState.phase = null;
        launchState.loading = true;
        note("showingLaunchActivity loading", K.showingLaunchActivity());
        launchState.loading = false;
        sameAll(K, F.launch, ["setAuthUrl", "canRetry", "loginWaiting", "setAuthNotice", "notifyControllerHealthy", "forceControllerRefresh", "beginRecoveryFlow", "endRecoveryFlow", "isCancelled", "resetCancelled"]);
        sameAll(K, F.turn, ["isBackendTurnActive", "isBackendTurnConfirmedIdle"]);
        same("block", K.block, view.model.blockAtom);
        same("model", K.model, F.paneModel);
        same("documentNodes", K.documentNodes, F.paneModel.document);
        same("log", K.log, F.log);
        note("provider", K.provider().id);
        note("pendingMessages", K.pendingMessages());
        K.backToPicker();
        K.quickFork();
        K.askSideQuestion("why");
        K.onSent();

        step("offline input, the sign-in row, the account binding and the auth health");
        const OF = opts("useOfflineInput");
        note("agentId", OF.agentId());
        note("provider", OF.provider().id);
        same("send", OF.send, F.commands.sendWithState);
        same("stop", OF.stop, F.commands.stopAgent);
        same("canRetry", opts("useSyntheticAuthRow").canRetry, F.launch.canRetry);
        const AB = opts("useAgentAccountBinding");
        note("providerId", AB.providerId());
        same("bindExistingAccount", AB.bindExistingAccount, F.launch.bindExistingAccount);
        note("signedOut", AB.signedOut());
        setAuthSig("unauthenticated");
        note("signedOut after sign-out", AB.signedOut());
        setAuthSig("unknown");
        AB.onLogin();
        F.paneModel.state.failure = { turnAttempted: true };
        AB.onLogin();
        F.paneModel.state.failure = null;
        const AH = opts("useAuthHealth");
        note("agentDefinitionId", AH.agentDefinitionId());
        same("status", AH.status, F.launch);
        same("refreshLinkedAccountId", AH.refreshLinkedAccountId, F.account.refreshLinkedAccountId);

        step("the failure row and held messages");
        const FR = opts("useFailureRow");
        sameAll(FR, { model: view.model, paneModel: F.paneModel, status: F.launch, account: F.account, offline: F.offline, send: F.commands.sendWithState, log: F.log }, ["model", "paneModel", "status", "account", "offline", "send", "log"]);
        lastUser = null;
        FR.retryLastTurn();
        lastUser = { text: "again", attachments: [], system: true };
        FR.retryLastTurn();
        const HM = opts("useHeldMessageDelivery");
        same("hasHeldMessages", HM.hasHeldMessages, F.commands.hasHeldMessages);
        same("flushHeldMessages", HM.flushHeldMessages, F.commands.flushHeldMessages);
        HM.flushPendingControllerRefresh();

        step("search, keyboard and scrolling");
        same("jumpTo", opts("useInSessionSearch").jumpTo, F.scroll.jumpTo);
        same("document", opts("useInSessionSearch").document, F.paneModel.document);
        const KB = opts("useAgentKeyboard");
        KB.onToggleSearch();
        searchState.visible = true;
        KB.onToggleSearch();
        sameAll(KB, { onCycleView: F.viewControls.cycle, onFollowDefaultView: F.viewControls.followDefault }, ["onCycleView", "onFollowDefaultView"]);

        step("the root element, snapshot persistence and the shell");
        same("drop-attach root", opts("useAgentDropAttach").rootRef(), root);
        same("pty width root", opts("usePtyWidth").elementRef(), root);
        same("pty width log", opts("usePtyWidth").log, F.log);
        const SP = opts("useSnapshotPersistence");
        same("atoms are stable", SP.getAtoms(), SP.getAtoms());
        note("detailsOpen", SP.getDetailsOpen());
        hist.foreign = true;
        note("snapshotIsForeignBlock", SP.snapshotIsForeignBlock());
        same("getDocument", SP.getDocument, F.paneModel.document);
        note("hidden", SP.hidden());
        note("definitionId", SP.definitionId);
        same("log", SP.log, F.log);
        same("decisions document", opts("useAgentDecisions").getDocument, F.paneModel.document);
        same("questions document", opts("useAgentQuestions").getDocument, F.paneModel.document);
        const SD = opts("useShellDrawerLifecycle");
        note("shellSubBlockId", SD.shellSubBlockId());
        same("clearTermWrite", SD.clearTermWrite, F.bridge.clearTermWrite);
        same("paneModel", SD.paneModel, F.paneModel);
        same("context reading model", h.argsOf.useContextReading[1], F.paneModel);
        same("context reading block", h.argsOf.useContextReading[2], view.model.blockAtom);
        same("turn ledger model", h.argsOf.useTurnLedger[1], F.paneModel);
        const LF = opts("useLiveFeedRollOff");
        same("live feed history", LF.history, F.history);
        same("live feed paneModel", LF.paneModel, F.paneModel);
        same("live feed block", LF.block, view.model.blockAtom);
        note("live feed hidden", LF.hidden());

        step("the strip");
        const SG = seen.AgentComposerStrip;
        SG.onToggleLog();
        same("onSwitchAccount", SG.onSwitchAccount, F.account.onSwitchAccount);
        SG.onOpenView();

        step("the transcript");
        const TR = seen.AgentDocumentView;
        TR.onAgentErrorLogin();
        same("onLoadOlder", TR.onLoadOlder, F.history.loadOlder);
        same("loadingOlder", TR.loadingOlder, F.history.loadingOlder);
        note("hasOlderHistory, nothing to page", TR.hasOlderHistory());
        live.canPage = true;
        note("hasOlderHistory, nothing older", TR.hasOlderHistory());
        hist.offset = 4;
        note("hasOlderHistory, paging allowed", TR.hasOlderHistory());
        live.canPage = false;
        note("hasOlderHistory, paging not allowed", TR.hasOlderHistory());
        TR.followingRef(true);
        same("highlightNodeId", TR.highlightNodeId, F.search.highlightId);
        same("scrollCommand", TR.scrollCommand, F.scroll.command);
        note("zoomFactor", TR.zoomFactor());
        note("new activity before the list reports", seen.NewActivityPill.visible());
        TR.newActivityRef(() => true);
        note("new activity after", seen.NewActivityPill.visible());
        seen.NewActivityPill.onJump();
        seen.AgentFooter.onTyping();
        const SB = seen.AgentSearchBar;
        same("search visible", SB.visible, F.search.visible);
        same("search onSearch", SB.onSearch, F.search.performSearch);
        same("search onNext", SB.onNext, F.search.next);
        same("search onPrev", SB.onPrev, F.search.prev);
        same("search onClose", SB.onClose, F.search.close);
        same("search matchIndex", SB.matchIndex, F.search.currentIndex);
        same("search matchCount", SB.matchCount, F.search.matchCount);

        step("the panels, the preparing row and the shell drawer");
        const BP = seen.AgentBottomPanels;
        sameAll(BP, { ...F.decisions, ...F.questions, pendingMessages: BP.pendingMessages }, ["pendingDecisions", "pendingQuestions"]);
        same("onDecide", BP.onDecide, F.decisions.handleDecide);
        same("onAnswer", BP.onAnswer, F.questions.handleAnswer);
        same("onCancel", BP.onCancel, F.questions.handleCancel);
        same("failureRow", BP.failureRow, F.failureUI.row);
        same("backgroundTasksAtom", BP.backgroundTasksAtom, F.bgTasks);
        same("workingRowVisible", BP.workingRowVisible, F.working.workingRowVisible);
        same("workingRowLoading", BP.workingRowLoading, F.working.paneBusy);
        same("status", BP.status, F.launch);
        same("offline", BP.offline, F.offline);
        same("paneModel", BP.paneModel, F.paneModel);
        same("log", BP.log, F.log);
        note("pendingMessages", BP.pendingMessages());
        note("enabled", seen.AgentPreparingRow.enabled());
        note("nodes", seen.AgentPreparingRow.nodes());
        const DR = seen.AgentShellDrawer;
        sameAll(DR, F.bridge, ["onTermReady", "onTermDispose"]);
        same("onShellExited", DR.onShellExited, F.drawer.handleShellExited);

        now.mockRestore();
        await expect(T.join("\n") + "\n").toMatchFileSnapshot("./__snapshots__/agent-view.wiring.callbacks.txt");
    });

    it("handles the corners: a hidden tab, the modal layer, an auth failure, an unknown provider, a menu region, the view popover and the view controls' hooks", async () => {
        const T = h.trace;
        const F = h.fns;
        const note = (label: string, v: unknown) => T.push(`${label} = ${fmt(v)}`);
        const step = (s: string) => T.push(`> ${s}`);

        mount();
        step("hidden follows the tab's visibility");
        for (const v of ["active", "hidden", "window-hidden"]) {
            corner.visibility = v;
            note(v, (h.argsOf.useRenderDormant[0] as any)());
        }
        const VC = opts("createAgentViewControls");
        step("the view controls read the document and write its state");
        note("nodes", VC.nodes());
        VC.setDocumentState((p: object) => ({ ...p, view: "x" }));
        cleanup();

        step("the model's modal opener: with a layer, then without");
        const model = { blockId: "blk-12345678", blockAtom: () => ({ meta: {} }), _openModal: null, backToPicker: () => {} } as any;
        const layer = { open: spy("modalLayer.open") } as any;
        const withLayer = render(() => (
            <ModalLayerContext.Provider value={layer}>
                <AgentPresentationView model={model} agentId="korp" agentDefinitions={() => []} progressBarMount={() => undefined} />
            </ModalLayerContext.Provider>
        ));
        model._openModal({ kind: "identity" });
        withLayer.unmount();
        render(() => <AgentPresentationView model={model} agentId="korp" agentDefinitions={() => []} progressBarMount={() => undefined} />);
        note("opener without a layer", model._openModal({ kind: "identity" }));
        cleanup();

        step("an auth failure overrides the durable sign-in status");
        F.paneModel.state.failure = { auth: true };
        mount();
        note("strip authStatus, failing", seen.AgentComposerStrip.authStatus);
        cleanup();
        F.paneModel.state.failure = null;
        setAuthSig("authenticated");
        mount();
        note("strip authStatus, signed in", seen.AgentComposerStrip.authStatus);
        cleanup();

        step("a provider the registry doesn't know");
        mount({ agentProvider: "unknown" });
        note("authProviderId", seen.AgentBottomPanels.authProviderId);
        note("panels providerId", seen.AgentBottomPanels.providerId);
        note("strip providerId", seen.AgentComposerStrip.providerId);
        cleanup();

        step("a context-menu region under the click keeps the copy menu away");
        const v = mount();
        const rootEl = v.container.querySelector(".agent-view") as HTMLElement;
        corner.region = true;
        const sel = vi.spyOn(window, "getSelection").mockReturnValue({ toString: () => "picked" } as unknown as Selection);
        fireEvent.contextMenu(rootEl);
        note("copy menu calls", F.copyMenu.mock.calls.length);
        sel.mockRestore();
        cleanup();

        step("the view popover hangs from the strip when the pane has no chip");
        corner.popoverOpen = true;
        mount();
        note("strip viewOpen", seen.AgentComposerStrip.viewOpen);
        T.push(`popover controls: ${seen.AgentViewPopover.controls === F.viewControls ? "same" : "DIFFERENT"}`);
        note("popover anchor", seen.AgentViewPopover.anchor);
        await expect(T.join("\n") + "\n").toMatchFileSnapshot("./__snapshots__/agent-view.wiring.corners.txt");
    });
});
