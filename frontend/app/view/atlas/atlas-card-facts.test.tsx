// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const openLink = vi.hoisted(() => vi.fn(async () => undefined));
vi.mock("@/app/store/misc-utils", () => ({ openLink }));

import { CardFacts } from "./atlas-card-facts";
import type { AtlasAgent } from "./atlas-graph";
import type { AtlasWork } from "./atlas-work";

function agent(name: string, over: Partial<AtlasAgent> = {}): AtlasAgent {
    return {
        key: `lab/${name}`,
        name,
        provider: "claude",
        state: "idle",
        line: null,
        machine: "lab",
        agentId: null,
        seat: null,
        contextTokens: null,
        currentTool: null,
        blockId: null,
        runtimeId: null,
        remote: true,
        stale: false,
        ...over,
    };
}

function show(a: AtlasAgent, others: AtlasAgent[] = [], work: AtlasWork | null = null) {
    const agents = new Map([a, ...others].map((x) => [x.key, x] as const));
    return render(() => (
        <dl>
            <CardFacts agent={a} agents={agents} work={work} />
        </dl>
    ));
}

afterEach(() => {
    cleanup();
    openLink.mockClear();
});

describe("CardFacts", () => {
    it("adds no row for an agent nothing is known of", () => {
        const { container } = show(agent("Ann"));
        expect(container.querySelector("dl")!.children).toHaveLength(0);
    });

    it("says whether the agent runs on its machine or in a container", () => {
        const { getByTestId } = show(agent("Ann", { remote: false, runtime: "container" }));
        expect(getByTestId("atlas-card-runtime").textContent).toMatch(/container/i);
    });

    it("names the account by this computer's label, and the other agents on it", () => {
        const work = { id: "claude:0123456789abcdef", label: "Work" };
        const { getByTestId } = show(agent("Ann", { account: work }), [
            agent("Cy", { account: { ...work } }),
            agent("Bo", { account: { ...work } }),
            agent("Dee", { account: { id: "claude:fedcba9876543210", label: null } }),
            agent("Eve"),
        ]);
        const row = getByTestId("atlas-card-account").textContent!;
        expect(row).toContain("Work");
        expect(row).toContain("Also on it: Bo, Cy");
        expect(row).not.toContain("Dee");
    });

    it("says so when the account is not signed in here, and nothing about sharing when nobody shares it", () => {
        const { getByTestId } = show(agent("Ann", { account: { id: "claude:0123456789abcdef", label: null } }));
        const row = getByTestId("atlas-card-account").textContent!;
        expect(row).toBe("Not signed in on this computer");
    });

    it("shows the repository and branch, or the repository alone", () => {
        const both = show(agent("Ann"), [], { repo: "agentmuxai/agentmux", branch: "ann/fix", tasks: [] });
        expect(both.getByTestId("atlas-card-branch").textContent).toBe("agentmuxai/agentmux · ann/fix");
        expect(both.getByText("Branch")).toBeTruthy();
        cleanup();
        const repoOnly = show(agent("Ann"), [], { repo: "agentmuxai/agentmux", branch: null, tasks: [] });
        expect(repoOnly.getByTestId("atlas-card-branch").textContent).toBe("agentmuxai/agentmux");
        expect(repoOnly.getByText("Repository")).toBeTruthy();
    });

    it("lists the tasks it holds, and opens a task's pull request", () => {
        const url = "https://github.com/agentmuxai/agentmux/pull/70";
        const { getByTestId, getByText } = show(agent("Ann"), [], {
            repo: null,
            branch: null,
            tasks: [
                { num: 7, title: "Fix the map", pr: { url, label: "#70" } },
                { num: 8, title: "Then the card", pr: null },
            ],
        });
        const items = getByTestId("atlas-card-tasks").querySelectorAll("li");
        expect(items).toHaveLength(2);
        expect(items[0].textContent).toContain("#7 Fix the map");
        // The title is cut after two lines by its style, so the whole of it is its hover label.
        expect(items[0].querySelector(".atlas-card__task")!.getAttribute("data-hover-tip")).toBe("Fix the map");
        expect(items[1].querySelector("button")).toBeNull();
        fireEvent.click(getByText("#70"));
        expect(openLink).toHaveBeenCalledWith(url);
    });
});
