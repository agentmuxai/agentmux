// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SetupNote: AgentMux's own setup prompts (the start-up document, the opening update) as one muted line
 * that opens on a click. For the agent, not the person: never the human's turn, never a coloured card.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import { OPENING_SUMMARY_PROMPT } from "../startup/opening-summary";
import type { UserMessageNode } from "../types";
import { SetupNote, setupLabel } from "./SetupNote";

afterEach(() => cleanup());

const startup: UserMessageNode = {
    type: "user_message",
    id: "user_startup",
    message: "# Session Context\n\n## Identity\n- Name: AgentA\n",
    isStartup: true,
};
const opening: UserMessageNode = { ...startup, id: "user_opening", message: OPENING_SUMMARY_PROMPT };

describe("SetupNote", () => {
    it("says what it is in one line, with the text hidden", () => {
        render(() => <SetupNote node={startup} pinned={false} onTogglePin={() => {}} />);
        expect(screen.getByText("Session set up")).toBeInTheDocument();
        expect(screen.queryByText(/Identity/)).toBeNull();
        expect(screen.getByRole("button").getAttribute("aria-expanded")).toBe("false");
    });

    it("names the opening update as one, whichever way the prompt is worded after its heading", () => {
        expect(setupLabel(OPENING_SUMMARY_PROMPT)).toBe("Asked for an opening update");
        expect(setupLabel(startup.message)).toBe("Session set up");
        // A document that merely mentions it is still the start-up document.
        expect(setupLabel("# Session Context\n\n## Identity\nsee ## Opening update below")).toBe("Session set up");
        render(() => <SetupNote node={opening} pinned={false} onTogglePin={() => {}} />);
        expect(screen.getByText("Asked for an opening update")).toBeInTheDocument();
    });

    it("opens on a click (the pin) and shows the whole text", () => {
        const toggle = vi.fn();
        render(() => <SetupNote node={startup} pinned={false} onTogglePin={toggle} />);
        fireEvent.click(screen.getByRole("button"));
        expect(toggle).toHaveBeenCalledTimes(1);
        cleanup();
        render(() => <SetupNote node={startup} pinned={true} onTogglePin={toggle} />);
        expect(screen.getByRole("button").getAttribute("aria-expanded")).toBe("true");
        expect(screen.getByText(/Identity/)).toBeInTheDocument();
    });

    it("is the origin container's quiet style, not the human's turn, and not a card with a colour", () => {
        const { container } = render(() => <SetupNote node={startup} pinned={true} onTogglePin={() => {}} />);
        const root = container.querySelector("[data-testid='agent-setup-note']")!;
        expect(root.classList.contains("agent-origin")).toBe(true);
        expect(root.classList.contains("agent-user-message")).toBe(false);
        expect(container.querySelector(".agent-user-message")).toBeNull();
        // No tag or badge: the label is the whole header.
        expect(container.querySelector(".ui-tag")).toBeNull();
    });
});
