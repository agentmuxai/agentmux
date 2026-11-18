// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The toast stack's own row: "+N more" and "Dismiss all", above the toasts.

import { setNotifications } from "@/app/store/flash-notifications";
import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const ensureSelectionFocused = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/focusManager", () => ({ focusManager: { ensureSelectionFocused } }));

import { NotificationBubbles } from "./notificationbubbles";

function toast(id: string, extra: Partial<NotificationType> = {}): NotificationType {
    return { id, icon: "bell", title: id, message: "", timestamp: "", ...extra };
}
const toasts = (n: number) => Array.from({ length: n }, (_, i) => toast(`t${i}`));

const bar = () => document.querySelector(".notification-stack-bar") as HTMLElement | null;
const moreButton = () => document.querySelector(".notification-more") as HTMLButtonElement | null;
const dismissAll = () => document.querySelector(".notification-dismiss-all") as HTMLButtonElement | null;
// Inside the stack: a toast that goes leaves a fading copy of itself outside it for a moment.
const cards = () => document.querySelectorAll(".notification-bubbles .notification-bubble").length;

afterEach(() => {
    cleanup();
    setNotifications([]);
    document.body.innerHTML = "";
    ensureSelectionFocused.mockClear();
});

describe("the toast stack's row", () => {
    it("is not there for a single toast, which has its own X", () => {
        setNotifications(toasts(1));
        render(() => <NotificationBubbles />);
        expect(cards()).toBe(1);
        expect(bar()).toBeNull();
    });

    it("offers Dismiss all as soon as two toasts are up, with no count when none is hidden", () => {
        setNotifications(toasts(2));
        render(() => <NotificationBubbles />);
        expect(dismissAll()?.textContent).toBe("Dismiss all");
        expect(moreButton()).toBeNull();
    });

    it("says how many toasts are hidden, and is the first thing in the stack", () => {
        setNotifications(toasts(8));
        render(() => <NotificationBubbles />);
        expect(cards()).toBe(5);
        expect(moreButton()?.textContent).toBe("+3 more");
        expect(moreButton()?.getAttribute("aria-expanded")).toBe("false");
        const body = document.querySelector(".notification-bubbles-body") as HTMLElement;
        expect(body.firstElementChild).toBe(bar());
        // The label first, the button after it: reading and tab order.
        expect([...bar()!.querySelectorAll("button")]).toEqual([moreButton(), dismissAll()]);
    });

    it("stays first, with Dismiss all, when the hidden toasts are shown", () => {
        setNotifications(toasts(8));
        render(() => <NotificationBubbles />);
        fireEvent.click(moreButton()!);
        expect(cards()).toBe(8);
        expect(moreButton()).toBeNull();
        const body = document.querySelector(".notification-bubbles-body") as HTMLElement;
        expect(body.firstElementChild).toBe(bar());
        expect(dismissAll()).not.toBeNull();
    });

    it("goes when the stack is back to one toast", () => {
        setNotifications(toasts(2));
        render(() => <NotificationBubbles />);
        expect(bar()).not.toBeNull();
        setNotifications(toasts(1));
        expect(bar()).toBeNull();
    });
});

describe("Dismiss all", () => {
    it("closes every toast, the hidden and the persistent ones too, and tells each sender once", () => {
        const closed = vi.fn();
        const list = [
            toast("sticky", { persistent: true, onClose: () => closed("sticky") }),
            ...Array.from({ length: 8 }, (_, i) => toast(`t${i}`, { onClose: () => closed(`t${i}`) })),
        ];
        setNotifications(list);
        render(() => <NotificationBubbles />);
        expect(moreButton()?.textContent).toBe("+4 more");
        fireEvent.click(dismissAll()!);
        expect(cards()).toBe(0);
        expect(document.querySelector(".notification-bubbles")).toBeNull();
        expect(closed.mock.calls.map((c) => c[0]).sort()).toEqual(list.map((n) => n.id).sort());
    });

    it("tells the other senders when one sender's close handler throws", () => {
        const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
        const closed = vi.fn();
        setNotifications([
            toast("bad", {
                onClose: () => {
                    throw new Error("boom");
                },
            }),
            toast("good", { onClose: closed }),
        ]);
        render(() => <NotificationBubbles />);
        fireEvent.click(dismissAll()!);
        expect(cards()).toBe(0);
        expect(closed).toHaveBeenCalledTimes(1);
        quiet.mockRestore();
    });

    it("returns the caret to the selected pane when it was on the button (keyboard)", () => {
        setNotifications(toasts(3));
        render(() => <NotificationBubbles />);
        dismissAll()!.focus();
        expect(document.activeElement).toBe(dismissAll());
        fireEvent.click(dismissAll()!);
        expect(ensureSelectionFocused).toHaveBeenCalledTimes(1);
    });

    it("leaves the caret alone when it was somewhere else (a pointer press does not take it)", () => {
        const field = document.createElement("input");
        document.body.appendChild(field);
        setNotifications(toasts(3));
        render(() => <NotificationBubbles />);
        field.focus();
        // The press itself must not move focus onto the button.
        const press = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
        dismissAll()!.dispatchEvent(press);
        expect(press.defaultPrevented).toBe(true);
        fireEvent.click(dismissAll()!);
        expect(cards()).toBe(0);
        expect(document.activeElement).toBe(field);
        expect(ensureSelectionFocused).not.toHaveBeenCalled();
    });
});
