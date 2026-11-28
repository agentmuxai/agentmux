// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A toast with a countdown closes itself after TOAST_COUNTDOWN_MS and says when: the seconds left and a shrinking bar.
// It holds while the pointer or focus is on it and starts again from the top when both leave; Esc and the X close it at
// once, and its buttons work until it closes.

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const reduce = vi.hoisted(() => ({ categories: new Set<string>() }));
vi.mock("@/app/motion/motion", () => ({
    shouldReduceMotion: (c: string) => reduce.categories.has(c),
    scaledMs: (ms: number) => ms,
}));

import { installFakeAnimate, type FakeAnimate } from "@/app/motion/test-utils";
import { NotificationItem } from "./notificationitem";
import { TOAST_COUNTDOWN_MS } from "./toast-live";

const toast = (extra: Partial<NotificationType> = {}) =>
    ({
        id: "n1",
        title: "Quen keeps running without a pane",
        message: "Closing keeps it running.",
        type: "info",
        timestamp: "",
        countdown: TOAST_COUNTDOWN_MS,
        actions: [{ label: "Open", actionKey: "open" }],
        ...extra,
    }) as NotificationType;

function mount(n: NotificationType = toast(), isBubble = true) {
    const onRemove = vi.fn();
    const onActionClick = vi.fn();
    const view = render(() => (
        <NotificationItem
            notification={n}
            onRemove={onRemove}
            onCopy={() => {}}
            onActionClick={onActionClick}
            formatTimestamp={() => ""}
            isBubble={isBubble}
        />
    ));
    const root = view.container.querySelector(".notification-bubble, .notification") as HTMLElement;
    const text = () => view.container.querySelector(".toast-countdown-text")?.textContent ?? null;
    const bar = () => view.container.querySelector(".toast-countdown-bar");
    return { view, root, onRemove, onActionClick, text, bar };
}

let fake: FakeAnimate;
beforeEach(() => {
    vi.useFakeTimers();
    fake = installFakeAnimate();
});
afterEach(() => {
    cleanup();
    fake.restore();
    reduce.categories.clear();
    vi.useRealTimers();
    document.body.innerHTML = "";
});

describe("a toast's countdown", () => {
    it("is ten seconds, in one place", () => {
        expect(TOAST_COUNTDOWN_MS).toBe(10_000);
    });

    it("shows the seconds left and a bar, and closes itself at ten seconds", () => {
        const m = mount();
        expect(m.text()).toBe("Closing in 10 s.");
        expect(m.bar()).not.toBeNull();
        expect((m.bar() as HTMLElement).style.animationDuration).toBe("10s");
        vi.advanceTimersByTime(3000);
        expect(m.text()).toBe("Closing in 7 s.");
        vi.advanceTimersByTime(6000);
        expect(m.text()).toBe("Closing in 1 s.");
        expect(m.onRemove).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1000);
        expect(m.onRemove).toHaveBeenCalledTimes(1);
        expect(m.onRemove).toHaveBeenCalledWith("n1");
        vi.advanceTimersByTime(30_000);
        expect(m.onRemove).toHaveBeenCalledTimes(1);
    });

    it("holds while the pointer is on it and starts again from ten when it leaves", () => {
        const m = mount();
        vi.advanceTimersByTime(4000);
        fireEvent.mouseEnter(m.root);
        vi.advanceTimersByTime(60_000);
        expect(m.onRemove).not.toHaveBeenCalled();
        expect(m.text()).toBe("Closing in 6 s.");
        expect(m.bar()?.classList.contains("toast-countdown-bar--held")).toBe(true);
        fireEvent.mouseLeave(m.root);
        expect(m.text()).toBe("Closing in 10 s.");
        expect(m.bar()?.classList.contains("toast-countdown-bar--held")).toBe(false);
        vi.advanceTimersByTime(9000);
        expect(m.onRemove).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1000);
        expect(m.onRemove).toHaveBeenCalledTimes(1);
    });

    it("holds while a button in it has keyboard focus, and not once focus has left", () => {
        const m = mount();
        const open = m.view.getByText("Open");
        fireEvent.focusIn(open);
        vi.advanceTimersByTime(60_000);
        expect(m.onRemove).not.toHaveBeenCalled();
        fireEvent.focusOut(open, { relatedTarget: document.body });
        vi.advanceTimersByTime(10_000);
        expect(m.onRemove).toHaveBeenCalledTimes(1);
    });

    it("keeps holding while the pointer is on it after focus left", () => {
        const m = mount();
        fireEvent.mouseEnter(m.root);
        fireEvent.focusIn(m.view.getByText("Open"));
        fireEvent.focusOut(m.view.getByText("Open"), { relatedTarget: document.body });
        vi.advanceTimersByTime(60_000);
        expect(m.onRemove).not.toHaveBeenCalled();
    });

    it("with reduced motion draws no bar and still counts the seconds", () => {
        reduce.categories.add("status");
        const m = mount();
        expect(m.bar()).toBeNull();
        expect(m.text()).toBe("Closing in 10 s.");
        vi.advanceTimersByTime(4000);
        expect(m.text()).toBe("Closing in 6 s.");
        vi.advanceTimersByTime(6000);
        expect(m.onRemove).toHaveBeenCalledTimes(1);
    });

    it("closes at once on Esc, from a button in it", () => {
        const m = mount();
        fireEvent.keyDown(m.view.getByText("Open"), { key: "Escape" });
        expect(m.onRemove).toHaveBeenCalledWith("n1");
    });

    it("closes at once with its X", () => {
        const m = mount();
        fireEvent.click(m.view.container.querySelector(".close-btn") as HTMLElement);
        expect(m.onRemove).toHaveBeenCalledWith("n1");
    });

    it("keeps its action button working until it closes", () => {
        const m = mount();
        vi.advanceTimersByTime(9000);
        fireEvent.click(m.view.getByText("Open"));
        expect(m.onActionClick).toHaveBeenCalledTimes(1);
        expect(m.onActionClick.mock.calls[0][1]).toMatchObject({ actionKey: "open" });
    });

    it("stops its timer when the toast goes", () => {
        const m = mount();
        m.view.unmount();
        vi.advanceTimersByTime(60_000);
        expect(m.onRemove).not.toHaveBeenCalled();
        expect(vi.getTimerCount()).toBe(0);
    });

    it("runs one timer for the toast, not one per frame", () => {
        mount(toast({ countdown: undefined }));
        const without = vi.getTimerCount();
        cleanup();
        mount();
        expect(vi.getTimerCount() - without).toBe(1);
    });

    it("is absent from a toast without a countdown, and from a toast in the list", () => {
        const plain = mount(toast({ countdown: undefined }));
        expect(plain.text()).toBeNull();
        vi.advanceTimersByTime(60_000);
        expect(plain.onRemove).not.toHaveBeenCalled();
        cleanup();
        const listed = mount(toast(), false);
        expect(listed.text()).toBeNull();
        vi.advanceTimersByTime(60_000);
        expect(listed.onRemove).not.toHaveBeenCalled();
    });

    it("Esc leaves a toast without a countdown alone", () => {
        const m = mount(toast({ countdown: undefined }));
        fireEvent.keyDown(m.view.getByText("Open"), { key: "Escape" });
        expect(m.onRemove).not.toHaveBeenCalled();
    });
});
