// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Broadcast section's dock: the composer, the recipient count, and the last ten broadcasts.
 * docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §3.3.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { BroadcastDock, MAX_RECENT_BROADCASTS, withRecent, type BroadcastRecord } from "./swarm-broadcast";
import { localFleetTargets } from "./swarm-fleet-targets";
import type { SwarmViewModel } from "./swarm-model";

afterEach(() => cleanup());

function modelStub(selectedKeys: string[], result = { succeeded: selectedKeys, failed: [] as { id: string; error: string }[] }) {
    const [selected] = createSignal<Set<string>>(new Set(selectedKeys));
    const known = new Map(localFleetTargets(["a", "b", "c"].map((k) => ({ blockId: k, agentName: k.toUpperCase() }))));
    return {
        selectedBlockIdsAtom: selected,
        fleetTargets: () => known,
        fleetActionInFlightAtom: () => false,
        broadcastToSelection: vi.fn(async () => ({ ...result, aborted_early: false })),
    } as unknown as SwarmViewModel;
}

const box = (c: HTMLElement) => c.querySelector<HTMLTextAreaElement>(".swarm-broadcast-input")!;
const sendButton = (c: HTMLElement) => c.querySelector<HTMLButtonElement>(".swarm-fleet-btn--primary")!;

describe("BroadcastDock", () => {
    it("counts the recipients on the button and sends nothing until there is a message", () => {
        const model = modelStub(["a", "b"]);
        const { container } = render(() => <BroadcastDock model={model} />);
        expect(sendButton(container).textContent).toContain("Send to 2");
        expect(sendButton(container).disabled).toBe(true);
        fireEvent.input(box(container), { target: { value: "  " } });
        expect(sendButton(container).disabled).toBe(true);
    });

    it("sends the trimmed message to the selection, clears the box and records the broadcast", async () => {
        const model = modelStub(["a", "b"], { succeeded: ["a"], failed: [{ id: "b", error: "gone" }] });
        const { container } = render(() => <BroadcastDock model={model} />);
        fireEvent.input(box(container), { target: { value: "  ship it \n" } });
        fireEvent.click(sendButton(container));
        await Promise.resolve();
        await Promise.resolve();
        expect(model.broadcastToSelection).toHaveBeenCalledWith("ship it");
        expect(box(container).value).toBe("");
        const row = container.querySelector(".swarm-broadcast-recent-row")!;
        expect(row.textContent).toContain("ship it");
        expect(row.textContent).toContain("2 sent · 1 delivered · 1 failed");
    });

    it("sends on Ctrl+Enter and not on a plain Enter", () => {
        const model = modelStub(["a"]);
        const { container } = render(() => <BroadcastDock model={model} />);
        fireEvent.input(box(container), { target: { value: "hi" } });
        fireEvent.keyDown(box(container), { key: "Enter" });
        expect(model.broadcastToSelection).not.toHaveBeenCalled();
        fireEvent.keyDown(box(container), { key: "Enter", ctrlKey: true });
        expect(model.broadcastToSelection).toHaveBeenCalledWith("hi");
    });

    it("keeps a half-typed message and the recent list across a section switch", async () => {
        const model = modelStub(["a"]);
        const first = render(() => <BroadcastDock model={model} />);
        fireEvent.input(box(first.container), { target: { value: "sent one" } });
        fireEvent.click(sendButton(first.container));
        await Promise.resolve();
        await Promise.resolve();
        fireEvent.input(box(first.container), { target: { value: "still typing" } });
        first.unmount();
        const second = render(() => <BroadcastDock model={model} />);
        expect(box(second.container).value).toBe("still typing");
        expect(second.container.querySelectorAll(".swarm-broadcast-recent-row")).toHaveLength(1);
    });

    it("says why fewer agents get the message than are selected", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a", "ghost"])} />);
        expect(container.querySelector(".swarm-fleet-broadcast-gap")!.textContent).toBe("1 of 2 can't be messaged: no longer listed");
        expect(sendButton(container).textContent).toContain("Send to 1");
    });
});

describe("withRecent", () => {
    it("keeps the last ten, newest first", () => {
        let list: BroadcastRecord[] = [];
        for (let i = 0; i < 12; i++) list = withRecent(list, { text: `m${i}`, at: i, recipients: 1, delivered: 1, failed: 0 });
        expect(list).toHaveLength(MAX_RECENT_BROADCASTS);
        expect(list[0].text).toBe("m11");
        expect(list[9].text).toBe("m2");
    });
});
