// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSessionNotices — the session banners, now rendered above the composer
 * strip instead of inside the Shell drawer
 * (SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md §3).
 *
 * The point of the move is that these are visible without opening anything,
 * so the tests assert the banners' own gating rather than any drawer state —
 * this component has no knowledge of the drawer at all, which is the property
 * that makes the disclosure unconditional.
 */

import { render, screen, waitFor } from "@solidjs/testing-library";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi, beforeEach } from "vitest";

import { AgentSessionNotices } from "./AgentSessionNotices";

const clearSessionFlag = vi.hoisted(() => vi.fn(async () => {}));
const archiveSession = vi.hoisted(() => vi.fn(async () => {}));

vi.mock("../session-actions", async (importOriginal) => {
    const actual = await importOriginal<typeof import("../session-actions")>();
    return { ...actual, clearSessionFlag, archiveSession };
});

const block = (meta: Record<string, unknown>) => () => ({ meta }) as never;

describe("AgentSessionNotices", () => {
    beforeEach(() => {
        clearSessionFlag.mockClear();
        archiveSession.mockClear();
    });

    it("renders nothing when no session flag is set", () => {
        const { container } = render(() => (
            <AgentSessionNotices blockId="b1" blockAtom={block({})} providerId="claude" />
        ));
        expect(container.querySelector(".agent-interrupted-banner")).toBeNull();
        expect(container.querySelector(".agent-archived-banner")).toBeNull();
    });

    it("discloses an interrupted session and dismisses it by clearing the flag", async () => {
        render(() => (
            <AgentSessionNotices
                blockId="b1"
                blockAtom={block({ "session:was_interrupted": true })}
                providerId="claude"
            />
        ));
        expect(screen.getByText(/interrupted by a restart/i)).toBeInTheDocument();
        await userEvent.click(screen.getByRole("button", { name: /Dismiss/ }));
        await waitFor(() => expect(clearSessionFlag).toHaveBeenCalledWith("b1", "session:was_interrupted"));
    });

    it("discloses a failed resume", () => {
        render(() => (
            <AgentSessionNotices blockId="b1" blockAtom={block({ "session:resume_failed": true })} providerId="claude" />
        ));
        expect(screen.getByText(/Couldn't resume the previous conversation/i)).toBeInTheDocument();
    });

    it("warns with an Archive action once the session passes the large threshold", async () => {
        render(() => (
            <AgentSessionNotices blockId="b1" blockAtom={block({ "session:line_count": 500_000 })} providerId="claude" />
        ));
        expect(screen.getByText(/500,000 lines/)).toBeInTheDocument();
        await userEvent.click(screen.getByRole("button", { name: /Archive/ }));
        await waitFor(() => expect(archiveSession).toHaveBeenCalledWith("b1"));
    });

    it("shows Restore and Export while archived, and no large-session warning", () => {
        render(() => (
            <AgentSessionNotices
                blockId="b1"
                blockAtom={block({ "session:line_count": 900_000, "session:archived_at": 1_700_000_000_000 })}
                providerId="claude"
            />
        ));
        expect(screen.getByRole("button", { name: /Restore/ })).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /Export/ })).toBeInTheDocument();
        expect(screen.queryByText(/Consider archiving/)).toBeNull();
    });

    it("renders nothing for non-Claude providers", () => {
        const { container } = render(() => (
            <AgentSessionNotices
                blockId="b1"
                blockAtom={block({ "session:was_interrupted": true })}
                providerId="gemini"
            />
        ));
        expect(container.querySelector(".agent-interrupted-banner")).toBeNull();
    });
});
