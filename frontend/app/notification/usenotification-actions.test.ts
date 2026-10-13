// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Runtime notification actions (an Undo) live only as long as a notification offers them. */

import { createSignal } from "solid-js";
import { describe, expect, it, vi } from "vitest";

const [notifications, setNotifications] = createSignal<NotificationType[]>([]);
vi.mock("@/store/global", () => ({
    atoms: { notifications: () => notifications(), notificationPopoverMode: () => false },
    getApi: () => ({}),
    setNotifications: vi.fn(),
}));
vi.mock("@/util/clipboard", () => ({ writeText: vi.fn() }));
vi.mock("@/app/hook/useTick", () => ({ useTick: () => () => 0 }));

import { hasNotificationAction, setNotificationAction } from "./usenotification";

const offering = (key: string): NotificationType => ({
    icon: "x",
    title: "t",
    message: "m",
    timestamp: "",
    actions: [{ label: "Undo", actionKey: key }],
});

describe("runtime notification actions", () => {
    it("are kept until offered, then dropped when the offering notification leaves", () => {
        setNotificationAction("undo:1", () => {});
        expect(hasNotificationAction("undo:1")).toBe(true); // registered, not yet shown
        setNotifications([offering("undo:1")]);
        expect(hasNotificationAction("undo:1")).toBe(true);
        setNotifications([{ ...offering("undo:1"), hidden: true }]); // hidden into the popover: still offered
        expect(hasNotificationAction("undo:1")).toBe(true);
        setNotifications([]); // expired or dismissed
        expect(hasNotificationAction("undo:1")).toBe(false);
    });

    it("never prunes the built-in actions", () => {
        setNotificationAction("undo:2", () => {});
        setNotifications([offering("undo:2")]);
        setNotifications([]);
        expect(hasNotificationAction("installUpdate")).toBe(true);
    });
});
