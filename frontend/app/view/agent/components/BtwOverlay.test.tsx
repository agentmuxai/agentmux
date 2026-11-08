// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { type Accessor, createMemo, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

interface Sub {
    eventType: string;
    handler: (data: unknown) => void;
    scope: string;
    unsub: ReturnType<typeof vi.fn>;
}

const hub = vi.hoisted(() => ({
    subs: [] as Array<{
        eventType: string;
        handler: (data: unknown) => void;
        scope: string;
        unsub: ReturnType<typeof vi.fn>;
    }>,
    throwOnUnsub: false,
}));

vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (eventType: string, handler: (data: unknown) => void, scope: string) => {
        const unsub = vi.fn(() => {
            if (hub.throwOnUnsub) throw new Error("boom");
        });
        hub.subs.push({ eventType, handler, scope, unsub });
        return unsub;
    },
}));

import { WpsEvent } from "@/app/store/mps-events";
import { recycleBlockers } from "@/app/store/recycle-blockers";
import { BtwOverlay, type BtwOverlayProps } from "./BtwOverlay";

const reason = "a /btw answer is still coming in";

function lastSub(): Sub {
    return hub.subs[hub.subs.length - 1];
}

function emit(event: unknown, done = false, sub: Sub = lastSub()): void {
    sub.handler({ event, done });
}

function baseProps(over: Partial<BtwOverlayProps> = {}): BtwOverlayProps {
    return { blockId: "b1", askId: 1, question: "why?", requestId: "r1", error: null, onClose: () => {}, ...over };
}

/** Mounts the overlay with props held in a signal so tests can change them live. */
function mount(initial: Partial<BtwOverlayProps> = {}) {
    const [props, setProps] = createSignal<BtwOverlayProps>(baseProps(initial));
    const view = render(() => <BtwOverlay {...propsProxy(props)} />);
    return {
        ...view,
        update: (next: Partial<BtwOverlayProps>) => setProps((p) => ({ ...p, ...next })),
    };
}

function propsProxy(props: Accessor<BtwOverlayProps>): BtwOverlayProps {
    const blockId = createMemo(() => props().blockId);
    const askId = createMemo(() => props().askId);
    const question = createMemo(() => props().question);
    const requestId = createMemo(() => props().requestId);
    const error = createMemo(() => props().error);
    return {
        get blockId() {
            return blockId();
        },
        get askId() {
            return askId();
        },
        get question() {
            return question();
        },
        get requestId() {
            return requestId();
        },
        get error() {
            return error();
        },
        onClose: () => props().onClose(),
    };
}

function q(container: HTMLElement, sel: string): HTMLElement | null {
    return container.querySelector<HTMLElement>(sel);
}

function key(target: EventTarget, k: string): KeyboardEvent {
    const ev = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true });
    target.dispatchEvent(ev);
    return ev;
}

function pointerDown(target: EventTarget): Event {
    const ev = new MouseEvent("pointerdown", { bubbles: true, cancelable: true });
    target.dispatchEvent(ev);
    return ev;
}

describe("BtwOverlay", () => {
    beforeEach(() => {
        hub.subs.length = 0;
        hub.throwOnUnsub = false;
    });
    afterEach(() => {
        cleanup();
        document.body.innerHTML = "";
    });

    describe("recycle blocker", () => {
        it("an answer still streaming blocks a renderer recycle until it is done", () => {
            mount();
            expect(recycleBlockers()).toContain(reason);
            emit({ type: "done", response: "because" }, true);
            expect(recycleBlockers()).not.toContain(reason);
        });

        it("stops blocking once unmounted", () => {
            const { unmount } = mount();
            expect(recycleBlockers()).toContain(reason);
            unmount();
            expect(recycleBlockers()).not.toContain(reason);
        });

        it("does not block when the request itself failed", () => {
            mount({ requestId: null, error: "nope" });
            expect(recycleBlockers()).not.toContain(reason);
        });

        it("does not block once the turn reported an error", () => {
            mount();
            expect(recycleBlockers()).toContain(reason);
            emit({ type: "error", message: "bad" });
            expect(recycleBlockers()).not.toContain(reason);
        });

        it("blocks while an answer is partway through", () => {
            mount();
            emit({ type: "assistant_text", delta: "par" });
            expect(recycleBlockers()).toContain(reason);
        });
    });

    describe("chrome", () => {
        it("renders a labelled dialog with the title, the question and the hint", () => {
            const { container } = mount({ question: "what is 2+2?" });
            const root = q(container, ".btw-overlay")!;
            expect(root.getAttribute("role")).toBe("dialog");
            expect(root.getAttribute("aria-label")).toBe("Side question");
            expect(q(container, ".btw-overlay__title")!.textContent).toBe("/btw");
            expect(q(container, ".btw-overlay__question")!.textContent).toBe("what is 2+2?");
            expect(q(container, ".btw-overlay__hint")!.textContent).toBe("Esc to close");
        });

        it("the close button calls onClose", () => {
            const onClose = vi.fn();
            const { getByLabelText } = mount({ onClose });
            fireEvent.click(getByLabelText("Close side question"));
            expect(onClose).toHaveBeenCalledTimes(1);
        });

        it("follows a changed question", () => {
            const { container, update } = mount({ question: "first" });
            update({ question: "second" });
            expect(q(container, ".btw-overlay__question")!.textContent).toBe("second");
        });
    });

    describe("body states", () => {
        it("shows Asking... while there is no request id, and no answer box", () => {
            const { container } = mount({ requestId: null });
            expect(q(container, ".btw-overlay__loading")!.textContent).toBe("Asking…");
            expect(q(container, ".btw-overlay__answer")).toBeNull();
            expect(q(container, ".btw-overlay__error")).toBeNull();
        });

        it("shows the answer box with a cursor once a request id exists", () => {
            const { container } = mount();
            expect(q(container, ".btw-overlay__loading")).toBeNull();
            expect(q(container, ".btw-overlay__answer")).not.toBeNull();
            const cursor = q(container, ".btw-overlay__cursor")!;
            expect(cursor.getAttribute("aria-hidden")).toBe("true");
        });

        it("a request id of empty string still counts as present", () => {
            const { container } = mount({ requestId: "" });
            expect(q(container, ".btw-overlay__answer")).not.toBeNull();
            expect(q(container, ".btw-overlay__loading")).toBeNull();
        });

        it("shows props.error alone, with no loading, answer or cursor", () => {
            const { container } = mount({ requestId: null, error: "rpc failed" });
            const errs = container.querySelectorAll(".btw-overlay__error");
            expect(errs.length).toBe(1);
            expect(errs[0].textContent).toBe("rpc failed");
            expect(q(container, ".btw-overlay__loading")).toBeNull();
            expect(q(container, ".btw-overlay__answer")).toBeNull();
        });

        it("props.error wins over a stream error", () => {
            const { container, update } = mount();
            emit({ type: "error", message: "stream bad" });
            update({ error: "rpc failed" });
            const errs = container.querySelectorAll(".btw-overlay__error");
            expect(errs.length).toBe(1);
            expect(errs[0].textContent).toBe("rpc failed");
        });

        it("a stream error replaces the answer box", () => {
            const { container } = mount();
            emit({ type: "assistant_text", delta: "partial" });
            emit({ type: "error", message: "stream bad" });
            const errs = container.querySelectorAll(".btw-overlay__error");
            expect(errs.length).toBe(1);
            expect(errs[0].textContent).toBe("stream bad");
            expect(q(container, ".btw-overlay__answer")).toBeNull();
            expect(q(container, ".btw-overlay__loading")).toBeNull();
        });
    });

    describe("subscription", () => {
        it("does not subscribe before a request id exists", () => {
            mount({ requestId: null });
            expect(hub.subs.length).toBe(0);
        });

        it("an empty request id is not a subscription scope", () => {
            mount({ requestId: "" });
            expect(hub.subs.length).toBe(0);
        });

        it("subscribes to BtwAnswerChunk scoped to the block and request", () => {
            mount({ blockId: "blk9", requestId: "req7" });
            expect(hub.subs.length).toBe(1);
            expect(hub.subs[0].eventType).toBe(WpsEvent.BtwAnswerChunk);
            expect(hub.subs[0].scope).toBe("block:blk9:btw:req7");
        });

        it("subscribes once for one request id", () => {
            const { update } = mount();
            update({ question: "other" });
            expect(hub.subs.length).toBe(1);
        });

        it("tears down the old subscription and resubscribes when the request id cycles", () => {
            const { update } = mount({ requestId: "r1" });
            const first = lastSub();
            update({ requestId: null });
            expect(first.unsub).toHaveBeenCalledTimes(1);
            expect(hub.subs.length).toBe(1);
            update({ requestId: "r2" });
            expect(hub.subs.length).toBe(2);
            expect(lastSub().scope).toBe("block:b1:btw:r2");
        });

        it("unsubscribes on unmount", () => {
            const { unmount } = mount();
            const sub = lastSub();
            expect(sub.unsub).not.toHaveBeenCalled();
            unmount();
            expect(sub.unsub).toHaveBeenCalledTimes(1);
        });

        it("a throwing unsubscribe does not break teardown", () => {
            const { unmount } = mount();
            hub.throwOnUnsub = true;
            expect(() => unmount()).not.toThrow();
            expect(lastSub().unsub).toHaveBeenCalledTimes(1);
        });
    });

    describe("streamed events", () => {
        it("appends assistant_text deltas in order", () => {
            const { container } = mount();
            emit({ type: "assistant_text", delta: "Hel" });
            emit({ type: "assistant_text", delta: "lo" });
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("Hello");
        });

        it("a done event's response replaces what streamed in", () => {
            const { container } = mount();
            emit({ type: "assistant_text", delta: "draft" });
            emit({ type: "done", response: "final" }, true);
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("final");
        });

        it("a done event with an empty response keeps the streamed text", () => {
            const { container } = mount();
            emit({ type: "assistant_text", delta: "kept" });
            emit({ type: "done", response: "" }, true);
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("kept");
        });

        it("a done event with no response field keeps the streamed text", () => {
            const { container } = mount();
            emit({ type: "assistant_text", delta: "kept" });
            emit({ type: "done" }, true);
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("kept");
        });

        it("a done event without the outer done flag does not finish the answer", () => {
            const { container } = mount();
            emit({ type: "done", response: "text" }, false);
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("text");
            expect(q(container, ".btw-overlay__cursor")).not.toBeNull();
            expect(q(container, ".btw-overlay__hint")!.textContent).toBe("Esc to close");
        });

        it("the outer done flag finishes the answer: no cursor and a Done hint", () => {
            const { container } = mount();
            emit({ type: "assistant_text", delta: "x" }, true);
            expect(q(container, ".btw-overlay__cursor")).toBeNull();
            expect(q(container, ".btw-overlay__hint")!.textContent).toBe("Done · Esc to close");
        });

        it("a streaming chunk keeps the cursor and the plain hint", () => {
            const { container } = mount();
            emit({ type: "assistant_text", delta: "x" }, false);
            expect(q(container, ".btw-overlay__cursor")).not.toBeNull();
            expect(q(container, ".btw-overlay__hint")!.textContent).toBe("Esc to close");
        });

        it("a terminal error event with the done flag shows the error and finishes", () => {
            const { container } = mount();
            emit({ type: "error", message: "bad turn" }, true);
            expect(q(container, ".btw-overlay__error")!.textContent).toBe("bad turn");
            expect(q(container, ".btw-overlay__hint")!.textContent).toBe("Done · Esc to close");
        });

        it("an error event alone does not mark the answer done", () => {
            const { container } = mount();
            emit({ type: "error", message: "bad turn" }, false);
            expect(q(container, ".btw-overlay__hint")!.textContent).toBe("Esc to close");
        });

        it("ignores tool and cost events but still honours the done flag", () => {
            const { container } = mount();
            emit({ type: "assistant_text", delta: "a" });
            emit({ type: "tool_use", name: "x" });
            emit({ type: "tool_result", output: "y" });
            emit({ type: "cost", usd: 1 });
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("a");
            expect(q(container, ".btw-overlay__error")).toBeNull();
            expect(q(container, ".btw-overlay__cursor")).not.toBeNull();
            expect(q(container, ".btw-overlay__hint")!.textContent).toBe("Esc to close");
            emit({ type: "cost", usd: 1 }, true);
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("a");
            expect(q(container, ".btw-overlay__cursor")).toBeNull();
        });
    });

    describe("a new ask reusing the instance", () => {
        it("clears the previous answer, error and done state when askId changes", () => {
            const { container, update } = mount({ askId: 1, requestId: "r1" });
            emit({ type: "assistant_text", delta: "old answer" }, true);
            expect(q(container, ".btw-overlay__hint")!.textContent).toBe("Done · Esc to close");
            update({ askId: 2, requestId: null, question: "next?" });
            update({ requestId: "r2" });
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("");
            expect(q(container, ".btw-overlay__cursor")).not.toBeNull();
            expect(q(container, ".btw-overlay__hint")!.textContent).toBe("Esc to close");
            emit({ type: "assistant_text", delta: "new" });
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("new");
        });

        it("clears a previous stream error when askId changes", () => {
            const { container, update } = mount({ askId: 1 });
            emit({ type: "error", message: "old failure" });
            expect(q(container, ".btw-overlay__error")).not.toBeNull();
            update({ askId: 2, requestId: "r2" });
            expect(q(container, ".btw-overlay__error")).toBeNull();
            expect(q(container, ".btw-overlay__answer")).not.toBeNull();
        });

        it("resets on a new askId even when the request id is unchanged", () => {
            const { container, update } = mount({ askId: 1, requestId: "r1" });
            emit({ type: "assistant_text", delta: "old" }, true);
            update({ askId: 2 });
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("");
            expect(q(container, ".btw-overlay__hint")!.textContent).toBe("Esc to close");
        });

        it("a request id cycle alone does not clear the answer", () => {
            const { container, update } = mount({ askId: 1, requestId: "r1" });
            emit({ type: "assistant_text", delta: "kept" });
            update({ requestId: null });
            update({ requestId: "r2" });
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("kept");
        });

        it("does not reset when something other than askId changes", () => {
            const { container, update } = mount({ askId: 1 });
            emit({ type: "assistant_text", delta: "keep" });
            update({ question: "reworded" });
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("keep");
        });

        it("the old subscription's late chunks no longer reach the new ask after teardown", () => {
            const { container, update } = mount({ askId: 1, requestId: "r1" });
            const oldSub = lastSub();
            update({ askId: 2, requestId: null });
            update({ requestId: "r2" });
            expect(oldSub.unsub).toHaveBeenCalled();
            expect(lastSub()).not.toBe(oldSub);
            emit({ type: "assistant_text", delta: "fresh" }, false, lastSub());
            expect(q(container, ".btw-overlay__answer")!.textContent).toBe("fresh");
        });
    });

    describe("closing from the keyboard", () => {
        it("Escape closes the overlay and is consumed", () => {
            const onClose = vi.fn();
            mount({ onClose });
            const ev = key(document.body, "Escape");
            expect(onClose).toHaveBeenCalledTimes(1);
            expect(ev.defaultPrevented).toBe(true);
        });

        it("other keys do not close it and are not consumed", () => {
            const onClose = vi.fn();
            mount({ onClose });
            const ev = key(document.body, "Enter");
            expect(onClose).not.toHaveBeenCalled();
            expect(ev.defaultPrevented).toBe(false);
        });

        it("Escape is seen in the capture phase even when a descendant stops propagation", () => {
            const onClose = vi.fn();
            const { container } = mount({ onClose });
            const input = document.createElement("input");
            container.appendChild(input);
            input.addEventListener("keydown", (e) => e.stopPropagation());
            key(input, "Escape");
            expect(onClose).toHaveBeenCalledTimes(1);
        });

        it("Escape typed in another pane does not close this one", () => {
            const onClose = vi.fn();
            const wrap = document.createElement("div");
            wrap.setAttribute("data-blockid", "b1");
            document.body.appendChild(wrap);
            const other = document.createElement("div");
            other.setAttribute("data-blockid", "b2");
            const otherInput = document.createElement("input");
            other.appendChild(otherInput);
            document.body.appendChild(other);
            const [props] = createSignal(baseProps({ onClose }));
            render(() => <BtwOverlay {...propsProxy(props)} />, { container: wrap });
            const ev = key(otherInput, "Escape");
            expect(onClose).not.toHaveBeenCalled();
            expect(ev.defaultPrevented).toBe(false);
        });

        it("Escape typed in this pane closes it", () => {
            const onClose = vi.fn();
            const wrap = document.createElement("div");
            wrap.setAttribute("data-blockid", "b1");
            const mine = document.createElement("input");
            wrap.appendChild(mine);
            document.body.appendChild(wrap);
            const [props] = createSignal(baseProps({ onClose }));
            render(() => <BtwOverlay {...propsProxy(props)} />, { container: wrap });
            key(mine, "Escape");
            expect(onClose).toHaveBeenCalledTimes(1);
        });

        it("Escape from outside every pane closes it only when this pane is the focused one", () => {
            const onClose = vi.fn();
            const wrap = document.createElement("div");
            wrap.setAttribute("data-blockid", "b1");
            document.body.appendChild(wrap);
            const [props] = createSignal(baseProps({ onClose }));
            render(() => <BtwOverlay {...propsProxy(props)} />, { container: wrap });
            key(document.body, "Escape");
            expect(onClose).not.toHaveBeenCalled();
            wrap.classList.add("block-focused");
            key(document.body, "Escape");
            expect(onClose).toHaveBeenCalledTimes(1);
        });

        it("stops listening for Escape once unmounted", () => {
            const onClose = vi.fn();
            const { unmount } = mount({ onClose });
            unmount();
            key(document.body, "Escape");
            expect(onClose).not.toHaveBeenCalled();
        });

        it("the latest onClose is used", () => {
            const first = vi.fn();
            const second = vi.fn();
            const { update } = mount({ onClose: first });
            update({ onClose: second });
            key(document.body, "Escape");
            expect(first).not.toHaveBeenCalled();
            expect(second).toHaveBeenCalledTimes(1);
        });
    });

    describe("closing with the pointer", () => {
        it("a click outside the panel closes it and passes through untouched", () => {
            const onClose = vi.fn();
            mount({ onClose });
            const outside = document.createElement("button");
            document.body.appendChild(outside);
            const ev = pointerDown(outside);
            expect(onClose).toHaveBeenCalledTimes(1);
            expect(ev.defaultPrevented).toBe(false);
        });

        it("a click inside the panel leaves it open", () => {
            const onClose = vi.fn();
            const { container } = mount({ onClose });
            pointerDown(q(container, ".btw-overlay__question")!);
            pointerDown(q(container, ".btw-overlay")!);
            expect(onClose).not.toHaveBeenCalled();
        });

        it("does not stop the click from reaching the pane underneath", () => {
            mount();
            const outside = document.createElement("button");
            document.body.appendChild(outside);
            const seen = vi.fn();
            outside.addEventListener("pointerdown", seen);
            pointerDown(outside);
            expect(seen).toHaveBeenCalledTimes(1);
        });

        it("sees the click in the capture phase even when a target stops propagation", () => {
            const onClose = vi.fn();
            mount({ onClose });
            const outside = document.createElement("button");
            document.body.appendChild(outside);
            outside.addEventListener("pointerdown", (e) => e.stopPropagation());
            pointerDown(outside);
            expect(onClose).toHaveBeenCalledTimes(1);
        });

        it("a click in the pane next to it leaves it open", () => {
            const onClose = vi.fn();
            const wrap = document.createElement("div");
            wrap.setAttribute("data-blockid", "b1");
            document.body.appendChild(wrap);
            const other = document.createElement("div");
            other.setAttribute("data-blockid", "b2");
            const otherBtn = document.createElement("button");
            other.appendChild(otherBtn);
            document.body.appendChild(other);
            const [props] = createSignal(baseProps({ onClose }));
            render(() => <BtwOverlay {...propsProxy(props)} />, { container: wrap });
            pointerDown(otherBtn);
            expect(onClose).not.toHaveBeenCalled();
        });

        it("a click elsewhere in this pane closes it", () => {
            const onClose = vi.fn();
            const wrap = document.createElement("div");
            wrap.setAttribute("data-blockid", "b1");
            const sibling = document.createElement("button");
            wrap.appendChild(sibling);
            document.body.appendChild(wrap);
            const [props] = createSignal(baseProps({ onClose }));
            render(() => <BtwOverlay {...propsProxy(props)} />, { container: wrap });
            pointerDown(sibling);
            expect(onClose).toHaveBeenCalledTimes(1);
        });

        it("a click outside every pane closes it only when this pane is the focused one", () => {
            const onClose = vi.fn();
            const wrap = document.createElement("div");
            wrap.setAttribute("data-blockid", "b1");
            document.body.appendChild(wrap);
            const [props] = createSignal(baseProps({ onClose }));
            render(() => <BtwOverlay {...propsProxy(props)} />, { container: wrap });
            const chrome = document.createElement("div");
            document.body.appendChild(chrome);
            pointerDown(chrome);
            expect(onClose).not.toHaveBeenCalled();
            wrap.classList.add("block-focused");
            pointerDown(chrome);
            expect(onClose).toHaveBeenCalledTimes(1);
        });

        it("stops listening for pointer events once unmounted", () => {
            const onClose = vi.fn();
            const { unmount } = mount({ onClose });
            unmount();
            const outside = document.createElement("button");
            document.body.appendChild(outside);
            pointerDown(outside);
            expect(onClose).not.toHaveBeenCalled();
        });
    });

    describe("document listeners", () => {
        it("registers both listeners on the document in the capture phase", () => {
            const add = vi.spyOn(document, "addEventListener");
            const remove = vi.spyOn(document, "removeEventListener");
            const { unmount } = mount();
            const added = add.mock.calls.filter(([t]) => t === "keydown" || t === "pointerdown");
            expect(added.map(([t, , o]) => [t, o])).toEqual([
                ["keydown", true],
                ["pointerdown", true],
            ]);
            unmount();
            const removed = remove.mock.calls.filter(([t]) => t === "keydown" || t === "pointerdown");
            expect(removed.map(([t, , o]) => [t, o])).toEqual([
                ["keydown", true],
                ["pointerdown", true],
            ]);
            expect(removed[0][1]).toBe(added[0][1]);
            expect(removed[1][1]).toBe(added[1][1]);
            add.mockRestore();
            remove.mockRestore();
        });
    });
});
