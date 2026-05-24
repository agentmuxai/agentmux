// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * UserMessageBlock — render-shape tests for the two variants:
 *
 *   - regular user input: always expanded, no hover/pin handlers.
 *   - startup injection (`isStartup === true`): collapsed by default,
 *     hover-expand after 150ms, click-to-pin.
 *
 * Spec: `docs/specs/SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md`.
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

const startupNode: UserMessageNode = {
    ...baseNode,
    id: "user_startup",
    message: "# Session Context\n\n## Identity\n- Name: AgentA\n",
    isStartup: true,
};

describe("UserMessageBlock — regular user input", () => {
    it("renders expanded with the message body, no summary row", () => {
        render(() => (
            <UserMessageBlock node={baseNode} pinned={false} onTogglePin={() => {}} />
        ));
        // Body present
        expect(screen.queryByText("Can you run the tests?")).not.toBeNull();
        // No collapsed summary row
        expect(screen.queryByText("Session context")).toBeNull();
    });

    it("ignores pin prop (regular input is always expanded)", () => {
        render(() => (
            <UserMessageBlock node={baseNode} pinned={true} onTogglePin={() => {}} />
        ));
        expect(screen.queryByText("Can you run the tests?")).not.toBeNull();
    });

    it("click on a regular row does NOT call onTogglePin", () => {
        const togglePin = vi.fn();
        const { container } = render(() => (
            <UserMessageBlock node={baseNode} pinned={false} onTogglePin={togglePin} />
        ));
        const root = container.querySelector(".agent-user-message")!;
        fireEvent.click(root);
        expect(togglePin).not.toHaveBeenCalled();
    });

    it("applies neither the --startup nor the --collapsed class", () => {
        const { container } = render(() => (
            <UserMessageBlock node={baseNode} pinned={false} onTogglePin={() => {}} />
        ));
        const root = container.querySelector(".agent-user-message")!;
        expect(root.classList.contains("agent-user-message--startup")).toBe(false);
        expect(root.classList.contains("agent-user-message--collapsed")).toBe(false);
    });
});

describe("UserMessageBlock — startup injection", () => {
    it("renders collapsed summary, body absent by default", () => {
        render(() => (
            <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
        ));
        expect(screen.queryByText("Session context")).not.toBeNull();
        expect(screen.queryByText(/Identity/)).toBeNull();
    });

    it("click on collapsed summary fires onTogglePin", () => {
        const togglePin = vi.fn();
        const { container } = render(() => (
            <UserMessageBlock node={startupNode} pinned={false} onTogglePin={togglePin} />
        ));
        const summary = container.querySelector(".agent-user-message-summary")!;
        fireEvent.click(summary);
        expect(togglePin).toHaveBeenCalledTimes(1);
    });

    it("click on expanded <pre> body does NOT fire onTogglePin", () => {
        // Codex P2 on PR #1020 first cut: clicking inside the
        // expanded body (to place the caret or select text for
        // copying) must not unpin the row. Click handler is
        // scoped to the summary, not the outer block.
        const togglePin = vi.fn();
        const { container } = render(() => (
            <UserMessageBlock node={startupNode} pinned={true} onTogglePin={togglePin} />
        ));
        const pre = container.querySelector(".agent-user-message-content pre")!;
        fireEvent.click(pre);
        expect(togglePin).not.toHaveBeenCalled();
    });

    it("explicit unpin button on pinned row fires onTogglePin", () => {
        const togglePin = vi.fn();
        const { container } = render(() => (
            <UserMessageBlock node={startupNode} pinned={true} onTogglePin={togglePin} />
        ));
        const unpin = container.querySelector(".agent-user-message-unpin");
        expect(unpin).not.toBeNull();
        fireEvent.click(unpin!);
        expect(togglePin).toHaveBeenCalledTimes(1);
    });

    it("unpin button is absent when not pinned", () => {
        const { container } = render(() => (
            <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
        ));
        expect(container.querySelector(".agent-user-message-unpin")).toBeNull();
    });

    it("pinned=true renders expanded (full markdown body visible)", () => {
        render(() => (
            <UserMessageBlock node={startupNode} pinned={true} onTogglePin={() => {}} />
        ));
        // Body visible — first identity bullet matches the fixture
        expect(screen.queryByText(/Identity/)).not.toBeNull();
        // No collapsed summary row when expanded
        expect(screen.queryByText("Session context")).toBeNull();
    });

    it("pinned=true gets the --pinned class on the root", () => {
        const { container } = render(() => (
            <UserMessageBlock node={startupNode} pinned={true} onTogglePin={() => {}} />
        ));
        const root = container.querySelector(".agent-user-message")!;
        expect(root.classList.contains("agent-user-message--pinned")).toBe(true);
        expect(root.classList.contains("agent-user-message--startup")).toBe(true);
        expect(root.classList.contains("agent-user-message--expanded")).toBe(true);
    });

    it("hover (mouseenter→delay) expands the body after 150ms", async () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
            ));
            const root = container.querySelector(".agent-user-message") as HTMLElement;
            // Pre-hover: collapsed summary present, body absent.
            expect(screen.queryByText("Session context")).not.toBeNull();
            expect(screen.queryByText(/Identity/)).toBeNull();
            fireEvent.mouseEnter(root);
            // 100ms in — still collapsed (under the 150ms threshold).
            vi.advanceTimersByTime(100);
            expect(screen.queryByText(/Identity/)).toBeNull();
            // Past the threshold — expanded.
            vi.advanceTimersByTime(60);
            expect(screen.queryByText(/Identity/)).not.toBeNull();
            expect(screen.queryByText("Session context")).toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });

    it("mouseleave before the enter-delay cancels the expansion", async () => {
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
            ));
            const root = container.querySelector(".agent-user-message") as HTMLElement;
            fireEvent.mouseEnter(root);
            vi.advanceTimersByTime(100);
            fireEvent.mouseLeave(root);
            vi.advanceTimersByTime(200);
            // Still collapsed — the pending timer was cleared on leave.
            expect(screen.queryByText(/Identity/)).toBeNull();
            expect(screen.queryByText("Session context")).not.toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });

    it("collapsed summary is a real <button> with aria-expanded", () => {
        // Codex P2 round 2: the summary must be keyboard-operable.
        // Rendering as <button> gives Tab focus + Space/Enter
        // activation for free. aria-expanded mirrors the pin state.
        const { container } = render(() => (
            <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
        ));
        const summary = container.querySelector(".agent-user-message-summary");
        expect(summary).not.toBeNull();
        expect(summary!.tagName).toBe("BUTTON");
        expect(summary!.getAttribute("type")).toBe("button");
        expect(summary!.getAttribute("aria-expanded")).toBe("false");
    });

    it("button.click() on the summary fires onTogglePin (Space/Enter contract)", () => {
        // Native <button> dispatches click on Space/Enter. Driving
        // .click() directly matches the keyboard-activation path that
        // jsdom would route through HTMLButtonElement.
        const togglePin = vi.fn();
        const { container } = render(() => (
            <UserMessageBlock node={startupNode} pinned={false} onTogglePin={togglePin} />
        ));
        const summary = container.querySelector(".agent-user-message-summary") as HTMLButtonElement;
        summary.click();
        expect(togglePin).toHaveBeenCalledTimes(1);
    });
});
