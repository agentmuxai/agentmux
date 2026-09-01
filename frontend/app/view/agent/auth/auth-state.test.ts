// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { initialState, update, type AuthState } from "./auth-state";

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

        it("is dropped from non-connect-able kinds (never clobbers ready/saving/authenticated/idle)", () => {
            for (const kind of [
                "ready",
                "saving",
                "authenticated",
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

    describe("ApiKey path", () => {
        it("ApiKeySubmitted transitions to waiting + emits request", () => {
            const seeded = seed({
                kind: "unauthenticated",
                providerId: "openclaw",
                bundleId: "",
            });
            const r = update(seeded, {
                type: "ApiKeySubmitted",
                apiKey: "sk-test",
                accountName: "default",
            });
            expect(r.state.kind).toBe("waiting");
            expect(r.events[0]).toMatchObject({
                type: "api-key-submit-requested",
                providerId: "openclaw",
                apiKey: "sk-test",
            });
        });

        it("ApiKeyAccepted transitions to ready with the new bundleId (single-phase)", () => {
            // PR C-1 (revised): api-key stays single-phase until C-2
            // backend lands. Backend persists in submitapikey itself.
            const seeded = seed({ kind: "waiting", providerId: "openclaw" });
            const r = update(seeded, {
                type: "ApiKeyAccepted",
                bundleId: "new-key-bundle",
            });
            expect(r.state.kind).toBe("ready");
            expect(r.state.bundleId).toBe("new-key-bundle");
            expect(r.events[0]).toMatchObject({
                type: "api-key-accepted",
                bundleId: "new-key-bundle",
            });
        });

        // Reagent P2 on #849: ApiKeyAccepted must drop if the user
        // exited `waiting` during the API-key RPC await.
        it("ApiKeyAccepted drops if kind has left `waiting`", () => {
            for (const kind of [
                "unauthenticated",
                "ready",
                "failed",
                "expired",
                "authenticated",
            ] as const) {
                const seeded = seed({ kind, providerId: "openclaw", bundleId: "user-pick" });
                const r = update(seeded, {
                    type: "ApiKeyAccepted",
                    bundleId: "stale-bundle",
                });
                expect(r.state).toBe(seeded);
                expect(r.state.bundleId).toBe("user-pick"); // unchanged
                expect(r.events[0]).toMatchObject({
                    type: "post-close-command-dropped",
                    commandType: "ApiKeyAccepted",
                });
            }
        });

        // Reagent P1 on #845: previously ApiKeySubmitted spread
        // `...state` which preserved an in-flight OAuth sessionId,
        // letting a late OAuth `success` poll match and bind the
        // wrong bundle. The reducer now clears those transients.
        it("ApiKeySubmitted clears any prior OAuth session id + transients", () => {
            const seeded = seed({
                kind: "unauthenticated",
                providerId: "openclaw",
                bundleId: "",
                sessionId: "stale-oauth-s1",
                authUrl: "https://x",
                deviceCode: { code: "X", verificationUrl: "Y" },
            });
            const r = update(seeded, {
                type: "ApiKeySubmitted",
                apiKey: "sk",
                accountName: "x",
            });
            expect(r.state.sessionId).toBe("");
            expect(r.state.authUrl).toBe("");
            expect(r.state.deviceCode).toBeNull();
            expect(r.state.kind).toBe("waiting");
        });

        it("ApiKeySubmitted is dropped from `ready` / `waiting`", () => {
            for (const kind of ["ready", "waiting"] as const) {
                const seeded = seed({ kind, providerId: "openclaw" });
                const r = update(seeded, {
                    type: "ApiKeySubmitted",
                    apiKey: "sk",
                    accountName: "x",
                });
                expect(r.state).toBe(seeded);
                expect(r.events[0]).toMatchObject({
                    type: "post-close-command-dropped",
                    commandType: "ApiKeySubmitted",
                });
            }
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

    // ── PR C-1: 2-phase commit (authenticated → saving → ready) ─────
    describe("2-phase commit (PR C-1)", () => {
        describe("Polled.authenticated", () => {
            it("flips waiting → authenticated, captures email, clears authUrl/deviceCode", () => {
                const seeded = seed({
                    kind: "waiting",
                    sessionId: "s1",
                    authUrl: "https://x",
                    deviceCode: { code: "Z", verificationUrl: "V" },
                });
                const r = update(seeded, {
                    type: "Polled",
                    sessionId: "s1",
                    status: { status: "authenticated", email: "asaf@x.com" },
                });
                expect(r.state.kind).toBe("authenticated");
                expect(r.state.email).toBe("asaf@x.com");
                expect(r.state.sessionId).toBe("s1"); // preserved for savebundle
                expect(r.state.authUrl).toBe("");
                expect(r.state.deviceCode).toBeNull();
                expect(r.events[0]).toMatchObject({
                    type: "authenticated",
                    email: "asaf@x.com",
                });
            });

            it("handles null email (CLI didn't print a logged-in-as line)", () => {
                const seeded = seed({ kind: "waiting", sessionId: "s1" });
                const r = update(seeded, {
                    type: "Polled",
                    sessionId: "s1",
                    status: { status: "authenticated", email: null },
                });
                expect(r.state.kind).toBe("authenticated");
                expect(r.state.email).toBe("");
            });
        });

        describe("SaveBundleClicked", () => {
            it("transitions authenticated → saving and emits save-bundle-requested", () => {
                const seeded = seed({
                    kind: "authenticated",
                    sessionId: "s1",
                    email: "asaf@x.com",
                    intoBundleId: "",
                });
                const r = update(seeded, {
                    type: "SaveBundleClicked",
                    name: "Claude (asaf@x.com)",
                });
                expect(r.state.kind).toBe("saving");
                expect(r.state.error).toBe("");
                expect(r.events[0]).toMatchObject({
                    type: "save-bundle-requested",
                    sessionId: "s1",
                    intoBundleId: "",
                    name: "Claude (asaf@x.com)",
                });
            });

            it("forwards intoBundleId for the re-auth/add-account path", () => {
                const seeded = seed({
                    kind: "authenticated",
                    sessionId: "s1",
                    email: "asaf@x.com",
                    intoBundleId: "existing-b1",
                });
                const r = update(seeded, {
                    type: "SaveBundleClicked",
                    name: "Claude (asaf@x.com)",
                });
                expect(r.events[0]).toMatchObject({
                    type: "save-bundle-requested",
                    intoBundleId: "existing-b1",
                });
            });

            it("drops from non-authenticated states", () => {
                for (const kind of [
                    "idle",
                    "unauthenticated",
                    "waiting",
                    "saving",
                    "ready",
                    "expired",
                    "failed",
                ] as const) {
                    const seeded = seed({ kind, sessionId: "s1" });
                    const r = update(seeded, {
                        type: "SaveBundleClicked",
                        name: "x",
                    });
                    expect(r.state).toBe(seeded);
                    expect(r.events[0]).toMatchObject({
                        type: "post-close-command-dropped",
                        commandType: "SaveBundleClicked",
                    });
                }
            });
        });

        describe("BundleSaved", () => {
            it("transitions saving → ready with the real bundleId", () => {
                const seeded = seed({
                    kind: "saving",
                    sessionId: "s1",
                    email: "asaf@x.com",
                });
                const r = update(seeded, {
                    type: "BundleSaved",
                    bundleId: "bundle-uuid-1",
                });
                expect(r.state.kind).toBe("ready");
                expect(r.state.bundleId).toBe("bundle-uuid-1");
                expect(r.state.sessionId).toBe("");
                expect(r.events[0]).toMatchObject({
                    type: "bundle-saved",
                    bundleId: "bundle-uuid-1",
                });
            });

            it("drops from non-saving states", () => {
                for (const kind of [
                    "authenticated",
                    "waiting",
                    "ready",
                    "failed",
                ] as const) {
                    const seeded = seed({ kind });
                    const r = update(seeded, {
                        type: "BundleSaved",
                        bundleId: "x",
                    });
                    expect(r.state).toBe(seeded);
                    expect(r.events[0]).toMatchObject({
                        type: "post-close-command-dropped",
                        commandType: "BundleSaved",
                    });
                }
            });
        });

        describe("BundleSaveFailed", () => {
            it("returns to authenticated (preserve email + session) with error", () => {
                const seeded = seed({
                    kind: "saving",
                    sessionId: "s1",
                    email: "asaf@x.com",
                });
                const r = update(seeded, {
                    type: "BundleSaveFailed",
                    error: "UNIQUE constraint failed: db_identity_bundles.name",
                });
                expect(r.state.kind).toBe("authenticated");
                expect(r.state.email).toBe("asaf@x.com");
                expect(r.state.sessionId).toBe("s1");
                expect(r.state.error).toContain("UNIQUE");
                expect(r.events[0]).toMatchObject({
                    type: "bundle-save-failed",
                });
            });

            it("drops from non-saving states", () => {
                const seeded = seed({ kind: "authenticated" });
                const r = update(seeded, {
                    type: "BundleSaveFailed",
                    error: "boom",
                });
                expect(r.events[0]).toMatchObject({
                    type: "post-close-command-dropped",
                    commandType: "BundleSaveFailed",
                });
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

        it("CancelClicked also works from authenticated (S21 — user changed mind)", () => {
            const seeded = seed({
                kind: "authenticated",
                sessionId: "s1",
                email: "asaf@x.com",
            });
            const r = update(seeded, { type: "CancelClicked" });
            expect(r.state.kind).toBe("unauthenticated");
            expect(r.state.sessionId).toBe("");
            expect(r.state.email).toBe("");
            expect(r.events[0]).toMatchObject({
                type: "cancel-requested",
                sessionId: "s1",
            });
        });

        it("CancelClicked also works from saving (reagent P1 round 10 — spinner cancellable)", () => {
            // Without honoring `saving`, clicking Cancel during the
            // auth.savebundle RPC would silently drop the dispatch and
            // leave the spinner forever. The controller's cancel() also
            // fires rpc.cancel for the live session.
            const seeded = seed({
                kind: "saving",
                sessionId: "s1",
                email: "asaf@x.com",
            });
            const r = update(seeded, { type: "CancelClicked" });
            expect(r.state.kind).toBe("unauthenticated");
            expect(r.state.sessionId).toBe("");
            expect(r.state.email).toBe("");
            expect(r.events[0]).toMatchObject({
                type: "cancel-requested",
                sessionId: "s1",
            });
        });
    });

    describe("ConnectFailed (reagent P2 on #853)", () => {
        it("flips connect-attempt states to failed with error", () => {
            // Codex P2 on #853 round 7: gated on connect-attempt kinds
            // only. `idle`/`ready`/`authenticated`/`saving` are NOT
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
            // arriving after the user moved to ready/authenticated/
            // saving must NOT clobber the newer state.
            for (const kind of ["idle", "ready", "authenticated", "saving"] as const) {
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
