// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * UserMessageBlock — the human's own input: always expanded, with a hover-to-peek time/estimate overlay
 * (2026-08-03 user request, same treatment ToolBlock/MarkdownBlock already have) and a one-line label for
 * a broadcast, a Supervisor continuation or a paired device. AgentMux's own setup prompts are a
 * SetupNote (SetupNote.test.tsx), never this block.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { UserMessageBlock } from "./UserMessageBlock";
import type { UserMessageNode } from "../types";

afterEach(() => {
    cleanup();
});

const baseNode: UserMessageNode = {
    type: "user_message",
    id: "user_0",
    message: "Can you run the tests?",
    timestamp: 0,
};

describe("UserMessageBlock — regular user input", () => {
    it("says which device a message was sent from, and nothing for one typed here", () => {
        render(() => (
            <UserMessageBlock node={{ ...baseNode, fromDevice: "Pixel 9" }} />
        ));
        expect(screen.getByText("From Pixel 9")).toBeInTheDocument();
        cleanup();
        render(() => <UserMessageBlock node={baseNode} />);
        expect(screen.queryByText(/^From /)).toBeNull();
    });

    it("renders expanded with the message body, no summary row", () => {
        render(() => (
            <UserMessageBlock node={baseNode} />
        ));
        // Body present
        expect(screen.queryByText("Can you run the tests?")).not.toBeNull();
        // No collapsed summary row
        expect(screen.queryByText("Session context")).toBeNull();
    });

    it("has no summary row and no pin: a click on it does nothing", () => {
        const { container } = render(() => <UserMessageBlock node={baseNode} />);
        expect(container.querySelector("button")).toBeNull();
        expect(container.querySelector(".agent-user-message")!.className).toBe("agent-user-message");
    });

    // 2026-08-03 user request: regular (non-startup, always-visible) input
    // gets the same hover-to-peek time/estimate treatment as tool calls
    // and thinking clumps. PeekOverlay is Portal-rendered at document.body,
    // so these query `document.body`, not `container` — same pattern as
    // ToolBlock.test.tsx / MarkdownBlock.test.tsx.
    describe("hover-to-peek (time + estimate)", () => {
        it("shows exact time + time-ago + an estimated token count on hover", () => {
            const timed: UserMessageNode = { ...baseNode, timestamp: Date.now() - 65_000 };
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <UserMessageBlock node={timed} />
                ));
                const root = container.querySelector(".agent-user-message") as HTMLElement;
                fireEvent.mouseEnter(root);
                vi.advanceTimersByTime(200);
                const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
                // One line: time and tokens side by side (PeekMetaRow).
                expect(metaLines.length).toBe(1);
                expect(metaLines[0].querySelector(".agent-node-peek-tooltip-time")?.textContent).toMatch(/\d{1,2}:\d{2}:\d{2} (?:AM|PM) · 1m ago/);
                expect(metaLines[0].querySelector(".agent-node-peek-tooltip-tokens")?.textContent).toMatch(/~\d+ tok \(est\.\)/);
            } finally {
                vi.useRealTimers();
            }
        });

        it("shows nothing before the enter-delay elapses", () => {
            const { container } = render(() => (
                <UserMessageBlock node={baseNode} />
            ));
            const root = container.querySelector(".agent-user-message") as HTMLElement;
            fireEvent.mouseEnter(root);
            // No advanceTimersByTime — still within the 50ms delay.
            expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
        });

        it("hides on mouseleave", () => {
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <UserMessageBlock node={baseNode} />
                ));
                const root = container.querySelector(".agent-user-message") as HTMLElement;
                fireEvent.mouseEnter(root);
                vi.advanceTimersByTime(200);
                expect(document.body.querySelector(".agent-node-peek-overlay")).not.toBeNull();
                fireEvent.mouseLeave(root);
                expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
            } finally {
                vi.useRealTimers();
            }
        });
    });
});

describe("UserMessageBlock — swarm broadcast note", () => {
    const broadcastNode: UserMessageNode = {
        ...baseNode,
        id: "user_bcast",
        message: "great, merge on approval",
        broadcast: { recipients: 7, msgId: "m1" },
    };

    it("shows the broadcast note with the recipient count above the body", () => {
        render(() => <UserMessageBlock node={broadcastNode} />);
        expect(screen.queryByText("Broadcast · 7 agents")).not.toBeNull();
        expect(screen.queryByText("great, merge on approval")).not.toBeNull();
    });

    it("says just Broadcast for one recipient or an unknown count", () => {
        render(() => <UserMessageBlock node={{ ...broadcastNode, broadcast: { recipients: 1 } }} />);
        expect(screen.queryByText("Broadcast")).not.toBeNull();
        cleanup();
        render(() => <UserMessageBlock node={{ ...broadcastNode, broadcast: {} }} />);
        expect(screen.queryByText("Broadcast")).not.toBeNull();
    });

    it("a plain message has no broadcast note", () => {
        render(() => <UserMessageBlock node={baseNode} />);
        expect(screen.queryByText(/Broadcast/)).toBeNull();
    });
});
