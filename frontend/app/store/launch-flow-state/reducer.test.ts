// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { update } from "./reducer";
import {
    accountsForProvider,
    accountSuppliesProvider,
    canSubmit,
    continueLocksIdentity,
    continueLocksMemory,
    initialState,
    isContinue,
    realMemories,
    type LaunchFlowCommand,
    type LaunchFlowState,
} from "./types";
import type { Account } from "@/app/view/identity/identity-model";

// Test fixtures. Match the wire shapes in frontend/types/gotypes.d.ts
// and frontend/app/view/identity/identity-model.ts.
const acct = (id: string, name: string, provider = "claude"): Account => ({
    id,
    name,
    provider: provider as Account["provider"],
    kind: "oauth",
    secret_ref: { backend: "env" },
    context: {},
    assigned_agents: [],
    status: "valid",
    created_at: "",
    updated_at: "",
});

const mem = (id: string, name: string, is_blank = false): Memory => ({
    id,
    name,
    is_blank,
    created_at: 0,
    updated_at: 0,
});

const dispatch = (state: LaunchFlowState, cmd: LaunchFlowCommand) => update(state, cmd);

// ── Reducer transitions ────────────────────────────────────────────

describe("launch-flow-state reducer", () => {
    describe("initial state", () => {
        it("starts with empty form + no accounts/memories + not closed", () => {
            const s = initialState();
            expect(s.form.name).toBe("");
            expect(s.form.accountId).toBe("");
            expect(s.form.memoryId).toBe("");
            expect(s.form.continueOfId).toBe(null);
            expect(s.accounts.list).toEqual([]);
            expect(s.memories.list).toEqual([]);
            expect(s.submit.inFlight).toBe(false);
            expect(s.closed).toBe(false);
        });
    });

    describe("Opened", () => {
        it("resets form to defaults when no initial supplied", () => {
            let s = initialState();
            s = dispatch(s, { type: "NameChanged", name: "stale" }).state;
            s = dispatch(s, { type: "Opened" }).state;
            expect(s.form.name).toBe("");
        });

        it("applies initial form overrides", () => {
            const s = dispatch(initialState(), {
                type: "Opened",
                initial: { name: "carry", runtime: "container", accountId: "a" },
            }).state;
            expect(s.form.name).toBe("carry");
            expect(s.form.runtime).toBe("container");
            expect(s.form.accountId).toBe("a");
        });

        it("clears the closed flag (reopen)", () => {
            let s = dispatch(initialState(), { type: "Closed" }).state;
            expect(s.closed).toBe(true);
            s = dispatch(s, { type: "Opened" }).state;
            expect(s.closed).toBe(false);
        });

        it("emits no events (accounts load once, no per-selection fetch)", () => {
            const r = update(initialState(), {
                type: "Opened",
                initial: { accountId: "preselect-a" },
            });
            expect(r.events).toEqual([]);
        });
    });

    describe("form fields", () => {
        it("NameChanged updates name", () => {
            const s = dispatch(initialState(), { type: "NameChanged", name: "n" }).state;
            expect(s.form.name).toBe("n");
        });

        it("set-to-current is a no-op (same state identity)", () => {
            const s0 = dispatch(initialState(), { type: "NameChanged", name: "x" }).state;
            const r = update(s0, { type: "NameChanged", name: "x" });
            expect(r.state).toBe(s0);
        });

        it("RuntimeChanged + ImageChanged update independently", () => {
            let s = dispatch(initialState(), { type: "RuntimeChanged", runtime: "container" }).state;
            s = dispatch(s, { type: "ImageChanged", image: "alpine" }).state;
            expect(s.form.runtime).toBe("container");
            expect(s.form.image).toBe("alpine");
        });
    });

    describe("AccountChanged", () => {
        it("updates accountId", () => {
            const s = dispatch(initialState(), { type: "AccountChanged", accountId: "a" }).state;
            expect(s.form.accountId).toBe("a");
        });

        it("emits no events — accounts are already loaded client-side", () => {
            const r = update(initialState(), { type: "AccountChanged", accountId: "a" });
            expect(r.events).toEqual([]);
        });

        it("set-to-current is a no-op (same state identity)", () => {
            const s0 = dispatch(initialState(), { type: "AccountChanged", accountId: "a" }).state;
            const r = update(s0, { type: "AccountChanged", accountId: "a" });
            expect(r.state).toBe(s0);
        });
    });

    describe("ContinueOfChanged", () => {
        it("locks identity + memory when carry-over is real", () => {
            const s = dispatch(initialState(), {
                type: "ContinueOfChanged",
                continueOfId: "inst-1",
                carry: { name: "prev", accountId: "a", memoryId: "m" },
            }).state;
            expect(s.form.continueOfId).toBe("inst-1");
            expect(s.form.name).toBe("prev");
            expect(s.form.accountId).toBe("a");
            expect(s.form.memoryId).toBe("m");
            expect(continueLocksIdentity(s)).toBe(true);
            expect(continueLocksMemory(s)).toBe(true);
        });

        it("does NOT lock when continued row carries empty values (legacy)", () => {
            const s = dispatch(initialState(), {
                type: "ContinueOfChanged",
                continueOfId: "inst-legacy",
                carry: { name: "old", accountId: "", memoryId: "" },
            }).state;
            expect(isContinue(s)).toBe(true);
            // Legacy continuation — user must pick replacements
            expect(continueLocksIdentity(s)).toBe(false);
            expect(continueLocksMemory(s)).toBe(false);
        });

        it("re-dispatch with same continueOfId is a no-op (preserves edits)", () => {
            let s = dispatch(initialState(), {
                type: "ContinueOfChanged",
                continueOfId: "inst-1",
                carry: { name: "prev", accountId: "a", memoryId: "m" },
            }).state;
            // User edits name mid-flow
            s = dispatch(s, { type: "NameChanged", name: "edited" }).state;
            // Stray repeat from the view shouldn't overwrite the edit
            const r = update(s, {
                type: "ContinueOfChanged",
                continueOfId: "inst-1",
                carry: { name: "prev", accountId: "a", memoryId: "m" },
            });
            expect(r.state).toBe(s);
            expect(r.state.form.name).toBe("edited");
        });

        it("clears form on `— New agent —` (null)", () => {
            let s = dispatch(initialState(), {
                type: "ContinueOfChanged",
                continueOfId: "inst-1",
                carry: { name: "prev", accountId: "a", memoryId: "m" },
            }).state;
            s = dispatch(s, { type: "ContinueOfChanged", continueOfId: null }).state;
            expect(s.form.continueOfId).toBe(null);
            expect(s.form.name).toBe("");
            expect(s.form.accountId).toBe("");
            expect(s.form.memoryId).toBe("");
            expect(isContinue(s)).toBe(false);
        });
    });

    describe("accounts resource", () => {
        it("AccountsLoading sets loading + clears error", () => {
            let s = dispatch(initialState(), { type: "AccountsFailed", error: "old" }).state;
            s = dispatch(s, { type: "AccountsLoading" }).state;
            expect(s.accounts.loading).toBe(true);
            expect(s.accounts.error).toBe(null);
        });

        it("AccountsLoaded sets list + clears loading", () => {
            const a = acct("a", "A");
            const s = dispatch(initialState(), { type: "AccountsLoaded", list: [a] }).state;
            expect(s.accounts.list).toEqual([a]);
            expect(s.accounts.loading).toBe(false);
        });

        it("AccountsFailed sets error + clears loading", () => {
            let s = dispatch(initialState(), { type: "AccountsLoading" }).state;
            s = dispatch(s, { type: "AccountsFailed", error: "boom" }).state;
            expect(s.accounts.error).toBe("boom");
            expect(s.accounts.loading).toBe(false);
        });

        it("accountsForProvider selector filters by provider", () => {
            const a = acct("a", "A", "claude");
            const b = acct("b", "B", "codex");
            const s = dispatch(initialState(), { type: "AccountsLoaded", list: [a, b] }).state;
            expect(accountsForProvider(s, "claude")).toEqual([a]);
            expect(accountsForProvider(s, "codex")).toEqual([b]);
        });

        it("realMemories selector filters is_blank", () => {
            const m = mem("m", "M");
            const b = mem("blank", "Blank", true);
            const s = dispatch(initialState(), { type: "MemoriesLoaded", list: [b, m] }).state;
            expect(realMemories(s)).toEqual([m]);
        });
    });

    describe("accountSuppliesProvider selector", () => {
        it("true when the selected account matches the provider", () => {
            const a = acct("a", "A", "claude");
            let s = dispatch(initialState(), { type: "AccountsLoaded", list: [a] }).state;
            s = dispatch(s, { type: "AccountChanged", accountId: "a" }).state;
            expect(accountSuppliesProvider(s, "claude")).toBe(true);
            expect(accountSuppliesProvider(s, "codex")).toBe(false);
        });

        it("false when no account is selected", () => {
            const s = initialState();
            expect(accountSuppliesProvider(s, "claude")).toBe(false);
        });

        it("false when the selected id isn't in the loaded list", () => {
            const s = dispatch(initialState(), { type: "AccountChanged", accountId: "ghost" }).state;
            expect(accountSuppliesProvider(s, "claude")).toBe(false);
        });
    });

    describe("submit lifecycle", () => {
        it("SubmitClicked sets inFlight + clears prior error", () => {
            let s = dispatch(initialState(), { type: "SubmitFailed", error: "x" }).state;
            s = dispatch(s, { type: "SubmitClicked" }).state;
            expect(s.submit.inFlight).toBe(true);
            expect(s.submit.error).toBe(null);
        });

        it("second SubmitClicked while in-flight is a no-op", () => {
            const s1 = dispatch(initialState(), { type: "SubmitClicked" }).state;
            const r = update(s1, { type: "SubmitClicked" });
            expect(r.state).toBe(s1);
        });

        it("SubmitSucceeded clears in-flight", () => {
            let s = dispatch(initialState(), { type: "SubmitClicked" }).state;
            s = dispatch(s, { type: "SubmitSucceeded" }).state;
            expect(s.submit.inFlight).toBe(false);
        });

        it("SubmitFailed records the error", () => {
            let s = dispatch(initialState(), { type: "SubmitClicked" }).state;
            s = dispatch(s, { type: "SubmitFailed", error: "boom" }).state;
            expect(s.submit.error).toBe("boom");
            expect(s.submit.inFlight).toBe(false);
        });
    });

    describe("Closed terminal", () => {
        it("subsequent non-Closed commands are no-ops", () => {
            const closed = dispatch(initialState(), { type: "Closed" }).state;
            const r = update(closed, { type: "NameChanged", name: "x" });
            expect(r.state).toBe(closed);
            expect(r.events).toEqual([]);
        });

        it("Opened re-arms the slice", () => {
            let s = dispatch(initialState(), { type: "Closed" }).state;
            s = dispatch(s, { type: "Opened", initial: { name: "fresh" } }).state;
            expect(s.closed).toBe(false);
            expect(s.form.name).toBe("fresh");
        });
    });

    describe("Auth state (folded-in)", () => {
        it("initial auth.kind is idle", () => {
            const s = initialState();
            expect(s.auth.kind).toBe("idle");
        });

        it("Auth Selected drives inner reducer + updates outer state", () => {
            const r = update(initialState(), {
                type: "Auth",
                cmd: { type: "Selected", providerId: "claude", bundleId: "", outcome: "needs-account" },
            });
            // SelectionKind for "needs-account" → "unauthenticated"
            expect(r.state.auth.kind).toBe("unauthenticated");
            expect(r.state.auth.providerId).toBe("claude");
        });

        it("Auth no-op (same inner state) returns the same outer state", () => {
            const s0 = update(initialState(), {
                type: "Auth",
                cmd: { type: "Selected", providerId: "claude", bundleId: "", outcome: "needs-account" },
            }).state;
            // Re-dispatching the same selection is the inner reducer's
            // own no-op surface. We just verify the outer wrapper
            // doesn't fabricate a state-change either.
            const r = update(s0, {
                type: "Auth",
                cmd: { type: "Selected", providerId: "claude", bundleId: "", outcome: "needs-account" },
            });
            expect(r.state.auth).toBe(s0.auth);
        });

        it("inner events are wrapped + surfaced via outer ReducerResult.events", () => {
            // Drive the inner machine into a state where it emits an
            // event. `Connect` from unauthenticated kicks off the
            // OAuth start RPC via `StartAuth` event.
            let s = update(initialState(), {
                type: "Auth",
                cmd: { type: "Selected", providerId: "claude", bundleId: "", outcome: "needs-account" },
            }).state;
            const r = update(s, { type: "Auth", cmd: { type: "ConnectClicked" } });
            // Outer events should contain at least one wrapped Auth
            // event. Spot-check it's tagged correctly.
            expect(r.events.length).toBeGreaterThan(0);
            for (const e of r.events) {
                expect(e.type).toBe("Auth");
            }
        });
    });

    describe("Cross-product: auth × form (the §6.9 regression suite)", () => {
        // Build an "auth ready" state — the bug we shipped Stage 1
        // to fix is "memory change after auth-ready resets the
        // auth back to Connect". With auth folded into the slice,
        // pure form-field commands must NEVER touch state.auth.
        const setupReady = (): LaunchFlowState => {
            let s = update(initialState(), {
                type: "Auth",
                cmd: {
                    type: "Selected",
                    providerId: "claude",
                    bundleId: "acct-work",
                    outcome: "ready",
                },
            }).state;
            return s;
        };

        it("auth.kind survives NameChanged", () => {
            let s = setupReady();
            const before = s.auth;
            s = update(s, { type: "NameChanged", name: "alpha" }).state;
            expect(s.auth).toBe(before); // same object identity
            expect(s.auth.kind).toBe("ready");
        });

        it("auth.kind survives MemoryChanged — THE original repro", () => {
            let s = setupReady();
            const before = s.auth;
            s = update(s, { type: "MemoryChanged", memoryId: "mem-notes" }).state;
            expect(s.auth).toBe(before);
            expect(s.auth.kind).toBe("ready");
        });

        it("auth.kind survives RuntimeChanged + ImageChanged", () => {
            let s = setupReady();
            const before = s.auth;
            s = update(s, { type: "RuntimeChanged", runtime: "container" }).state;
            s = update(s, { type: "ImageChanged", image: "alpine:latest" }).state;
            expect(s.auth).toBe(before);
            expect(s.auth.kind).toBe("ready");
        });

        it("auth.kind survives AccountsLoaded + MemoriesLoaded", () => {
            let s = setupReady();
            const before = s.auth;
            s = update(s, {
                type: "AccountsLoaded",
                list: [acct("acct-work", "Work")],
            }).state;
            s = update(s, {
                type: "MemoriesLoaded",
                list: [mem("mem-notes", "Notes")],
            }).state;
            expect(s.auth).toBe(before);
            expect(s.auth.kind).toBe("ready");
        });

        it("auth.kind survives SubmitClicked + SubmitFailed", () => {
            let s = setupReady();
            const before = s.auth;
            s = update(s, { type: "SubmitClicked" }).state;
            s = update(s, { type: "SubmitFailed", error: "no creds on disk" }).state;
            expect(s.auth).toBe(before);
            expect(s.auth.kind).toBe("ready");
        });

        it("form state survives Auth Connect (auth path doesn't touch form)", () => {
            let s = update(initialState(), { type: "NameChanged", name: "carry-name" }).state;
            s = update(s, { type: "AccountChanged", accountId: "acct-a" }).state;
            s = update(s, { type: "MemoryChanged", memoryId: "mem-a" }).state;
            const formBefore = s.form;
            // Drive auth machine
            s = update(s, {
                type: "Auth",
                cmd: { type: "Selected", providerId: "claude", bundleId: "", outcome: "needs-account" },
            }).state;
            s = update(s, { type: "Auth", cmd: { type: "ConnectClicked" } }).state;
            expect(s.form).toBe(formBefore);
            expect(s.form.name).toBe("carry-name");
            expect(s.form.accountId).toBe("acct-a");
            expect(s.form.memoryId).toBe("mem-a");
        });
    });

    describe("canSubmit cross-product", () => {
        const baseAuth = { authReady: true, nameValid: true };

        it("blocks when in-flight", () => {
            const s = dispatch(initialState(), { type: "SubmitClicked" }).state;
            expect(canSubmit(s, baseAuth)).toBe(false);
        });

        it("blocks when name invalid", () => {
            const s = initialState();
            expect(canSubmit(s, { ...baseAuth, nameValid: false })).toBe(false);
        });

        it("blocks when accountId empty", () => {
            const s = initialState();
            expect(canSubmit(s, baseAuth)).toBe(false);
        });

        it("blocks when memoryId empty", () => {
            let s = dispatch(initialState(), { type: "AccountChanged", accountId: "a" }).state;
            expect(canSubmit(s, baseAuth)).toBe(false);
        });

        it("blocks when authReady false", () => {
            let s = dispatch(initialState(), { type: "AccountChanged", accountId: "a" }).state;
            s = dispatch(s, { type: "MemoryChanged", memoryId: "m" }).state;
            expect(canSubmit(s, { ...baseAuth, authReady: false })).toBe(false);
        });

        it("passes when name + account + memory + auth are all good", () => {
            let s = dispatch(initialState(), { type: "AccountChanged", accountId: "a" }).state;
            s = dispatch(s, { type: "MemoryChanged", memoryId: "m" }).state;
            expect(canSubmit(s, baseAuth)).toBe(true);
        });
    });
});
