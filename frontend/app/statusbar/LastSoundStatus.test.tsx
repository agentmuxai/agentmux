// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";

vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/app/store/agent-runtime-live", () => ({ sharedAgentRuntime: () => ({ rows: () => [] }) }));
const openAt = vi.hoisted(() => vi.fn());
vi.mock("@/app/view/audit/open-audit", () => ({ openAuditAtSound: openAt }));
const reveal = vi.hoisted(() => vi.fn());
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: reveal }));

import { __resetPlayedSounds, noteSound } from "@/app/notification/sound/sound-history";
import { LAST_SOUND_TIP, LastSoundStatus } from "./LastSoundStatus";

afterEach(() => {
    cleanup();
    __resetPlayedSounds();
    vi.clearAllMocks();
});

// An agent's id is its UID; the item names it by what the person calls it.
const agent = (name: string, block_id: string): AgentRuntime => ({
    agent_id: `6b6e9e6a-125d-47de-9594-${name}`,
    block_id,
    state: "working",
    desired: "running",
    name,
    provider: "claude",
    icon: "",
    runtime: "host",
});

describe("LastSoundStatus", () => {
    it("shows nothing until a sound plays", () => {
        const { container } = render(() => <LastSoundStatus rows={() => []} />);
        expect(container.querySelector(".status-last-sound")).toBeNull();
    });

    it("says who made the last sound and how long ago, as it plays", async () => {
        render(() => <LastSoundStatus rows={() => [agent("korp", "blk-korp")]} />);
        noteSound({ blockId: "blk-korp", event: "tool", tool: "Bash", at: Date.now() });
        await waitFor(() => expect(screen.getByRole("button", { name: /^♪ korp · Bash · \d+s ago\./ })).toBeInTheDocument());
        expect(screen.getByRole("button").getAttribute("data-tip")).toBe(LAST_SOUND_TIP);
    });

    it("leaves the agent out when no running agent has the pane", async () => {
        render(() => <LastSoundStatus rows={() => []} />);
        noteSound({ blockId: "blk-gone", event: "agent.turn.complete", at: Date.now() });
        await waitFor(() => expect(screen.getByRole("button").textContent).toMatch(/^♪ Agent turn completed · \d+s ago$/));
    });

    it("has the bar fit again as it comes, since a hidden slot changes no size the bar watches", async () => {
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => (cb(0), 0));
        const bar = document.createElement("div");
        bar.className = "status-bar";
        bar.setAttribute("data-fit", "2");
        document.body.append(bar);
        try {
            render(() => <LastSoundStatus rows={() => []} />, { container: bar });
            expect(bar.getAttribute("data-fit")).toBe("2");
            noteSound({ blockId: "blk-korp", event: "tool", tool: "Bash", at: Date.now() });
            await waitFor(() => expect(bar.hasAttribute("data-fit")).toBe(false));
        } finally {
            bar.remove();
            vi.unstubAllGlobals();
        }
    });

    it("goes away a minute after the sound", async () => {
        const { container } = render(() => <LastSoundStatus rows={() => []} />);
        noteSound({ blockId: "blk-korp", event: "tool", tool: "Bash", at: Date.now() - 61_000 });
        await new Promise((r) => setTimeout(r, 0));
        expect(container.querySelector(".status-last-sound")).toBeNull();
    });

    it("opens the Audit at the sound's row on a click, and the agent's pane on a shift-click", async () => {
        render(() => <LastSoundStatus rows={() => [agent("korp", "blk-korp")]} />);
        const sound = { blockId: "blk-korp", event: "tool" as const, tool: "Bash", at: Date.now() };
        noteSound(sound);
        const item = await screen.findByRole("button");
        fireEvent.click(item);
        expect(openAt).toHaveBeenCalledWith(sound);
        expect(reveal).not.toHaveBeenCalled();
        fireEvent.click(item, { shiftKey: true });
        expect(reveal).toHaveBeenCalledWith("blk-korp", { source: "audit" });
        expect(openAt).toHaveBeenCalledTimes(1);
    });
});
