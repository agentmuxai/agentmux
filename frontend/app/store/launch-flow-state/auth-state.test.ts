// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { initialState, update, type AuthState } from "@/app/store/launch-flow-state/auth-state";

const seed = (overrides: Partial<AuthState> = {}): AuthState => ({
    ...initialState(),
    ...overrides,
});

describe("auth-state reducer", () => {
    describe("Selected", () => {
        it("transitions to `ready` when the bundle is authenticated", () => {
            const r = update(initialState(), {
                type: "Selected",
                providerId: "claude",
                bundleId: "b1",
                outcome: "ready",
            });
            expect(r.state.kind).toBe("ready");
            expect(r.state.providerId).toBe("claude");
            expect(r.state.bundleId).toBe("b1");
            expect(r.events[0]).toMatchObject({
                type: "selection-changed",
                kind: "ready",
            });
        });

        it("transitions to `expired` for stale bundles", () => {
            const r = update(initialState(), {
                type: "Selected",
                providerId: "claude",
                bundleId: "b1",
                outcome: "expired",
            });
            expect(r.state.kind).toBe("expired");
        });

        it("transitions to `unauthenticated` for needs-account", () => {
            const r = update(initialState(), {
                type: "Selected",
                providerId: "claude",
                bundleId: "b1",
                outcome: "needs-account",
            });
            expect(r.state.kind).toBe("unauthenticated");
        });

        it("clears prior session + error state on selection change", () => {
            const dirty = seed({
                kind: "failed",
                sessionId: "s1",
                authUrl: "https://x",
                error: "oops",
            });
            const r = update(dirty, {
                type: "Selected",
                providerId: "claude",
                bundleId: "b1",
                outcome: "ready",
            });
            expect(r.state.sessionId).toBe("");
            expect(r.state.authUrl).toBe("");
            expect(r.state.error).toBe("");
        });

        it("is idempotent on identical selection", () => {
            const seeded = seed({
                providerId: "claude",
                bundleId: "b1",
                kind: "ready",
            });
            const r = update(seeded, {
                type: "Selected",
                providerId: "claude",
                bundleId: "b1",
                outcome: "ready",
            });
            expect(r.state).toBe(seeded);
            expect(r.events).toEqual([]);
        });
    });

    describe("ConnectClicked", () => {
        it("transitions unauthenticated → waiting and emits start-requested", () => {
            const seeded = seed({
                kind: "unauthenticated",
                providerId: "claude",
                bundleId: "",
            });
            const r = update(seeded, { type: "ConnectClicked" });
            expect(r.state.kind).toBe("waiting");
            expect(r.events[0]).toMatchObject({
                type: "start-requested",
                providerId: "claude",
            });
        });

        it("transitions expired → waiting (re-auth path)", () => {
            const seeded = seed({ kind: "expired", providerId: "codex" });
            const r = update(seeded, { type: "ConnectClicked" });
            expect(r.state.kind).toBe("waiting");
        });

        it("clears prior error when re-attempting after Failed", () => {
            const seeded = seed({
                kind: "failed",
                error: "previous failure",
                providerId: "claude",
            });
            const r = update(seeded, { type: "ConnectClicked" });
            expect(r.state.kind).toBe("waiting");
            expect(r.state.error).toBe("");
        });

        it("is dropped (no transition) from waiting / idle", () => {
            for (const kind of ["waiting", "idle"] as const) {
                const seeded = seed({ kind });
                const r = update(seeded, { type: "ConnectClicked" });
                expect(r.state).toBe(seeded);
                expect(r.events[0]).toMatchObject({
                    type: "post-close-command-dropped",
                });
            }
        });

        it("transitions ready → waiting — the Reconnect CTA for a stale needs_reauth/expired account (reagent P0 on #2262)", () => {
            // outcomeFor() only ever returns "needs-account"/"ready" — a
            // bound-but-stale account still lands in `ready`, with
            // PreLaunchAuthPanel.tsx's separate accountStatus check
            // deciding whether to show the Reconnect wording. Without this
            // transition, ConnectClicked from `ready` was a silent no-op:
            // runProviderLogin still ran the real backend login/refresh,
            // but its outcome dispatches (Seeded, ConnectFailed) were ALSO
            // dropped from `ready`, since they only accept `waiting` as an
            // origin besides the fresh-connect kinds — the UI just sat on
            // the same CTA forever regardless of success or failure.
            const seeded = seed({
                kind: "ready",
                providerId: "openclaw",
                bundleId: "acct-existing",
            });
            const r = update(seeded, { type: "ConnectClicked" });
            expect(r.state.kind).toBe("waiting");
            expect(r.state.bundleId).toBe("acct-existing");
            expect(r.events[0]).toMatchObject({
                type: "start-requested",
                providerId: "openclaw",
            });
        });
    });

    describe("Seeded (a registered login → ready)", () => {
        it("transitions unauthenticated → ready single-phase", () => {
            const r = update(
                seed({ kind: "unauthenticated", providerId: "claude", bundleId: "b1" }),
                { type: "Seeded", bundleId: "b1" },
            );
            expect(r.state.kind).toBe("ready");
            expect(r.state.bundleId).toBe("b1");
            expect(r.state.sessionId).toBe("");
            expect(r.events[0]).toMatchObject({ type: "seeded", bundleId: "b1" });
        });

        it("empty bundleId falls back to the existing bundle (default identity)", () => {
            const r = update(
                seed({ kind: "expired", providerId: "claude", bundleId: "b2" }),
                { type: "Seeded", bundleId: "" },
            );
            expect(r.state.kind).toBe("ready");
            expect(r.state.bundleId).toBe("b2");
        });

        it("clears a prior error when seeding from `failed`", () => {
            const r = update(seed({ kind: "failed", error: "boom" }), {
                type: "Seeded",
                bundleId: "b3",
            });
            expect(r.state.kind).toBe("ready");
            expect(r.state.error).toBe("");
        });

        it("is honored from `waiting` too — the runProviderLogin-driven connect path (PreLaunchAuthPanel's requiresLoginTty branch) dispatches ConnectClicked up front so the panel shows progress during tier 3's terminal wait, then Seeded on success", () => {
            const r = update(seed({ kind: "waiting", bundleId: "keep", sessionId: "provider-login" }), {
                type: "Seeded",
                bundleId: "new-account",
            });
            expect(r.state.kind).toBe("ready");
            expect(r.state.bundleId).toBe("new-account");
            expect(r.state.sessionId).toBe("");
            expect(r.events[0]).toMatchObject({ type: "seeded", bundleId: "new-account" });
        });

        it("is dropped from non-connect-able kinds (never clobbers ready/idle)", () => {
            for (const kind of [
                "ready",
                "idle",
            ] as const) {
                const r = update(seed({ kind, bundleId: "keep" }), {
                    type: "Seeded",
                    bundleId: "other",
                });
                expect(r.state.kind).toBe(kind);
                expect(r.state.bundleId).toBe("keep");
                expect(r.events[0]).toMatchObject({
                    type: "post-close-command-dropped",
                    commandType: "Seeded",
                });
            }
        });
    });

    describe("Polled", () => {
        it("`pending` is a no-op", () => {
            const seeded = seed({ kind: "waiting", sessionId: "s1" });
            const r = update(seeded, {
                type: "Polled",
                sessionId: "s1",
                status: { status: "pending" },
            });
            expect(r.state).toBe(seeded);
            expect(r.events).toEqual([]);
        });

        it("`url-available` captures the URL idempotently", () => {
            const seeded = seed({ kind: "waiting", sessionId: "s1" });
            const r1 = update(seeded, {
                type: "Polled",
                sessionId: "s1",
                status: { status: "url-available", authUrl: "https://x" },
            });
            expect(r1.state.authUrl).toBe("https://x");
            const r2 = update(r1.state, {
                type: "Polled",
                sessionId: "s1",
                status: { status: "url-available", authUrl: "https://x" },
            });
            expect(r2.events).toEqual([]);
        });

        it("`code-emitted` captures device code idempotently", () => {
            const seeded = seed({ kind: "waiting", sessionId: "s1" });
            const r1 = update(seeded, {
                type: "Polled",
                sessionId: "s1",
                status: {
                    status: "code-emitted",
                    deviceCode: "ABCD-1234",
                    verificationUrl: "https://github.com/login/device",
                },
            });
            expect(r1.state.deviceCode).toEqual({
                code: "ABCD-1234",
                verificationUrl: "https://github.com/login/device",
            });
            const r2 = update(r1.state, {
                type: "Polled",
                sessionId: "s1",
                status: {
                    status: "code-emitted",
                    deviceCode: "ABCD-1234",
                    verificationUrl: "https://github.com/login/device",
                },
            });
            expect(r2.events).toEqual([]);
        });

        it("`success` flips to ready and clears transients", () => {
            const seeded = seed({
                kind: "waiting",
                sessionId: "s1",
                authUrl: "https://x",
                deviceCode: { code: "X", verificationUrl: "Y" },
            });
            const r = update(seeded, {
                type: "Polled",
                sessionId: "s1",
                status: {
                    status: "success",
                    bundleId: "new-bundle",
                    email: "u@x.com",
                },
            });
            expect(r.state.kind).toBe("ready");
            expect(r.state.bundleId).toBe("new-bundle");
            expect(r.state.sessionId).toBe("");
            expect(r.state.authUrl).toBe("");
            expect(r.state.deviceCode).toBeNull();
            expect(r.events[0]).toMatchObject({
                type: "succeeded",
                bundleId: "new-bundle",
                email: "u@x.com",
            });
        });

        it("`failed` flips to failed with error", () => {
            const seeded = seed({ kind: "waiting", sessionId: "s1" });
            const r = update(seeded, {
                type: "Polled",
                sessionId: "s1",
                status: { status: "failed", error: "timeout" },
            });
            expect(r.state.kind).toBe("failed");
            expect(r.state.error).toBe("timeout");
            expect(r.state.sessionId).toBe("");
        });

        // Codex P1 on PR #845: a poll response from a cancelled or
        // superseded session must NOT mutate state.
        it("drops a stale `success` for a different session", () => {
            const seeded = seed({
                kind: "waiting",
                sessionId: "s2",
                providerId: "claude",
                bundleId: "current-bundle",
            });
            const r = update(seeded, {
                type: "Polled",
                sessionId: "s1", // old session
                status: {
                    status: "success",
                    bundleId: "wrong-bundle",
                    email: "u@x.com",
                },
            });
            expect(r.state).toBe(seeded);
            expect(r.events[0]).toMatchObject({
                type: "post-close-command-dropped",
                commandType: "Polled",
            });
        });

        it("drops a stale `failed` after CancelClicked cleared sessionId", () => {
            // User clicks Connect → s1 starts → user clicks Cancel →
            // state.sessionId becomes "" → late `failed` from s1 must
            // not flip state to "failed" with the old error.
            const seeded = seed({
                kind: "unauthenticated",
                sessionId: "",
                providerId: "claude",
            });
            const r = update(seeded, {
                type: "Polled",
                sessionId: "s1",
                status: { status: "failed", error: "old timeout" },
            });
            expect(r.state).toBe(seeded);
            expect(r.state.error).toBe("");
            expect(r.events[0]).toMatchObject({
                type: "post-close-command-dropped",
            });
        });

        it("drops a stale poll after Selected swapped the bundle", () => {
            // Mid-OAuth on session s1 (bundle b1) → user picks bundle
            // b2 → state.sessionId cleared → late `success` from s1
            // must not flip state to ready with bundle from old session.
            const sessionGoingOn = seed({
                kind: "waiting",
                sessionId: "s1",
                providerId: "claude",
                bundleId: "b1",
            });
            const afterSelected = update(sessionGoingOn, {
                type: "Selected",
                providerId: "claude",
                bundleId: "b2",
                outcome: "ready",
            }).state;
            expect(afterSelected.sessionId).toBe(""); // sanity
            const r = update(afterSelected, {
                type: "Polled",
                sessionId: "s1",
                status: {
                    status: "success",
                    bundleId: "wrong",
                    email: null,
                },
            });
            expect(r.state).toBe(afterSelected);
            expect(r.state.bundleId).toBe("b2");
            expect(r.state.kind).toBe("ready");
        });
    });

    describe("CancelClicked", () => {
        it("transitions waiting → unauthenticated and emits cancel-requested", () => {
            const seeded = seed({
                kind: "waiting",
                sessionId: "s1",
                authUrl: "https://x",
            });
            const r = update(seeded, { type: "CancelClicked" });
            expect(r.state.kind).toBe("unauthenticated");
            expect(r.state.sessionId).toBe("");
            expect(r.state.authUrl).toBe("");
            expect(r.events[0]).toMatchObject({
                type: "cancel-requested",
                sessionId: "s1",
            });
        });

        it("is dropped from non-waiting states", () => {
            const seeded = seed({ kind: "ready" });
            const r = update(seeded, { type: "CancelClicked" });
            expect(r.events[0]).toMatchObject({
                type: "post-close-command-dropped",
            });
        });
    });

    describe("CallbackSubmitted", () => {
        it("emits callback-submit-requested when waiting with a session", () => {
            const seeded = seed({ kind: "waiting", sessionId: "s1" });
            const r = update(seeded, {
                type: "CallbackSubmitted",
                callbackUrl: "https://cb?code=x",
            });
            expect(r.state).toBe(seeded); // no state change
            expect(r.events[0]).toMatchObject({
                type: "callback-submit-requested",
                sessionId: "s1",
                callbackUrl: "https://cb?code=x",
            });
        });

        it("is dropped without an active session", () => {
            const seeded = seed({ kind: "waiting", sessionId: "" });
            const r = update(seeded, {
                type: "CallbackSubmitted",
                callbackUrl: "x",
            });
            expect(r.events[0]).toMatchObject({
                type: "post-close-command-dropped",
            });
        });
    });

    // Reagent P2 on #845: SessionStarted lacked test coverage.
    describe("SessionStarted", () => {
        it("records sessionId + auth URL emitted by the inline RPC response", () => {
            const seeded = seed({ kind: "waiting", providerId: "claude" });
            const r = update(seeded, {
                type: "SessionStarted",
                sessionId: "s1",
                authUrl: "https://x",
            });
            expect(r.state.sessionId).toBe("s1");
            expect(r.state.authUrl).toBe("https://x");
            expect(r.state.kind).toBe("waiting");
            expect(r.events[0]).toMatchObject({
                type: "session-started",
                sessionId: "s1",
                authUrl: "https://x",
            });
        });

        it("works without a captured URL (CLI emits it asynchronously via Polled)", () => {
            const seeded = seed({ kind: "waiting", providerId: "claude" });
            const r = update(seeded, { type: "SessionStarted", sessionId: "s1" });
            expect(r.state.sessionId).toBe("s1");
            expect(r.state.authUrl).toBe("");
        });

        // Reagent P1 on #849: if the user cancelled during the
        // auth.start RPC await, kind leaves "waiting" before
        // SessionStarted arrives. The late SessionStarted must NOT
        // resurrect a zombie session (because CancelClicked would
        // then refuse to clear it — sessionId mismatch on the cancel
        // path).
        it("drops if kind has already left `waiting` (cancel-during-start race)", () => {
            for (const kind of ["unauthenticated", "ready", "failed", "expired"] as const) {
                const seeded = seed({ kind, providerId: "claude", sessionId: "" });
                const r = update(seeded, {
                    type: "SessionStarted",
                    sessionId: "s1",
                    authUrl: "https://x",
                });
                expect(r.state).toBe(seeded);
                expect(r.events[0]).toMatchObject({
                    type: "post-close-command-dropped",
                    commandType: "SessionStarted",
                });
            }
        });
    });

    describe("Selected populates intoBundleId from outcome", () => {
        it("expired → intoBundleId = bundleId (re-auth)", () => {
            const r = update(initialState(), {
                type: "Selected",
                providerId: "claude",
                bundleId: "existing-b1",
                outcome: "expired",
            });
            expect(r.state.intoBundleId).toBe("existing-b1");
        });

        it("needs-account → intoBundleId = bundleId (add account)", () => {
            const r = update(initialState(), {
                type: "Selected",
                providerId: "claude",
                bundleId: "existing-b1",
                outcome: "needs-account",
            });
            expect(r.state.intoBundleId).toBe("existing-b1");
        });

        it("ready → intoBundleId = '' (no save flow needed)", () => {
            const r = update(initialState(), {
                type: "Selected",
                providerId: "claude",
                bundleId: "existing-b1",
                outcome: "ready",
            });
            expect(r.state.intoBundleId).toBe("");
        });
    });

    describe("ConnectFailed (reagent P2 on #853)", () => {
        it("flips connect-attempt states to failed with error", () => {
            // Codex P2 on #853 round 7: gated on connect-attempt kinds
            // only. `idle`/`ready` are NOT
            // connect-attempt states — a stale ResolveCli reject
            // landing there would clobber the user's newer selection.
            for (const kind of [
                "unauthenticated",
                "waiting",
                "expired",
                "failed",
            ] as const) {
                const seeded = seed({
                    kind,
                    sessionId: "s1",
                    authUrl: "https://x",
                    deviceCode: { code: "Z", verificationUrl: "V" },
                });
                const r = update(seeded, {
                    type: "ConnectFailed",
                    error: "cli not found",
                });
                expect(r.state.kind).toBe("failed");
                expect(r.state.error).toBe("cli not found");
                expect(r.state.sessionId).toBe("");
                expect(r.state.authUrl).toBe("");
                expect(r.state.deviceCode).toBeNull();
                expect(r.events[0]).toMatchObject({
                    type: "failed",
                    error: "cli not found",
                });
            }
        });

        it("is dropped from non-connect-attempt states", () => {
            // Codex P2 on #853 round 7: a stale ResolveCli reject
            // arriving after the user moved to ready must NOT clobber the
            // newer state.
            for (const kind of ["idle", "ready"] as const) {
                const seeded = seed({ kind, sessionId: "s1" });
                const r = update(seeded, {
                    type: "ConnectFailed",
                    error: "stale cli error",
                });
                expect(r.state).toBe(seeded);
                expect(r.events[0]).toMatchObject({
                    type: "post-close-command-dropped",
                    commandType: "ConnectFailed",
                });
            }
        });

        it("is dropped after Disposed (post-close gate)", () => {
            const closed = update(initialState(), { type: "Disposed" }).state;
            const r = update(closed, { type: "ConnectFailed", error: "x" });
            expect(r.state).toBe(closed);
            expect(r.events[0]).toMatchObject({
                type: "post-close-command-dropped",
                commandType: "ConnectFailed",
            });
        });
    });

    describe("Disposed gate", () => {
        it("Disposed sets closed=true", () => {
            const r = update(initialState(), { type: "Disposed" });
            expect(r.state.closed).toBe(true);
        });

        it("Disposed is idempotent", () => {
            const r1 = update(initialState(), { type: "Disposed" });
            const r2 = update(r1.state, { type: "Disposed" });
            expect(r2.state).toBe(r1.state);
            expect(r2.events).toEqual([]);
        });

        it("commands after Disposed are no-ops emitting post-close-command-dropped", () => {
            const closed = update(initialState(), { type: "Disposed" }).state;
            const r = update(closed, {
                type: "Selected",
                providerId: "x",
                bundleId: "y",
                outcome: "ready",
            });
            expect(r.state).toBe(closed);
            expect(r.events).toEqual([
                {
                    type: "post-close-command-dropped",
                    commandType: "Selected",
                },
            ]);
        });
    });
});
