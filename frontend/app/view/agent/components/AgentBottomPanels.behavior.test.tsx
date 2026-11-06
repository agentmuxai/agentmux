// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Everything docked between the transcript and the composer, pinned as a whole ahead of splitting it into parts:
 * each scenario feeds AgentBottomPanels the pane's state (the turn phase, the pending decisions and questions, the
 * failure row, the offline input, the login) and logs what it draws, as a tree of stand-ins that show the props they
 * were given, and, in order, every callback pressed on them. The logs are snapshots recorded on the unsplit
 * component; the split must leave them byte for byte. (agent-view.wiring.test.tsx mocks this component; nothing else
 * pins what it hands each panel.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { calls } from "./agent-bottom-panels.behavior.stub";

const h = vi.hoisted(() => ({
    windowFocused: true,
    focusedBlock: "blk-1" as string | null,
    stand: async (name: string, spec?: object) => (await import("./agent-bottom-panels.behavior.stub")).stand(name, spec),
}));

vi.mock("../sensitive/SensitiveBanner", async () => ({ SensitiveBanner: await h.stand("SensitiveBanner") }));
vi.mock("../shutdown/ShutdownPendingBanner", async () => ({ ShutdownPendingBanner: await h.stand("ShutdownPendingBanner") }));
vi.mock("./ActivityDock", async () => ({ ActivityDock: await h.stand("ActivityDock") }));
vi.mock("./AgentCredentialsRevokedChip", async () => ({
    AgentCredentialsRevokedChip: await h.stand("AgentCredentialsRevokedChip", { read: ["flowRunning"] }),
}));
vi.mock("./AgentDecisionPanel", async () => ({
    AgentDecisionPanel: await h.stand("AgentDecisionPanel", { read: ["pending"], call: { onDecide: ["tool-1", "allow"], onDefer: [] } }),
}));
vi.mock("./AgentDisconnectedBanner", async () => ({
    AgentDisconnectedBanner: await h.stand("AgentDisconnectedBanner", { read: ["phase"], call: { onReconnect: [] } }),
}));
vi.mock("./DesktopDrivingBar", async () => ({ DesktopDrivingBar: await h.stand("DesktopDrivingBar") }));
vi.mock("./AgentDocumentView", async () => ({
    AgentAuthPanel: await h.stand("AgentAuthPanel", {
        read: ["authUrl", "authNotice", "launchPhase"],
        call: { onDismissAuthNotice: [], onCancelLogin: [], onUseTerminal: [] },
    }),
}));
vi.mock("./working-row", async () => ({ AgentWorkingRow: await h.stand("AgentWorkingRow", { call: { onCancelLogin: [] } }) }));
vi.mock("./ApiRetryRow", async () => ({ ApiRetryRow: await h.stand("ApiRetryRow", { call: { onStop: [] } }) }));
vi.mock("../offline/HeldInputPanel", async () => ({
    HeldInputPanel: await h.stand("HeldInputPanel", { call: { onSendNow: [], onCancel: [], onDismissNotice: ["n-1"] } }),
}));
vi.mock("./AgentQuestionPanel", async () => ({
    AgentQuestionPanel: await h.stand("AgentQuestionPanel", { read: ["pending"], call: { onAnswer: ["q-1", "yes"], onCancel: ["q-1"] } }),
}));
vi.mock("../pane-requests/PaneRequestDock", async () => ({ PaneRequestDock: await h.stand("PaneRequestDock") }));
vi.mock("./AgentSessionNotices", async () => ({ AgentSessionNotices: await h.stand("AgentSessionNotices", { read: ["blockAtom"] }) }));
vi.mock("./AgentStoppedBanner", async () => ({ AgentStoppedBanner: await h.stand("AgentStoppedBanner", { read: ["block"] }) }));
vi.mock("./ForkProviderFallbackBanner", async () => ({ ForkProviderFallbackBanner: await h.stand("ForkProviderFallbackBanner", { read: ["meta"] }) }));
vi.mock("./HistoryLoadFailedBanner", async () => ({ HistoryLoadFailedBanner: await h.stand("HistoryLoadFailedBanner", { read: ["phase"] }) }));
vi.mock("./PaneRow", async () => ({ PaneRow: await h.stand("PaneRow", { children: true }) }));
vi.mock("./TurnAwayDigest", async () => ({ TurnAwayDigest: await h.stand("TurnAwayDigest", { read: ["looking", "busy"] }) }));
vi.mock("./PendingMessagesPanel", async () => ({ PendingMessagesPanel: await h.stand("PendingMessagesPanel", { read: ["pendingMessages"] }) }));
vi.mock("@/app/store/focusManager", () => ({ focusManager: { blockFocusAtom: () => h.focusedBlock } }));
vi.mock("@/app/window/window-focus", () => ({ makeWindowFocusSignal: () => () => h.windowFocused }));
vi.mock("@/store/muxbus-delivery", () => ({ clockTime: Object.assign((ms: number) => `clock ${ms}`, { id: "clockTime" }) }));
vi.mock("@/app/store/activitySummary", () => ({
    readSwarmSummary: (meta: any) => {
        calls.push(`readSwarmSummary(${JSON.stringify(meta)})`);
        return meta?.summary ?? null;
    },
}));
vi.mock("../compaction-estimate", () => ({
    compactionModelKey: (meta: any, resolved: any) => `key(${meta?.model}, ${resolved})`,
}));

import { AgentBottomPanels } from "./AgentBottomPanels";

const T = calls;
const log = (line: string): void => void T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/AgentBottomPanels.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href)$/;
/** The panels as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim().replace(/\s+/g, " ");
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${a.value}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n"));
};
const press = (stand: string, cb: string) => {
    const el = host.querySelector(`[data-stand="${stand}"] > [data-cb="${cb}"]`) as HTMLElement | null;
    if (!el) throw new Error(`no ${stand}.${cb}`);
    fireEvent.click(el);
};

const phase = (kind: string, extra: Record<string, unknown> = {}) => ({ kind, ...extra });

/** Every input of the panels as a signal or a store, so a scenario can change it while they are mounted. */
function mount(initial: Record<string, any> = {}) {
    const [decisions, setDecisions] = createSignal<any[]>(initial.decisions ?? []);
    const [questions, setQuestions] = createSignal<any[]>(initial.questions ?? []);
    const [pendingMessages, setPendingMessages] = createSignal<any[]>(initial.pendingMessages ?? []);
    const [failureRow, setFailureRow] = createSignal<any>(initial.failureRow ?? null);
    const [visible, setVisible] = createSignal<boolean>(initial.visible ?? false);
    const [loading, setLoading] = createSignal<boolean>(initial.loading ?? false);
    const [block, setBlock] = createSignal<any>(initial.block ?? { meta: { model: "sonnet" } });
    const [authUrl, setAuthUrl] = createSignal<string | null>(initial.authUrl ?? null);
    const [authNotice, setAuthNotice] = createSignal<string | null>(initial.authNotice ?? null);
    const [launchPhase, setLaunchPhase] = createSignal<string>(initial.launchPhase ?? "idle");
    const [flowRunning, setFlowRunning] = createSignal<boolean>(initial.flowRunning ?? false);
    const [apiRetry, setApiRetry] = createSignal<any>(initial.apiRetry ?? null);
    const [held, setHeld] = createSignal<any>(initial.held ?? []);
    const [notices, setNotices] = createSignal<any>(initial.notices ?? []);
    const [state, setState] = createStore<Record<string, any>>({
        turnPhase: phase("Idle"),
        initPhase: "Ready",
        turnTokens: 0,
        turnLedger: null,
        turnCarry: null,
        activity: null,
        sessionStats: null,
        compacting: false,
        context: null,
        lastContextModel: null,
        reconnecting: false,
        ...(initial.state ?? {}),
    });
    const paneModel = {
        state,
        document: initial.document ?? [],
        dispatchPane: (action: unknown, who: string) => log(`paneModel.dispatchPane(${JSON.stringify(action)}, ${who})`),
    };
    const props = {
        blockId: "blk-1",
        agentId: "agent-7",
        agentName: "Seven",
        authProviderId: "claude",
        providerId: "claude-code",
        block,
        paneModel,
        status: {
            authUrl,
            authNotice,
            launchPhase,
            flowRunning,
            setAuthNotice: (v: unknown) => log(`status.setAuthNotice(${JSON.stringify(v)})`),
            cancelLogin: () => log("status.cancelLogin()"),
            useTerminalInstead: () => log("status.useTerminalInstead()"),
        },
        log: (area: string, msg: string) => log(`log(${area}, ${msg})`),
        pendingDecisions: decisions,
        onDecide: (...a: unknown[]) => (log(`onDecide ran ${JSON.stringify(a)}`), "decided"),
        pendingQuestions: questions,
        onAnswer: (...a: unknown[]) => log(`onAnswer ran ${JSON.stringify(a)}`),
        onCancel: (...a: unknown[]) => log(`onCancel ran ${JSON.stringify(a)}`),
        pendingMessages,
        failureRow,
        backgroundTasksAtom: () => [],
        workingRowVisible: visible,
        workingRowLoading: loading,
        onStop: () => log("onStop ran"),
        offline: {
            apiRetry,
            held,
            notices,
            sendNow: async () => void log("offline.sendNow()"),
            cancel: () => log("offline.cancel()"),
            dismissNotice: (id: string) => log(`offline.dismissNotice(${id})`),
        },
    } as any;
    const view = render(() => <AgentBottomPanels {...props} />);
    host = view.container;
    return { setDecisions, setQuestions, setPendingMessages, setFailureRow, setVisible, setLoading, setBlock, setAuthUrl, setAuthNotice, setLaunchPhase, setFlowRunning, setApiRetry, setHeld, setNotices, setState };
}

beforeEach(() => {
    T.length = 0;
    h.windowFocused = true;
    h.focusedBlock = "blk-1";
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(1_700_000_000_000);
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

const DECISION = { toolName: "Bash", params: { description: "list the files" } };

describe("Agent bottom panels: what is docked, in order", () => {
    it("draws every panel in its place with what it is handed, the request stack as the one wrapper", () => {
        mount();
        look("idle");
        return snapshot("idle");
    });

    it("presses every callback: each one reaches the pane's handler with its arguments", () => {
        mount();
        for (const [stand, cb] of [
            ["AgentAuthPanel", "onDismissAuthNotice"],
            ["AgentAuthPanel", "onCancelLogin"],
            ["AgentAuthPanel", "onUseTerminal"],
            ["AgentDecisionPanel", "onDecide"],
            ["AgentDecisionPanel", "onDefer"],
            ["AgentQuestionPanel", "onAnswer"],
            ["AgentQuestionPanel", "onCancel"],
            ["AgentDisconnectedBanner", "onReconnect"],
            ["ApiRetryRow", "onStop"],
            ["HeldInputPanel", "onSendNow"],
            ["HeldInputPanel", "onCancel"],
            ["HeldInputPanel", "onDismissNotice"],
        ] as const) {
            press(stand, cb);
        }
        return snapshot("callbacks");
    });
});

describe("Agent bottom panels: the pane's state reaches the panels", () => {
    it("the login, the decisions, the questions and the queue follow the pane", () => {
        const c = mount();
        c.setAuthUrl("https://example.com/login");
        c.setAuthNotice("Signed out");
        c.setLaunchPhase("login");
        c.setFlowRunning(true);
        c.setDecisions([DECISION]);
        c.setQuestions([{ id: "q-1" }, { id: "q-2" }]);
        c.setPendingMessages([{ id: "m-1", text: "queued" }]);
        look("everything pending");
        c.setDecisions([]);
        c.setQuestions([]);
        c.setPendingMessages([]);
        c.setAuthUrl(null);
        c.setAuthNotice(null);
        look("cleared");
        return snapshot("follow");
    });

    it("the stopped, disconnected, fork and history banners read the turn phase, the init phase and the block", () => {
        const c = mount();
        c.setState("turnPhase", reconcile(phase("Disconnected")));
        c.setState("initPhase", "InitFailed");
        c.setBlock({ meta: { model: "opus", "fork:fallback": true } });
        look("disconnected, init failed, a fork");
        return snapshot("banners");
    });

    it("the failure row shows as a row with its detail, and the stderr tail only when there is one", () => {
        const row = (extra: Record<string, unknown> = {}) => ({
            sigil: "!",
            title: "Auth failed",
            meta: "401",
            accent: "danger",
            actions: [{ label: "Log in" }],
            expanded: true,
            detail: "The login expired",
            stderrTail: undefined,
            ...extra,
        });
        const c = mount({ failureRow: row() });
        look("without stderr");
        c.setFailureRow(row({ stderrTail: "line 1\nline 2" }));
        look("with stderr");
        c.setFailureRow(null);
        look("gone");
        return snapshot("failure-row");
    });
});

describe("Agent bottom panels: the working row", () => {
    it("is drawn only while it is visible, with the turn's numbers and the block's summary", () => {
        const c = mount({ document: [{ id: "n-1" }], block: { meta: { model: "sonnet", summary: "Fixing the build" } } });
        look("hidden");
        c.setState({
            turnTokens: 1200,
            turnLedger: { in: 1, out: 2 },
            turnCarry: { carried: 3 },
            activity: { label: "Reading" },
            sessionStats: { turns: 4 },
            lastContextModel: "claude-sonnet-5-5",
        });
        c.setVisible(true);
        c.setLoading(true);
        look("visible and loading");
        c.setLoading(false);
        look("worked");
        c.setVisible(false);
        look("hidden again");
        return snapshot("working-row");
    });

    it("says what it waits on: a decision, a question, or nothing", () => {
        const c = mount({ visible: true });
        look("nothing pending");
        c.setQuestions([{ id: "q-1" }]);
        look("a question");
        c.setDecisions([DECISION]);
        look("a decision, which wins");
        c.setDecisions([{ tool: "Read", params: undefined }]);
        look("a decision named by tool");
        c.setDecisions([{ toolName: "mcp__agentmux__Tasks", params: {} }]);
        look("an mcp tool");
        return snapshot("working-row-needs");
    });

    it("reads the waiting reason and retry time only while streaming, and stopping only while interrupting", () => {
        const c = mount({ visible: true });
        c.setState("turnPhase", reconcile(phase("Streaming", { waitingReason: "api overloaded", retryAtMs: 1_700_000_005_000 })));
        look("streaming, waiting");
        c.setState("turnPhase", reconcile(phase("Streaming")));
        look("streaming, nothing to wait for");
        c.setState("turnPhase", reconcile(phase("Interrupting", { waitingReason: "ignored" })));
        look("interrupting");
        c.setState("turnPhase", reconcile(phase("Idle")));
        look("idle");
        return snapshot("working-row-phase");
    });

    it("hands the compaction the context tokens only when the reading is plausible, and the model it was measured on", () => {
        const c = mount({ visible: true });
        c.setState({ compacting: true, context: { tokens: 50_000, model: "m", window: 200_000, windowSource: "reported", source: "usage", at: 1 } });
        look("plausible");
        c.setState("context", reconcile({ tokens: 300_000, window: 200_000 }));
        look("more tokens than the window");
        c.setState({ context: null, reconnecting: true });
        look("no reading, reconnecting");
        return snapshot("working-row-compaction");
    });

    it("hands the login to the row: the phase, whether there is a link, and Cancel", () => {
        const c = mount({ visible: true });
        c.setLaunchPhase("login");
        c.setAuthUrl("https://example.com/login");
        look("logging in");
        press("AgentWorkingRow", "onCancelLogin");
        return snapshot("working-row-login");
    });
});

describe("Agent bottom panels: the digest and the offline input", () => {
    it("the digest is looking only while the window has focus and this pane is the focused block", () => {
        const c = mount();
        const looking = () => host.querySelector('[data-stand="TurnAwayDigest"]')!.getAttribute("data-props");
        log(`focused, this pane: ${looking()}`);
        h.windowFocused = false;
        c.setLoading(true);
        log(`window blurred: ${looking()}`);
        h.windowFocused = true;
        h.focusedBlock = "blk-2";
        c.setLoading(false);
        log(`another pane focused: ${looking()}`);
        h.focusedBlock = null;
        c.setLoading(true);
        log(`no pane focused: ${looking()}`);
        return snapshot("digest");
    });

    it("hands the retry, the held input, its notices and the clock to the offline panels", () => {
        const c = mount();
        c.setApiRetry({ attempt: 2, at: 1_700_000_010_000 });
        c.setHeld([{ id: "h-1", text: "held message" }]);
        c.setNotices([{ id: "n-1", text: "sent after reconnect" }]);
        look("retrying, holding");
        c.setApiRetry(null);
        c.setHeld([]);
        c.setNotices([]);
        look("back online");
        return snapshot("offline");
    });
});
