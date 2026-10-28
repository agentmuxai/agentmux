// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createSignInNotice, SIGN_IN_NOTICE_ID } from "./sign-in-notice";

describe("createSignInNotice", () => {
    it("posts the sign-in line once, then replaces it when an account is bound", () => {
        const say = vi.fn();
        const notice = createSignInNotice(say);
        notice.needed("Claude Code needs you to sign in");
        notice.bound("Work");
        notice.bound("Work");
        expect(say).toHaveBeenCalledTimes(2);
        expect(say).toHaveBeenNthCalledWith(1, "Claude Code needs you to sign in", "info", SIGN_IN_NOTICE_ID, false);
        expect(say).toHaveBeenNthCalledWith(2, "Signed in with **Work**", "success", SIGN_IN_NOTICE_ID, true);
    });

    it("says nothing for a bind when the pane never asked for a sign-in", () => {
        const say = vi.fn();
        createSignInNotice(say).bound("Work");
        expect(say).not.toHaveBeenCalled();
    });

    it("replaces the line with the login's own when a login answers it", () => {
        const say = vi.fn();
        const notice = createSignInNotice(say);
        notice.needed("Claude Code needs you to sign in");
        notice.answered("Logged in as **a@b.c**");
        notice.bound("Work");
        expect(say).toHaveBeenCalledTimes(2);
        expect(say).toHaveBeenLastCalledWith("Logged in as **a@b.c**", "success", SIGN_IN_NOTICE_ID, true);
    });

    it("lets a login that nothing asked for stand as its own line", () => {
        const say = vi.fn();
        createSignInNotice(say).answered("Logged in as **a@b.c**");
        expect(say).toHaveBeenCalledTimes(1);
        const [, , id, replaces] = say.mock.calls[0];
        expect(id).not.toBe(SIGN_IN_NOTICE_ID);
        expect(replaces).toBe(false);
    });

    it("asks again in the same line when a later launch needs a sign-in once more", () => {
        const say = vi.fn();
        const notice = createSignInNotice(say);
        notice.needed("sign in");
        notice.bound("Work");
        notice.needed("sign in again");
        expect(say).toHaveBeenLastCalledWith("sign in again", "info", SIGN_IN_NOTICE_ID, true);
        notice.bound("Work");
        expect(say).toHaveBeenCalledTimes(4);
    });
});
