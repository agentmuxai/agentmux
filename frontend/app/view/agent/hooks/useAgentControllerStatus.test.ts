// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Recovery-flow tests for `useAgentControllerStatus` — principally the
 * `loginWaiting()` signal that `useAgentCommands.ts`'s fast-fail guard
 * (`canRetry() || loginWaiting()`) reads, so a message typed mid-recovery
 * can't reach `AgentInputCommand` on a credential the pane already distrusts.
 *
 * Historical origin (reagent P1 on PR #2338): a third recovery entry point,
 * `useGlobalLogin()`, never set `loginWaiting` while its async credential-seed
 * work was in flight, leaving that guard no signal to check. That function was
 * removed outright 2026-08-31 (per-channel auth enforcement — see
 * docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md), so the
 * remaining flows here are `relogin()` and `loginViaTerminal()`.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { createRoot } from "solid-js";

const hub = vi.hoisted(() => ({
    runProviderLogin: vi.fn(),
    // undefined by default (matches every existing test's assumption that
    // this agent has no definition id) — set per-test to exercise
    // existingAccountIdFor's ListAgentIdentitiesCommand lookup.
    agentDefinitionId: undefined as string | undefined,
}));

vi.mock("@/app/store/global", () => ({
    getApi: () => ({
        cancelCliLogin: () => Promise.resolve(),
        ensureAuthDir: () => Promise.resolve("/tmp/auth-dir"),
    }),
    getBlockMetaKeyAtom: (_blockId: string, key: string) => () => {
        if (key === "cmd") return "claude-cli";
        if (key === "agentId") return hub.agentDefinitionId;
        return undefined;
    },
    staticTabId: () => "tab-1",
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ControllerResyncCommand: vi.fn().mockResolvedValue(undefined),
        LinkAgentIdentityCommand: vi.fn().mockResolvedValue(undefined),
        ListAgentIdentitiesCommand: vi.fn().mockResolvedValue([]),
        SetMetaCommand: vi.fn().mockResolvedValue(undefined),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/services", () => ({
    BlockService: { GetControllerStatus: vi.fn().mockResolvedValue(undefined) },
}));
vi.mock("@/app/store/wos", () => ({ makeORef: () => ({}) }));
vi.mock("../flows/launch-flow", () => ({ runLaunchFlow: vi.fn() }));
vi.mock("../flows/run-provider-login", () => ({
    persistAndLinkAccount: vi.fn(),
    runProviderLogin: (...args: unknown[]) => hub.runProviderLogin(...args),
}));

import { useAgentControllerStatus } from "./useAgentControllerStatus";
import { RpcApi } from "@/app/store/rpc-api";

const claude = { id: "claude" } as any; // no authConfigDirEnvVar — skips the link-env sub-path

afterEach(() => {
    vi.clearAllMocks();
    vi.useRealTimers();
    hub.agentDefinitionId = undefined;
});

// REMOVED 2026-08-31 — three describes that exercised `useGlobalLogin()`:
//   - "useGlobalLogin sets loginWaiting while in flight" (reagent P1, PR #2338)
//   - "loginWaiting clears BEFORE onRecovered fires" (reagent P0, PR #2338)
//   - "overlapping recovery flows share one counter" (codex P2, PR #2338 r4)
//
// `useGlobalLogin()` itself is gone: it seeded this agent from the operator's
// personal ~/.claude, defeating per-channel isolation (see
// docs/analysis/ANALYSIS_PER_CHANNEL_AUTH_BYPASSES_2026_08_31.md #3). These
// tests could not be retargeted verbatim — the third one's whole premise was
// TWO independent in-flight guards (seedInFlight vs. reloginInFlight), and only
// reloginInFlight now remains.
//
// The invariants they protected are NOT left uncovered. `loginWaiting()`'s
// shared-counter semantics are still exercised below by the relogin /
// useTerminalInstead / in-app-session suites — in particular "reagent P1 on PR
// #2413 (round 3)", which covers the surviving cross-flow overlap case
// (a /login-driven session never touches reloginInFlight, only the shared
// counter). Anyone reintroducing a second concurrent recovery flow must
// restore an equivalent of the third test with it.

describe("useAgentControllerStatus — existingAccountIdFor canonicalizes provider IDs (codex P1 on PR #2377, second round)", () => {
    it("relogin() passes the alias-linked account_id as existingAccountId, not undefined", async () => {
        // Before this fix, existingAccountIdFor's strict `l.provider ===
        // providerId` comparison missed a link stored under a legacy alias
        // ("claude-code"), so runProviderLogin would mint a brand-new
        // canonical account instead of reusing/refreshing the existing one —
        // leaving both rows present, and since spawn injection processes the
        // canonical row first and the alias row last, the stale alias
        // directory would silently overwrite the freshly-authenticated one.
        hub.agentDefinitionId = "def-1";
        vi.mocked(RpcApi.ListAgentIdentitiesCommand).mockResolvedValue([
            { provider: "claude-code", account_id: "acct-under-alias" },
        ] as any);
        hub.runProviderLogin.mockResolvedValue("terminal-unavailable");

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });

            await status.relogin({ retryAfterLogin: false });

            expect(hub.runProviderLogin).toHaveBeenCalledWith(
                expect.objectContaining({ existingAccountId: "acct-under-alias" }),
            );
            dispose();
        });
    });
});

describe("useAgentControllerStatus — in-app login session (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.3 surface 2)", () => {
    it("relogin() requests the awaited in-app session, not the hand-rolled 'opened' poll", async () => {
        hub.runProviderLogin.mockResolvedValue("terminal-unavailable");

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });
            await status.relogin({ retryAfterLogin: false });
            expect(hub.runProviderLogin).toHaveBeenCalledWith(
                expect.objectContaining({ awaitTier1Completion: true }),
            );
            dispose();
        });
    });

    it("'inapp-success' drives the same success path as 'seeded'/'terminal-success' — refreshes the controller and clears the failure banner", async () => {
        // runProviderLogin persists+links internally for this outcome (see
        // its own doc comment) and reports back via onAccountRegistered
        // exactly like tiers 2/3 — relogin() must not require a distinct
        // completion poll for it to recognize success.
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.setAuthUrl?.("https://claude.com/cai/oauth/authorize?code=true");
            opts.onAccountRegistered?.("acct-inapp", "/tmp/acct-inapp");
            return "inapp-success";
        });

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });
            await status.relogin({ retryAfterLogin: false });

            expect(RpcApi.ControllerResyncCommand).toHaveBeenCalled();
            expect(status.authNotice()).toBeNull();
            // codex P2 on PR #2413: AuthUrlBox must not stay mounted (still
            // offering paste/cancel/"use terminal instead") after a login
            // that already succeeded.
            expect(status.authUrl()).toBeNull();
            dispose();
        });
    });

    it("'inapp-timeout' surfaces a notice pointing back at the still-open login, not a generic failure, and clears the now-dead auth URL", async () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.setAuthUrl?.("https://claude.com/cai/oauth/authorize?code=true");
            return "inapp-timeout";
        });

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });
            await status.relogin({ retryAfterLogin: false });

            expect(status.authNotice()).toMatch(/login link timed out/i);
            expect(RpcApi.ControllerResyncCommand).not.toHaveBeenCalled();
            // codex P2 on PR #2413: runProviderLogin already cancelled and
            // reaped the login child by the time it returns "inapp-timeout"
            // — the URL/paste box must not stay mounted implying it still
            // works (a paste at this point goes nowhere).
            expect(status.authUrl()).toBeNull();
            dispose();
        });
    });

    it("useTerminalInstead waits for the in-flight relogin to actually finish before starting the terminal flow, even past a short fixed bound (codex P2 on PR #2413)", async () => {
        let resolveFirstAttempt: (v: string) => void;
        const firstAttempt = new Promise<string>((res) => { resolveFirstAttempt = res; });
        let secondAttemptStarted = false;

        hub.runProviderLogin
            .mockImplementationOnce(() => firstAttempt) // relogin()'s own call — held open
            .mockImplementationOnce(async () => {
                secondAttemptStarted = true;
                return "terminal-success";
            }); // loginViaTerminal()'s call, fired from useTerminalInstead

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });

            const reloginPromise = status.relogin({ retryAfterLogin: false });
            const terminalPromise = status.useTerminalInstead();

            // Held well past the old fixed ~4.5s bound (simulated instantly
            // here since runProviderLogin is mocked, not timed) — the
            // second attempt must NOT have started while the first is
            // still unresolved. Flush several microtask ticks (relogin()'s
            // own setup does a few awaited hops before reaching
            // runProviderLogin) rather than asserting after just one.
            for (let i = 0; i < 10; i++) await Promise.resolve();
            expect(secondAttemptStarted).toBe(false);

            resolveFirstAttempt!("inapp-timeout");
            await reloginPromise;
            await terminalPromise;

            expect(secondAttemptStarted).toBe(true);
            dispose();
        });
    });

    it("reagent P2 on PR #2413 (re-review): clears authUrl on the persist-FAILURE branch too, not just the success branch", async () => {
        // registeredAccountId/Dir never get set — onAccountRegistered
        // simply doesn't fire, mirroring a persist failure inside
        // runProviderLogin (REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_
        // WORKING_2026_07_27.md) — the credential is on disk but the
        // Armory row couldn't be saved.
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.setAuthUrl?.("https://claude.com/cai/oauth/authorize?code=true");
            return "inapp-success";
        });

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });
            await status.relogin({ retryAfterLogin: false });

            expect(status.authNotice()).toMatch(/couldn't save the account record/i);
            // The whole point: AuthUrlBox must not stay mounted (offering
            // paste/cancel/"use terminal instead") against a session
            // runProviderLogin already reaped.
            expect(status.authUrl()).toBeNull();
            dispose();
        });
    });

    it("reagent P2 on PR #2413 (re-review): useTerminalInstead surfaces an explicit failure instead of silently no-op'ing when the 20s teardown backstop fires on a genuinely wedged relogin", async () => {
        vi.useFakeTimers();
        // relogin() never resolves within this test — simulates a
        // genuinely wedged teardown (e.g. getCliLoginStatus() itself hung).
        hub.runProviderLogin.mockImplementation(() => new Promise(() => {}));

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });

            void status.relogin({ retryAfterLogin: false });
            await vi.advanceTimersByTimeAsync(10);
            const terminalPromise = status.useTerminalInstead();
            await vi.advanceTimersByTimeAsync(20_000);
            await terminalPromise;

            expect(status.authNotice()).toMatch(/taking longer than expected/i);
            dispose();
        });
    });

    it("reagent P1 on PR #2413 (round 3): useTerminalInstead waits out a /login-driven session too, not just relogin()'s — /login never touches reloginInFlight/reloginDonePromise, only the shared loginWaiting() counter", async () => {
        // /login (commands/global/login.ts) drives beginRecoveryFlow/
        // endRecoveryFlow directly — it never calls relogin(), so
        // reloginInFlight stays false throughout. Before this fix,
        // useTerminalInstead awaited reloginDonePromise, which is
        // ALREADY RESOLVED in this scenario (relogin() was never called),
        // so it fell straight through to loginViaTerminal() — and since
        // reloginInFlight is false, that guard didn't catch it either,
        // starting a second concurrent login child while /login's own
        // poll was still tearing the first one down.
        let secondAttemptStarted = false;
        hub.runProviderLogin.mockImplementation(async () => {
            secondAttemptStarted = true;
            return "terminal-success";
        }); // only ever hit by loginViaTerminal(), fired from useTerminalInstead

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });

            // Simulates /login's in-flight session.
            status.beginRecoveryFlow();
            expect(status.loginWaiting()).toBe(true);

            const terminalPromise = status.useTerminalInstead();

            // Give the poll a few real ticks to run — it must NOT proceed
            // to loginViaTerminal() while the simulated /login session is
            // still marked in-flight.
            await new Promise((r) => setTimeout(r, 50));
            expect(secondAttemptStarted).toBe(false);
            expect(hub.runProviderLogin).not.toHaveBeenCalled();

            // Simulates /login's own poll finishing and tearing down.
            status.endRecoveryFlow();
            await terminalPromise;

            expect(secondAttemptStarted).toBe(true);
            dispose();
        });
    });

    it("reagent P2 on PR #2413 (round 3): 'inapp-timeout' from a relogin() cancelled via useTerminalInstead does not overwrite the terminal flow's own notice with the generic timeout message", async () => {
        let resolveFirstAttempt: (v: string) => void;
        const firstAttempt = new Promise<string>((res) => { resolveFirstAttempt = res; });

        hub.runProviderLogin
            .mockImplementationOnce((opts: any) => {
                opts.setAuthUrl?.("https://claude.com/cai/oauth/authorize?code=true");
                return firstAttempt;
            }) // relogin()'s own call — held open
            .mockImplementationOnce(async () => "terminal-success"); // loginViaTerminal()'s call

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });

            const reloginPromise = status.relogin({ retryAfterLogin: false });
            // Fires cancelLogin() synchronously (loginCancelled = true)
            // before the still-in-flight relogin() resolves below — exactly
            // the ordering a real click produces.
            const terminalPromise = status.useTerminalInstead();

            // The cancellation is what makes runProviderLogin's own poll
            // resolve "inapp-timeout" in practice (run-provider-login.ts);
            // simulated directly here since runProviderLogin is mocked.
            resolveFirstAttempt!("inapp-timeout");
            await reloginPromise;

            // Before this fix, relogin()'s "inapp-timeout" branch
            // unconditionally overwrote whatever notice the cancel/terminal
            // flow had just set (or was about to set) with "the login link
            // timed out" — telling the user their login failed when they'd
            // simply asked to switch to a terminal.
            expect(status.authNotice() ?? "").not.toMatch(/login link timed out/i);

            await terminalPromise;
            dispose();
        });
    });
});

describe("useAgentControllerStatus — loginViaTerminal clears canRetry on success (reagent P1 on PR #2951)", () => {
    // Deadlock guard. `relogin` clears canRetry synchronously on click;
    // loginViaTerminal never did. The only other clear is
    // notifyControllerHealthy, which requires an ACTIVE TURN — and
    // checkAuthGuard refuses to start one while canRetry is true. So a
    // successful terminal login left canRetry stuck true forever and every
    // later send was fast-failed as "not logged in" on an agent that had just
    // logged in successfully.
    //
    // Only reachable since PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02
    // exposed "Login via terminal" on the pre-launch row — the old blue bar
    // offered "Log in" (relogin) alone.

    it("clears canRetry after a successful terminal login", async () => {
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });

            // Reach the pre-launch state the consolidated row represents:
            // canRetry TRUE. A FAILED no-retry relogin is what restores it
            // (:894) — a succeeding one would leave it false and make this
            // test vacuous, which an earlier version of it was.
            hub.runProviderLogin.mockResolvedValue("terminal-unavailable");
            await status.relogin({ retryAfterLogin: false });
            expect(status.canRetry()).toBe(true); // precondition, not the assertion

            hub.runProviderLogin.mockImplementation(async (opts: any) => {
                opts.onAccountRegistered?.("acct-term", "/tmp/acct-term");
                return "terminal-success";
            });
            await status.loginViaTerminal();

            // The regression: this read `true`, and nothing could ever clear it
            // — notifyControllerHealthy needs a turn the guard won't allow.
            expect(status.canRetry()).toBe(false);
            dispose();
        });
    });

    it("leaves canRetry alone when the terminal login does NOT succeed", async () => {
        // Cleared on success only — deliberately not at the start like relogin,
        // which pairs its early clear with a restore-on-failure. Dropping it
        // here on failure would remove both the send guard and the failure row
        // for an agent that is still unauthenticated.
        hub.runProviderLogin.mockResolvedValue("terminal-unavailable");

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });
            await status.relogin({ retryAfterLogin: false });
            const before = status.canRetry();

            await status.loginViaTerminal();

            expect(status.canRetry()).toBe(before);
            dispose();
        });
    });
});

describe("useAgentControllerStatus — loginViaTerminal routes by retryAfterLogin, like relogin (reagent + manoz P1 on PR #2951)", () => {
    // The cross-hook race both reviewers found: loginViaTerminal used to call
    // onRecovered unconditionally, leaving the caller to infer retry-vs-startup
    // from pane state AFTER the fact. But this branch's own setCanRetry(false)
    // is an unbatched Solid write that synchronously flushes agent-view's
    // effect, which clears the very failure that inference read — so the
    // pre-launch case silently fell back to "retry" and resent a stale message.
    //
    // Fixed by removing the inference rather than timing it: the caller decides
    // at click time, exactly as relogin has always worked. These pin the
    // routing, which is what makes the decision independent of effect ordering.
    const terminalSuccess = () => {
        hub.runProviderLogin.mockImplementation(async (opts: any) => {
            opts.onAccountRegistered?.("acct-term", "/tmp/acct-term");
            return "terminal-success";
        });
    };

    it("retryAfterLogin:false routes to onReady (startup), never onRecovered (retry)", async () => {
        const onRecovered = vi.fn();
        const onReady = vi.fn();
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
                onRecovered,
                onReady,
            });
            terminalSuccess();
            await status.loginViaTerminal({ retryAfterLogin: false });

            // The stale-resend bug is exactly onRecovered firing here.
            expect(onRecovered).not.toHaveBeenCalled();
            expect(onReady).toHaveBeenCalled();
            dispose();
        });
    });

    it("retryAfterLogin:true routes to onRecovered (retry), matching the mid-turn 401 case", async () => {
        const onRecovered = vi.fn();
        const onReady = vi.fn();
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
                onRecovered,
                onReady,
            });
            terminalSuccess();
            await status.loginViaTerminal({ retryAfterLogin: true });

            expect(onRecovered).toHaveBeenCalled();
            dispose();
        });
    });

    it("defaults to retry when the caller says nothing (pre-existing behaviour)", async () => {
        const onRecovered = vi.fn();
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
                onRecovered,
            });
            terminalSuccess();
            await status.loginViaTerminal();

            expect(onRecovered).toHaveBeenCalled();
            dispose();
        });
    });
});

describe("useAgentControllerStatus — a guard-rejected loginViaTerminal must not corrupt the live intent (reagent P1 + manoz on PR #2951)", () => {
    // loginViaTerminal wrote inFlightRetryAfterLogin BEFORE checking
    // reloginInFlight, so a call the guard rejected still clobbered the flag on
    // its way out. Reachable through the inline transcript CTA this PR keeps
    // (surface C), which can coexist with a different-valued row:
    //   1. old inline "Login Again"        -> relogin(true), in flight
    //   2. row's "Login via terminal"      -> (false) no-ops, used to write false
    //   3. "Use terminal instead" on (1)'s panel -> read false -> onReady()
    // i.e. silently dropping the retry of a turn that genuinely ran.
    it("keeps the live flow's intent when a differently-valued call is rejected mid-flight", async () => {
        const onRecovered = vi.fn();
        const onReady = vi.fn();
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1", provider: () => claude, log: () => {},
                onRecovered, onReady,
            });

            let release!: () => void;
            const gate = new Promise<void>((r) => { release = r; });
            hub.runProviderLogin.mockImplementation(async () => {
                await gate;
                return "terminal-unavailable";
            });

            const live = status.relogin({ retryAfterLogin: true });   // in flight
            await status.loginViaTerminal({ retryAfterLogin: false }); // rejected by the guard
            const escape = status.useTerminalInstead();
            release();
            await live;

            hub.runProviderLogin.mockImplementation(async (o: any) => {
                o.onAccountRegistered?.("acct-term", "/tmp/acct-term");
                return "terminal-success";
            });
            await escape;

            expect(onRecovered).toHaveBeenCalled();
            expect(onReady).not.toHaveBeenCalled();
            dispose();
        });
    });
});

describe("useAgentControllerStatus — the recovery intent is scoped to one flow (reagent P1 on PR #2951)", () => {
    // `/login` drives its own OAuth, shares only the activeRecoveryFlows
    // counter, and never writes inFlightRetryAfterLogin — yet its session
    // shows the same AuthUrlBox and so the same "Use terminal instead"
    // handler. The flag used to survive between flows, so a /login session
    // read whatever an unrelated earlier relogin had left behind.
    it("does not leak a previous flow's intent into a later one that never declared one", async () => {
        const onRecovered = vi.fn();
        const onReady = vi.fn();
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
                onRecovered,
                onReady,
            });

            // An earlier mid-turn recovery records retryAfterLogin:true and
            // fully completes.
            hub.runProviderLogin.mockResolvedValue("terminal-unavailable");
            await status.relogin({ retryAfterLogin: true });

            // A later flow that declares nothing (the /login shape) escapes to
            // the terminal and succeeds. It must NOT inherit that true.
            hub.runProviderLogin.mockImplementation(async (o: any) => {
                o.onAccountRegistered?.("acct-term", "/tmp/acct-term");
                return "terminal-success";
            });
            onRecovered.mockClear();
            onReady.mockClear();
            await status.useTerminalInstead();

            // Inheriting `true` would resend the agent's last message.
            expect(onRecovered).not.toHaveBeenCalled();
            expect(onReady).toHaveBeenCalled();
            dispose();
        });
    });
});

describe("useAgentControllerStatus — 'Use terminal instead' carries the LIVE flow's intent (reagent P1 on PR #2951)", () => {
    // Models the real lifecycle: the user clicks "Use terminal instead" on the
    // AuthUrlBox WHILE the originating flow is still in flight. An earlier
    // version of these tests awaited the relogin to completion first, which is
    // not how the panel is reached — and once the intent became flow-scoped,
    // that shape stopped exercising the mechanism at all (it read the cleared
    // resting value and would have passed either way). reagent flagged the
    // sequential-only coverage; this is the fix for the tests, not just the code.
    const inFlightLogin = () => {
        let release!: () => void;
        const gate = new Promise<void>((r) => { release = r; });
        hub.runProviderLogin.mockImplementation(async () => {
            await gate;
            return "terminal-unavailable";
        });
        return { release };
    };
    const terminalSucceeds = () => {
        hub.runProviderLogin.mockImplementation(async (o: any) => {
            o.onAccountRegistered?.("acct-term", "/tmp/acct-term");
            return "terminal-success";
        });
    };

    it("escaping a PRE-LAUNCH relogin routes to onReady, never a retry", async () => {
        const onRecovered = vi.fn();
        const onReady = vi.fn();
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1", provider: () => claude, log: () => {},
                onRecovered, onReady,
            });
            const { release } = inFlightLogin();
            const live = status.relogin({ retryAfterLogin: false }); // in flight
            const escape = status.useTerminalInstead();              // clicked NOW
            release();
            await live;
            terminalSucceeds();
            await escape;

            expect(onRecovered).not.toHaveBeenCalled();
            expect(onReady).toHaveBeenCalled();
            dispose();
        });
    });

    it("escaping a MID-TURN relogin still retries the failed turn", async () => {
        const onRecovered = vi.fn();
        const onReady = vi.fn();
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1", provider: () => claude, log: () => {},
                onRecovered, onReady,
            });
            const { release } = inFlightLogin();
            const live = status.relogin({ retryAfterLogin: true });
            const escape = status.useTerminalInstead();
            release();
            await live;
            terminalSucceeds();
            await escape;

            // The mirror-image bug: dropping a retry that genuinely was owed.
            expect(onRecovered).toHaveBeenCalled();
            dispose();
        });
    });
});

describe("useAgentControllerStatus — a caller-declared intent survives to the terminal escape (manoz on PR #2951)", () => {
    // `/login` drives its own OAuth and never writes the intent directly, but
    // its session shows the same AuthUrlBox and so the same "Use terminal
    // instead". With the flag resting at false, that escape called onReady()
    // and silently dropped a retry that WAS owed when a real auth failure was
    // pending — while the row's own "Login via terminal" retried in the
    // identical state. Same situation, opposite outcome, by entry point.
    //
    // Also a behaviour CHANGE rather than a pre-existing gap: before this PR
    // that handler defaulted to true and the retry happened.
    it("beginRecoveryFlow(true) makes the escape retry, matching the row button", async () => {
        const onRecovered = vi.fn();
        const onReady = vi.fn();
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1", provider: () => claude, log: () => {},
                onRecovered, onReady,
            });
            // What /login now does when a real auth failure is pending.
            status.beginRecoveryFlow(true);

            hub.runProviderLogin.mockImplementation(async (o: any) => {
                o.onAccountRegistered?.("acct-term", "/tmp/acct-term");
                return "terminal-success";
            });
            // The escape waits for THIS flow to tear down before starting the
            // terminal one — so end it while the wait is in progress, which is
            // the real sequence (cancelLogin -> flow unwinds -> terminal runs).
            const escape = status.useTerminalInstead();
            status.endRecoveryFlow();
            await escape;

            expect(onRecovered).toHaveBeenCalled();
            expect(onReady).not.toHaveBeenCalled();
            dispose();
        });
    });

    it("beginRecoveryFlow(false) keeps the no-retry branch for a pure credential operation", async () => {
        const onRecovered = vi.fn();
        const onReady = vi.fn();
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1", provider: () => claude, log: () => {},
                onRecovered, onReady,
            });
            status.beginRecoveryFlow(false); // /login with no pending failure

            hub.runProviderLogin.mockImplementation(async (o: any) => {
                o.onAccountRegistered?.("acct-term", "/tmp/acct-term");
                return "terminal-success";
            });
            const escape = status.useTerminalInstead();
            status.endRecoveryFlow();
            await escape;

            expect(onRecovered).not.toHaveBeenCalled();
            expect(onReady).toHaveBeenCalled();
            dispose();
        });
    });
});

describe("useAgentControllerStatus — a no-provider bail-out leaves no intent behind (reagent P2 on PR #2951)", () => {
    // Same rule as the in-flight guard: a call that returns WITHOUT starting a
    // flow must not record its intent. There is no beginRecoveryFlow on that
    // path, hence no paired endRecoveryFlow to clear it, so the value would
    // persist until some unrelated flow happened to finish — and a later
    // /login session (which declares its own intent only when a failure is
    // pending) could read it from "Use terminal instead".
    it("relogin with no provider does not record an intent", async () => {
        const onRecovered = vi.fn();
        const onReady = vi.fn();
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => undefined, // no provider: bails immediately
                log: () => {},
                onRecovered,
                onReady,
            });
            await status.relogin({ retryAfterLogin: true });
            dispose();
        });

        // A separate pane whose provider IS available: if the bail-out above
        // had recorded `true`, this flow — which declares nothing — would
        // inherit it and retry.
        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-2", provider: () => claude, log: () => {},
                onRecovered, onReady,
            });
            status.beginRecoveryFlow(); // no declared intent, as /login does with no failure
            hub.runProviderLogin.mockImplementation(async (o: any) => {
                o.onAccountRegistered?.("acct-term", "/tmp/acct-term");
                return "terminal-success";
            });
            const escape = status.useTerminalInstead();
            status.endRecoveryFlow();
            await escape;

            expect(onRecovered).not.toHaveBeenCalled();
            expect(onReady).toHaveBeenCalled();
            dispose();
        });
    });
});
