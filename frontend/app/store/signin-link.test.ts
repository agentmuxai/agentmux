// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const openExternal = vi.fn();
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ openExternal }) }));

const pushed: NotificationType[] = [];
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: (n: NotificationType) => pushed.push(n) }));

const actions: Record<string, () => void> = {};
vi.mock("@/app/notification/usenotification", () => ({
    setNotificationAction: (key: string, fn: () => void) => {
        actions[key] = fn;
    },
}));

import { acceptSigninLink, isSigninLink } from "./signin-link";

const LINK = "https://claude.com/cai/oauth/authorize?code=true&client_id=x";

describe("acceptSigninLink", () => {
    beforeEach(() => {
        pushed.length = 0;
        openExternal.mockClear();
    });

    it("raises a notice that stays, names the machine and provider, and opens the link on Open", () => {
        acceptSigninLink({ provider: "claude", machine: "charlie", url: LINK });
        expect(pushed).toHaveLength(1);
        const n = pushed[0];
        expect(n.title).toBe("Sign charlie in to Claude");
        expect(n.persistent).toBe(true);
        expect(n.actions?.map((a) => a.label)).toEqual(["Open"]);
        actions[n.actions![0].actionKey]();
        expect(openExternal).toHaveBeenCalledWith(LINK);
    });

    it("tells the person the code to enter when the sign-in has one", () => {
        acceptSigninLink({ provider: "codex", machine: "charlie", url: "https://auth.openai.com/codex/device", code: "RC63-52E85" });
        expect(pushed[0].title).toBe("Sign charlie in to Codex");
        expect(pushed[0].message).toContain("RC63-52E85");
    });

    it("ignores a link that isn't https or isn't a link", () => {
        for (const url of ["http://claude.com/x", "javascript:alert(1)", "https://u:p@claude.com/x", "", undefined]) {
            acceptSigninLink({ provider: "claude", machine: "charlie", url: url as string });
        }
        acceptSigninLink(null);
        expect(pushed).toHaveLength(0);
        expect(isSigninLink(LINK)).toBe(true);
    });
});
