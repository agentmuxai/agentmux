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
    registerSeededAccount: vi.fn(),
    runProviderLogin: vi.fn(),
}));

vi.mock("@/app/store/global", () => ({
    getApi: () => ({
        cancelCliLogin: () => Promise.resolve(),
        ensureAuthDir: () => Promise.resolve("/tmp/auth-dir"),
    }),
    getBlockMetaKeyAtom: (_blockId: string, key: string) => () => (key === "cmd" ? "claude-cli" : undefined),
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
vi.mock("../flows/register-seeded-account", () => ({
    registerSeededAccount: (...args: unknown[]) => hub.registerSeededAccount(...args),
}));

import { useAgentControllerStatus } from "./useAgentControllerStatus";

const claude = { id: "claude" } as any; // no authConfigDirEnvVar — skips the link-env sub-path

afterEach(() => {
    vi.clearAllMocks();
});

describe("useAgentControllerStatus — useGlobalLogin sets loginWaiting while in flight (reagent P1 on PR #2338)", () => {
    it("loginWaiting() is true while registerSeededAccount is unresolved, and false after it resolves", async () => {
        let resolveSeed!: (v: { ok: boolean; accountId?: string; dir?: string }) => void;
        hub.registerSeededAccount.mockImplementation(
            () => new Promise((resolve) => { resolveSeed = resolve; }),
        );

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });

            expect(status.loginWaiting()).toBe(false);

            const p = status.useGlobalLogin();
            // Yield a microtask so the async function body runs up to the
            // unresolved registerSeededAccount await.
            await Promise.resolve();
            await Promise.resolve();

            expect(status.loginWaiting()).toBe(true);

            resolveSeed({ ok: true, accountId: "acct-1", dir: "/tmp/acct-1" });
            await p;

            expect(status.loginWaiting()).toBe(false);
            dispose();
        });
    });

    it("clears loginWaiting even when registerSeededAccount fails", async () => {
        hub.registerSeededAccount.mockRejectedValue(new Error("boom"));

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });

            await status.useGlobalLogin();

            expect(status.loginWaiting()).toBe(false);
            dispose();
        });
    });
});

describe("useAgentControllerStatus — loginWaiting clears BEFORE onRecovered fires (reagent P0 on PR #2338)", () => {
    it("useGlobalLogin: loginWaiting() is already false inside the onRecovered callback", async () => {
        // onRecovered (agent-view.tsx's retryLastTurn) synchronously resends
        // the failed turn straight into useAgentCommands.ts's
        // canRetry() || loginWaiting() guard. Clearing loginWaiting only in
        // this function's trailing `finally` (which runs AFTER onRecovered
        // returns) let that resend get spuriously rejected as "not logged in
        // yet" even though recovery just succeeded — breaking the
        // auto-retry-after-relogin flow entirely.
        hub.registerSeededAccount.mockResolvedValue({ ok: true, accountId: "acct-1", dir: "/tmp/acct-1" });
        let loginWaitingInsideCallback: boolean | undefined;

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
                onRecovered: () => {
                    loginWaitingInsideCallback = status.loginWaiting();
                },
            });

            await status.useGlobalLogin();

            expect(loginWaitingInsideCallback).toBe(false);
            dispose();
        });
    });
});

describe("useAgentControllerStatus — overlapping recovery flows share one counter, not independent booleans (codex P2 on PR #2338, fourth re-review)", () => {
    it("loginWaiting() stays true while loginViaTerminal is still in flight, even after a concurrent useGlobalLogin() finishes", async () => {
        // Nothing disables the failure row's other recovery buttons while one
        // is running (verified against failure-accessory.ts: only the
        // generic "Retry now" action has a disabled binding), so a user can
        // genuinely start both flows concurrently. useGlobalLogin() and
        // loginViaTerminal() each manage their own in-flight guard
        // (seedInFlight vs. reloginInFlight) — a bare shared boolean would
        // let whichever finishes first clear loginWaiting for both.
        hub.registerSeededAccount.mockResolvedValue({ ok: true, accountId: "acct-1", dir: "/tmp/acct-1" });
        let resolveTerminalLogin!: (outcome: string) => void;
        hub.runProviderLogin.mockImplementation(
            (opts: any) =>
                new Promise((resolve) => {
                    resolveTerminalLogin = (outcome: string) => {
                        opts.onAccountRegistered?.("acct-2", "/tmp/acct-2");
                        resolve(outcome);
                    };
                }),
        );

        await createRoot(async (dispose) => {
            const status = useAgentControllerStatus({
                blockId: "block-1",
                provider: () => claude,
                log: () => {},
            });

            const terminalPromise = status.loginViaTerminal();
            await Promise.resolve();
            await Promise.resolve();
            expect(status.loginWaiting()).toBe(true);

            // A second, independent recovery flow starts and fully completes
            // WHILE the first is still unresolved.
            await status.useGlobalLogin();

            // The bug: useGlobalLogin() finishing first would clear the
            // shared boolean even though loginViaTerminal is still polling
            // for credentials — reopening the exact "send during an
            // unconfirmed recovery" window this signal exists to close.
            expect(status.loginWaiting()).toBe(true);

            resolveTerminalLogin("seeded");
            await terminalPromise;

            expect(status.loginWaiting()).toBe(false);
            dispose();
        });
    });
});
