// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import { createEffect, createRoot } from "solid-js";

import { __resetDispatchLog, getRecentDispatches } from "../command-source";
import { createLaunchPhaseStore } from "./launch-phase-store";

afterEach(() => __resetDispatchLog());

describe("createLaunchPhaseStore", () => {
    it("projects reducer state into per-field accessors", () => {
        createRoot((dispose) => {
            const store = createLaunchPhaseStore("block-1", () => 0);
            store.dispatch({ type: "LoginRequested", via: "relogin", retryAfterLogin: false });
            expect(store.loginWaiting()).toBe(true);
            expect(store.phase()).toEqual({ kind: "checking-auth" });
            expect(store.snapshot().login?.id).toBe(1);
            store.dispatch({ type: "LoginEnded", id: 1 });
            expect(store.loginWaiting()).toBe(false);
            expect(store.canRetry()).toBe(true);
            dispose();
        });
    });

    it("returns the command's events", () => {
        createRoot((dispose) => {
            const store = createLaunchPhaseStore("block-1");
            expect(store.dispatch({ type: "MountStarted" })).toEqual([{ type: "mount-started" }]);
            expect(store.dispatch({ type: "MountStarted" })).toEqual([{ type: "mount-start-ignored", reason: "already-running" }]);
            dispose();
        });
    });

    it("a consumer re-runs only when its own field changes", () => {
        createRoot((dispose) => {
            const store = createLaunchPhaseStore("block-1");
            let runs = 0;
            createEffect(() => {
                store.canRetry();
                runs += 1;
            });
            const before = runs;
            store.dispatch({ type: "AuthUrlSet", url: "https://u" });
            store.dispatch({ type: "AuthNoticeSet", notice: "n" });
            store.dispatch({ type: "RecoveryBegan" });
            expect(runs).toBe(before);
            dispose();
        });
    });

    it("records every dispatch in the audit ring, keyed by pane", () => {
        createRoot((dispose) => {
            const store = createLaunchPhaseStore("block-7");
            store.dispatch({ type: "CancelRequested" }, "user");
            const [rec] = getRecentDispatches(1);
            expect(rec).toMatchObject({
                slice: "launch-phase-state",
                key: "block-7",
                command: { type: "CancelRequested" },
                events: [{ type: "login-cancelled" }],
                source: "user",
            });
            dispose();
        });
    });
});
