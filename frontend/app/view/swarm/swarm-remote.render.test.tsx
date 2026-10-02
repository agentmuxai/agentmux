// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The other-instance sections, through the real component: names only, read only,
 * collapsible, and absent when there is nothing to show.
 * docs/specs/SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md section 3.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));

import type { SwarmViewModel } from "./swarm-model";
import type { SwarmOtherInstances } from "./swarm-remote";
import { OtherInstanceSections } from "./swarm-view";

afterEach(cleanup);

function modelWith(data: SwarmOtherInstances | null) {
    const [collapsed, setCollapsed] = createSignal<Set<string>>(new Set());
    return {
        otherInstancesAtom: () => data,
        isRemoteCollapsed: (key: string) => collapsed().has(key),
        toggleRemoteCollapsed: (key: string) =>
            setCollapsed((prev) => {
                const next = new Set(prev);
                if (next.has(key)) next.delete(key);
                else next.add(key);
                return next;
            }),
    } as unknown as SwarmViewModel;
}

const data: SwarmOtherInstances = {
    hostname: "narko",
    channel: "stable",
    hosts: [
        {
            host_id: "host:narko",
            display_name: "narko",
            tier: "host",
            channels: [
                { channel: "dev-fix-lan", seen_at_ms: Date.now(), stale: false, agents: [{ name: "Loap", block_id: "b1" }, { name: "Korp", block_id: "b2" }] },
                { channel: "old", seen_at_ms: Date.now() - 120_000, stale: true, agents: [{ name: "Opaz", block_id: "b3" }] },
            ],
        },
    ],
};

describe("OtherInstanceSections", () => {
    it("lists each channel of this machine with its agents' names", () => {
        const { container } = render(() => <OtherInstanceSections model={modelWith(data)} />);
        const titles = [...container.querySelectorAll(".swarm-remote-title")].map((e) => e.textContent);
        expect(titles).toEqual(["narko · dev-fix-lan", "narko · old"]);
        const names = [...container.querySelectorAll(".swarm-remote-agent")].map((e) => e.textContent);
        expect(names).toEqual(["Korp", "Loap", "Opaz"]);
        expect(container.querySelector(".swarm-remote-badge")?.textContent).toBe("this machine");
    });

    it("is read only: no checkbox, nothing that could join a fleet action", () => {
        const { container } = render(() => <OtherInstanceSections model={modelWith(data)} />);
        expect(container.querySelectorAll("input[type=checkbox]")).toHaveLength(0);
    });

    it("dims a stale channel and says when it was last seen", () => {
        const { container } = render(() => <OtherInstanceSections model={modelWith(data)} />);
        const stale = container.querySelectorAll(".swarm-remote-section")[1];
        expect(stale.classList.contains("swarm-remote-section--stale")).toBe(true);
        expect(stale.querySelector(".swarm-remote-seen")?.textContent).toBe("seen 2m ago");
    });

    it("collapses a section to a count", () => {
        // Built outside the JSX: a prop expression is re-evaluated on each read.
        const model = modelWith(data);
        const { container } = render(() => <OtherInstanceSections model={model} />);
        fireEvent.click(container.querySelector(".swarm-remote-header")!);
        expect([...container.querySelectorAll(".swarm-remote-agent")].map((e) => e.textContent)).toEqual(["Opaz"]);
        expect(container.querySelector(".swarm-agent-collapsed-count")?.textContent).toBe("2");
    });

    it("renders nothing with no other instances", () => {
        for (const d of [null, { ...data, hosts: [] }]) {
            const { container } = render(() => <OtherInstanceSections model={modelWith(d)} />);
            expect(container.querySelector(".swarm-remote")).toBeNull();
            cleanup();
        }
    });
});
