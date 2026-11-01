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
        const { container } = render(() => <StoppedAgents agents={() => []} hasLive={() => true} onOpen={() => {}} onStart={() => {}} />);
        expect(container.querySelector(".swarm-stopped")).toBeNull();
    });

    it("is open when the stopped agents are all there is, and says each one's state", () => {
        const { container } = render(() => (
            <StoppedAgents agents={() => [agent("a"), agent("b", "error")]} hasLive={() => false} onOpen={() => {}} onStart={() => {}} />
        ));
        expect(container.querySelectorAll(".swarm-stopped__row").length).toBe(2);
        expect(container.querySelector(".swarm-stopped__count")!.textContent).toBe("2");
        expect(container.querySelector(".agent-runtime-chip--error")!.textContent).toBe("error");
    });

    it("starts folded under live agents, unfolds from its heading, and folds again", () => {
        const { container } = render(() => (
            <StoppedAgents agents={() => [agent("a")]} hasLive={() => true} onOpen={() => {}} onStart={() => {}} />
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
            <StoppedAgents agents={() => [agent("a"), agent("b")]} hasLive={() => false} onOpen={onOpen} onStart={() => {}} />
        ));
        fireEvent.click(getByRole("button", { name: "Open Agent b" }));
        expect(onOpen).toHaveBeenCalledTimes(1);
        expect(onOpen.mock.calls[0][0].agent_id).toBe("b");
    });

    it("opens the agent when its row is clicked or Enter is pressed on it, but a press on Start is only Start", async () => {
        const onOpen = vi.fn();
        const onStart = vi.fn();
        const { container, getByRole } = render(() => (
            <StoppedAgents agents={() => [agent("a")]} hasLive={() => false} onOpen={onOpen} onStart={onStart} />
        ));
        const row = container.querySelector<HTMLElement>(".swarm-stopped__row")!;
        expect(row.title).toContain("click to open");
        fireEvent.click(container.querySelector(".swarm-stopped__name")!);
        expect(onOpen).toHaveBeenCalledTimes(1);
        fireEvent.click(container.querySelector(".swarm-stopped__name")!);
        expect(onOpen, "a second press while the first is out waits").toHaveBeenCalledTimes(1);
        await vi.waitFor(() => expect(row.querySelector("button:disabled")).toBeNull());
        fireEvent.keyDown(row, { key: "Enter" });
        expect(onOpen).toHaveBeenCalledTimes(2);
        await vi.waitFor(() => expect(row.querySelector("button:disabled")).toBeNull());
        fireEvent.click(getByRole("button", { name: "Start Agent a" }));
        expect(onStart).toHaveBeenCalledTimes(1);
        expect(onOpen).toHaveBeenCalledTimes(2);
    });

    it("says why srv refused an Open, under that row, and clears it on the next press", async () => {
        const onOpen = vi
            .fn<(a: AgentRuntime) => Promise<string | undefined>>()
            .mockResolvedValueOnce("AGENT_LIVE_ELSEWHERE: it runs in another AgentMux")
            .mockResolvedValueOnce(undefined);
        const { getByRole, container, findByRole } = render(() => (
            <StoppedAgents agents={() => [agent("a"), agent("b")]} hasLive={() => false} onOpen={onOpen} onStart={() => {}} />
        ));
        fireEvent.click(getByRole("button", { name: "Open Agent b" }));
        const alert = await findByRole("alert");
        expect(alert.textContent).toContain("AGENT_LIVE_ELSEWHERE");
        expect(container.querySelectorAll('[role="alert"]').length).toBe(1);
        fireEvent.click(getByRole("button", { name: "Open Agent b" }));
        await vi.waitFor(() => expect(container.querySelector('[role="alert"]')).toBeNull());
    });

    it("starts the agent whose Start was pressed, and holds both buttons until srv answers", async () => {
        let answer: (why: string | undefined) => void = () => {};
        const onStart = vi.fn((_agent: AgentRuntime) => new Promise<string | undefined>((resolve) => (answer = resolve)));
        const onOpen = vi.fn();
        const { getByRole } = render(() => (
            <StoppedAgents agents={() => [agent("a"), agent("b")]} hasLive={() => false} onOpen={onOpen} onStart={onStart} />
        ));
        fireEvent.click(getByRole("button", { name: "Start Agent b" }));
        expect(onStart).toHaveBeenCalledTimes(1);
        expect(onStart.mock.calls[0][0].agent_id).toBe("b");
        const starting = getByRole("button", { name: "Start Agent b" }) as HTMLButtonElement;
        expect(starting.textContent).toBe("Starting…");
        expect(starting.disabled).toBe(true);
        expect((getByRole("button", { name: "Open Agent b" }) as HTMLButtonElement).disabled).toBe(true);
        fireEvent.click(starting);
        expect(onStart).toHaveBeenCalledTimes(1);
        // The other agent's row is not held.
        expect((getByRole("button", { name: "Start Agent a" }) as HTMLButtonElement).disabled).toBe(false);
        answer(undefined);
        await vi.waitFor(() => expect(getByRole("button", { name: "Start Agent b" }).textContent).toBe("Start"));
        expect(onOpen).not.toHaveBeenCalled();
    });

    it("says why srv refused a Start, under that row", async () => {
        const onStart = vi
            .fn<(a: AgentRuntime) => Promise<string | undefined>>()
            .mockResolvedValue("a terminal-tier agent can't run without a pane");
        const { getByRole, findByRole } = render(() => (
            <StoppedAgents agents={() => [agent("a")]} hasLive={() => false} onOpen={() => {}} onStart={onStart} />
        ));
        fireEvent.click(getByRole("button", { name: "Start Agent a" }));
        expect((await findByRole("alert")).textContent).toContain("terminal-tier");
    });
});
