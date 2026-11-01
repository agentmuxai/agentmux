// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { removeNotificationById, notifications, pushNotification, setNotifications } from "@/app/store/flash-notifications";
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NotificationBubbles } from "./notificationbubbles";
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

    it("renders its action and leaves focus where it was; it is plain content, so it is not read twice", () => {
        const input = document.createElement("input");
        document.body.appendChild(input);
        input.focus();
        const { container } = renderItem(toast({ actions: [{ label: "Show here", actionKey: "a" }] }));
        const el = container.querySelector(".notification-bubble") as HTMLElement;
        expect(el.textContent).toContain("Show here");
        expect(el.getAttribute("role")).toBeNull();
        expect(document.activeElement).toBe(input);
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

describe("ToastAnnouncer", () => {
    const regions = () => ({
        status: document.querySelector('.toast-announcer[role="status"]') as HTMLElement,
        alert: document.querySelector('.toast-announcer[role="alert"]') as HTMLElement,
    });

    it("is mounted, empty, before any toast, and takes a toast's words when it arrives", async () => {
        const input = document.createElement("input");
        document.body.appendChild(input);
        input.focus();
        render(() => <NotificationBubbles />);
        const before = regions();
        expect(before.status.getAttribute("aria-live")).toBe("polite");
        expect(before.alert.getAttribute("aria-live")).toBe("assertive");
        expect(before.status.textContent).toBe("");
        pushNotification(toast({ id: "n1", title: "Settings is open in Window 2", message: "Show here or go there." }));
        await Promise.resolve();
        const after = regions();
        expect(after.status, "the same element, not a new one").toBe(before.status);
        expect(after.status.textContent).toBe("Settings is open in Window 2. Show here or go there.");
        expect(after.alert.textContent).toBe("");
        expect(document.activeElement, "focus stays where it was").toBe(input);
    });

    it("puts an error in the assertive region, and does not announce a toast twice", async () => {
        render(() => <NotificationBubbles />);
        pushNotification(toast({ id: "e1", type: "error", title: "Couldn't show that pane", message: "it is gone" }));
        await Promise.resolve();
        expect(regions().alert.textContent).toBe("Couldn't show that pane. it is gone");
        expect(regions().status.textContent).toBe("");
        pushNotification(toast({ id: "n2", title: "Done", message: "" }));
        await Promise.resolve();
        expect(regions().status.textContent).toBe("Done");
        expect(regions().alert.textContent).toBe("Couldn't show that pane. it is gone");
    });

    it("does not announce what was already there when it mounted", () => {
        setNotifications([toast({ id: "old", title: "Earlier", message: "" })]);
        render(() => <NotificationBubbles />);
        expect(regions().status.textContent).toBe("");
    });
});

describe("ToastAnnouncer, a toast that comes again", () => {
    const regions = () => document.querySelector('.toast-announcer[role="status"]') as HTMLElement;

    it("is announced again when it is re-raised under the same id, updated in place, or says the same words", async () => {
        render(() => <NotificationBubbles />);
        pushNotification(toast({ id: "signin-link", title: "Sign in", message: "to continue" }));
        await Promise.resolve();
        expect(regions().textContent).toBe("Sign in. to continue");
        removeNotificationById("signin-link");
        await Promise.resolve();
        pushNotification(toast({ id: "signin-link", title: "Sign in", message: "to continue" }));
        await Promise.resolve();
        expect(regions().textContent, "re-raised: the same words, made different so they are read again").toBe("Sign in. to continue​");
        pushNotification(toast({ id: "signin-link", title: "Signed in", message: "" }));
        await Promise.resolve();
        expect(regions().textContent, "updated in place").toBe("Signed in");
        pushNotification(toast({ id: "other", title: "Signed in", message: "" }));
        await Promise.resolve();
        expect(regions().textContent, "a second toast with the same words").toBe("Signed in​");
    });
});

describe("pausing expiry", () => {
    it("holds every toast that is hovered or focused, not only the last one touched", async () => {
        vi.useFakeTimers();
        vi.setSystemTime(10_000_000);
        const { container } = render(() => <NotificationBubbles />);
        pushNotification(toast({ id: "a", title: "A", message: "", expiration: 10_000_000 + 7000 }));
        pushNotification(toast({ id: "b", title: "B", message: "", expiration: 10_000_000 + 7000 }));
        await Promise.resolve();
        const items = () => [...container.ownerDocument.querySelectorAll<HTMLElement>(".notification-bubble")];
        const [a, b] = items();
        a.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
        b.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
        b.dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));
        vi.advanceTimersByTime(9000);
        await Promise.resolve();
        expect(items().map((el) => el.textContent), "A is still held; B ran out").toEqual([expect.stringContaining("A")]);
    });
});
