// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { removeNotificationById, notifications, pushNotification, setNotifications } from "@/app/store/flash-notifications";
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NotificationItem } from "./notificationitem";
import { liveRole, MIN_TOAST_MS, withToastLifetime } from "./toast-live";

const toast = (extra: Partial<NotificationType> = {}): NotificationType => ({
    id: "t",
    icon: "bell",
    title: "Settings is open in Window 2",
    message: "",
    timestamp: "",
    ...extra,
});

afterEach(() => {
    cleanup();
    setNotifications([]);
    document.body.innerHTML = "";
    vi.useRealTimers();
});

describe("liveRole", () => {
    it("is status for a result and alert for an error, unless the toast says", () => {
        expect(liveRole(toast())).toBe("status");
        expect(liveRole(toast({ type: "info" }))).toBe("status");
        expect(liveRole(toast({ type: "error" }))).toBe("alert");
        expect(liveRole(toast({ type: "error", live: "status" }))).toBe("status");
        expect(liveRole(toast({ live: "alert" }))).toBe("alert");
    });
});

describe("withToastLifetime", () => {
    const now = 1_000_000;
    it("keeps a toast with an action until it is acted on or closed", () => {
        const kept = withToastLifetime(toast({ expiration: now + 5000, actions: [{ label: "Show here", actionKey: "a" }] }), now);
        expect(kept.expiration).toBeUndefined();
    });
    it("shows any other toast for at least six seconds", () => {
        expect(withToastLifetime(toast({ expiration: now + 5000 }), now).expiration).toBe(now + MIN_TOAST_MS);
        expect(withToastLifetime(toast({ expiration: now + 20000 }), now).expiration).toBe(now + 20000);
        expect(withToastLifetime(toast(), now).expiration).toBeUndefined();
    });
});

describe("pushNotification", () => {
    it("stores an action toast with no expiry and a short one with the minimum", () => {
        vi.useFakeTimers();
        vi.setSystemTime(2_000_000);
        pushNotification(toast({ id: "act", expiration: 2_000_000 + 5000, actions: [{ label: "Go there", actionKey: "g" }] }));
        pushNotification(toast({ id: "quick", expiration: 2_000_000 + 5000 }));
        expect(notifications().find((n) => n.id === "act")?.expiration).toBeUndefined();
        expect(notifications().find((n) => n.id === "quick")?.expiration).toBe(2_000_000 + MIN_TOAST_MS);
        removeNotificationById("act");
    });
});

describe("NotificationItem", () => {
    const renderItem = (n: NotificationType, hover = vi.fn()) =>
        render(() => (
            <NotificationItem
                notification={n}
                onRemove={() => {}}
                onCopy={() => {}}
                onActionClick={() => {}}
                formatTimestamp={() => ""}
                isBubble={true}
                onMouseEnter={hover}
                onMouseLeave={() => {}}
            />
        ));

    it("is a status region with its action, and leaves focus where it was", () => {
        const input = document.createElement("input");
        document.body.appendChild(input);
        input.focus();
        const { container } = renderItem(toast({ actions: [{ label: "Show here", actionKey: "a" }] }));
        const el = container.querySelector(".notification-bubble") as HTMLElement;
        expect(el.getAttribute("role")).toBe("status");
        expect(el.getAttribute("aria-atomic")).toBe("true");
        expect(document.activeElement).toBe(input);
    });

    it("announces an error as an alert", () => {
        const { container } = renderItem(toast({ type: "error" }));
        expect(container.querySelector(".notification-bubble")?.getAttribute("role")).toBe("alert");
    });

    it("pauses like a hover while a button inside it has focus", () => {
        const hover = vi.fn();
        const { container } = renderItem(toast({ actions: [{ label: "Show here", actionKey: "a" }] }), hover);
        const action = [...container.querySelectorAll("button")].find((b) => b.textContent?.includes("Show here")) as HTMLElement;
        action.dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
        expect(hover).toHaveBeenCalled();
    });

    it("stays paused until neither the pointer nor the focus is on it", () => {
        const paused: boolean[] = [];
        const { container } = render(() => (
            <NotificationItem
                notification={toast({ expiration: Date.now() + 10000 })}
                onRemove={() => {}}
                onCopy={() => {}}
                onActionClick={() => {}}
                formatTimestamp={() => ""}
                isBubble={true}
                onMouseEnter={() => paused.push(true)}
                onMouseLeave={() => paused.push(false)}
            />
        ));
        const el = container.querySelector(".notification-bubble") as HTMLElement;
        const close = el.querySelector("button") as HTMLElement;
        const fire = (target: Element, type: string, init: EventInit & { relatedTarget?: Element | null } = {}) =>
            target.dispatchEvent(new (type.startsWith("focus") ? FocusEvent : MouseEvent)(type, { bubbles: true, ...init }));
        fire(close, "focusin");
        fire(el, "mouseenter");
        fire(el, "mouseleave");
        expect(paused.at(-1), "focus still holds it after the pointer leaves").toBe(true);
        fire(close, "focusout", { relatedTarget: el.querySelector(".notification-inner") });
        expect(paused.at(-1), "focus moving inside the toast is still focus").toBe(true);
        fire(close, "focusout", { relatedTarget: null });
        expect(paused.at(-1), "neither: it runs again").toBe(false);
        fire(close, "focusin");
        fire(el, "mouseenter");
        fire(close, "focusout", { relatedTarget: null });
        expect(paused.at(-1), "the pointer still holds it after focus leaves").toBe(true);
    });
});
