// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { createSignInNotice } from "./sign-in-notice";

describe("createSignInNotice", () => {
    it("answers a sign-in line when an account is bound, once", () => {
        const post = vi.fn();
        const notice = createSignInNotice(post);
        notice.seen("sign-in");
        notice.bound("Work");
        notice.bound("Work");
        expect(post).toHaveBeenCalledTimes(1);
        expect(post).toHaveBeenCalledWith("Signed in with **Work**", "success");
    });

    it("says nothing for a bind when the pane never asked for a sign-in", () => {
        const post = vi.fn();
        const notice = createSignInNotice(post);
        notice.seen(undefined);
        notice.bound("Work");
        expect(post).not.toHaveBeenCalled();
    });

    it("says nothing after a login answered the line itself", () => {
        const post = vi.fn();
        const notice = createSignInNotice(post);
        notice.seen("sign-in");
        notice.answered();
        notice.bound("Work");
        expect(post).not.toHaveBeenCalled();
    });
});
