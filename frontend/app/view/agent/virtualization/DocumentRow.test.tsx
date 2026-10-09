// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * DocumentRow inline auth-error CTA tests (P2.3 of
 * SPEC_REAUTH_FROM_AUTH_ERROR_2026_06_20 §7).
 *
 * An `agent_error` node whose `code` is an auth status (401/403) renders a
 * "Login Again" button that drives the same re-auth flow as the failure
 * banner. Any other code (or code 0 = non-HTTP) renders no button — those
 * errors have no in-place fix.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
    writeText: vi.fn((_text: string) => Promise.resolve()),
    showContextMenu: vi.fn((_oid: unknown, _menu: { label: string; id: string }[], _position: unknown) => {}),
}));
vi.mock("@/util/clipboard", () => ({ writeText: mocks.writeText, readText: () => Promise.resolve("") }));
vi.mock("@/app/store/global", async (importOriginal) => ({
    ...(await importOriginal<Record<string, unknown>>()),
    getApi: () => ({ showContextMenu: mocks.showContextMenu }),
}));

import { ContextMenuModel } from "@/app/store/contextmenu";
import { DocumentRow } from "./DocumentRow";
import type { AgentDispatch } from "../../swarm/swarm-model";
import type {
    AgentErrorNode,
    CompactionStartedNode,
    ContextCompactedNode,
    DayDividerNode,
    DocumentNode,
    DocumentState,
    HistoryLinkNode,
    MemoryReinjectionNode,
    SessionOutcomeNode,
    ToolNode,
} from "../types";

afterEach(() => cleanup());

/**
 * DocumentRow — a finished tool result must not be rebuilt on every stream
 * flush (ANALYSIS_AGENT_PANE_FLUSH_REMOUNT_CHURN_2026_09_23.md §2).
 *
 * `dispatchMatches` is a memo over the WHOLE document array, so it yields a
 * new Map identity on every flush. Reading it through an inline prop getter
 * (`dispatchMatch={props.dispatchMatches?.().get(id)}`) subscribed every
 * ToolOverlayResult to that identity, so `renderToolResultBody` re-ran — and
 * rebuilt the whole result subtree (for `.md` Read previews: a full markdown
 * parse plus an OverlayScrollbars construction with its forced layouts) —
 * for every finished tool in the streaming buffer, every flush. Measured at
 * 46% + 38% of `flushPendingNodes` under 4-pane load.
 */
describe("DocumentRow — finished tool results survive dispatchMatches identity churn", () => {
    const readNode = (filePath: string): ToolNode => ({
        type: "tool",
        id: "t-read-1",
        tool: "Read",
        params: { file_path: filePath },
        status: "success",
        result: { content: "# Title\n\nSome body text.\n" },
        timestamp: 1,
    } as ToolNode);

    const renderToolRow = (node: ToolNode) => {
        const [n] = createSignal<DocumentNode>(node);
        // Pinned open: a tool row builds its result body only once it has
        // been opened (ToolBlock's `bodyMounted`), and these tests are
        // about that body.
        const [state] = createSignal<DocumentState>({ ...emptyState(), pinnedNodes: new Set([node.id]) });
        const [matches, setMatches] = createSignal<Map<string, AgentDispatch>>(new Map());
        const r = render(() => (
            <DocumentRow
                node={n}
                documentState={state}
                onToggleCollapse={() => {}}
                onTogglePin={() => {}}
                dispatchMatches={matches}
            />
        ));
        return { ...r, setMatches };
    };

    it("keeps the rendered result DOM when dispatchMatches is replaced by an equal (empty) Map", () => {
        const { container, setMatches } = renderToolRow(readNode("C:/repo/src/index.ts"));
        const before = container.querySelector(".agent-tool-read");
        expect(before).not.toBeNull();

        // Simulate what every stream flush does: a new Map identity with no
        // change for this tool.
        setMatches(new Map());
        setMatches(new Map());

        const after = container.querySelector(".agent-tool-read");
        expect(after).toBe(before);
    });

    it("renders a .md Read preview as non-scrollable markdown (no per-block OverlayScrollbars)", () => {
        const { container } = renderToolRow(readNode("C:/repo/docs/notes.md"));
        const md = container.querySelector(".agent-tool-read-md");
        expect(md).not.toBeNull();
        expect(md!.querySelector(".content.non-scrollable")).not.toBeNull();
        expect(md!.querySelector("[data-overlayscrollbars-initialize]")).toBeNull();
    });
});

const emptyState = (): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    heldOpenNodes: new Set(),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

const errorNode = (code: number, message = "boom"): AgentErrorNode => ({
    type: "agent_error",
    id: "err-1",
    code,
    message,
});

const renderRow = (node: DocumentNode, onAgentErrorLogin?: () => void) => {
    const [n] = createSignal<DocumentNode>(node);
    const [state] = createSignal<DocumentState>(emptyState());
    return render(() => (
        <DocumentRow
            node={n}
            documentState={state}
            onToggleCollapse={() => {}}
            onTogglePin={() => {}}
            onAgentErrorLogin={onAgentErrorLogin}
        />
    ));
};

describe("DocumentRow — inline auth-error CTA", () => {
    it("renders a Login Again button for a 401 error and fires onAgentErrorLogin on click", async () => {
        const onLogin = vi.fn();
        renderRow(errorNode(401, "Invalid authentication credentials"), onLogin);

        const btn = screen.getByRole("button", { name: /Login Again/i });
        expect(btn).toBeInTheDocument();
        expect(screen.getByText("HTTP 401")).toBeInTheDocument();

        await userEvent.click(btn);
        expect(onLogin).toHaveBeenCalledTimes(1);
    });

    it("renders the CTA for a 403 error too", () => {
        renderRow(errorNode(403, "Forbidden"), vi.fn());
        expect(screen.getByRole("button", { name: /Login Again/i })).toBeInTheDocument();
    });

    it("renders NO CTA for a non-auth error code (500)", () => {
        renderRow(errorNode(500, "Internal error"), vi.fn());
        expect(screen.queryByRole("button", { name: /Login Again/i })).toBeNull();
        expect(screen.getByText("HTTP 500")).toBeInTheDocument();
    });

    it("renders NO CTA for a non-HTTP error (code 0) and shows 'Error' not 'HTTP 0'", () => {
        renderRow(errorNode(0, "Network connection lost"), vi.fn());
        expect(screen.queryByRole("button", { name: /Login Again/i })).toBeNull();
        expect(screen.getByText("Error")).toBeInTheDocument();
        expect(screen.queryByText(/HTTP 0/)).toBeNull();
    });

    it("renders NO CTA when onAgentErrorLogin is not provided, even for a 401", () => {
        renderRow(errorNode(401), undefined);
        expect(screen.queryByRole("button", { name: /Login Again/i })).toBeNull();
    });
});

/** SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md surface 3. */
describe("DocumentRow — agent_error copy", () => {
    afterEach(() => vi.clearAllMocks());

    it("the hover-icon button copies a redacted formatted report", async () => {
        const { container } = renderRow(errorNode(401, "Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz0123"));
        const btn = container.querySelector<HTMLButtonElement>(".agent-error-copy .copy-error-button-icon");
        expect(btn).not.toBeNull();
        fireEvent.click(btn!);
        await waitFor(() => expect(mocks.writeText).toHaveBeenCalled());
        const copied = mocks.writeText.mock.calls[0][0] as string;
        expect(copied).toContain("AgentMux error: HTTP 401");
        expect(copied).not.toContain("ghp_abcdef");
        expect(copied).toContain("[redacted");
    });

    it("right-click offers a Copy error context-menu entry that copies the same report", async () => {
        const { container } = renderRow(errorNode(500, "Internal error"));
        const block = container.querySelector(".agent-error-block")!;
        const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
        block.dispatchEvent(e);

        expect(mocks.showContextMenu).toHaveBeenCalledOnce();
        const menu = mocks.showContextMenu.mock.calls[0][1] as { label: string; id: string }[];
        const item = menu.find((i) => i.label === "Copy error");
        expect(item).toBeDefined();

        ContextMenuModel.handleContextMenuClick(item!.id);
        await waitFor(() => expect(mocks.writeText).toHaveBeenCalled());
        expect(mocks.writeText.mock.calls[0][0]).toContain("AgentMux error: HTTP 500");
    });
});

/**
 * DocumentRow — compaction nodes (SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md).
 *
 * Two DISTINCT node types render two DISTINCT rows: `compaction_started`
 * (in-progress announcement, no outcome data yet) and `context_compacted`
 * (the completed record — real backend data or the heuristic fallback).
 * They must never be visually confusable — that's the exact bug this
 * split guards against (an in-progress compaction reading as finished).
 */
describe("DocumentRow — compaction nodes", () => {
    const realCompactedNode = (): ContextCompactedNode => ({
        type: "context_compacted",
        id: "cc-1",
        tokensBefore: 100_000,
        tokensAfter: 5_000,
        timestamp: Date.now(),
        source: "real",
        trigger: "manual",
        durationMs: 12_345,
    });

    const heuristicCompactedNode = (): ContextCompactedNode => ({
        type: "context_compacted",
        id: "cc-2",
        tokensBefore: 60_000,
        tokensAfter: 4_000,
        timestamp: Date.now(),
        source: "heuristic",
    });

    const startedNode = (trigger: "manual" | "auto"): CompactionStartedNode => ({
        type: "compaction_started",
        id: "cs-1",
        trigger,
        startedAt: Date.now(),
    });

    it("real context_compacted shows the trigger label and real duration", () => {
        renderRow(realCompactedNode());
        expect(screen.getByText(/context compacted/i)).toBeInTheDocument();
        expect(screen.getByText(/you ran \/compact/i)).toBeInTheDocument();
        // post_tokens is the summary's size, not the context's: said so until
        // the next call reports the real size.
        expect(screen.getByText(/100k tokens summarized to 5\.0k/i)).toBeInTheDocument();
        expect(screen.getByText(/took 12\.3s/i)).toBeInTheDocument();
    });

    it("real context_compacted shows the real size after once the next call reported it", () => {
        renderRow({ ...realCompactedNode(), contextAfter: 39_490 });
        expect(screen.getByText(/100k → 39k tokens · summary 5\.0k/i)).toBeInTheDocument();
    });

    it("heuristic context_compacted keeps before → after", () => {
        renderRow(heuristicCompactedNode());
        expect(screen.getByText(/60k → 4\.0k tokens/i)).toBeInTheDocument();
    });

    it("real context_compacted with auto trigger shows the auto-compacted label", () => {
        renderRow({ ...realCompactedNode(), trigger: "auto" });
        expect(screen.getByText(/auto-compacted/i)).toBeInTheDocument();
    });

    it("heuristic context_compacted renders WITHOUT a trigger label or duration", () => {
        renderRow(heuristicCompactedNode());
        expect(screen.getByText(/context compacted/i)).toBeInTheDocument();
        expect(screen.queryByText(/you ran \/compact/i)).toBeNull();
        expect(screen.queryByText(/auto-compacted/i)).toBeNull();
        expect(screen.queryByText(/took/i)).toBeNull();
        expect(screen.getByText(/60k → 4\.0k tokens/i)).toBeInTheDocument();
    });

    it("compaction_started renders the in-progress announcement, distinct from context_compacted", () => {
        renderRow(startedNode("manual"));
        expect(screen.getByText(/Compacting conversation/i)).toBeInTheDocument();
        expect(screen.getByText(/you ran \/compact/i)).toBeInTheDocument();
        // Must NOT render anything from the completed-record copy — an
        // in-progress compaction must never look like a finished one.
        expect(screen.queryByText(/context compacted/i)).toBeNull();
    });

    it("compaction_started with auto trigger shows a distinct reason label", () => {
        renderRow(startedNode("auto"));
        expect(screen.getByText(/Compacting conversation/i)).toBeInTheDocument();
        expect(screen.getByText(/context filled up/i)).toBeInTheDocument();
    });
});

/**
 * DocumentRow — memory_reinjection node
 * (SPEC_HIDDEN_MEMORY_REINJECTION_AFTER_COMPACTION_2026_09_22.md §3.2).
 *
 * The label-only row for a hidden memory reinjection. The single most
 * important property under test here isn't what renders — it's what
 * DOESN'T: this node's own data never carries the actual memory body text
 * (§3.2's hiding mechanism is "never stored," not "stored but hidden"), so
 * there is nothing for a rendering bug in this component to leak even in
 * principle. The negative assertions below exist to catch a future
 * regression that accidentally threads real content onto this node type.
 */
describe("DocumentRow — memory_reinjection node", () => {
    const reinjectionNode = (overrides: Partial<MemoryReinjectionNode> = {}): MemoryReinjectionNode => ({
        type: "memory_reinjection",
        id: "mr-1",
        globalMemoryCount: 2,
        personalMemoryCount: 3,
        estimatedTokens: 450,
        perEntryTokens: [
            { label: "global-note", source: "global", tokens: 100, sizeBytes: 400 },
            { label: "personal-file.md", source: "personal", tokens: 350, sizeBytes: 1400 },
        ],
        totalSizeBytes: { global: 800, personal: 4200 },
        sizeBand: "low",
        at: Date.now(),
        ...overrides,
    });

    it("renders the label with counts and the estimated token total, never any body text", () => {
        const { container } = renderRow(reinjectionNode());
        // JSX interpolation splits this across several text nodes, so a
        // getByText string/regex match (which doesn't span nodes) can't be
        // used directly — check the row's full textContent instead, same
        // as the hover-tooltip assertions below already do.
        const label = container.querySelector(".agent-memory-reinjection-label");
        expect(label?.textContent).toMatch(/Memory reinjected/i);
        expect(label?.textContent).toMatch(/2 global, 3 personal/i);
        expect(label?.textContent).toMatch(/~450 tok, est\.\)/i);
    });

    it("applies the size-band modifier class the node carries", () => {
        const { container } = renderRow(reinjectionNode({ sizeBand: "critical" }));
        expect(container.querySelector(".agent-memory-reinjection--critical")).not.toBeNull();
        expect(container.querySelector(".agent-memory-reinjection--low")).toBeNull();
    });

    it("on hover: shows the per-entry breakdown and real byte totals, still no body text", () => {
        vi.useFakeTimers();
        const { container } = renderRow(reinjectionNode());
        const el = container.querySelector(".agent-memory-reinjection") as HTMLElement;
        fireEvent.mouseEnter(el);
        vi.advanceTimersByTime(100);

        const tooltip = document.body.querySelector(".agent-memory-reinjection-tooltip");
        expect(tooltip).not.toBeNull();
        expect(tooltip?.textContent).toMatch(/global-note/);
        expect(tooltip?.textContent).toMatch(/~100 tok \(est\.\), 400 B/);
        expect(tooltip?.textContent).toMatch(/personal-file\.md/);
        // formatCompactNumber compacts 4200 -> "4.2k" (matches format-count.test.ts's own documented behavior), not "4,200".
        expect(tooltip?.textContent).toMatch(/Total: 800 B global, 4\.2k B personal/);
        vi.useRealTimers();
    });

    it("below the mid band, the hover tooltip shows NO compress/delegate suggestion — §3.4.2's silent-below-mid rule", () => {
        vi.useFakeTimers();
        const { container } = renderRow(reinjectionNode({ sizeBand: "low" }));
        fireEvent.mouseEnter(container.querySelector(".agent-memory-reinjection") as HTMLElement);
        vi.advanceTimersByTime(100);

        const tooltip = document.body.querySelector(".agent-memory-reinjection-tooltip");
        expect(tooltip?.textContent).not.toMatch(/consider/i);
        expect(tooltip?.textContent).not.toMatch(/WorkEnqueue/);
        vi.useRealTimers();
    });

    it("at high/critical bands, the hover tooltip includes the compress-or-delegate suggestion — informational only, §3.4.3", () => {
        vi.useFakeTimers();
        for (const band of ["high", "critical"] as const) {
            cleanup();
            const { container } = renderRow(reinjectionNode({ sizeBand: band }));
            fireEvent.mouseEnter(container.querySelector(".agent-memory-reinjection") as HTMLElement);
            vi.advanceTimersByTime(100);

            const tooltip = document.body.querySelector(".agent-memory-reinjection-tooltip");
            expect(tooltip?.textContent).toMatch(/consider/i);
            expect(tooltip?.textContent).toMatch(/MemoryWrite/);
            expect(tooltip?.textContent).toMatch(/WorkEnqueue/);
            // Informational text only — no button/link for this suggestion.
            // Confirmed by absence, not just by not asserting for one: this
            // node type renders no interactive elements at all today.
            expect(container.querySelectorAll("button, a").length).toBe(0);
        }
        vi.useRealTimers();
    });
});

/**
 * DocumentRow — peek tooltip for the inline node kinds
 * (SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS_2026_08_25). These six kinds
 * (agent_error/context_compacted/compaction_started/day_divider/
 * session_outcome) render inline in DocumentNodeBody rather than through
 * their own dedicated component, and previously had NO peek at all.
 * history_link is the one deliberate exception — no timestamp/content field
 * exists on it to peek.
 */
describe("DocumentRow — peek tooltip on the inline node kinds", () => {
    const hover = (container: HTMLElement, selector: string) => {
        const el = container.querySelector(selector) as HTMLElement;
        fireEvent.mouseEnter(el);
        vi.advanceTimersByTime(100);
    };

    afterEach(() => vi.useRealTimers());

    it("agent_error: shows only the estimate line — no timestamp field exists on this node", () => {
        vi.useFakeTimers();
        const { container } = renderRow(errorNode(500, "a somewhat longer error message body"));
        hover(container, ".agent-error-block");
        const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
        expect(metaLines.length).toBe(1);
        expect(metaLines[0].textContent).toMatch(/~\d+ tok \(est\.\)/);
    });

    it("context_compacted: shows a time-only peek", () => {
        vi.useFakeTimers();
        const node: ContextCompactedNode = {
            type: "context_compacted",
            id: "cc-3",
            tokensBefore: 100_000,
            tokensAfter: 5_000,
            timestamp: Date.now() - 65_000,
            source: "real",
            trigger: "manual",
        };
        const { container } = renderRow(node);
        hover(container, ".agent-context-compacted");
        const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
        expect(metaLines.length).toBe(1);
        expect(metaLines[0].textContent).toMatch(/\d{1,2}:\d{2}:\d{2} (?:AM|PM) · 1m ago/);
    });

    it("compaction_started: shows a time-only peek from startedAt", () => {
        vi.useFakeTimers();
        const node: CompactionStartedNode = {
            type: "compaction_started",
            id: "cs-2",
            trigger: "manual",
            startedAt: Date.now() - 65_000,
        };
        const { container } = renderRow(node);
        hover(container, ".agent-compaction-started");
        const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
        expect(metaLines.length).toBe(1);
        expect(metaLines[0].textContent).toMatch(/\d{1,2}:\d{2}:\d{2} (?:AM|PM) · 1m ago/);
    });

    it("day_divider: shows the exact local-midnight instant on hover", () => {
        vi.useFakeTimers();
        const node: DayDividerNode = {
            type: "day_divider",
            id: "day-2026-08-25",
            dayLabel: "Tue, Aug 25 2026",
            timestamp: Date.now() - 65_000,
        };
        const { container } = renderRow(node);
        hover(container, ".agent-day-divider");
        const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
        expect(metaLines.length).toBe(1);
        expect(metaLines[0].textContent).toMatch(/\d{1,2}:\d{2}:\d{2} (?:AM|PM) · 1m ago/);
    });

    it("session_outcome: shows time + attempted/actual session ids", () => {
        vi.useFakeTimers();
        const node: SessionOutcomeNode = {
            type: "session_outcome",
            id: "so-1",
            outcome: "fresh",
            attemptedSid: "sid-attempted",
            actualSid: "sid-actual",
            timestamp: Date.now() - 65_000,
        };
        const { container } = renderRow(node);
        hover(container, ".agent-session-outcome");
        expect(document.body.querySelector(".agent-node-peek-tooltip-meta")?.textContent).toMatch(/\d{1,2}:\d{2}:\d{2} (?:AM|PM) · 1m ago/);
        expect(document.body.querySelector(".agent-node-peek-tooltip-body")?.textContent).toBe(
            "attempted: sid-attempted · actual: sid-actual"
        );
    });

    it("session_outcome: an empty attemptedSid reads as '—', not a blank", () => {
        vi.useFakeTimers();
        // srv emits `attempted_sid: ""` when the spawn had no session id to
        // resume at all (the cross-channel-open case its
        // `fresh_start_needs_disclosure` gate covers) — distinct from having
        // attempted an id that was rejected.
        const node: SessionOutcomeNode = {
            type: "session_outcome",
            id: "so-2",
            outcome: "fresh",
            attemptedSid: "",
            actualSid: null,
            timestamp: Date.now() - 65_000,
        };
        const { container } = renderRow(node);
        hover(container, ".agent-session-outcome");
        expect(document.body.querySelector(".agent-node-peek-tooltip-body")?.textContent).toBe(
            "attempted: — · actual: —"
        );
    });

    it("history_link: no peek anchor at all — nothing to show", () => {
        vi.useFakeTimers();
        const node: HistoryLinkNode = { type: "history_link", id: "history-link" };
        const [n] = createSignal<DocumentNode>(node);
        const [state] = createSignal<DocumentState>(emptyState());
        const { container } = render(() => (
            <DocumentRow node={n} documentState={state} onToggleCollapse={() => {}} onTogglePin={() => {}} />
        ));
        const row = container.querySelector(".agent-history-link-row") as HTMLElement;
        fireEvent.mouseEnter(row);
        vi.advanceTimersByTime(100);
        expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
    });
});

describe("DocumentRow — context delivery card (SPEC_CONTEXT_DELIVERY_2026_09_30 §3.2)", () => {
    const BODY = "This session is being continued from a previous conversation.\n\nSummary:\n1. Intent:\n   Fix the console sign-in.";
    const card = (): DocumentNode => ({
        type: "context_delivery",
        id: "context-delivery-compaction-x",
        reason: "compaction",
        trigger: "manual",
        timestamp: 0,
        items: [
            {
                kind: "compaction_summary",
                name: "Conversation summary (written by Claude Code)",
                sizeBytes: BODY.length,
                tokens: 30,
                excerpt: "Fix the console sign-in.",
                body: BODY,
            },
        ],
    });

    const renderCard = (pinned: boolean, onTogglePin = () => {}) => {
        const node = card();
        const [n] = createSignal<DocumentNode>(node);
        const [state] = createSignal<DocumentState>({
            ...emptyState(),
            pinnedNodes: pinned ? new Set([node.id]) : new Set(),
        });
        return render(() => (
            <DocumentRow node={n} documentState={state} onToggleCollapse={() => {}} onTogglePin={onTogglePin} />
        ));
    };

    it("shows the title and excerpt collapsed, not the summary text", () => {
        const { container } = renderCard(false);
        expect(screen.getByText("Agent given a summary of the conversation (manual compact)")).toBeInTheDocument();
        expect(screen.getByText("Fix the console sign-in.")).toBeInTheDocument();
        expect(container.querySelector(".agent-context-delivery-body")).toBeNull();
        expect(container.querySelector(".agent-user-message-content")).toBeNull();
    });

    it("shows each item's name and full text when pinned open", () => {
        const { container } = renderCard(true);
        expect(screen.getByText("Conversation summary (written by Claude Code)")).toBeInTheDocument();
        // One block per line (PreviewLines), so compare line by line.
        const lines = [...container.querySelectorAll(".agent-context-delivery-body .agent-preview-line")].map((l) => l.textContent);
        expect(lines.join("\n")).toBe(BODY);
    });

    it("pins on click", async () => {
        const onTogglePin = vi.fn();
        renderCard(false, onTogglePin);
        await userEvent.click(screen.getByText("Agent given a summary of the conversation (manual compact)"));
        expect(onTogglePin).toHaveBeenCalledTimes(1);
    });
});

describe("DocumentRow — memory delivery card (SPEC_CONTEXT_DELIVERY_2026_09_30 §3.4, CD2a)", () => {
    const memoryCard = (sizeBand: "low" | "high" = "low"): DocumentNode => ({
        type: "context_delivery",
        id: "memory-injected-s1-compact-1",
        reason: "compaction",
        timestamp: 0,
        sizeBand,
        items: [
            { kind: "global_memory", name: "App API", tier: "system", bundleId: "b-1", sizeBytes: 3400, tokens: 850 },
            { kind: "global_memory", name: "Rules", tier: "workspace", delivered: "partial", sizeBytes: 600, tokens: 150 },
            { kind: "personal_memory", name: "notes.md", path: "/mem/notes.md", delivered: "omitted", sizeBytes: 0, tokens: 0 },
        ],
    });

    const renderMemory = (node: DocumentNode, onTogglePin = () => {}) => {
        const [n] = createSignal<DocumentNode>(node);
        const [state] = createSignal<DocumentState>(emptyState());
        return render(() => (
            <DocumentRow node={n} documentState={state} onToggleCollapse={() => {}} onTogglePin={onTogglePin} />
        ));
    };

    it("lists every item without opening the card", () => {
        renderMemory(memoryCard());
        expect(screen.getByText("Memory re-delivered after compaction · 3 items · 2 cut")).toBeInTheDocument();
        expect(screen.getByText("App API")).toBeInTheDocument();
        expect(screen.getByText("AgentMux system")).toBeInTheDocument();
        expect(screen.getByText("Workspace")).toBeInTheDocument();
        expect(screen.getByText("Personal")).toBeInTheDocument();
        expect(screen.getByText("notes.md")).toHaveAttribute("title", "/mem/notes.md");
    });

    it("marks items the part cap cut", () => {
        renderMemory(memoryCard());
        expect(screen.getByText("cut")).toBeInTheDocument();
        expect(screen.getByText("not sent")).toBeInTheDocument();
    });

    it("has no chevron and doesn't pin when there's no text to open", async () => {
        const onTogglePin = vi.fn();
        const { container } = renderMemory(memoryCard(), onTogglePin);
        expect(container.querySelector(".agent-context-delivery-chevron")).toBeNull();
        await userEvent.click(screen.getByText("App API"));
        expect(onTogglePin).not.toHaveBeenCalled();
    });

    it("lists startup files with their owner, a count for listings, and the Global Memory they repeat (LC2)", () => {
        renderMemory({
            type: "context_delivery",
            id: "memory-injected-s1-startup-1",
            reason: "startup",
            timestamp: 0,
            items: [
                { kind: "startup_file", name: "~/.agentmux/agents/CLAUDE.md", role: "instructions", owner: "external", sizeBytes: 24000, tokens: 6000 },
                {
                    kind: "startup_file",
                    name: "AGENTMUX_MEMORY.md",
                    role: "instructions_import",
                    owner: "agentmux",
                    contains: ["global_memory"],
                    sizeBytes: 10000,
                    tokens: 2500,
                },
                { kind: "startup_file", name: "MCP servers: agentmux", role: "mcp_servers", owner: "agentmux", count: 1, sizeBytes: 0, tokens: 0 },
                { kind: "global_memory", name: "App API", tier: "system", sizeBytes: 3400, tokens: 850 },
            ],
        });
        expect(screen.getByText("Given to the agent · new session · 4 items")).toBeInTheDocument();
        expect(screen.getByText("Hand-maintained")).toBeInTheDocument();
        expect(screen.getAllByText("AgentMux")).toHaveLength(2);
        expect(screen.getByText("+ Global Memory")).toHaveAttribute(
            "title",
            "This file also carries the Global Memory, so a new session gets it twice",
        );
        expect(screen.getByText("1 listed")).toBeInTheDocument();
        expect(screen.getByText("AgentMux system")).toBeInTheDocument();
    });

    it("says a Global entry came in the startup file instead of sizing it (LC3)", () => {
        renderMemory({
            type: "context_delivery",
            id: "memory-injected-s1-startup-2",
            reason: "startup",
            timestamp: 0,
            items: [
                { kind: "startup_file", name: "CLAUDE.md", owner: "agentmux", contains: ["global_memory"], sizeBytes: 12, tokens: 3 },
                { kind: "global_memory", name: "Rules", tier: "workspace", via: "startup_file", sizeBytes: 0, tokens: 0 },
            ],
        });
        expect(screen.getByText("in startup file")).toBeInTheDocument();
        expect(screen.queryByText(/0 B/)).toBeNull();
        // The file is the one delivery, not a repeat (Codex on #4131).
        expect(screen.getByText("+ Global Memory")).toHaveAttribute("title", "The Global Memory reached the agent through this file");
    });

    it("still marks a duplicate when two loaded startup files carry the Global Memory", () => {
        renderMemory({
            type: "context_delivery",
            id: "memory-injected-s1-startup-3",
            reason: "startup",
            timestamp: 0,
            items: [
                { kind: "startup_file", name: "~/agents/CLAUDE.md", owner: "agentmux", contains: ["global_memory"], sizeBytes: 12, tokens: 3 },
                { kind: "startup_file", name: "CLAUDE.md", owner: "agentmux", contains: ["global_memory"], sizeBytes: 12, tokens: 3 },
                { kind: "global_memory", name: "Rules", tier: "workspace", via: "startup_file", sizeBytes: 0, tokens: 0 },
            ],
        });
        for (const mark of screen.getAllByText("+ Global Memory")) {
            expect(mark).toHaveAttribute("title", "This file also carries the Global Memory, so a new session gets it twice");
        }
    });

    it("shows the size advice when Personal Memory is large", () => {
        renderMemory(memoryCard("high"));
        expect(screen.getByText(/Personal memory has grown large/)).toBeInTheDocument();
    });
});
