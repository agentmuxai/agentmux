// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The other instances' agents as rows of the Swarm's one list, through the real
 * component: names, where each runs (machine, how it was found, platform), the
 * same selection checkbox as a local row, and no machine headers or separator.
 * docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md §5, building on
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
import { remoteRows, remoteSections, type SwarmOtherInstances } from "./swarm-remote";
import { RemoteAgentRows } from "./swarm-view";

afterEach(cleanup);

function modelWith() {
    const [selected, setSelected] = createSignal<Set<string>>(new Set());
    return {
        selectedBlockIdsAtom: selected,
        isSelected: (key: string) => selected().has(key),
        toggleSelected: (key: string) =>
            setSelected((prev) => {
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
                { channel: "dev-fix-lan", seen_at_ms: Date.now(), stale: false, os: "windows", agents: [{ name: "Loap", block_id: "b1", status: "working", line: "Fixing the login race", provider: "claude" }, { name: "Korp", block_id: "b2", status: "waiting" }] },
                { channel: "old", seen_at_ms: Date.now() - 120_000, stale: true, os: "windows", agents: [{ name: "Opaz", block_id: "b3", status: "working", line: "Old news" }] },
            ],
        },
        {
            host_id: "lan:area54#1",
            display_name: "Area54",
            tier: "lan",
            channels: [{ channel: "stable", seen_at_ms: Date.now(), stale: false, os: "macos", agents: [{ name: "Manoz", block_id: "", status: "stopped" }, { name: "Opaz", block_id: "" }] }],
        },
        {
            host_id: "lan:legacy#2",
            display_name: "legacy",
            tier: "lan",
            channels: [{ channel: "stable", seen_at_ms: Date.now(), stale: false, agents: [{ name: "Zed", block_id: "" }] }],
        },
    ],
};

function setup(d: SwarmOtherInstances | null = data, model = modelWith()) {
    const rows = remoteRows(remoteSections(d));
    const r = render(() => <RemoteAgentRows model={model} rows={rows} />);
    return { ...r, model };
}

const rows = (container: HTMLElement) => [...container.querySelectorAll(".swarm-remote-agent-group")];
const text = (el: Element, sel: string) => el.querySelector(sel)?.textContent ?? null;
const box = (row: Element) => row.querySelector<HTMLInputElement>("input.swarm-agent-select-checkbox")!;

describe("RemoteAgentRows", () => {
    it("lists every remote agent as a row: this machine's channels, then the LAN, by machine, by name", () => {
        const { container } = setup();
        expect(rows(container).map((r) => [text(r, ".swarm-remote-agent-name"), text(r, ".swarm-remote-title")])).toEqual([
            ["Korp", "narko · dev-fix-lan"],
            ["Loap", "narko · dev-fix-lan"],
            ["Opaz", "narko · old"],
            ["Manoz", "Area54"],
            ["Opaz", "Area54"],
            ["Zed", "legacy"],
        ]);
    });

    it("has no machine headers, collapse buttons or separator of its own", () => {
        const { container } = setup();
        expect(container.querySelector(".swarm-remote")).toBeNull();
        expect(container.querySelector(".swarm-remote-section")).toBeNull();
        expect(container.querySelector(".swarm-remote-select-all")).toBeNull();
        expect(container.querySelector("button")).toBeNull();
        // Each row is an agent group like a local agent's, so the list's own
        // spacing applies between all of them alike.
        expect(rows(container).every((r) => r.classList.contains("swarm-agent-group"))).toBe(true);
    });

    it("tags each row with how its machine was found and its platform", () => {
        const { container } = setup();
        expect(rows(container).map((r) => [text(r, ".swarm-remote-badge"), text(r, ".swarm-remote-platform")])).toEqual([
            ["this machine", "Windows"],
            ["this machine", "Windows"],
            ["this machine", "Windows"],
            ["LAN", "macOS"],
            ["LAN", "macOS"],
            ["LAN", null],
        ]);
    });

    it("dims a stale agent, says when it was last seen, and can't select it", () => {
        const { container } = setup();
        const stale = rows(container)[2];
        expect(stale.classList.contains("swarm-remote-agent-group--stale")).toBe(true);
        expect(text(stale, ".swarm-remote-seen")).toBe("seen 2m ago");
        expect(box(stale).disabled).toBe(true);
        expect(box(rows(container)[0]).disabled).toBe(false);
    });

    it("shows each agent's reported state as a status chip, and none when it has no state or is stale", () => {
        const { container } = setup();
        const chips = rows(container).map((r) => {
            const chip = r.querySelector(".swarm-remote-agent-status");
            // The label span only: the chip also holds the spinner glyph.
            return chip ? [chip.querySelector(".swarm-status-label")!.textContent, chip.getAttribute("title")] : null;
        });
        expect(chips).toEqual([
            ["question", "Waiting for an answer"], // Korp
            ["working", "Working"], // Loap
            null, // stale: whatever it last said may be long out of date
            ["idle", "Stopped"], // Manoz: the chip says idle, the tooltip says which
            null, // Opaz gave no state
            null, // Zed
        ]);
    });

    it("shows an agent's summary line under its row, and none when it shared none or is stale", () => {
        const { container } = setup();
        expect(rows(container).map((r) => text(r, ".swarm-remote-agent-line"))).toEqual([
            null,
            "Fixing the login race",
            null,
            null,
            null,
            null,
        ]);
    });

    // docs/specs/SPEC_SWARM_REFINEMENTS_2026_10_17.md §3: left of the name, as on a local row.
    // A provider nobody reported is a neutral mark, not the AgentMux logo
    // (REPORT_SWARM_REMOTE_PROVIDER_ICON_RCA_2026_10_18.md §6.2).
    it("shows each agent's provider icon before its name, and a neutral mark when none was reported", () => {
        const { container } = setup();
        const [korp, loap] = rows(container);
        const icon = (r: Element) => r.querySelector(".provider-logo, .swarm-provider-unknown")!;
        expect(loap.querySelector(".provider-logo")).not.toBeNull();
        expect(korp.querySelector(".provider-logo")).toBeNull();
        expect(korp.querySelector(".swarm-provider-unknown")).not.toBeNull();
        expect(korp.querySelector("img")).toBeNull();
        for (const r of [korp, loap]) {
            const name = r.querySelector(".swarm-remote-agent-name")!;
            expect(icon(r).compareDocumentPosition(name) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
        }
    });

    it("renders nothing with no other instances", () => {
        for (const d of [null, { ...data, hosts: [] }]) {
            const { container } = setup(d);
            expect(rows(container)).toHaveLength(0);
            cleanup();
        }
    });

    it("selects an agent on another machine by where it is and its name, and one on this machine by its block id", () => {
        const { container, model } = setup();
        fireEvent.click(box(rows(container)[3]));
        expect([...model.selectedBlockIdsAtom()]).toEqual(["remote:lan:area54#1/stable/Manoz"]);
        fireEvent.click(box(rows(container)[0]));
        expect(model.isSelected("b2")).toBe(true);
    });
});
