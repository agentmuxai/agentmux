// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The startup notice pointing to Settings › Agents when agents have an
// outdated id: shown once, until dismissed.

import { beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    migrate: vi.fn(),
    setConfig: vi.fn(() => Promise.resolve()),
    paneOpen: vi.fn(() => Promise.resolve({})),
    pushNotification: vi.fn(),
    openSettingsAt: vi.fn(() => Promise.resolve()),
    settings: {} as Record<string, unknown>,
    actions: new Map<string, () => void>(),
}));

vi.mock("@/app/store/global", () => ({ pushNotification: h.pushNotification, settingsAtom: () => h.settings }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { AgentsMigrateForkSlugsCommand: h.migrate, SetConfigCommand: h.setConfig, PaneOpenCommand: h.paneOpen },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("./open-settings", () => ({ openSettingsAt: h.openSettingsAt }));
vi.mock("@/app/notification/usenotification", () => ({
    setNotificationAction: (key: string, fn: () => void) => h.actions.set(key, fn),
}));

import { NOTICE_ID, NOTICE_SETTING, offerForkSlugUpdate } from "./fork-slug-notice";

function planWith(n: number) {
    const candidates = Array.from({ length: n }, (_, i) => ({
        uid: `u${i}`,
        name: `Loap #${i + 2}`,
        old_slug: `loap-loap-${i + 2}`,
        new_slug: `loap-${i + 2}`,
        locations: [],
        running: [],
    }));
    return { mode: "plan", plan: { candidates, skipped: [], blockers: [], summary: "" }, pending: [], wait_ms: 0, summary: "" };
}

describe("offerForkSlugUpdate", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        h.actions.clear();
        h.settings = {};
    });

    it("asks for the candidates only, and shows one persistent notice pointing to Settings › Agents", async () => {
        h.migrate.mockResolvedValue(planWith(2));
        await offerForkSlugUpdate(0);
        expect(h.migrate).toHaveBeenCalledWith({}, { mode: "plan", quick: true });
        expect(h.pushNotification).toHaveBeenCalledTimes(1);
        const n = h.pushNotification.mock.calls[0][0];
        expect(n.id).toBe(NOTICE_ID);
        expect(n.persistent).toBe(true);
        expect(n.title).toBe("2 agents have an outdated id");
        expect(n.actions.map((a: { label: string }) => a.label)).toEqual(["Open Settings", "Don't show again"]);
    });

    it("opens Settings at Agents and never shows again once acted on or closed", async () => {
        h.migrate.mockResolvedValue(planWith(1));
        await offerForkSlugUpdate(0);
        const n = h.pushNotification.mock.calls[0][0];
        expect(n.title).toBe("1 agent has an outdated id");
        h.actions.get(n.actions[0].actionKey)!();
        expect(h.openSettingsAt).toHaveBeenCalledWith("agents");
        expect(h.paneOpen).not.toHaveBeenCalled();
        expect(h.setConfig).toHaveBeenCalledWith({}, { [NOTICE_SETTING]: false });
        h.setConfig.mockClear();
        n.onClose();
        expect(h.setConfig).toHaveBeenCalledWith({}, { [NOTICE_SETTING]: false });
    });

    it("says nothing once dismissed, or when no agent has an outdated id", async () => {
        h.settings = { [NOTICE_SETTING]: false };
        await offerForkSlugUpdate(0);
        expect(h.migrate).not.toHaveBeenCalled();

        h.settings = {};
        h.migrate.mockResolvedValue(planWith(0));
        await offerForkSlugUpdate(0);
        expect(h.pushNotification).not.toHaveBeenCalled();

        h.migrate.mockRejectedValue(new Error("srv is older"));
        await offerForkSlugUpdate(0);
        expect(h.pushNotification).not.toHaveBeenCalled();
    });
});
