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
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => {
    const calls: string[] = [];
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
            return typeof ret === "function" ? (ret as (...a: unknown[]) => unknown)(...args) : ret;
        };
    /** A module factory for one stand-in child component (agent-view.wiring.stub.tsx). */
    const comp = (name: string, withChildren = false) => async () => ({ [name]: (await import("./agent-view.wiring.stub")).stub(name, withChildren) });
    return { calls, fns, any, withAny, hook, comp };
});

// ---- the model and what its hooks return ----
vi.mock("@/app/block/pane-tab-visibility", () => ({ usePaneTabVisibility: h.hook("usePaneTabVisibility", () => "active") }));
vi.mock("@/app/block/context-menu-region", () => ({ resolveContextMenuRegion: () => false }));
vi.mock("@/app/store/contextmenu", () => ({ showCopyContextMenu: (...a: unknown[]) => h.fns.copyMenu(...a) }));
vi.mock("@/app/store/global", () => ({ getBlockMetaKeyAtom: () => () => undefined }));
vi.mock("@/app/store/pane-zoom-local", () => ({ paneZoom: () => 1 }));
vi.mock("@/app/store/agent-pane-state/types", () => ({ isAuthFailure: () => false, isStopping: () => false }));
vi.mock("@/app/window/window-focus", () => ({ makeWindowFocusSignal: () => () => true }));
vi.mock("../../agent-model/live-feed", () => ({ LIVE_FEED_RESTORE_BYTES: 1000 }));
vi.mock("../../agent-model/state", () => ({ createAgentAtoms: h.hook("createAgentAtoms", () => ({ documentStateAtom: [() => ({}), () => {}] })) }));
vi.mock("./agent-view-commands", () => ({ registerAgentViewCommands: () => {} }));
vi.mock("./agent-view-controls", () => ({
    createAgentViewControls: h.hook("createAgentViewControls", () => h.fns.viewControls),
    registerAgentViewControls: h.hook("registerAgentViewControls", () => () => {}),
    viewChipLabel: () => "",
}));
vi.mock("./activity/promotion-clock", () => ({ createPromotionClock: h.hook("createPromotionClock", () => () => 0) }));
vi.mock("./activity/useAttachedTaskAxis", () => ({ useAttachedTaskAxis: h.hook("useAttachedTaskAxis") }));
vi.mock("./btw", () => ({ askSideQuestion: () => {} }));
vi.mock("./quick-fork", () => ({ quickForkAgent: () => {} }));
vi.mock("./open-history-tab", () => ({ openOrFocusHistoryTab: (...a: unknown[]) => h.fns.openHistory(...a) }));
vi.mock("./providers", () => ({ getProvider: () => ({ id: "claude", displayName: "Claude" }) }));
vi.mock("./render-dormant", () => ({ useRenderDormant: h.hook("useRenderDormant", () => () => false) }));
vi.mock("./sign-in-notice", () => ({ createSignInNotice: h.hook("createSignInNotice", () => ({ answered: () => {}, needed: () => {}, bound: () => {} })) }));
vi.mock("./system-notification", () => ({ createSystemNotifier: h.hook("createSystemNotifier", () => () => {}) }));
vi.mock("./startup/startupOnReady", () => ({ startupOnReady: h.hook("startupOnReady", () => () => {}) }));
vi.mock("./transcript-cursor", () => ({ createTranscriptSettleLatch: h.hook("createTranscriptSettleLatch", () => ({})) }));
vi.mock("./inject-history-link", () => ({ injectGapRows: (x: unknown) => x, injectHistoryLink: (x: unknown) => x }));
vi.mock("./inject-resume-preflight", () => ({ buildResumePreflightNode: () => null, injectResumePreflight: (x: unknown) => x }));
vi.mock("./virtualization/verbosity", () => ({ deriveDisplayNodes: (x: unknown) => x }));
vi.mock("./failure/failure-writer", () => ({ clearFailure: () => {} }));
vi.mock("./failure/resolved-auth-errors", () => ({ markResolvedAuthErrors: (x: unknown) => x }));
vi.mock("./failure/useAgentAccountBinding", () => ({
    useAgentAccountBinding: h.hook("useAgentAccountBinding", () => h.withAny({ authEmail: () => null, canSwitchAccount: () => false, pendingAccountLabel: () => null })),
}));
vi.mock("./failure/useAuthHealth", () => ({
    useAuthHealth: h.hook("useAuthHealth", () => ({ declareAuthHealthy: () => {} })),
    useSyntheticAuthRow: h.hook("useSyntheticAuthRow"),
}));
vi.mock("./failure/useFailureRow", () => ({ useFailureRow: h.hook("useFailureRow", () => ({ row: () => null })) }));
vi.mock("./offline/useOfflineInput", () => ({
    lastUserMessage: () => null,
    useOfflineInput: h.hook("useOfflineInput", () => h.fns.offline),
}));
vi.mock("./hooks/turn-confirmation", () => ({
    createTurnConfirmation: h.hook("createTurnConfirmation", () => ({
        turnJustEndedAtom: () => false,
        reconcileTurnActive: () => {},
        trackTurnJustEnded: () => {},
        isBackendTurnActive: () => false,
        isBackendTurnConfirmedIdle: () => false,
    })),
}));
vi.mock("./hooks/useAgentActivitySummary", () => ({ useAgentActivitySummary: h.hook("useAgentActivitySummary") }));
vi.mock("./hooks/useAgentCommands", () => ({ useAgentCommands: h.hook("useAgentCommands", () => h.fns.commands) }));
vi.mock("./hooks/useAgentControllerStatus", () => ({
    useAgentControllerStatus: h.hook("useAgentControllerStatus", () =>
        h.withAny({ isLoading: () => false, launchPhase: () => null, authStatus: () => "unknown", loginWaiting: () => false, startLaunchFlow: () => {} }),
    ),
}));
vi.mock("./hooks/useAgentDecisions", () => ({ useAgentDecisions: h.hook("useAgentDecisions", () => ({ pendingDecisions: () => [], handleDecide: () => {} })) }));
vi.mock("./hooks/useAgentDropAttach", () => ({ useAgentDropAttach: h.hook("useAgentDropAttach") }));
vi.mock("./hooks/useAgentKeyboard", () => ({ useAgentKeyboard: h.hook("useAgentKeyboard") }));
vi.mock("./hooks/useAgentQuestions", () => ({
    useAgentQuestions: h.hook("useAgentQuestions", () => ({ pendingQuestions: () => [], handleAnswer: () => {}, handleCancel: () => {} })),
}));
vi.mock("./hooks/useAmbientNarration", () => ({ useAmbientNarration: h.hook("useAmbientNarration") }));
vi.mock("./hooks/useBackgroundHandoffNarration", () => ({ useBackgroundHandoffNarration: h.hook("useBackgroundHandoffNarration") }));
vi.mock("./hooks/useBlockActivity", () => ({ useBlockActivity: h.hook("useBlockActivity") }));
vi.mock("./hooks/useContextReading", () => ({
    useContextReading: h.hook("useContextReading", () => ({ reading: () => null, note: () => null, autoCompact: () => false })),
}));
vi.mock("./hooks/useControllerStatusEvents", () => ({ useControllerStatusEvents: h.hook("useControllerStatusEvents") }));
vi.mock("./hooks/useHistoryPagination", () => ({
    useHistoryPagination: h.hook("useHistoryPagination", () => ({
        loadOlder: () => {},
        loadingOlder: () => false,
        historyOffset: () => 0,
        snapshotIsForeignBlock: () => false,
    })),
}));
vi.mock("./hooks/useInSessionSearch", () => ({
    useInSessionSearch: h.hook("useInSessionSearch", () => ({
        visible: () => false,
        setVisible: () => {},
        performSearch: () => {},
        next: () => {},
        prev: () => {},
        close: () => {},
        currentIndex: () => 0,
        matchCount: () => 0,
        highlightId: () => null,
    })),
}));
vi.mock("./hooks/useLiveFeedRollOff", () => ({
    useLiveFeedRollOff: h.hook("useLiveFeedRollOff", () => ({
        liveFeedOn: () => false,
        gapsBefore: () => [],
        earlierHistoryAvailable: () => false,
        earlierTurnsKnown: () => 0,
        scheduleRollOff: () => {},
        followingBottom: () => true,
        canPageOlder: () => false,
        setFollowingBottom: () => {},
        loadToolResult: () => {},
        liveFeedTurns: 0,
    })),
}));
vi.mock("./hooks/useNextPromptSuggestion", () => ({ useNextPromptSuggestion: h.hook("useNextPromptSuggestion") }));
vi.mock("./hooks/usePaneRegistration", () => ({ usePaneRegistration: h.hook("usePaneRegistration", () => ({ paneModel: h.fns.paneModel, layoutView: () => null })) }));
vi.mock("./hooks/usePaneReveal", () => ({
    usePaneReveal: h.hook("usePaneReveal", () => ({ readiness: { phase: () => "ready" }, startPaintWait: () => {}, connectLaunchStatus: () => {} })),
}));
vi.mock("./hooks/usePtyWidth", () => ({ usePtyWidth: h.hook("usePtyWidth"), computeTermSizeFromEl: () => ({ cols: 80, rows: 24 }) }));
vi.mock("./hooks/useResumePreflight", () => ({ useResumePreflight: h.hook("useResumePreflight", () => ({ result: () => null, showSteps: () => false })) }));
vi.mock("./hooks/useScrollToNode", () => ({ useScrollToNode: h.hook("useScrollToNode", () => ({ jumpTo: () => {}, command: () => null })) }));
vi.mock("./hooks/useShellDrawerLifecycle", () => ({ useShellDrawerLifecycle: h.hook("useShellDrawerLifecycle", () => ({ handleShellExited: () => {} })) }));
vi.mock("./hooks/useShellLogBridge", () => ({
    useShellLogBridge: h.hook("useShellLogBridge", () => ({
        log: () => {},
        onTermReady: () => {},
        onTermDispose: () => {},
        clearTermWrite: () => {},
        withShellFocus: (f: unknown) => f,
    })),
}));
vi.mock("./hooks/useSnapshotPersistence", () => ({ useSnapshotPersistence: h.hook("useSnapshotPersistence") }));
vi.mock("./hooks/useTurnEndSweep", () => ({ useTurnEndSweep: h.hook("useTurnEndSweep") }));
vi.mock("./hooks/useTurnLedger", () => ({ useTurnLedger: h.hook("useTurnLedger") }));
vi.mock("./hooks/useTurnReconciliation", () => ({ useFocusRepoll: h.hook("useFocusRepoll"), useHeldMessageDelivery: h.hook("useHeldMessageDelivery") }));
vi.mock("./hooks/useWorkingIndicator", () => ({ useWorkingIndicator: h.hook("useWorkingIndicator", () => ({ paneBusy: () => false, workingRowVisible: () => false })) }));
vi.mock("./useAgentStream", () => ({ useAgentStream: h.hook("useAgentStream", () => () => []) }));

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

function freshFns() {
    h.calls.length = 0;
    for (const k of Object.keys(seen)) delete seen[k];
    h.fns.copyMenu = vi.fn();
    h.fns.openHistory = vi.fn();
    h.fns.paneModel = h.withAny({
        state: { turnPhase: "idle", detailsOpen: false, compacting: false, pending: [], failure: null, sessionTotals: null, lastContextModel: null },
        document: () => [],
        dispatchPane: vi.fn(),
        dispatchDoc: vi.fn(),
    });
    h.fns.offline = h.withAny({ submit: vi.fn() });
    h.fns.viewControls = {
        resolved: () => ({ view: "full" }),
        popoverOpen: () => false,
        setPopoverOpen: vi.fn(),
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
        sendWithState: vi.fn(async () => {}),
        stopAgent: vi.fn(),
        escapeOnEmptyComposer: vi.fn(),
        recallLatestHeld: vi.fn(),
        completions: vi.fn(),
        resolvePicker: vi.fn(),
        dismissPicker: vi.fn(),
    });
}

function mount() {
    const block = () => ({ meta: { agentProvider: "claude", agentName: "Korp", agentOutputFormat: "claude-stream-json", "cmd:cwd": "/work", agentMode: "host" } });
    const model = { blockId: "blk-12345678", blockAtom: block, _openModal: null, backToPicker: vi.fn() } as any;
    return render(() => (
        <AgentPresentationView model={model} agentId="korp" agentDefinitions={() => [{ id: "korp" } as any]} progressBarMount={() => undefined} />
    ));
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
