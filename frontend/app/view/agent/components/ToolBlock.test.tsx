// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolBlock — panel-mode render tests.
 *
 * Post-SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28: hover is no longer a
 * trigger for expansion. Tests assert the two surviving render modes
 * (`--flow` when pinned or auto-expanded, `--hidden` otherwise) and the
 * negative case that `mouseenter` does not flip the panel open. The
 * older hover-anchor overlay variants (`--overlay-above/below`) were
 * removed alongside the hover trigger.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";

import { ToolBlock } from "./ToolBlock";
import type { ToolNode } from "../types";

afterEach(() => {
    cleanup();
});

const baseTool: ToolNode = {
    type: "tool",
    id: "tc-1",
    tool: "Bash",
    params: { command: "ls" },
    status: "success",
    collapsed: true,
    summary: "Bash ls",
};

describe("ToolBlock — panel mode", () => {
    it("collapsed (default success) → panel has `--hidden` class", () => {
        const { container } = render(() => (
            <ToolBlock node={baseTool} pinned={false} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--hidden")).toBe(true);
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(false);
    });

    it("pinned → panel is in-flow (`--flow`)", () => {
        const { container } = render(() => (
            <ToolBlock node={baseTool} pinned={true} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(true);
        expect(panel!.classList.contains("agent-tool-panel--hidden")).toBe(false);
    });

    it("running (auto-expanded) → panel is in-flow (`--flow`)", () => {
        const running: ToolNode = { ...baseTool, status: "running" };
        const { container } = render(() => (
            <ToolBlock node={running} pinned={false} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(true);
    });

    it("pending_approval (auto-expanded) → panel is in-flow", () => {
        const pending: ToolNode = { ...baseTool, status: "pending_approval" };
        const { container } = render(() => (
            <ToolBlock node={pending} pinned={false} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(true);
    });

    it("heldOpen (completed, held after live completion) → panel is in-flow", () => {
        // The scroll-driven post-completion hold: a completed tool held open
        // renders expanded until its row scrolls off the top.
        const { container } = render(() => (
            <ToolBlock node={baseTool} pinned={false} heldOpen={true} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(true);
        expect(panel!.classList.contains("agent-tool-panel--hidden")).toBe(false);
    });

    it("heldOpen + failed → panel is in-flow, same as a held-open success", () => {
        // A `failed` tool is an agent-caused error, not a user dismissal — it
        // should hold open exactly like `success` until scrolled off, per
        // ANALYSIS_TOOL_BLOCK_SCROLL_DRIVEN_COLLAPSE_2026_06_16.md's own
        // recommendation. Previously this rendered `--hidden` immediately.
        const failed: ToolNode = { ...baseTool, status: "failed" };
        const { container } = render(() => (
            <ToolBlock node={failed} pinned={false} heldOpen={true} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel");
        expect(panel).not.toBeNull();
        expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(true);
        expect(panel!.classList.contains("agent-tool-panel--hidden")).toBe(false);
    });

    it.each(["denied", "canceled"] as const)(
        "heldOpen + %s → panel stays hidden (user-dismissed, collapses immediately)",
        (status) => {
            const node: ToolNode = { ...baseTool, status };
            const { container } = render(() => (
                <ToolBlock node={node} pinned={false} heldOpen={true} onTogglePin={() => {}} />
            ));
            const panel = container.querySelector(".agent-tool-panel");
            expect(panel).not.toBeNull();
            expect(panel!.classList.contains("agent-tool-panel--hidden")).toBe(true);
            expect(panel!.classList.contains("agent-tool-panel--flow")).toBe(false);
        },
    );

    it("calls onHoldOpen once when a running tool completes (active→inactive)", () => {
        const onHoldOpen = vi.fn();
        const [node, setNode] = createSignal<ToolNode>({ ...baseTool, status: "running" });
        render(() => (
            <ToolBlock node={node()} pinned={false} onTogglePin={() => {}} onHoldOpen={onHoldOpen} />
        ));
        expect(onHoldOpen).not.toHaveBeenCalled(); // still running
        setNode({ ...baseTool, status: "success" }); // completes live
        expect(onHoldOpen).toHaveBeenCalledTimes(1);
    });

    it("does NOT call onHoldOpen for an already-completed tool on mount (loaded history)", () => {
        const onHoldOpen = vi.fn();
        render(() => (
            <ToolBlock node={baseTool} pinned={false} onTogglePin={() => {}} onHoldOpen={onHoldOpen} />
        ));
        expect(onHoldOpen).not.toHaveBeenCalled();
    });

    // ── Hover is not a trigger ────────────────────────────────────────
    // Asserts the core invariant of SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28:
    // hovering a completed tool row produces zero visible state change.
    // No expansion, no overlay class, no inline max-height.
    it("mouseenter on a completed tool does NOT expand the panel", () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <ToolBlock node={baseTool} pinned={false} onTogglePin={() => {}} />
            ));
            const root = container.querySelector(".agent-tool-block") as HTMLElement;
            fireEvent.mouseEnter(root);
            // Wait well past the old 150ms hover-enter delay so we
            // catch a regression where the hover timer creeps back in.
            vi.advanceTimersByTime(1000);
            const panel = container.querySelector(".agent-tool-panel") as HTMLElement;
            expect(panel.classList.contains("agent-tool-panel--hidden")).toBe(true);
            expect(panel.classList.contains("agent-tool-panel--flow")).toBe(false);
            expect(panel.style.maxHeight).toBe("");
        } finally {
            vi.useRealTimers();
        }
    });

    it("mouseenter on a running tool keeps the panel in flow (no change)", () => {
        // Auto-expand keeps the panel open for running tools regardless
        // of hover state. Verifying the in-flow render survives the
        // mouseenter event guards against an over-zealous hover-trigger
        // removal that would also drop the auto-expand path.
        const running: ToolNode = { ...baseTool, status: "running" };
        const { container } = render(() => (
            <ToolBlock node={running} pinned={false} onTogglePin={() => {}} />
        ));
        const root = container.querySelector(".agent-tool-block") as HTMLElement;
        fireEvent.mouseEnter(root);
        const panel = container.querySelector(".agent-tool-panel") as HTMLElement;
        expect(panel.classList.contains("agent-tool-panel--flow")).toBe(true);
    });

    it("pinned panel has no inline `max-height`", () => {
        const { container } = render(() => (
            <ToolBlock node={baseTool} pinned={true} onTogglePin={() => {}} />
        ));
        const panel = container.querySelector(".agent-tool-panel") as HTMLElement;
        expect(panel.style.maxHeight).toBe("");
    });

    // ── Command tooltip — narrower than the removed hover-to-peek system:
    // static text only (the bare command), no expansion, suppressed once
    // the panel is already expanded. See ToolBlock.tsx's header comment.
    describe("command tooltip", () => {
        // The peek overlay is Portal-rendered at document.body (PeekOverlay.tsx
        // — escapes each virtualized row's own CSS stacking context, see that
        // file's doc comment), so it lives in `document.body`, not `container`.
        // A real 150ms enter-delay gates its DOM presence (mirrors
        // UserMessageBlock.tsx's "Session context" hover-to-peek), so these
        // tests use fake timers and advance past it.
        const hoverToolName = (container: HTMLElement) => {
            const anchor = container.querySelector(".agent-tool-name-peek-anchor") as HTMLElement;
            fireEvent.mouseEnter(anchor);
            vi.advanceTimersByTime(200);
        };

        it("collapsed: hovering the name shows the bare command, not the decorated summary", () => {
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <ToolBlock node={baseTool} pinned={false} onTogglePin={() => {}} />
                ));
                expect(container.querySelector(".agent-tool-name-peek-anchor")).not.toBeNull();
                hoverToolName(container);
                const tip = document.body.querySelector(".agent-node-peek-tooltip-body");
                expect(tip).not.toBeNull();
                expect(tip!.textContent).toBe("ls"); // bare params.command, not "Bash ls"
            } finally {
                vi.useRealTimers();
            }
        });

        // SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md §2.3 — the peek
        // overlay gains time + estimated-token lines above the bare command.
        it("shows exact time + time-ago + an estimated token count when the node has a timestamp", () => {
            const timed: ToolNode = { ...baseTool, timestamp: Date.now() - 65_000 };
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <ToolBlock node={timed} pinned={false} onTogglePin={() => {}} />
                ));
                hoverToolName(container);
                const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
                expect(metaLines.length).toBe(2);
                expect(metaLines[0].textContent).toMatch(/\d{2}:\d{2}:\d{2} · 1m ago/);
                expect(metaLines[1].textContent).toMatch(/~\d+ tok \(est\.\)/);
            } finally {
                vi.useRealTimers();
            }
        });

        it("shows no time line when the node has no timestamp", () => {
            const untimed: ToolNode = { ...baseTool, timestamp: undefined };
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <ToolBlock node={untimed} pinned={false} onTogglePin={() => {}} />
                ));
                hoverToolName(container);
                const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
                // Still one line: the token estimate (params always give SOME text).
                expect(metaLines.length).toBe(1);
                expect(metaLines[0].textContent).toMatch(/~\d+ tok \(est\.\)/);
            } finally {
                vi.useRealTimers();
            }
        });

        it("expanded (pinned): hovering the name shows no overlay — command is visible in the panel already", () => {
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <ToolBlock node={baseTool} pinned={true} onTogglePin={() => {}} />
                ));
                hoverToolName(container);
                expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });

        // reagent P2 on PR #2392: this used to assert NO tooltip at all for a
        // tool with no extractable command text — but the time/estimate
        // lines don't depend on cmdText(), and every tool call has at least
        // SOME estimable content (its params, even `{}`), so the fix is that
        // a peek now DOES show here — just without a body line.
        it("a tool kind with no extractable detail (e.g. an untyped tool) still shows a peek with time/estimate, but no body line", () => {
            const opaque: ToolNode = { ...baseTool, tool: "Other", params: {}, timestamp: Date.now() };
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <ToolBlock node={opaque} pinned={false} onTogglePin={() => {}} />
                ));
                hoverToolName(container);
                expect(document.body.querySelector(".agent-node-peek-overlay")).not.toBeNull();
                expect(document.body.querySelector(".agent-node-peek-tooltip-body")).toBeNull();
                const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
                expect(metaLines.length).toBe(2); // time + estimate, both independent of cmdText()
            } finally {
                vi.useRealTimers();
            }
        });

        it("a tool with no cmdText but SOME estimable content still shows the overlay", () => {
            // estimateTokenCount(JSON.stringify({}) + "") = ceil(2/4) = 1 > 0,
            // so an "Other" tool with empty params still has an estimate —
            // documenting that "truly nothing to show" is essentially
            // unreachable via real data, not asserting a null overlay here.
            const trulyEmpty: ToolNode = { ...baseTool, tool: "Other", params: {}, timestamp: undefined };
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <ToolBlock node={trulyEmpty} pinned={false} onTogglePin={() => {}} />
                ));
                hoverToolName(container);
                expect(document.body.querySelector(".agent-node-peek-overlay")).not.toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });

        // ToolBlock instances are reused across status transitions via
        // index-based virtualization (no remount) -- these two assert the
        // suppression is reactive to a live status/pin change on an
        // ALREADY-MOUNTED instance, not just correct on first render.
        it("shows the overlay once a running tool completes, with the cursor already stationary over it (no second mouseenter)", () => {
            // The scenario a naive implementation misses: the user's cursor
            // never moves, only the tool's own status (and hence
            // `expanded()`) changes out from under it. If the anchor's hover
            // handling only acted inside the mouseenter/mouseleave handlers
            // themselves, this would require a SECOND mouseenter that never
            // comes in real usage — a stationary cursor doesn't generate one
            // just because an unrelated prop changed.
            const [node, setNode] = createSignal<ToolNode>({ ...baseTool, status: "running" });
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <ToolBlock node={node()} pinned={false} onTogglePin={() => {}} />
                ));
                const anchor = container.querySelector(".agent-tool-name-peek-anchor") as HTMLElement;
                fireEvent.mouseEnter(anchor); // cursor arrives while still running (panel auto-expanded)
                vi.advanceTimersByTime(200);
                expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
                setNode({ ...baseTool, status: "success" }); // completes; cursor never moves
                const tip = document.body.querySelector(".agent-node-peek-tooltip-body");
                expect(tip).not.toBeNull();
                expect(tip!.textContent).toBe("ls");
            } finally {
                vi.useRealTimers();
            }
        });

        it("hides the overlay the instant an already-hovered tool gets pinned open, with no mouseleave", () => {
            const [pinned, setPinned] = createSignal(false);
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <ToolBlock node={baseTool} pinned={pinned()} onTogglePin={() => {}} />
                ));
                hoverToolName(container);
                expect(document.body.querySelector(".agent-node-peek-overlay")).not.toBeNull();
                setPinned(true); // user clicks elsewhere to pin the panel open; cursor stays put
                expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });
    });

    // ── Result-pill one-shot fade-in (reagent P2 on PR #1975) ────────────
    // The fade-in must be a one-shot class added by a `ref` callback at
    // genuine mount time, NOT a persistent CSS `animation:` on the
    // selector — the pill's `display` also toggles via an unrelated
    // `@container` width breakpoint (_responsive.scss), and a persistent
    // animation would replay on every resize across that breakpoint even
    // for an already-completed, unrelated tool call.
    describe("result-pill one-shot fade-in", () => {
        const withResult: ToolNode = {
            ...baseTool,
            result: { exitCode: 0 } as any,
        };

        it("adds the one-shot class on mount and removes it after animationend", async () => {
            const { container } = render(() => (
                <ToolBlock node={withResult} pinned={false} onTogglePin={() => {}} />
            ));
            const pill = container.querySelector(".agent-tool-result-pill") as HTMLElement;
            expect(pill).not.toBeNull();

            // The class is added a microtask after mount — deferred past
            // Solid's own dynamic `class={...}` effect, which sets
            // `className` wholesale and would otherwise clobber a
            // classList mutation made synchronously in the ref (see
            // `fadeInOnMount`'s doc comment in ToolBlock.tsx).
            await Promise.resolve();
            expect(pill.classList.contains("agent-tool-fade-in-once")).toBe(true);

            pill.dispatchEvent(new Event("animationend"));
            expect(pill.classList.contains("agent-tool-fade-in-once")).toBe(false);
        });

        it("does not re-add the class on an unrelated re-render (only a genuine remount)", async () => {
            // Regression guard for the actual bug: re-rendering with the
            // SAME resultPill (simulating a resize-driven display toggle,
            // not a real remount) must not re-add the one-shot class,
            // since the ref callback only fires on genuine DOM insertion.
            const [node, setNode] = createSignal<ToolNode>(withResult);
            const { container } = render(() => <ToolBlock node={node()} pinned={false} onTogglePin={() => {}} />);
            const pill = container.querySelector(".agent-tool-result-pill") as HTMLElement;
            await Promise.resolve();
            pill.dispatchEvent(new Event("animationend"));
            expect(pill.classList.contains("agent-tool-fade-in-once")).toBe(false);

            // Same status/result, just a new object reference (mirrors an
            // unrelated parent re-render) — the pill is NOT unmounted by
            // Solid (same <Show> branch stays true), so the ref never
            // re-fires and the class must stay off.
            setNode({ ...withResult });
            await Promise.resolve();
            expect(pill.classList.contains("agent-tool-fade-in-once")).toBe(false);
        });
    });
});

// SPEC_ASK_USER_QUESTION_HISTORY_STYLING_2026_08_17.md: a resolved
// AskUserQuestion renders as a user message (`.agent-user-message`), not the
// generic collapsed tool row, so it reads as user input once it scrolls
// into history.
describe("ToolBlock — answered AskUserQuestion renders as a user message", () => {
    const answeredQuestion: ToolNode = {
        type: "tool",
        id: "q-1",
        tool: "Other",
        toolName: "AskUserQuestion",
        params: {},
        status: "success",
        collapsed: true,
        summary: "❓ Answered — Yes",
        answerText: "Yes",
    };

    it("renders `.agent-user-message` with the raw answer text, not `.agent-tool-summary`", () => {
        const { container } = render(() => (
            <ToolBlock node={answeredQuestion} pinned={false} onTogglePin={() => {}} />
        ));
        expect(container.querySelector(".agent-user-message")).not.toBeNull();
        expect(container.querySelector(".agent-tool-summary")).toBeNull();
        expect(container.querySelector(".agent-user-message-content pre")?.textContent).toBe("Yes");
    });

    it("shows a muted timeout note for an auto-answered question", () => {
        const autoAnswered: ToolNode = {
            ...answeredQuestion,
            summary: "⏱️ Auto-answered (no response in 30s) — Yes",
        };
        const { container } = render(() => (
            <ToolBlock node={autoAnswered} pinned={false} onTogglePin={() => {}} />
        ));
        expect(container.querySelector(".agent-user-message-timeout-note")?.textContent).toBe(
            "⏱️ Auto-answered (no response in 30s)",
        );
    });

    it("does not show a timeout note for a manually-answered question", () => {
        const { container } = render(() => (
            <ToolBlock node={answeredQuestion} pinned={false} onTogglePin={() => {}} />
        ));
        expect(container.querySelector(".agent-user-message-timeout-note")).toBeNull();
    });

    it("falls back to the generic collapsed row when answerText is missing (legacy transcript)", () => {
        const legacy: ToolNode = { ...answeredQuestion, answerText: undefined };
        const { container } = render(() => (
            <ToolBlock node={legacy} pinned={false} onTogglePin={() => {}} />
        ));
        expect(container.querySelector(".agent-tool-summary")).not.toBeNull();
        expect(container.querySelector(".agent-user-message")).toBeNull();
    });

    it("does not apply the user-message treatment to a non-question tool, even with matching status", () => {
        const bash: ToolNode = {
            type: "tool",
            id: "b-1",
            tool: "Bash",
            params: { command: "ls" },
            status: "success",
            collapsed: true,
            summary: "Bash ls",
        };
        const { container } = render(() => <ToolBlock node={bash} pinned={false} onTogglePin={() => {}} />);
        expect(container.querySelector(".agent-user-message")).toBeNull();
    });
});
