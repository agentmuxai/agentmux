// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * reagent P1 on PR #2338: unlike relogin()/loginViaTerminal(), useGlobalLogin()
 * never set loginWaiting while its async credential-seed work was in flight —
 * useAgentCommands.ts's fast-fail guard (canRetry() || loginWaiting()) had no
 * signal to check, so a message typed while "Use existing login" was still
 * resolving bypassed the guard entirely and reached AgentInputCommand on the
 * same stale credential the failure banner exists to warn about.
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
