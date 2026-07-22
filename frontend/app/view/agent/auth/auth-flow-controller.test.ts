// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it } from "vitest";

import { AuthFlowController, type AuthRpc } from "./auth-flow-controller";
import type { AuthSessionStatusWire } from "./auth-state";

function flush(): Promise<void> {
    return new Promise((r) => setTimeout(r, 0));
}

interface FakeTimers {
    setTimeout: (fn: () => void, ms: number) => unknown;
    clearTimeout: (h: unknown) => void;
    pending: Map<unknown, () => void>;
    tick: () => Promise<void>;
}

function fakeTimers(): FakeTimers {
    const pending = new Map<unknown, () => void>();
    let next = 0;
    return {
        setTimeout(fn) {
            const h = ++next;
            pending.set(h, fn);
            return h;
        },
        clearTimeout(h) {
            pending.delete(h);
        },
        pending,
        async tick() {
            const fns = [...pending.values()];
            pending.clear();
            for (const fn of fns) fn();
            await flush();
        },
    };
}

function fakeRpc(overrides: Partial<AuthRpc> = {}): AuthRpc {
    return {
        start: async () => ({ sessionId: "s1" }),
        poll: async () => ({ status: "pending" }) as AuthSessionStatusWire,
        submitCallback: async () => {},
        cancel: async () => {},
        submitApiKey: async () => ({ bundleId: "new-bundle" }),
        ...overrides,
    };
}

describe("AuthFlowController", () => {
    it("selected() transitions through reducer + clears any prior poll", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            const ctrl = new AuthFlowController({ rpc: fakeRpc(), timers });
            ctrl.selected("claude", "b1", "needs-account");
            expect(ctrl.state().kind).toBe("unauthenticated");
            expect(ctrl.state().providerId).toBe("claude");
            dispose();
        });
    });

    it("connect() starts a session and schedules a poll", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            let pollCount = 0;
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => ({ sessionId: "s1", authUrl: "https://x" }),
                    poll: async () => {
                        pollCount += 1;
                        return { status: "pending" };
                    },
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: ["login"],
                authCheckArgs: ["whoami"],
            });
            expect(ctrl.state().kind).toBe("waiting");
            expect(ctrl.state().sessionId).toBe("s1");
            expect(ctrl.state().authUrl).toBe("https://x");
            expect(timers.pending.size).toBe(1);
            await timers.tick();
            expect(pollCount).toBe(1);
            expect(timers.pending.size).toBe(1); // re-scheduled
            dispose();
        });
    });

    it("poll loop stops on terminal `success`", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            let pollCount = 0;
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => ({ sessionId: "s1" }),
                    poll: async () => {
                        pollCount += 1;
                        return {
                            status: "success",
                            bundleId: "new-bundle",
                            email: "u@x.com",
                        };
                    },
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            await timers.tick();
            expect(pollCount).toBe(1);
            expect(ctrl.state().kind).toBe("ready");
            expect(ctrl.state().bundleId).toBe("new-bundle");
            expect(timers.pending.size).toBe(0); // no re-schedule on terminal
            dispose();
        });
    });

    it("cancel() stops polling + fires backend cancel", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            let cancelCalls = 0;
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => ({ sessionId: "s1" }),
                    cancel: async () => {
                        cancelCalls += 1;
                    },
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            expect(timers.pending.size).toBe(1);
            await ctrl.cancel();
            expect(ctrl.state().kind).toBe("unauthenticated");
            expect(timers.pending.size).toBe(0);
            expect(cancelCalls).toBe(1);
            dispose();
        });
    });

    it("wasCancelled() is true only after an explicit cancel() — not for any other exit from `waiting` (reagent P2 on #2262)", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => ({ sessionId: "s1" }),
                    cancel: async () => {},
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            expect(ctrl.wasCancelled()).toBe(false);

            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            expect(ctrl.state().kind).toBe("waiting");
            expect(ctrl.wasCancelled()).toBe(false);

            // Leaving `waiting` for a reason OTHER than a user cancel (e.g.
            // switching provider mid-flow) must NOT read as cancelled.
            ctrl.selected("openai", "", "needs-account");
            expect(ctrl.wasCancelled()).toBe(false);

            // A fresh connect + a real cancel() DOES flip it.
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            await ctrl.cancel();
            expect(ctrl.wasCancelled()).toBe(true);

            dispose();
        });
    });

    it("selected() during waiting cancels the backend session", async () => {
        // Reagent + Codex P2 on #850 round 6: switching selection
        // mid-OAuth must fire auth.cancel for the live sessionId so
        // the backend CLI subprocess doesn't run until timeout.
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            const cancelled: string[] = [];
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => ({ sessionId: "s1", authUrl: "https://x" }),
                    cancel: async (sessionId) => {
                        cancelled.push(sessionId);
                    },
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            expect(ctrl.state().kind).toBe("waiting");
            expect(ctrl.state().sessionId).toBe("s1");
            // User switches provider while OAuth is in-flight.
            ctrl.selected("openai", "", "needs-account");
            // Microtask flush so the fire-and-forget cancel resolves.
            await Promise.resolve();
            await Promise.resolve();
            expect(cancelled).toEqual(["s1"]);
            expect(ctrl.state().kind).toBe("unauthenticated");
            expect(ctrl.state().providerId).toBe("openai");
            dispose();
        });
    });

    it("submitApiKey() is dropped from non-entry states", async () => {
        // Codex P2 on #850 round 6: controller must mirror the
        // reducer's ApiKeySubmitted gate so double-clicks and stale
        // invocations don't double-submit.
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            let submitCalls = 0;
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => ({ sessionId: "s1", authUrl: "https://x" }),
                    submitApiKey: async () => {
                        submitCalls += 1;
                        return { bundleId: "b1" };
                    },
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            // Kind is now "waiting" — submitApiKey should be a no-op.
            await ctrl.submitApiKey("sk-x", "acc");
            expect(submitCalls).toBe(0);
            expect(ctrl.state().kind).toBe("waiting");
            dispose();
        });
    });

    it("cancel() during startup window drops in-flight auth.start", async () => {
        // Reagent P1 on #850: when the user clicks Cancel after
        // ConnectClicked dispatched but before SessionStarted (auth.start
        // still in flight, sessionId === ""), cancel must bump
        // actionToken so the pending start's stale-token gate fires
        // and SessionStarted is never dispatched.
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            let startCancelCalls = 0;
            let resolveStart: ((v: { sessionId: string }) => void) | null = null;
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: () =>
                        new Promise<{ sessionId: string }>((resolve) => {
                            resolveStart = resolve;
                        }),
                    cancel: async () => {
                        startCancelCalls += 1;
                    },
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            const connectP = ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            // We are now in the startup window: kind="waiting",
            // sessionId="". User cancels.
            expect(ctrl.state().kind).toBe("waiting");
            expect(ctrl.state().sessionId).toBe("");
            await ctrl.cancel();
            expect(ctrl.state().kind).toBe("unauthenticated");
            // Now resolve auth.start. The connect()'s stale-token gate
            // must fire, cancel the orphan session, and bail before
            // dispatching SessionStarted.
            resolveStart!({ sessionId: "orphan-s1" });
            await connectP;
            expect(ctrl.state().kind).toBe("unauthenticated");
            expect(startCancelCalls).toBe(1);
            dispose();
        });
    });

    it("submitCallback() invokes backend with active sessionId", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            let cbCall: { sessionId: string; url: string } | null = null;
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => ({ sessionId: "s1", authUrl: "https://x" }),
                    submitCallback: async (sessionId, url) => {
                        cbCall = { sessionId, url };
                    },
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            await ctrl.submitCallback("https://cb?code=abc");
            expect(cbCall).toEqual({
                sessionId: "s1",
                url: "https://cb?code=abc",
            });
            dispose();
        });
    });

    it("submitApiKey() transitions to ready with the persisted bundleId (single-phase)", async () => {
        // API-key flow stays single-phase until backend C-2's
        // auth.savebundle lands. Backend persists the bundle inside
        // auth.submitapikey itself; controller dispatches the real
        // bundleId straight through to `ready`.
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    submitApiKey: async () => ({ bundleId: "persisted-bundle" }),
                }),
                timers,
            });
            ctrl.selected("openclaw", "", "needs-account");
            await ctrl.submitApiKey("sk-test", "my-key-account");
            expect(ctrl.state().kind).toBe("ready");
            expect(ctrl.state().bundleId).toBe("persisted-bundle");
            dispose();
        });
    });

    it("submitApiKey() transitions to failed on rejection", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    submitApiKey: async () => {
                        throw new Error("invalid key");
                    },
                }),
                timers,
            });
            ctrl.selected("openclaw", "", "needs-account");
            await ctrl.submitApiKey("sk-bad", "default");
            expect(ctrl.state().kind).toBe("failed");
            expect(ctrl.state().error).toBe("invalid key");
            dispose();
        });
    });

    it("connect() surfaces auth.start failure as `failed` kind", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => {
                        throw new Error("CLI missing");
                    },
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            expect(ctrl.state().kind).toBe("failed");
            expect(ctrl.state().error).toBe("CLI missing");
            dispose();
        });
    });

    it("selected() during waiting cancels the poll loop", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => ({ sessionId: "s1" }),
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            expect(timers.pending.size).toBe(1);
            ctrl.selected("claude", "b2", "ready"); // pick a different bundle
            expect(timers.pending.size).toBe(0);
            expect(ctrl.state().kind).toBe("ready");
            dispose();
        });
    });

    it("late poll for a stale session is dropped by the reducer gate", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            let resolvePoll!: (s: AuthSessionStatusWire) => void;
            const pollPromise = new Promise<AuthSessionStatusWire>((r) => {
                resolvePoll = r;
            });
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => ({ sessionId: "s1" }),
                    poll: () => pollPromise,
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            await timers.tick(); // fires pollOnce; awaits pollPromise
            // User picks a different bundle while poll is mid-flight
            ctrl.selected("claude", "b2", "ready");
            // Now resolve the stale poll
            resolvePoll({
                status: "success",
                bundleId: "wrong",
                email: null,
            });
            await flush();
            // State should still be "ready" with bundle b2, NOT "wrong"
            expect(ctrl.state().kind).toBe("ready");
            expect(ctrl.state().bundleId).toBe("b2");
            dispose();
        });
    });

    it("dispose() clears the poll loop and locks the slot", async () => {
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    start: async () => ({ sessionId: "s1" }),
                }),
                timers,
            });
            ctrl.selected("claude", "", "needs-account");
            await ctrl.connect({
                cliPath: "/x",
                authLoginArgs: [],
                authCheckArgs: [],
            });
            ctrl.dispose();
            expect(timers.pending.size).toBe(0);
            expect(ctrl.state().closed).toBe(true);
            dispose();
        });
    });
});
