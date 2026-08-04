// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * UserMessageBlock — render-shape tests for the two variants:
 *
 *   - regular user input: always expanded, no hover/pin handlers, but
 *     gets a hover-to-peek time/estimate overlay (2026-08-03 user request,
 *     same treatment ToolBlock/MarkdownBlock already have).
 *   - startup injection (`isStartup === true`): collapsed by default,
 *     hover-expand after 150ms, click-to-pin. The hover-expand body is
 *     PeekOverlay (Portal-rendered at document.body, top-anchored to the
 *     row) — migrated off its own bespoke position:absolute overlay,
 *     which had the same virtualized-row stacking-context bug PeekOverlay
 *     was built to fix for ToolBlock/MarkdownBlock.
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
                    <UserMessageBlock node={timed} pinned={false} onTogglePin={() => {}} />
                ));
                const root = container.querySelector(".agent-user-message") as HTMLElement;
                fireEvent.mouseEnter(root);
                vi.advanceTimersByTime(200);
                const metaLines = document.body.querySelectorAll(".agent-node-peek-tooltip-meta");
                expect(metaLines.length).toBe(2);
                expect(metaLines[0].textContent).toMatch(/\d{2}:\d{2}:\d{2} · 1m ago/);
                expect(metaLines[1].textContent).toMatch(/~\d+ tok \(est\.\)/);
            } finally {
                vi.useRealTimers();
            }
        });

        it("shows nothing before the enter-delay elapses", () => {
            const { container } = render(() => (
                <UserMessageBlock node={baseNode} pinned={false} onTogglePin={() => {}} />
            ));
            const root = container.querySelector(".agent-user-message") as HTMLElement;
            fireEvent.mouseEnter(root);
            // No advanceTimersByTime — still within the 150ms delay.
            expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
        });

        it("hides on mouseleave", () => {
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <UserMessageBlock node={baseNode} pinned={false} onTogglePin={() => {}} />
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
        // scoped to the summary, not the outer block. Pinned → flow
        // mode → still a real DOM descendant of `container`.
        const togglePin = vi.fn();
        const { container } = render(() => (
            <UserMessageBlock node={startupNode} pinned={true} onTogglePin={togglePin} />
        ));
        const pre = container.querySelector(".agent-user-message-content pre")!;
        fireEvent.click(pre);
        expect(togglePin).not.toHaveBeenCalled();
    });

    it("explicit unpin button on pinned row fires onTogglePin", () => {
        // Pinned → flow mode → still in `container`.
        const togglePin = vi.fn();
        const { container } = render(() => (
            <UserMessageBlock node={startupNode} pinned={true} onTogglePin={togglePin} />
        ));
        const unpin = container.querySelector(".agent-user-message-unpin");
        expect(unpin).not.toBeNull();
        fireEvent.click(unpin!);
        expect(togglePin).toHaveBeenCalledTimes(1);
    });

    it("unpin button is absent when not pinned (pin button takes its place)", () => {
        // Codex P2 round 3: when the row is expanded but not pinned
        // (e.g. via hover), the user must still be able to pin from
        // inside the body. The button slot is shared:
        //   pinned → ✕ unpin
        //   !pinned → 📌 pin
        // Both call onTogglePin.
        const { container } = render(() => (
            <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
        ));
        // Not pinned + collapsed-by-default → body itself isn't
        // rendered yet, in either location.
        expect(container.querySelector(".agent-user-message-pin")).toBeNull();
        expect(container.querySelector(".agent-user-message-unpin")).toBeNull();
        expect(document.body.querySelector(".agent-user-message-pin")).toBeNull();
        expect(document.body.querySelector(".agent-user-message-unpin")).toBeNull();
    });

    it("pin button is visible during hover-expansion and fires onTogglePin", async () => {
        // Drives the codex round-3 flow: hover to expand the body,
        // then click 📌 to pin. Without this affordance, the user
        // would have to leave + re-enter the collapsed summary
        // before the 150ms enter-delay restarted — defeating the
        // "hover to peek · click to pin" hint. Hovering (not pinned)
        // → overlay mode → Portal-rendered at document.body.
        vi.useFakeTimers();
        try {
            const togglePin = vi.fn();
            const { container } = render(() => (
                <UserMessageBlock node={startupNode} pinned={false} onTogglePin={togglePin} />
            ));
            const root = container.querySelector(".agent-user-message") as HTMLElement;
            fireEvent.mouseEnter(root);
            vi.advanceTimersByTime(200);
            // Hover-expanded — pin button now present in document.body.
            const pin = document.body.querySelector(".agent-user-message-pin");
            expect(pin).not.toBeNull();
            expect((pin as HTMLElement).getAttribute("aria-label")).toContain("Pin");
            // Click it.
            (pin as HTMLButtonElement).click();
            expect(togglePin).toHaveBeenCalledTimes(1);
        } finally {
            vi.useRealTimers();
        }
    });

    it("pinned=true renders expanded (full markdown body visible)", () => {
        // Per SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24, the
        // summary is ALWAYS rendered for collapsible rows (so the
        // ARIA / keyboard surface is stable across collapsed/
        // expanded states). The body's visibility is what changes.
        // Hint copy in the summary toggles to reflect the state.
        render(() => (
            <UserMessageBlock node={startupNode} pinned={true} onTogglePin={() => {}} />
        ));
        // Body visible — first identity bullet matches the fixture.
        expect(screen.queryByText(/Identity/)).not.toBeNull();
        // Summary still mounted; hint reflects the pinned state.
        expect(screen.queryByText("Session context")).not.toBeNull();
        expect(screen.queryByText(/click ✕ to collapse/)).not.toBeNull();
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
        // The summary is now ALWAYS mounted. Only the BODY's
        // presence changes across the hover transition. `screen`
        // queries the whole document (including document.body, where
        // PeekOverlay Portals to), so this is unaffected by the
        // Portal migration.
        vi.useFakeTimers();
        try {
            const { container } = render(() => (
                <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
            ));
            const root = container.querySelector(".agent-user-message") as HTMLElement;
            // Pre-hover: summary present, body absent.
            expect(screen.queryByText("Session context")).not.toBeNull();
            expect(screen.queryByText(/Identity/)).toBeNull();
            fireEvent.mouseEnter(root);
            // 100ms in — still pre-threshold.
            vi.advanceTimersByTime(100);
            expect(screen.queryByText(/Identity/)).toBeNull();
            // Past the threshold — body now visible alongside summary.
            vi.advanceTimersByTime(60);
            expect(screen.queryByText(/Identity/)).not.toBeNull();
            expect(screen.queryByText("Session context")).not.toBeNull();
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

    describe("body positioning (Portal overlay vs in-flow)", () => {
        // Hover-expanded body is PeekOverlay (Portal-rendered at
        // document.body, top-anchored to the row — see PeekOverlay.tsx);
        // pinned body drops back into normal document flow (Option B of
        // SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24).

        it("hover-expanded body renders via PeekOverlay in document.body, not in container", async () => {
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
                ));
                const root = container.querySelector(".agent-user-message") as HTMLElement;
                fireEvent.mouseEnter(root);
                vi.advanceTimersByTime(200);
                // Nothing in container — content lives at document.body.
                expect(container.querySelector(".agent-user-message-content")).toBeNull();
                const overlay = document.body.querySelector(".agent-node-peek-overlay");
                expect(overlay).not.toBeNull();
                // Keeps its own accent-bordered visual identity, layered
                // on top of the shared base chrome.
                expect(overlay!.classList.contains("agent-user-message-peek-overlay")).toBe(true);
                const body = overlay!.querySelector(".agent-user-message-content");
                expect(body).not.toBeNull();
                expect(body!.classList.contains("agent-user-message-content--flow")).toBe(false);
            } finally {
                vi.useRealTimers();
            }
        });

        it("pinned body uses the in-flow positioning class inside container", () => {
            const { container } = render(() => (
                <UserMessageBlock node={startupNode} pinned={true} onTogglePin={() => {}} />
            ));
            const body = container.querySelector(".agent-user-message-content");
            expect(body).not.toBeNull();
            expect(body!.classList.contains("agent-user-message-content--flow")).toBe(true);
            // No Portal overlay while pinned (flow mode, not overlay mode).
            expect(document.body.querySelector(".agent-node-peek-overlay")).toBeNull();
        });

        it("regular (non-startup) body uses the in-flow positioning class", () => {
            const { container } = render(() => (
                <UserMessageBlock node={baseNode} pinned={false} onTogglePin={() => {}} />
            ));
            const body = container.querySelector(".agent-user-message-content");
            expect(body).not.toBeNull();
            expect(body!.classList.contains("agent-user-message-content--flow")).toBe(true);
        });

        it("hover-expanded body has an inline max-height on the Portal'd overlay", () => {
            // The overlay's max-height is computed per hover from the
            // scroll container's available space, set inline on the
            // Portal'd wrapper (not the inner .agent-user-message-content
            // div). We can't assert a specific px value (jsdom's
            // getBoundingClientRect returns zeros), but we CAN assert
            // that the style attribute carries a `max-height` rule —
            // proving the inline path fired.
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
                ));
                const root = container.querySelector(".agent-user-message") as HTMLElement;
                fireEvent.mouseEnter(root);
                vi.advanceTimersByTime(200);
                const overlay = document.body.querySelector(".agent-node-peek-overlay") as HTMLElement;
                expect(overlay).not.toBeNull();
                expect(overlay.style.maxHeight).toMatch(/^\d+(\.\d+)?px$/);
            } finally {
                vi.useRealTimers();
            }
        });

        it("pinned body has no inline max-height (in-flow, no cap)", () => {
            const { container } = render(() => (
                <UserMessageBlock node={startupNode} pinned={true} onTogglePin={() => {}} />
            ));
            const body = container.querySelector(".agent-user-message-content") as HTMLElement;
            expect(body).not.toBeNull();
            expect(body.style.maxHeight).toBe("");
        });
    });

    describe("aria-expanded", () => {
        it("is false when collapsed (not pinned, not hovering)", () => {
            const { container } = render(() => (
                <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
            ));
            const summary = container.querySelector(".agent-user-message-summary");
            expect(summary!.getAttribute("aria-expanded")).toBe("false");
        });

        it("is true when pinned (persistent expanded state)", () => {
            const { container } = render(() => (
                <UserMessageBlock node={startupNode} pinned={true} onTogglePin={() => {}} />
            ));
            const summary = container.querySelector(".agent-user-message-summary");
            expect(summary!.getAttribute("aria-expanded")).toBe("true");
        });

        it("is true when transiently hover-expanded (mirrors visual state)", () => {
            vi.useFakeTimers();
            try {
                const { container } = render(() => (
                    <UserMessageBlock node={startupNode} pinned={false} onTogglePin={() => {}} />
                ));
                const root = container.querySelector(".agent-user-message") as HTMLElement;
                fireEvent.mouseEnter(root);
                vi.advanceTimersByTime(200);
                const summary = container.querySelector(".agent-user-message-summary");
                expect(summary!.getAttribute("aria-expanded")).toBe("true");
            } finally {
                vi.useRealTimers();
            }
        });
    });
});
