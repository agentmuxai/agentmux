// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** "Hide from widget bar": each hide's Undo restores its own snapshot, even when a second
 *  hide happens while the first notification is still up. */

import { describe, expect, it, vi } from "vitest";

const setConfig = vi.fn(async (_client: unknown, _data: unknown) => {});
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { SetConfigCommand: (c: unknown, d: unknown) => setConfig(c, d) } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const pushed: NotificationType[] = [];
vi.mock("@/store/global", () => ({
    pushNotification: (n: NotificationType) => pushed.push(n),
    createBlock: vi.fn(),
    openOrFocusPaneByView: vi.fn(),
}));
const actions = new Map<string, () => void>();
vi.mock("@/app/notification/usenotification", () => ({
    setNotificationAction: (k: string, fn: () => void) => actions.set(k, fn),
    clearNotificationAction: (k: string) => actions.delete(k),
}));

import { hideFromWidgetBar } from "./widget-bar-hide";

const w = (label: string): WidgetConfigType => ({ label, icon: "circle", blockdef: { meta: { view: label } } }) as WidgetConfigType;
const wmap = { "defwidget@a": w("A"), "defwidget@b": w("B") };
const flush = () => new Promise((r) => setTimeout(r, 0));

describe("hideFromWidgetBar", () => {
    it("gives each hide its own Undo, restoring that hide's snapshot", async () => {
        hideFromWidgetBar("defwidget@a", { "widget:pinned": ["a", "b"] }, wmap);
        await flush();
        hideFromWidgetBar("defwidget@b", { "widget:pinned": ["b"], "widget:hidden": ["defwidget@a"] }, wmap);
        await flush();
        const [first, second] = pushed.map((n) => n.actions![0].actionKey);
        expect(first).not.toBe(second);
        setConfig.mockClear();
        actions.get(first)!();
        expect(setConfig.mock.calls[0][1]).toEqual({ "widget:pinned": ["a", "b"], "widget:hidden": null });
        expect(actions.has(first)).toBe(false);
        expect(actions.has(second)).toBe(true);
    });
});
