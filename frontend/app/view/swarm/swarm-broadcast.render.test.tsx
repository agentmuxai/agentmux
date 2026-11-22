// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Broadcast input: one box, Enter sends to the selection, the count in the placeholder and the tooltip,
 * and a line under it for how the send went.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { FleetBroadcastProgressEvent } from "@/types/rpc/FleetBroadcastProgressEvent";

// srv's events, by hand: a test reports a broadcast's progress with `progressEvents.emit`.
const progressEvents = vi.hoisted(() => {
    const handlers = new Set<(e: FleetBroadcastProgressEvent) => void>();
    return { handlers, emit: (e: FleetBroadcastProgressEvent) => handlers.forEach((h) => h(e)) };
});
vi.mock("@/app/store/mps", async (original) => ({
    ...(await original<typeof import("@/app/store/mps")>()),
    onMuxEvent: (_type: string, handler: (e: FleetBroadcastProgressEvent) => void) => {
        progressEvents.handlers.add(handler);
        return () => progressEvents.handlers.delete(handler);
    },
}));

import { BroadcastDock, fittedHeight, followBroadcastProgress, MAX_AUTO_HEIGHT, PROGRESS_AFTER_MS, type BroadcastProgress } from "./swarm-broadcast";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";

afterEach(() => cleanup());

const agents = { tree: () => ["a", "b", "c"].map((k) => ({ blockId: k, agentName: k.toUpperCase() })) as unknown as AgentTreeNode[] };

function modelStub(
    selectedKeys: string[],
    result: { succeeded: string[]; failed: { id: string; error: string }[] } = { succeeded: selectedKeys, failed: [] }
) {
    const [selected] = createSignal<Set<string>>(new Set(selectedKeys));
    return {
        selectedBlockIdsAtom: selected,
        otherInstancesAtom: () => null,
        fleetTargets: () => {
            throw new Error("the dock must not rebuild the agent tree");
        },
        fleetActionInFlightAtom: () => false,
        broadcastToSelection: vi.fn(async () => ({ ...result, aborted_early: false })),
    } as unknown as SwarmViewModel;
}

const box = (c: HTMLElement) => c.querySelector<HTMLTextAreaElement>(".swarm-broadcast-input")!;
const note = (c: HTMLElement) => c.querySelector<HTMLElement>(".swarm-broadcast-note");
const enter = (c: HTMLElement) => fireEvent.keyDown(box(c), { key: "Enter" });
const settle = async () => {
    await Promise.resolve();
    await Promise.resolve();
};

describe("BroadcastDock", () => {
    it("is one box with no label, no collapse button and no Send button", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        expect(container.querySelectorAll("textarea")).toHaveLength(1);
        expect(container.querySelector("button")).toBeNull();
        expect(container.textContent).not.toContain("Send to");
        expect(container.textContent).not.toContain("Broadcast");
    });

    it("says in its ghost text who it goes to: the agents, then the count of those selected", () => {
        const none = render(() => <BroadcastDock model={modelStub([])} agents={agents} />);
        expect(box(none.container).placeholder).toBe("Broadcast to agents");
        none.unmount();
        const one = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        expect(box(one.container).placeholder).toBe("Broadcast to 1 agent");
        one.unmount();
        const two = render(() => <BroadcastDock model={modelStub(["a", "b"])} agents={agents} />);
        expect(box(two.container).placeholder).toBe("Broadcast to 2 agents");
    });

    it("tells what Enter does, with the count, on hover and as the box's description", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a", "b"])} agents={agents} />);
        const text = box(container).getAttribute("data-hover-tip")!;
        expect(text).toMatch(/^Enter sends to 2 agents · (Shift\+Enter|⇧Enter) adds a line · each agent gets it as your own message$/);
        expect(box(container).getAttribute("aria-description")).toBe(text);
    });

    it("sends the trimmed message to the selection on Enter, clears the box and says how many it went to", async () => {
        const model = modelStub(["a", "b"], { succeeded: ["a"], failed: [{ id: "b", error: "gone" }] });
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "  ship it \n" } });
        enter(container);
        await settle();
        expect(model.broadcastToSelection).toHaveBeenCalledWith("ship it", expect.any(String));
        expect(box(container).value).toBe("");
        expect(note(container)!.textContent).toContain("Sent to 1 agent");
    });

    it("sends on Ctrl+Enter too", () => {
        const model = modelStub(["a"]);
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "hi" } });
        fireEvent.keyDown(box(container), { key: "Enter", ctrlKey: true });
        expect(model.broadcastToSelection).toHaveBeenCalledWith("hi", expect.any(String));
    });

    it("adds a line on Shift+Enter and sends nothing", () => {
        const model = modelStub(["a"]);
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "hi" } });
        const notCancelled = fireEvent.keyDown(box(container), { key: "Enter", shiftKey: true });
        expect(model.broadcastToSelection).not.toHaveBeenCalled();
        // Not cancelled: the browser still inserts the newline.
        expect(notCancelled).toBe(true);
    });

    it("leaves Enter to an IME that is converting", () => {
        const model = modelStub(["a"]);
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "かな" } });
        fireEvent.keyDown(box(container), { key: "Enter", isComposing: true });
        fireEvent.keyDown(box(container), { key: "Enter", keyCode: 229 });
        expect(model.broadcastToSelection).not.toHaveBeenCalled();
    });

    it("sends nothing for an empty or blank message", () => {
        const model = modelStub(["a"]);
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        expect(enter(container)).toBe(false);
        fireEvent.input(box(container), { target: { value: "  \n " } });
        enter(container);
        expect(model.broadcastToSelection).not.toHaveBeenCalled();
        expect(note(container)).toBeNull();
    });

    it("with nobody selected sends nothing, keeps the text and says to select agents first, never falling back to everyone", () => {
        const model = modelStub([]);
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "hello all" } });
        enter(container);
        expect(model.broadcastToSelection).not.toHaveBeenCalled();
        expect(box(container).value).toBe("hello all");
        expect(note(container)!.textContent).toBe("Select the agents to message first");
        expect(note(container)!.getAttribute("role")).toBe("status");
        fireEvent.input(box(container), { target: { value: "hello all!" } });
        expect(note(container)).toBeNull();
    });

    it("does not send a second time while the first is in flight", async () => {
        let finish!: (r: unknown) => void;
        const model = modelStub(["a"]);
        (model.broadcastToSelection as unknown as ReturnType<typeof vi.fn>).mockImplementationOnce(() => new Promise((resolve) => (finish = resolve)));
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "once" } });
        enter(container);
        fireEvent.input(box(container), { target: { value: "twice" } });
        enter(container);
        expect(model.broadcastToSelection).toHaveBeenCalledTimes(1);
        finish({ succeeded: ["a"], failed: [], aborted_early: false });
        await settle();
    });

    it("keeps a half-typed message across switching away and back", () => {
        const model = modelStub(["a"]);
        const first = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(first.container), { target: { value: "still typing" } });
        first.unmount();
        const second = render(() => <BroadcastDock model={model} agents={agents} />);
        expect(box(second.container).value).toBe("still typing");
    });

    it("keeps the message when the call itself fails", async () => {
        const model = modelStub(["a"]);
        (model.broadcastToSelection as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("rpc down"));
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "do not lose me" } });
        enter(container);
        await settle();
        expect(box(container).value).toBe("do not lose me");
    });

    it("keeps the message and says so when every delivery fails", async () => {
        const model = modelStub(["a", "b"], {
            succeeded: [],
            failed: [
                { id: "a", error: "gone" },
                { id: "b", error: "gone" },
            ],
        });
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "try again" } });
        enter(container);
        await settle();
        expect(box(container).value).toBe("try again");
        expect(note(container)!.textContent).toBe("Not sent: none of the 2 agents could be reached");
    });

    it("says why fewer agents get the message than are selected, and counts only the reachable ones", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a", "ghost"])} agents={agents} />);
        expect(container.querySelector(".swarm-fleet-broadcast-gap")!.textContent).toBe("1 of 2 can't be messaged: no longer listed");
        expect(box(container).placeholder).toBe("Broadcast to 1 agent");
    });
});

describe("a paced broadcast's progress", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => {
        vi.useRealTimers();
        progressEvents.handlers.clear();
    });

    it("shows nothing for a broadcast that is over quickly, and stops listening", () => {
        const seen: (BroadcastProgress | null)[] = [];
        const stop = followBroadcastProgress("b1", (p) => seen.push(p));
        progressEvents.emit({ id: "b1", sent: 1, total: 2 });
        progressEvents.emit({ id: "b1", sent: 2, total: 2 });
        stop();
        vi.advanceTimersByTime(PROGRESS_AFTER_MS * 2);
        expect(seen).toEqual([null]);
        expect(progressEvents.handlers.size).toBe(0);
    });

    it("reports the latest count once the send has lasted, then each one, and only its own broadcast's", () => {
        const seen: (BroadcastProgress | null)[] = [];
        const stop = followBroadcastProgress("b1", (p) => seen.push(p));
        progressEvents.emit({ id: "b1", sent: 1, total: 30 });
        progressEvents.emit({ id: "b1", sent: 2, total: 30 });
        progressEvents.emit({ id: "someone-else", sent: 9, total: 9 });
        vi.advanceTimersByTime(PROGRESS_AFTER_MS);
        progressEvents.emit({ id: "b1", sent: 3, total: 30 });
        progressEvents.emit({ id: "someone-else", sent: 1, total: 4 });
        stop();
        expect(seen).toEqual([{ sent: 2, total: 30 }, { sent: 3, total: 30 }, null]);
    });

    it("says how many have been sent under the box while a paced broadcast goes out, then how many it went to", async () => {
        let finish!: (r: unknown) => void;
        const model = modelStub(["a", "b", "c"]);
        (model.broadcastToSelection as unknown as ReturnType<typeof vi.fn>).mockImplementationOnce(() => new Promise((resolve) => (finish = resolve)));
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "go" } });
        enter(container);
        const id = (model.broadcastToSelection as unknown as ReturnType<typeof vi.fn>).mock.calls[0][1] as string;
        progressEvents.emit({ id, sent: 2, total: 3 });
        expect(container.querySelector(".swarm-broadcast-progress")).toBeNull();
        vi.advanceTimersByTime(PROGRESS_AFTER_MS);
        expect(container.querySelector(".swarm-broadcast-progress")!.textContent).toBe("Sent 2 of 3…");
        expect(note(container)!.getAttribute("role")).toBe("status");
        finish({ succeeded: ["a", "b", "c"], failed: [], aborted_early: false });
        await vi.advanceTimersByTimeAsync(0);
        expect(container.querySelector(".swarm-broadcast-progress")).toBeNull();
        expect(note(container)!.textContent).toBe("Sent to 3 agents");
    });
});

describe("fittedHeight", () => {
    it("is the text's own height plus the borders, capped, and nothing while the box has no layout", () => {
        expect(fittedHeight(24, 2)).toBe(26);
        expect(fittedHeight(MAX_AUTO_HEIGHT + 100, 2)).toBe(MAX_AUTO_HEIGHT);
        expect(fittedHeight(0, 2)).toBeNull();
    });
});

describe("BroadcastDock - the box starts at one line and grows", () => {
    let scrollHeight = 0;
    let offsetHeight = 0;
    let resized: (() => void) | undefined;
    const real = { sh: Object.getOwnPropertyDescriptor(Element.prototype, "scrollHeight"), oh: Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetHeight") };

    beforeEach(() => {
        scrollHeight = 22;
        offsetHeight = 0;
        resized = undefined;
        Object.defineProperty(HTMLTextAreaElement.prototype, "scrollHeight", { configurable: true, get: () => scrollHeight });
        Object.defineProperty(HTMLTextAreaElement.prototype, "offsetHeight", { configurable: true, get: () => offsetHeight });
        vi.stubGlobal(
            "ResizeObserver",
            class {
                constructor(cb: () => void) {
                    resized = cb;
                }
                observe() {}
                disconnect() {}
            }
        );
    });
    afterEach(() => {
        vi.unstubAllGlobals();
        if (real.sh) Object.defineProperty(Element.prototype, "scrollHeight", real.sh);
        delete (HTMLTextAreaElement.prototype as unknown as Record<string, unknown>).scrollHeight;
        if (real.oh) Object.defineProperty(HTMLElement.prototype, "offsetHeight", real.oh);
        delete (HTMLTextAreaElement.prototype as unknown as Record<string, unknown>).offsetHeight;
    });

    const type = (c: HTMLElement, value: string, height: number) => {
        scrollHeight = height;
        fireEvent.input(box(c), { target: { value } });
    };

    it("is one row with no height of its own until there is text", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        expect(box(container).rows).toBe(1);
        expect(box(container).style.height).toBe("");
    });

    it("grows with the text, stops at the cap and shrinks back when text is removed", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        type(container, "one\ntwo\nthree", 66);
        expect(box(container).style.height).toBe("66px");
        type(container, "a very long message", MAX_AUTO_HEIGHT + 200);
        expect(box(container).style.height).toBe(`${MAX_AUTO_HEIGHT}px`);
        type(container, "short", 22);
        expect(box(container).style.height).toBe("22px");
    });

    it("goes back to one line after a send", async () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        type(container, "one\ntwo", 44);
        expect(box(container).style.height).toBe("44px");
        enter(container);
        await settle();
        expect(box(container).value).toBe("");
        expect(box(container).style.height).toBe("");
    });

    it("keeps fitting when a short pane squeezes the box without a drag", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        type(container, "one", 44);
        resized!(); // layout squeezed the box: its inline height is still the one we set
        type(container, "one\ntwo\nthree", 66);
        expect(box(container).style.height).toBe("66px");
    });

    it("keeps a height dragged to while the draft is still empty, through the first keystroke", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        box(container).style.height = "90px";
        resized!();
        type(container, "a", 22);
        expect(box(container).style.height).toBe("90px");
        type(container, "", 22);
        expect(box(container).style.height).toBe("");
    });

    it("keeps a height the person dragged to until the draft is empty again", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        type(container, "one", 22);
        expect(box(container).style.height).toBe("22px");
        // The resize handle writes its own inline height.
        box(container).style.height = "120px";
        resized!();
        type(container, "one\ntwo\nthree", 66);
        expect(box(container).style.height).toBe("120px");
        type(container, "", 22);
        expect(box(container).style.height).toBe("");
        type(container, "again", 44);
        expect(box(container).style.height).toBe("44px");
    });
});
