// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { t } from "@/app/i18n/i18n";
import { ensureLease, joinLease, releaseLease, releaseOnPageHide, resetLeaseState } from "./lease-step";

type Answer = { status: number; body: unknown };
const answer = (status: number, body: unknown): Answer => ({ status, body });

/** A fetch that answers from a list, one per call (the last repeats), recording every call. */
function fake(answers: Answer[]) {
    const calls: { input: string; init: RequestInit }[] = [];
    const fetchImpl = async (input: string, init: RequestInit) => {
        calls.push({ input, init });
        const a = answers[Math.min(calls.length - 1, answers.length - 1)];
        return {
            status: a.status,
            json: async () => {
                if (a.body === undefined) throw new Error("not json");
                return a.body;
            },
        };
    };
    return { calls, fetchImpl };
}

const instantly = async () => {};

function run(answers: Answer[], extra: Partial<Parameters<typeof ensureLease>[0]> = {}) {
    const f = fake(answers);
    const starting = vi.fn();
    const ctl = new AbortController();
    const promise = ensureLease({ signal: ctl.signal, onStarting: starting, fetchImpl: f.fetchImpl, sleep: instantly, ...extra });
    return { ...f, starting, ctl, promise };
}

afterEach(() => resetLeaseState());

describe("asking the server for the pane's browser", () => {
    it("asks with a POST for this page's own origin, with no body of any use and nothing kept", async () => {
        const r = run([answer(200, { ready: true })]);
        await r.promise;
        expect(r.calls).toHaveLength(1);
        expect(r.calls[0].input).toBe("/me/browser");
        expect(r.calls[0].init).toMatchObject({ method: "POST", credentials: "same-origin", body: "{}" });
    });

    it("goes straight on when the browser is ready, without a word about starting", async () => {
        const r = run([answer(200, { ready: true, lease: "x" })]);
        expect(await r.promise).toEqual({ ok: true });
        expect(r.starting).not.toHaveBeenCalled();
    });

    it("keeps asking while it starts, says so each time, and goes on when it is ready", async () => {
        const sleeps: number[] = [];
        const r = run([answer(200, { ready: false }), answer(200, { ready: false }), answer(200, { ready: true })], { sleep: async (ms) => void sleeps.push(ms) });
        expect(await r.promise).toEqual({ ok: true });
        expect(r.calls).toHaveLength(3);
        expect(r.starting).toHaveBeenCalledTimes(2);
        expect(sleeps).toEqual([2000, 2000]);
    });

    it("gives up on a browser that is never ready, with its own words", async () => {
        let clock = 0;
        const r = run([answer(200, { ready: false })], {
            now: () => clock,
            sleep: async (ms) => void (clock += ms * 30),
        });
        expect(await r.promise).toEqual({ ok: false, message: t("browser.streamed.leaseTimeout") });
    });
});

describe("a server that does not hand out browsers", () => {
    it.each([
        ["a 404 that is no answer of the route's", answer(404, { message: "nope" })],
        ["a 404 with no body", answer(404, undefined)],
        ["a 405", answer(405, { error: "method" })],
        ["a 200 that is the page's own fallback, not JSON", answer(200, undefined)],
        ["a 502 from something in front", answer(502, undefined)],
    ])("goes straight to the socket on %s, and releases nothing later", async (_what, a) => {
        const r = run([a]);
        expect(await r.promise).toEqual({ ok: true });
        expect(r.starting).not.toHaveBeenCalled();
        const del = fake([answer(200, {})]);
        releaseLease(del.fetchImpl);
        expect(del.calls).toHaveLength(0);
    });

    it("goes to the socket when the server cannot be reached, which says so if it cannot be opened either", async () => {
        const r = run([], { fetchImpl: async () => Promise.reject(new Error("offline")) });
        expect(await r.promise).toEqual({ ok: true });
    });
});

describe("a server that refuses", () => {
    it("ends the pane with the server's own words", async () => {
        const r = run([answer(403, { error: "not-allowed", message: "Not available for this account." })]);
        expect(await r.promise).toEqual({ ok: false, message: "Not available for this account." });
        expect(r.calls).toHaveLength(1);
    });

    it("has words of its own for a refusal that says none", async () => {
        const r = run([answer(503, { error: "unavailable" })]);
        expect(await r.promise).toEqual({ ok: false, message: t("browser.streamed.leaseFailed") });
    });

    it("takes a 404 that names a workspace as the route's answer, not as no route", async () => {
        const r = run([answer(404, { error: "not-found", message: "That workspace was not found." })]);
        expect(await r.promise).toEqual({ ok: false, message: "That workspace was not found." });
    });

    it("stops asking at the first refusal, even after some waiting", async () => {
        const r = run([answer(200, { ready: false }), answer(429, { error: "daily-cap", message: "Used up for today." }), answer(200, { ready: true })]);
        expect(await r.promise).toEqual({ ok: false, message: "Used up for today." });
        expect(r.calls).toHaveLength(2);
    });
});

describe("leaving", () => {
    it("stops asking when the pane goes, whether it is waiting to ask again or for an answer", async () => {
        const f = fake([answer(200, { ready: false })]);
        const ctl = new AbortController();
        let release!: () => void;
        const sleep = (_ms: number, signal: AbortSignal) => new Promise<void>((resolve) => (release = resolve) && signal.addEventListener("abort", () => resolve()));
        const p = ensureLease({ signal: ctl.signal, onStarting: () => {}, fetchImpl: f.fetchImpl, sleep });
        await vi.waitFor(() => expect(release).toBeDefined());
        ctl.abort();
        expect(await p).toMatchObject({ ok: false, aborted: true });
        expect(f.calls).toHaveLength(1);

        const ctl2 = new AbortController();
        const p2 = ensureLease({ signal: ctl2.signal, onStarting: () => {}, fetchImpl: () => new Promise((_r, rej) => ctl2.signal.addEventListener("abort", () => rej(new Error("aborted")))), sleep: instantly });
        ctl2.abort();
        expect(await p2).toMatchObject({ ok: false, aborted: true });
    });

    it("gives the browser back once, with a request that outlives the page, only if a server handed one out", async () => {
        await run([answer(200, { ready: true })]).promise;
        const del = fake([answer(200, { ended: true })]);
        releaseLease(del.fetchImpl);
        releaseLease(del.fetchImpl);
        expect(del.calls).toHaveLength(1);
        expect(del.calls[0].input).toBe("/me/browser");
        expect(del.calls[0].init).toMatchObject({ method: "DELETE", credentials: "same-origin", keepalive: true });
    });

    it("gives the browser back only when the page's last pane goes: one closing must not take it from another that is open", async () => {
        const del = fake([answer(200, {})]);
        const a = joinLease(del.fetchImpl);
        const b = joinLease(del.fetchImpl);
        await run([answer(200, { ready: true })]).promise;
        a();
        a(); // a second call for the same pane counts for nothing
        expect(del.calls).toHaveLength(0);
        b();
        expect(del.calls).toHaveLength(1);
        expect(del.calls[0].init).toMatchObject({ method: "DELETE", keepalive: true });
    });

    it("gives it back after a pane has come and gone and another has taken its place", async () => {
        const del = fake([answer(200, {})]);
        await run([answer(200, { ready: true })]).promise;
        joinLease(del.fetchImpl)();
        expect(del.calls).toHaveLength(1);
        await run([answer(200, { ready: true })]).promise;
        const next = joinLease(del.fetchImpl);
        next();
        expect(del.calls).toHaveLength(2);
    });

    it("gives back a browser an ask may have started even when the pane went before the answer came", async () => {
        const del = fake([answer(200, {})]);
        const leave = joinLease(del.fetchImpl);
        const ctl = new AbortController();
        const p = ensureLease({ signal: ctl.signal, onStarting: () => {}, fetchImpl: () => new Promise((_r, rej) => ctl.signal.addEventListener("abort", () => rej(new Error("aborted")))), sleep: instantly });
        ctl.abort();
        expect(await p).toMatchObject({ ok: false, aborted: true });
        leave();
        expect(del.calls).toHaveLength(1);
    });

    it("gives back a browser when the pane went just after the answer came, before it was read", async () => {
        const del = fake([answer(200, {})]);
        const leave = joinLease(del.fetchImpl);
        const ctl = new AbortController();
        const f = fake([answer(200, { ready: false })]);
        const p = ensureLease({
            signal: ctl.signal,
            onStarting: () => {},
            fetchImpl: async (input, init) => {
                const r = await f.fetchImpl(input, init);
                ctl.abort();
                return r;
            },
            sleep: instantly,
        });
        expect(await p).toMatchObject({ ok: false, aborted: true });
        leave();
        expect(del.calls).toHaveLength(1);
    });

    it("gives back nothing for a server that has no such route, however many panes come and go", async () => {
        const del = fake([answer(200, {})]);
        await run([answer(404, undefined)]).promise;
        joinLease(del.fetchImpl)();
        expect(del.calls).toHaveLength(0);
    });

    it("does not mind a release that fails", async () => {
        await run([answer(200, { ready: true })]).promise;
        expect(() => releaseLease(async () => Promise.reject(new Error("offline")))).not.toThrow();
    });

    it("gives the browser back when the page goes, wired once", async () => {
        const listeners: (() => void)[] = [];
        const target = { addEventListener: (_n: string, l: () => void) => void listeners.push(l) };
        releaseOnPageHide(target as unknown as Window);
        releaseOnPageHide(target as unknown as Window);
        expect(listeners).toHaveLength(1);
    });
});
