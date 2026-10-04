// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The other-instance sections, through the real component: names, the machine's
 * platform and how it was found, the same selection checkbox as a local row,
 * collapsible, and absent when there is nothing to show.
 * docs/specs/SPEC_SWARM_OTHER_HOSTS_AND_CHANNELS_2026_10_02.md section 3 and
 * docs/specs/SPEC_SWARM_REMOTE_AGENTS_PLATFORM_TAG_AND_SELECTION_2026_10_03.md.
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
    const [selected, setSelected] = createSignal<Set<string>>(new Set());
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
        selectedBlockIdsAtom: selected,
        isSelected: (key: string) => selected().has(key),
        toggleSelected: (key: string) =>
            setSelected((prev) => {
                const next = new Set(prev);
                if (next.has(key)) next.delete(key);
                else next.add(key);
                return next;
            }),
        setManySelected: (keys: string[], on: boolean) =>
            setSelected((prev) => {
                const next = new Set(prev);
                for (const k of keys) {
                    if (on) next.add(k);
                    else next.delete(k);
                }
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
                { channel: "dev-fix-lan", seen_at_ms: Date.now(), stale: false, os: "windows", agents: [{ name: "Loap", block_id: "b1" }, { name: "Korp", block_id: "b2" }] },
                { channel: "old", seen_at_ms: Date.now() - 120_000, stale: true, os: "windows", agents: [{ name: "Opaz", block_id: "b3" }] },
            ],
        },
        {
            host_id: "lan:area54#1",
            display_name: "Area54",
            tier: "lan",
            channels: [{ channel: "stable", seen_at_ms: Date.now(), stale: false, os: "macos", agents: [{ name: "Manoz", block_id: "" }, { name: "Opaz", block_id: "" }] }],
        },
        {
            host_id: "lan:legacy#2",
            display_name: "legacy",
            tier: "lan",
            channels: [{ channel: "stable", seen_at_ms: Date.now(), stale: false, agents: [{ name: "Zed", block_id: "" }] }],
        },
    ],
};

const sections = (container: HTMLElement) => [...container.querySelectorAll(".swarm-remote-section")];
const names = (el: Element) => [...el.querySelectorAll(".swarm-remote-agent-name")].map((e) => e.textContent);

describe("OtherInstanceSections", () => {
    it("lists each channel of this machine with its agents' names", () => {
        const { container } = render(() => <OtherInstanceSections model={modelWith(data)} />);
        const titles = [...container.querySelectorAll(".swarm-remote-title")].map((e) => e.textContent);
        // This machine first, then the LAN.
        expect(titles).toEqual(["narko · dev-fix-lan", "narko · old", "Area54", "legacy"]);
        expect(names(sections(container)[0])).toEqual(["Korp", "Loap"]);
        expect(names(sections(container)[2])).toEqual(["Manoz", "Opaz"]);
    });

    it("tags each section with the machine's platform and how it was found", () => {
        const { container } = render(() => <OtherInstanceSections model={modelWith(data)} />);
        const tags = sections(container).map((s) => [
            s.querySelector(".swarm-remote-title")!.textContent,
            s.querySelector(".swarm-remote-platform")?.textContent ?? null,
            s.querySelector(".swarm-remote-badge")!.textContent,
        ]);
        expect(tags).toEqual([
            ["narko · dev-fix-lan", "Windows", "this machine"],
            ["narko · old", "Windows", "this machine"],
            ["Area54", "macOS", "LAN"],
            ["legacy", null, "LAN"],
        ]);
    });

    it("dims a stale channel and says when it was last seen", () => {
        const { container } = render(() => <OtherInstanceSections model={modelWith(data)} />);
        const stale = sections(container)[1];
        expect(stale.classList.contains("swarm-remote-section--stale")).toBe(true);
        expect(stale.querySelector(".swarm-remote-seen")?.textContent).toBe("seen 2m ago");
    });

    it("collapses a section to a count", () => {
        // Built outside the JSX: a prop expression is re-evaluated on each read.
        const model = modelWith(data);
        const { container } = render(() => <OtherInstanceSections model={model} />);
        fireEvent.click(container.querySelector(".swarm-remote-header")!);
        expect(names(sections(container)[0])).toEqual([]);
        expect(sections(container)[0].querySelector(".swarm-agent-collapsed-count")?.textContent).toBe("2");
    });

    it("renders nothing with no other instances", () => {
        for (const d of [null, { ...data, hosts: [] }]) {
            const { container } = render(() => <OtherInstanceSections model={modelWith(d)} />);
            expect(container.querySelector(".swarm-remote")).toBeNull();
            cleanup();
        }
    });
});

describe("OtherInstanceSections: selection", () => {
    const rowBox = (section: Element, i: number) =>
        section.querySelectorAll<HTMLInputElement>(".swarm-remote-agent input[type=checkbox]")[i];
    const headerBox = (section: Element) => section.querySelector<HTMLInputElement>(".swarm-remote-select-all")!;

    it("gives every remote agent the same checkbox a local row has", () => {
        const model = modelWith(data);
        const { container } = render(() => <OtherInstanceSections model={model} />);
        const area54 = sections(container)[2];
        expect(area54.querySelectorAll(".swarm-remote-agent input.swarm-agent-select-checkbox")).toHaveLength(2);

        fireEvent.click(rowBox(area54, 0));
        // An agent on another machine is keyed by where it is and its name; one on
        // this machine by its block id.
        expect([...model.selectedBlockIdsAtom()]).toEqual(["remote:lan:area54#1/stable/Manoz"]);
        fireEvent.click(rowBox(sections(container)[0], 0));
        expect(model.isSelected("b2")).toBe(true);
    });

    it("a header checkbox selects exactly that machine's agents, and clears them", () => {
        const model = modelWith(data);
        const { container } = render(() => <OtherInstanceSections model={model} />);
        const area54 = sections(container)[2];

        fireEvent.click(headerBox(area54));
        expect([...model.selectedBlockIdsAtom()].sort()).toEqual([
            "remote:lan:area54#1/stable/Manoz",
            "remote:lan:area54#1/stable/Opaz",
        ]);
        expect(headerBox(area54).checked).toBe(true);

        fireEvent.click(headerBox(area54));
        expect(model.selectedBlockIdsAtom().size).toBe(0);
    });

    it("shows a header checkbox half-checked when only some of its agents are selected", () => {
        const model = modelWith(data);
        const { container } = render(() => <OtherInstanceSections model={model} />);
        const area54 = sections(container)[2];
        fireEvent.click(rowBox(area54, 1));
        expect(headerBox(area54).indeterminate).toBe(true);
        expect(headerBox(area54).checked).toBe(false);

        // Clicking a half-checked header completes the selection.
        fireEvent.click(headerBox(area54));
        expect(headerBox(area54).indeterminate).toBe(false);
        expect(headerBox(area54).checked).toBe(true);
    });

    it("can't select the agents of a stale section", () => {
        const { container } = render(() => <OtherInstanceSections model={modelWith(data)} />);
        const stale = sections(container)[1];
        expect(headerBox(stale).disabled).toBe(true);
        expect(rowBox(stale, 0).disabled).toBe(true);
        expect(headerBox(sections(container)[0]).disabled).toBe(false);
    });

    it("keeps the checkbox out of the collapse button", () => {
        const { container } = render(() => <OtherInstanceSections model={modelWith(data)} />);
        expect(container.querySelector("button input")).toBeNull();
    });
});
