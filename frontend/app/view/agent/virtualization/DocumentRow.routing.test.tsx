// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Which block a document row draws for each kind of node, and what it hands that block: the pinned,
 * held-open, collapsed and dispatch flags from the document state, the callbacks that carry the
 * node's id, the shared hover peek, and the startup-prompt split of a user message. The blocks are
 * probes that record the props they receive; the row and its disclosure rule are the real ones.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

type Peek = {
    isPeeking: () => boolean;
    peekPanelVisible: () => boolean;
    peekTick: () => number;
};
/** The props a probe records; what a block does not receive is simply undefined. */
type Probed = {
    node: { id: string; cmd?: string } | (() => { id: string });
    pinned: boolean;
    heldOpen: boolean;
    userCollapsed: boolean;
    collapsed: boolean;
    dispatchMatch: unknown;
    peek: Peek;
    onTogglePin: () => void;
    onToggle: () => void;
    onToggleCollapse: () => void;
    onHoldOpen: () => void;
    onAgentErrorLogin: () => void;
    onOpenHistory: () => void;
};

const h = vi.hoisted(() => ({ props: {} as Record<string, Probed> }));

vi.mock("../components/MarkdownBlock", () => ({ MarkdownBlock: (p: Probed) => probe("MarkdownBlock", p) }));
vi.mock("../components/HiddenRunBlock", () => ({ HiddenRunBlock: (p: Probed) => probe("HiddenRunBlock", p) }));
vi.mock("../components/AmbientNarrationBlock", () => ({
    AmbientNarrationBlock: (p: Probed) => probe("AmbientNarrationBlock", p),
}));
vi.mock("../components/ToolBlock", () => ({ ToolBlock: (p: Probed) => probe("ToolBlock", p) }));
vi.mock("../components/AgentMessageBlock", () => ({ AgentMessageBlock: (p: Probed) => probe("AgentMessageBlock", p) }));
vi.mock("../components/OriginContainer", () => ({ OriginContainer: (p: Probed) => probe("OriginContainer", p) }));
vi.mock("../components/ContextDeliveryCard", () => ({
    ContextDeliveryCard: (p: Probed) => probe("ContextDeliveryCard", p),
}));
vi.mock("../components/UserMessageBlock", () => ({ UserMessageBlock: (p: Probed) => probe("UserMessageBlock", p) }));
vi.mock("../components/SetupNote", () => ({ SetupNote: (p: Probed) => probe("SetupNote", p) }));
vi.mock("../components/PersistentShellBlock", () => ({
    PersistentShellBlock: (p: Probed) => probe("PersistentShellBlock", p),
}));
vi.mock("./InlineNodeRows", () => ({
    AgentErrorRow: (p: Probed) => probe("AgentErrorRow", p),
    CliNoticeRow: (p: Probed) => probe("CliNoticeRow", p),
    CompactionStartedRow: (p: Probed) => probe("CompactionStartedRow", p),
    ContextCompactedRow: (p: Probed) => probe("ContextCompactedRow", p),
    DayDividerRow: (p: Probed) => probe("DayDividerRow", p),
    HistoryLinkRow: (p: Probed) => probe("HistoryLinkRow", p),
    MemoryReinjectionRow: (p: Probed) => probe("MemoryReinjectionRow", p),
    ResumePreflightRow: (p: Probed) => probe("ResumePreflightRow", p),
    SessionOutcomeRow: (p: Probed) => probe("SessionOutcomeRow", p),
}));

function probe(name: string, p: Probed) {
    h.props[name] = p;
    return <div data-probe={name} />;
}

import type { DocumentNode, DocumentState } from "../../../agent-model/types";
import type { AgentDispatch } from "../../swarm/swarm-model";
import { DocumentRow } from "./DocumentRow";

const emptyState = (over: Partial<DocumentState> = {}): DocumentState =>
    ({
        collapsedNodes: new Set<string>(),
        pinnedNodes: new Set<string>(),
        heldOpenNodes: new Set<string>(),
        ...over,
    }) as DocumentState;

type IdFn = Mock<(id: string) => void>;
type NoArgFn = Mock<() => void>;
type Fns = {
    onTogglePin: IdFn;
    onToggleCollapse: IdFn;
    onHoldNodeOpen: IdFn;
    onAgentErrorLogin: NoArgFn;
    onOpenHistory: NoArgFn;
};

function mount(
    node: Record<string, unknown>,
    state: DocumentState = emptyState(),
    matches?: Map<string, AgentDispatch>
) {
    const fns: Fns = {
        onTogglePin: vi.fn<(id: string) => void>(),
        onToggleCollapse: vi.fn<(id: string) => void>(),
        onHoldNodeOpen: vi.fn<(id: string) => void>(),
        onAgentErrorLogin: vi.fn<() => void>(),
        onOpenHistory: vi.fn<() => void>(),
    };
    const [n] = createSignal(node as unknown as DocumentNode);
    const [s] = createSignal(state);
    const [m] = createSignal(matches ?? new Map<string, AgentDispatch>());
    const view = render(() => <DocumentRow node={n} documentState={s} dispatchMatches={m} {...fns} />);
    const drawn = () =>
        Array.from(view.container.querySelectorAll("[data-probe]")).map((e) => e.getAttribute("data-probe"));
    return { view, fns, drawn };
}

beforeEach(() => {
    h.props = {};
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

const ID = "n1";

describe("which block a row draws", () => {
    const base = (type: string, extra: Record<string, unknown> = {}) => ({ type, id: ID, timestamp: 1, ...extra });
    const kinds: [string, string, Record<string, unknown>][] = [
        ["markdown", "MarkdownBlock", {}],
        ["hidden_run", "HiddenRunBlock", {}],
        ["ambient_narration", "AmbientNarrationBlock", {}],
        ["tool", "ToolBlock", { status: "success" }],
        ["agent_message", "AgentMessageBlock", {}],
        ["jekt_message", "OriginContainer", {}],
        ["system_note", "OriginContainer", {}],
        ["context_delivery", "ContextDeliveryCard", { items: [] }],
        ["user_message", "UserMessageBlock", {}],
        ["shell", "PersistentShellBlock", {}],
        ["agent_error", "AgentErrorRow", {}],
        ["context_compacted", "ContextCompactedRow", {}],
        ["compaction_started", "CompactionStartedRow", {}],
        ["memory_reinjection", "MemoryReinjectionRow", {}],
        ["day_divider", "DayDividerRow", {}],
        ["history_link", "HistoryLinkRow", {}],
        ["resume_preflight", "ResumePreflightRow", {}],
        ["session_outcome", "SessionOutcomeRow", {}],
        ["cli_notice", "CliNoticeRow", {}],
    ];

    it.each(kinds)("a %s node is drawn by %s, and by nothing else", (type, block, extra) => {
        const t = mount(base(type, extra));
        expect(t.drawn()).toEqual([block]);
        const given = h.props[block].node;
        expect((typeof given === "function" ? given() : given).id).toBe(ID);
    });

    it("an unknown kind draws nothing", () => {
        expect(mount(base("from_the_future")).drawn()).toEqual([]);
    });

    it("a user message is the human's turn, unless it is AgentMux's own startup prompt", () => {
        expect(mount(base("user_message")).drawn()).toEqual(["UserMessageBlock"]);
        cleanup();
        h.props = {};
        const startup = mount(base("user_message", { isStartup: true }));
        expect(startup.drawn()).toEqual(["SetupNote"]);
    });
});

describe("what a row hands the block", () => {
    const base = (type: string, extra: Record<string, unknown> = {}) => ({ type, id: ID, timestamp: 1, ...extra });

    it("a markdown block: whether it is pinned, and a pin that carries its id", () => {
        const t = mount(base("markdown"), emptyState({ pinnedNodes: new Set([ID]) }));
        expect(h.props.MarkdownBlock.pinned).toBe(true);
        h.props.MarkdownBlock.onTogglePin();
        expect(t.fns.onTogglePin).toHaveBeenCalledWith(ID);
        cleanup();
        mount(base("markdown"));
        expect(h.props.MarkdownBlock.pinned).toBe(false);
    });

    it("a hidden run: its toggle pins it", () => {
        const t = mount(base("hidden_run"));
        h.props.HiddenRunBlock.onToggle();
        expect(t.fns.onTogglePin).toHaveBeenCalledWith(ID);
        expect(t.fns.onToggleCollapse).not.toHaveBeenCalled();
    });

    it("a tool: its pinned, held-open and collapsed flags, and each callback with its id", () => {
        const t = mount(
            base("tool", { status: "success" }),
            emptyState({ pinnedNodes: new Set([ID]), heldOpenNodes: new Set([ID]), collapsedNodes: new Set([ID]) })
        );
        const p = h.props.ToolBlock;
        expect([p.pinned, p.heldOpen, p.userCollapsed]).toEqual([true, true, true]);
        p.onTogglePin();
        p.onHoldOpen();
        p.onToggleCollapse();
        expect(t.fns.onTogglePin).toHaveBeenCalledWith(ID);
        expect(t.fns.onHoldNodeOpen).toHaveBeenCalledWith(ID);
        expect(t.fns.onToggleCollapse).toHaveBeenCalledWith(ID);
        expect(t.fns.onTogglePin).toHaveBeenCalledTimes(1);
    });

    it("a tool with none of those flags set gets none of them", () => {
        mount(base("tool", { status: "success" }));
        const p = h.props.ToolBlock;
        expect([p.pinned, p.heldOpen, p.userCollapsed]).toEqual([false, false, false]);
    });

    it("a tool: only the held-open set makes it held open, and only the pinned set pins it", () => {
        mount(base("tool", { status: "success" }), emptyState({ heldOpenNodes: new Set([ID]) }));
        expect([h.props.ToolBlock.pinned, h.props.ToolBlock.heldOpen]).toEqual([false, true]);
        cleanup();
        mount(base("tool", { status: "success" }), emptyState({ pinnedNodes: new Set([ID]) }));
        expect([h.props.ToolBlock.pinned, h.props.ToolBlock.heldOpen]).toEqual([true, false]);
    });

    it("a tool: its own dispatch, found by its id", () => {
        const dispatch = { id: "d1" } as unknown as AgentDispatch;
        mount(
            base("tool", { status: "success" }),
            emptyState(),
            new Map([
                [ID, dispatch],
                ["other", { id: "d2" } as unknown as AgentDispatch],
            ])
        );
        expect(h.props.ToolBlock.dispatchMatch).toBe(dispatch);
        cleanup();
        mount(base("tool", { status: "success" }), emptyState(), new Map([["other", dispatch]]));
        expect(h.props.ToolBlock.dispatchMatch).toBeUndefined();
    });

    it("an agent message is open until the user collapses it, and its toggle collapses", () => {
        const t = mount(base("agent_message"));
        expect(h.props.AgentMessageBlock.collapsed).toBe(false);
        h.props.AgentMessageBlock.onToggle();
        expect(t.fns.onToggleCollapse).toHaveBeenCalledWith(ID);
        expect(t.fns.onTogglePin).not.toHaveBeenCalled();
        cleanup();
        mount(base("agent_message"), emptyState({ collapsedNodes: new Set([ID]) }));
        expect(h.props.AgentMessageBlock.collapsed).toBe(true);
    });

    it("a jekt is closed until it is pinned or held open; its toggle pins, and it can hold itself open", () => {
        const t = mount(base("jekt_message"));
        expect(h.props.OriginContainer.collapsed).toBe(true);
        h.props.OriginContainer.onToggle();
        expect(t.fns.onTogglePin).toHaveBeenCalledWith(ID);
        h.props.OriginContainer.onHoldOpen();
        expect(t.fns.onHoldNodeOpen).toHaveBeenCalledWith(ID);
        cleanup();
        mount(base("jekt_message"), emptyState({ pinnedNodes: new Set([ID]) }));
        expect(h.props.OriginContainer.collapsed).toBe(false);
    });

    it("a system note takes only its node", () => {
        mount(base("system_note"));
        expect(h.props.OriginContainer.collapsed).toBeUndefined();
        expect(h.props.OriginContainer.onToggle).toBeUndefined();
    });

    it("a context delivery with text is closed until pinned; its toggle pins", () => {
        const t = mount(base("context_delivery", { items: [{ body: "text" }] }));
        expect(h.props.ContextDeliveryCard.collapsed).toBe(true);
        h.props.ContextDeliveryCard.onToggle();
        expect(t.fns.onTogglePin).toHaveBeenCalledWith(ID);
        cleanup();
        mount(base("context_delivery", { items: [{ body: "text" }] }), emptyState({ pinnedNodes: new Set([ID]) }));
        expect(h.props.ContextDeliveryCard.collapsed).toBe(false);
    });

    it("the startup note: its pinned flag, and a pin that carries its id", () => {
        const t = mount(base("user_message", { isStartup: true }), emptyState({ pinnedNodes: new Set([ID]) }));
        expect(h.props.SetupNote.pinned).toBe(true);
        h.props.SetupNote.onTogglePin();
        expect(t.fns.onTogglePin).toHaveBeenCalledWith(ID);
        cleanup();
        mount(base("user_message", { isStartup: true }));
        expect(h.props.SetupNote.pinned).toBe(false);
    });

    it("a shell: its node, its pinned flag, and a pin that carries its id", () => {
        const t = mount(base("shell", { cmd: "ls" }), emptyState({ pinnedNodes: new Set([ID]) }));
        expect((h.props.PersistentShellBlock.node as { cmd?: string }).cmd).toBe("ls");
        expect(h.props.PersistentShellBlock.pinned).toBe(true);
        h.props.PersistentShellBlock.onTogglePin();
        expect(t.fns.onTogglePin).toHaveBeenCalledWith(ID);
        cleanup();
        mount(base("shell"));
        expect(h.props.PersistentShellBlock.pinned).toBe(false);
    });

    it("an agent error can offer the login again; a history link and a resume preflight can open the history", () => {
        const t = mount(base("agent_error"));
        h.props.AgentErrorRow.onAgentErrorLogin();
        expect(t.fns.onAgentErrorLogin).toHaveBeenCalledTimes(1);
        cleanup();
        const link = mount(base("history_link"));
        h.props.HistoryLinkRow.onOpenHistory();
        expect(link.fns.onOpenHistory).toHaveBeenCalledTimes(1);
        cleanup();
        const pre = mount(base("resume_preflight"));
        h.props.ResumePreflightRow.onOpenHistory();
        expect(pre.fns.onOpenHistory).toHaveBeenCalledTimes(1);
    });
});

describe("the hover peek the inline rows share", () => {
    const base = (type: string) => ({ type, id: ID, timestamp: 1 });
    const peeking = [
        "agent_error",
        "context_compacted",
        "compaction_started",
        "memory_reinjection",
        "day_divider",
        "session_outcome",
    ];
    const names: Record<string, string> = {
        agent_error: "AgentErrorRow",
        context_compacted: "ContextCompactedRow",
        compaction_started: "CompactionStartedRow",
        memory_reinjection: "MemoryReinjectionRow",
        day_divider: "DayDividerRow",
        session_outcome: "SessionOutcomeRow",
    };

    it.each(peeking)("a %s row gets the peek: its state, its element and both handlers", (type) => {
        mount(base(type));
        const peek = h.props[names[type]].peek;
        expect(Object.keys(peek).sort()).toEqual(
            [
                "handlePeekEnter",
                "handlePeekLeave",
                "isPeeking",
                "peekPanelVisible",
                "peekRowEl",
                "peekTick",
                "setPeekRowEl",
            ].sort()
        );
        expect(peek.isPeeking()).toBe(false);
        expect(peek.peekPanelVisible()).toBe(false);
    });

    it("the rows with nothing to peek at get none", () => {
        mount(base("history_link"));
        expect(h.props.HistoryLinkRow.peek).toBeUndefined();
        cleanup();
        mount(base("resume_preflight"));
        expect(h.props.ResumePreflightRow.peek).toBeUndefined();
        cleanup();
        mount(base("cli_notice"));
        expect(h.props.CliNoticeRow.peek).toBeUndefined();
    });

    it("its clock ticks once a second", () => {
        vi.useFakeTimers();
        mount(base("agent_error"));
        const tick = h.props.AgentErrorRow.peek.peekTick;
        const before = tick();
        vi.advanceTimersByTime(999);
        expect(tick()).toBe(before);
        vi.advanceTimersByTime(1);
        expect(tick()).toBe(before + 1);
    });
});
