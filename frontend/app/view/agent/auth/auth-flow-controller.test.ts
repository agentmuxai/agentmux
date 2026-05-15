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
            ctrl.selected("claude", "b1", "needs-bundle");
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
            ctrl.selected("claude", "", "needs-bundle");
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
            ctrl.selected("claude", "", "needs-bundle");
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
            ctrl.selected("claude", "", "needs-bundle");
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
            ctrl.selected("claude", "", "needs-bundle");
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

    it("submitApiKey() transitions to authenticated (2-phase, PR C-1)", async () => {
        // PR C-1 reducer extension: api-key path now mirrors OAuth —
        // backend validates the key but the controller dispatches
        // ApiKeyAccepted which transitions to `authenticated`. User
        // names + saves the bundle in the SaveBundle panel.
        // Backend bundle persistence moves to PR C-2's auth.savebundle.
        await createRoot(async (dispose) => {
            const timers = fakeTimers();
            const ctrl = new AuthFlowController({
                rpc: fakeRpc({
                    submitApiKey: async () => ({ bundleId: "ignored-until-c2" }),
                }),
                timers,
            });
            ctrl.selected("openclaw", "", "needs-bundle");
            await ctrl.submitApiKey("sk-test", "my-key-account");
            expect(ctrl.state().kind).toBe("authenticated");
            // Until C-2, controller passes accountName as the email
            // placeholder so the SaveBundle prefill has something to
            // show. C-2 will replace this with backend-surfaced email.
            expect(ctrl.state().email).toBe("my-key-account");
            // bundleId stays empty — only set when BundleSaved fires
            // after auth.savebundle commits.
            expect(ctrl.state().bundleId).toBe("");
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
            ctrl.selected("openclaw", "", "needs-bundle");
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
            ctrl.selected("claude", "", "needs-bundle");
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
            ctrl.selected("claude", "", "needs-bundle");
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
            ctrl.selected("claude", "", "needs-bundle");
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
            ctrl.selected("claude", "", "needs-bundle");
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
