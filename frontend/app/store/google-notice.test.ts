// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";

const pushed: NotificationType[] = [];
vi.mock("@/app/store/flash-notifications", () => ({ pushNotification: (n: NotificationType) => pushed.push(n) }));

import { acceptGoogleNotice } from "./google-notice";

describe("acceptGoogleNotice", () => {
    beforeEach(() => {
        pushed.length = 0;
    });

    it("shows the line as a short-lived notice that asks nothing", () => {
        acceptGoogleNotice({ text: "Naki-2 gave Agent5 Calendar read on me@example.com" });
        expect(pushed).toHaveLength(1);
        expect(pushed[0].message).toBe("Naki-2 gave Agent5 Calendar read on me@example.com");
        expect(pushed[0].persistent).toBeFalsy();
        expect(pushed[0].actions).toBeUndefined();
    });

    it("ignores an event with no text", () => {
        acceptGoogleNotice({ text: "  " });
        acceptGoogleNotice({});
        acceptGoogleNotice(null);
        expect(pushed).toHaveLength(0);
    });
});
