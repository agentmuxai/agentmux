// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Keep running and Stop now from AgentMux Web: which route each asks, with what, and what a person is told when it does not work.

import { describe, expect, it, vi } from "vitest";
import { keepRunning, stopNow } from "./workspace-stop-actions";

const answer = (status: number, body: unknown = {}) => ({ status, json: async () => body });

describe("keepRunning", () => {
    it("asks the page's own server, with the page's cookie and no workspace of its own choosing", async () => {
        const f = vi.fn(async () => answer(200));
        expect(await keepRunning(f)).toEqual({ ok: true });
        expect(f).toHaveBeenCalledWith("/me/keep-running", expect.objectContaining({ method: "POST", credentials: "same-origin", body: "{}" }));
    });

    it("says what the server said when it refuses, and a plain sentence when it says nothing", async () => {
        const said = await keepRunning(async () => answer(409, { error: "too-late", message: "That workspace is already stopping." }));
        expect(said).toEqual({ ok: false, message: "That workspace is already stopping." });
        const none = await keepRunning(async () => answer(500, null));
        expect(none.ok).toBe(false);
        expect(none.message).toMatch(/could not/i);
    });

    it("tells a signed-out page to sign in, and one the server cannot reach that it could not be reached", async () => {
        const out = await keepRunning(async () => answer(403, { error: "signed-out" }));
        expect(out.message).toMatch(/sign in/i);
        const down = await keepRunning(async () => Promise.reject(new Error("offline")));
        expect(down.message).toMatch(/reach/i);
    });
});

describe("stopNow", () => {
    it("asks for the stop to be at once", async () => {
        const f = vi.fn(async () => answer(200));
        expect(await stopNow(f)).toEqual({ ok: true });
        expect(f).toHaveBeenCalledWith("/me/stop", expect.objectContaining({ method: "POST", credentials: "same-origin", body: JSON.stringify({ mode: "now" }) }));
    });
});
