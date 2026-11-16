// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { notStoredNotice, whyFromStoreProblem, type NotStoredWhy } from "./login-store-notice";

const WHYS: NotStoredWhy[] = ["no-keychain", "unfinished"];

describe("notStoredNotice", () => {
    it("never says the link timed out: the person did everything right", () => {
        for (const why of WHYS) expect(notStoredNotice(why)).not.toMatch(/timed out|too slow/i);
    });

    it("says what happened: no login keychain, or a code that was sent without the sign-in finishing", () => {
        expect(notStoredNotice("no-keychain")).toMatch(/no login keychain/);
        expect(notStoredNotice("unfinished")).toMatch(/code was sent, but the sign-in didn't finish/);
    });

    it("tells the person what to do, and only ever says Cancel to a macOS dialog, never Reset To Defaults", () => {
        for (const why of WHYS) {
            const text = notStoredNotice(why);
            expect(text).toMatch(/choose Cancel, never Reset To Defaults/);
            expect(text.match(/Reset To Defaults/g)).toHaveLength(1);
        }
        expect(notStoredNotice("no-keychain")).toMatch(/start it from your usual macOS login/);
    });
});

describe("whyFromStoreProblem", () => {
    it("knows the one problem srv reports", () => {
        expect(whyFromStoreProblem("no-default-keychain")).toBe("no-keychain");
    });

    it("blocks nothing on an answer it does not know", () => {
        expect(whyFromStoreProblem(undefined)).toBeUndefined();
        expect(whyFromStoreProblem(null)).toBeUndefined();
        expect(whyFromStoreProblem("something-new")).toBeUndefined();
    });
});
