// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AgentRuntime } from "@/types/rpc/AgentRuntime";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { StoppedAgents } from "./swarm-stopped-agents";

afterEach(cleanup);

const agent = (id: string, state: AgentRuntime["state"] = "stopped"): AgentRuntime => ({
    agent_id: id,
    state,
    desired: "stopped",
    name: `Agent ${id}`,
    provider: "claude",
    icon: "",
});

describe("StoppedAgents", () => {
    it("shows nothing when no agent is stopped", () => {
        const { container } = render(() => <StoppedAgents agents={() => []} hasLive={() => true} onOpen={() => {}} />);
        expect(container.querySelector(".swarm-stopped")).toBeNull();
    });

    it("is open when the stopped agents are all there is, and says each one's state", () => {
        const { container } = render(() => (
            <StoppedAgents agents={() => [agent("a"), agent("b", "error")]} hasLive={() => false} onOpen={() => {}} />
        ));
        expect(container.querySelectorAll(".swarm-stopped__row").length).toBe(2);
        expect(container.querySelector(".swarm-stopped__count")!.textContent).toBe("2");
        expect(container.querySelector(".swarm-stopped__state--error")!.textContent).toBe("error");
    });

    it("starts folded under live agents, unfolds from its heading, and folds again", () => {
        const { container } = render(() => (
            <StoppedAgents agents={() => [agent("a")]} hasLive={() => true} onOpen={() => {}} />
        ));
        expect(container.querySelectorAll(".swarm-stopped__row").length).toBe(0);
        const heading = container.querySelector(".swarm-stopped__heading")!;
        expect(heading.getAttribute("aria-expanded")).toBe("false");
        fireEvent.click(heading);
        expect(container.querySelectorAll(".swarm-stopped__row").length).toBe(1);
        fireEvent.click(heading);
        expect(container.querySelectorAll(".swarm-stopped__row").length).toBe(0);
    });

    it("opens the agent whose Open was pressed", () => {
        const onOpen = vi.fn();
        const { getByRole } = render(() => (
            <StoppedAgents agents={() => [agent("a"), agent("b")]} hasLive={() => false} onOpen={onOpen} />
        ));
        fireEvent.click(getByRole("button", { name: "Open Agent b" }));
        expect(onOpen).toHaveBeenCalledTimes(1);
        expect(onOpen.mock.calls[0][0].agent_id).toBe("b");
    });
});
