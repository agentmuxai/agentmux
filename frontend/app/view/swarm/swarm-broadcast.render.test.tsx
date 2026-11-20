// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Broadcast section's dock: the composer, the recipient count, and the last ten broadcasts.
 * docs/specs/SPEC_SWARM_SECTIONS_SUPERVISOR_SETTINGS_AND_KANBAN_2026_10_24.md §3.3.
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

import { BroadcastDock, fittedHeight, followBroadcastProgress, MAX_AUTO_HEIGHT, MAX_RECENT_BROADCASTS, PROGRESS_AFTER_MS, withRecent, type BroadcastProgress, type BroadcastRecord } from "./swarm-broadcast";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";

afterEach(() => cleanup());

const agents = { tree: () => ["a", "b", "c"].map((k) => ({ blockId: k, agentName: k.toUpperCase() })) as unknown as AgentTreeNode[] };

function modelStub(selectedKeys: string[], result = { succeeded: selectedKeys, failed: [] as { id: string; error: string }[] }) {
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
const sendButton = (c: HTMLElement) => c.querySelector<HTMLButtonElement>(".swarm-fleet-btn--primary")!;

describe("BroadcastDock", () => {
    it("counts the recipients on the button and sends nothing until there is a message", () => {
        const model = modelStub(["a", "b"]);
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        expect(sendButton(container).textContent).toContain("Send to 2");
        expect(sendButton(container).disabled).toBe(true);
        fireEvent.input(box(container), { target: { value: "  " } });
        expect(sendButton(container).disabled).toBe(true);
    });

    it("sends the trimmed message to the selection, clears the box and records the broadcast", async () => {
        const model = modelStub(["a", "b"], { succeeded: ["a"], failed: [{ id: "b", error: "gone" }] });
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "  ship it \n" } });
        fireEvent.click(sendButton(container));
        await Promise.resolve();
        await Promise.resolve();
        expect(model.broadcastToSelection).toHaveBeenCalledWith("ship it", expect.any(String));
        expect(box(container).value).toBe("");
        const row = container.querySelector(".swarm-broadcast-recent-row")!;
        expect(row.textContent).toContain("ship it");
        expect(row.textContent).toContain("2 sent · 1 delivered · 1 failed");
    });

    it("says Enter sends and Shift+Enter adds a line", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a"])} agents={agents} />);
        expect(container.querySelector(".swarm-broadcast-hint")!.textContent).toMatch(/^Enter sends · (Shift\+Enter|⇧Enter) adds a line$/);
    });

    it("sends on Enter, and on Ctrl+Enter too", () => {
        const model = modelStub(["a"]);
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "hi" } });
        fireEvent.keyDown(box(container), { key: "Enter" });
        expect(model.broadcastToSelection).toHaveBeenCalledWith("hi", expect.any(String));
        fireEvent.input(box(container), { target: { value: "again" } });
        fireEvent.keyDown(box(container), { key: "Enter", ctrlKey: true });
        expect(model.broadcastToSelection).toHaveBeenLastCalledWith("again", expect.any(String));
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

    it("sends nothing on Enter with an empty box, and does not insert a line break", () => {
        const model = modelStub(["a"]);
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        const notCancelled = fireEvent.keyDown(box(container), { key: "Enter" });
        expect(model.broadcastToSelection).not.toHaveBeenCalled();
        expect(notCancelled).toBe(false);
    });

    it("keeps a half-typed message and the recent list across a section switch", async () => {
        const model = modelStub(["a"]);
        const first = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(first.container), { target: { value: "sent one" } });
        fireEvent.click(sendButton(first.container));
        await Promise.resolve();
        await Promise.resolve();
        fireEvent.input(box(first.container), { target: { value: "still typing" } });
        first.unmount();
        const second = render(() => <BroadcastDock model={model} agents={agents} />);
        expect(box(second.container).value).toBe("still typing");
        expect(second.container.querySelectorAll(".swarm-broadcast-recent-row")).toHaveLength(1);
    });

    it("keeps the message when the call itself fails, and records nothing", async () => {
        const model = modelStub(["a"]);
        (model.broadcastToSelection as unknown as ReturnType<typeof vi.fn>).mockRejectedValueOnce(new Error("rpc down"));
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "do not lose me" } });
        fireEvent.click(sendButton(container));
        await Promise.resolve();
        await Promise.resolve();
        expect(box(container).value).toBe("do not lose me");
        expect(container.querySelectorAll(".swarm-broadcast-recent-row")).toHaveLength(0);
    });

    it("does not count an unreachable agent as a failed delivery", async () => {
        const model = modelStub(["a", "ghost"], { succeeded: ["a"], failed: [{ id: "ghost", error: "no longer listed" }] });
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "hi" } });
        fireEvent.click(sendButton(container));
        await Promise.resolve();
        await Promise.resolve();
        expect(container.querySelector(".swarm-broadcast-recent-meta")!.textContent).toContain("1 sent · 1 delivered");
        expect(container.querySelector(".swarm-broadcast-recent-meta")!.textContent).not.toContain("failed");
    });

    it("says why fewer agents get the message than are selected", () => {
        const { container } = render(() => <BroadcastDock model={modelStub(["a", "ghost"])} agents={agents} />);
        expect(container.querySelector(".swarm-fleet-broadcast-gap")!.textContent).toBe("1 of 2 can't be messaged: no longer listed");
        expect(sendButton(container).textContent).toContain("Send to 1");
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

    it("says how many have been sent in place of the hint while a paced broadcast goes out, then the hint again", async () => {
        let finish!: (r: unknown) => void;
        const model = modelStub(["a", "b", "c"]);
        (model.broadcastToSelection as unknown as ReturnType<typeof vi.fn>).mockImplementationOnce(() => new Promise((resolve) => (finish = resolve)));
        const { container } = render(() => <BroadcastDock model={model} agents={agents} />);
        fireEvent.input(box(container), { target: { value: "go" } });
        fireEvent.click(sendButton(container));
        const id = (model.broadcastToSelection as unknown as ReturnType<typeof vi.fn>).mock.calls[0][1] as string;
        // The hint stays in the page under the count (hidden, and not read out), so the row keeps its height.
        const hint = () => container.querySelector(".swarm-broadcast-hint-text")!;
        const count = () => container.querySelector(".swarm-broadcast-progress");
        expect(hint().textContent).toContain("Enter sends");
        progressEvents.emit({ id, sent: 2, total: 3 });
        expect(count()).toBeNull();
        expect(hint().classList.contains("is-covered")).toBe(false);
        vi.advanceTimersByTime(PROGRESS_AFTER_MS);
        expect(count()!.textContent).toBe("Sent 2 of 3…");
        expect(count()!.getAttribute("role")).toBe("status");
        expect(count()!.parentElement).toBe(hint().parentElement);
        expect(hint().textContent).toContain("Enter sends");
        expect(hint().classList.contains("is-covered")).toBe(true);
        expect(hint().getAttribute("aria-hidden")).toBe("true");
        finish({ succeeded: ["a", "b", "c"], failed: [], aborted_early: false });
        await vi.advanceTimersByTimeAsync(0);
        expect(count()).toBeNull();
        expect(hint().classList.contains("is-covered")).toBe(false);
        expect(hint().hasAttribute("aria-hidden")).toBe(false);
        expect(container.querySelector(".swarm-broadcast-recent-meta")!.textContent).toContain("3 sent · 3 delivered");
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
        fireEvent.click(sendButton(container));
        await Promise.resolve();
        await Promise.resolve();
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
