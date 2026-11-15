// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A browser that is not signed in to a headless server is told so, in words, with no Restore button.

import { StartupFailureHandled } from "@/app/init/error-display";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { notSignedInText, requireSignIn, showNotSignedIn, signInProblem } from "./not-signed-in";

beforeEach(() => {
    document.body.innerHTML = '<div id="main"></div>';
    (window as any).api = { getAuthKey: () => "", getSessionToken: () => undefined };
});
afterEach(() => {
    vi.unstubAllGlobals();
    delete (window as any).api;
});

const answering = (status: number) => vi.stubGlobal("fetch", vi.fn(async () => ({ status })));

describe("whether this browser is signed in", () => {
    it("is not, when the server refuses a browser that holds no sign-in", async () => {
        expect(await signInProblem(undefined, async () => ({ status: 401 }))).toBe("none");
    });

    it("has ended, when the server refuses the sign-in the browser holds", async () => {
        expect(await signInProblem("tok", async () => ({ status: 401 }))).toBe("ended");
    });

    it("is fine when the server answers, with or without a token (a proxy may add the key)", async () => {
        expect(await signInProblem(undefined, async () => ({ status: 200 }))).toBeNull();
        expect(await signInProblem("tok", async () => ({ status: 200 }))).toBeNull();
    });

    it("is not blamed when the server cannot be reached or fails: that is another failure with its own recovery", async () => {
        expect(await signInProblem("tok", async () => Promise.reject(new TypeError("Failed to fetch")))).toBeNull();
        expect(await signInProblem("tok", async () => ({ status: 503 }))).toBeNull();
    });
});

describe("the card", () => {
    it("says plainly that this browser is not signed in and what to run, and has no Restore button", () => {
        showNotSignedIn("none");
        const card = document.querySelector('[role="alert"]')!;
        expect(card.textContent).toContain("This browser isn't signed in");
        expect(card.textContent).toContain("agentmux open");
        expect(card.textContent).not.toContain("agentmux-srv");
        expect(card.textContent).not.toMatch(/Restore|couldn't start|Reconnecting/);
        expect(card.querySelector("button")).toBeNull();
    });

    it("says an ended sign-in ended", () => {
        expect(notSignedInText("ended").lead).toContain("sign-in has ended");
        expect(notSignedInText("none").lead).toContain("hasn't been signed in");
    });
});

describe("starting the page", () => {
    it("stops with the card, as a failure already handled, when the server refuses this browser", async () => {
        answering(401);
        await expect(requireSignIn(undefined)).rejects.toBeInstanceOf(StartupFailureHandled);
        expect(document.body.textContent).toContain("This browser isn't signed in");
    });

    it("sends the tab's session token, and nothing else, to the server", async () => {
        const fetchMock = vi.fn(async (_input: string, _init: RequestInit) => ({ status: 200 }));
        await signInProblem("tok", fetchMock);
        expect(fetchMock).toHaveBeenCalledWith("/api/bus/agents", {
            headers: { "X-AM-Session": "tok" },
            credentials: "same-origin",
            signal: expect.any(AbortSignal),
        });
    });

    it("stops asking after three seconds, so the app can start inside the five seconds it waits for the host API", async () => {
        vi.useFakeTimers();
        try {
            const never = (_input: string, init: RequestInit) =>
                new Promise<{ status: number }>((_resolve, reject) => init.signal?.addEventListener("abort", () => reject(new Error("aborted"))));
            const pending = signInProblem("tok", never);
            await vi.advanceTimersByTimeAsync(3000);
            expect(await pending).toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });

    it("goes on, and shows nothing, when the server accepts it", async () => {
        answering(200);
        await expect(requireSignIn("tok")).resolves.toBeUndefined();
        expect(document.querySelector('[role="alert"]')).toBeNull();
    });
});
