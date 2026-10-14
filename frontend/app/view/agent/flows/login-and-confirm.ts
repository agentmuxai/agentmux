// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one "log in and confirm it landed" core, plus the provider steps the
 * mount-time launch flow and the pane's login actions share: resolve the
 * CLI, find the agent's linked account, and check auth. (Registering the
 * controller, which both also share, is register-controller.ts.)
 *
 * `logInAndConfirm` runs `runProviderLogin` and turns its outcome into one
 * answer: signed in, signed in but not saved, not detected, or unavailable.
 * For a tier-1 "opened" outcome it polls for completion itself and persists
 * the account, which `useAgentControllerStatus`'s relogin and `/login` used
 * to do in two copies of the same loop.
 * REPORT_REDUCER_STACK_AUDIT_2026_07_26.md §4.3-§4.4, items 3 and 12.
 */

import { RpcApi, type ResolveCliResult } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { sleep } from "@/util/util";
import type { CheckCliAuthResult } from "@/types/rpc/CheckCliAuthResult";
import { lastLinkedAccountId } from "../providers/provider-id-aliases";
import type { ProviderDefinition } from "../providers";
import { persistAndLinkAccount, runProviderLogin, type RunProviderLoginParams } from "./run-provider-login";

/** How long a login page or terminal is given to finish: the same window
 *  run-provider-login.ts's own polls use. */
export const LOGIN_COMPLETION_TIMEOUT_MS = 5 * 60 * 1000;

type CliProvider = Pick<
    ProviderDefinition,
    "id" | "cliCommand" | "npmPackage" | "pinnedVersion" | "windowsInstallCommand" | "unixInstallCommand"
>;

/** Find (or install) the provider's CLI. Throws what the RPC throws. */
export function resolveProviderCli(provider: CliProvider, blockId: string): Promise<ResolveCliResult> {
    return RpcApi.ResolveCliCommand(TabRpcClient, {
        provider_id: provider.id,
        cli_command: provider.cliCommand,
        npm_package: provider.npmPackage,
        pinned_version: provider.pinnedVersion,
        windows_install_command: provider.windowsInstallCommand,
        unix_install_command: provider.unixInstallCommand,
        block_id: blockId,
    }, { timeout: 300000 });
}

/** The account linked to this agent for `providerId`, matched across legacy
 *  provider aliases. Best-effort: undefined when the lookup fails. */
export async function linkedAccountFor(agentDefinitionId: string, providerId: string): Promise<string | undefined> {
    try {
        const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: agentDefinitionId });
        return lastLinkedAccountId(links, providerId);
    } catch {
        return undefined;
    }
}

/** Ask the CLI whether it is signed in, in `authEnv`'s config dir. Throws
 *  what the RPC throws; callers decide what an unknown answer means. */
export function checkCliAuth(
    cliPath: string,
    authCheckArgs: string[],
    authEnv: Record<string, string> | undefined,
    timeoutMs: number,
): Promise<CheckCliAuthResult> {
    return RpcApi.CheckCliAuthCommand(TabRpcClient, {
        cli_path: cliPath,
        auth_check_args: authCheckArgs,
        auth_env: authEnv,
    }, { timeout: timeoutMs });
}

/** Poll the CLI's auth check every `pollMs` until it reports signed in, the
 *  deadline passes, or `isCancelled()`. Transient RPC errors keep polling.
 *  Resolves the signed-in email, or null if it never signed in. */
export async function pollUntilAuthenticated(p: {
    cliPath: string;
    authCheckArgs: string[];
    authEnv: Record<string, string> | undefined;
    isCancelled: () => boolean;
    deadlineMs: number;
    pollMs?: number;
}): Promise<{ email: string | null } | null> {
    const pollMs = p.pollMs ?? 2000;
    while (!p.isCancelled() && Date.now() < p.deadlineMs) {
        await sleep(pollMs);
        if (p.isCancelled()) break;
        try {
            const r = await checkCliAuth(p.cliPath, p.authCheckArgs, p.authEnv, 10000);
            if (r.authenticated) return { email: r.email ?? null };
        } catch {
            // keep polling on transient RPC errors
        }
    }
    return null;
}

export type LoginConfirmation =
    /** The credential landed and its account is saved and linked (or the
     *  CLI keeps its own login and has no account). */
    | { kind: "signed-in"; email: string | null }
    /** The credential landed, but its account row couldn't be saved: the
     *  next spawn would still be blocked. */
    | { kind: "save-failed" }
    /** Nothing signed in before the window closed, or the user cancelled.
     *  `stage`: what was open at the time. */
    | { kind: "not-detected"; stage: "link" | "inapp" | "terminal" }
    /** No login could even be started on this platform. */
    | { kind: "unavailable" };

export interface LogInAndConfirmParams extends RunProviderLoginParams {
    isCancelled: () => boolean;
}

/** Run the provider login and confirm it landed. Never reports success for
 *  an account that wasn't saved: `runProviderLogin` fires
 *  `onAccountRegistered` only once the account row exists (tier 1's
 *  "opened" fires it for the minted dir, which this persists itself). */
export async function logInAndConfirm(p: LogInAndConfirmParams): Promise<LoginConfirmation> {
    let account: { id: string; dir: string } | undefined;
    // runProviderLogin points the login at the account's own dir; a recheck
    // must look there, not at the caller's original env.
    let recheckEnv = p.authEnv;
    const outcome = await runProviderLogin({
        ...p,
        onAccountRegistered: (accountId, dir) => {
            account = { id: accountId, dir };
            if (p.provider.authConfigDirEnvVar) {
                recheckEnv = { ...p.authEnv, [p.provider.authConfigDirEnvVar]: dir };
            }
            p.onAccountRegistered?.(accountId, dir);
        },
    });
    switch (outcome) {
        case "opened": {
            p.log("auth", "waiting for login to complete...");
            const deadlineMs = Date.now() + LOGIN_COMPLETION_TIMEOUT_MS;
            p.onTierChange?.({ tier: "polling", deadlineMs });
            const authed = await pollUntilAuthenticated({
                cliPath: p.cliPath,
                authCheckArgs: p.provider.authCheckCommand,
                authEnv: recheckEnv,
                isCancelled: p.isCancelled,
                deadlineMs,
            });
            // No minted account (its dir mint failed) means nothing to save:
            // reported as not detected, as both callers always did.
            if (!authed || !account) return { kind: "not-detected", stage: "link" };
            const persisted = await persistAndLinkAccount(
                {
                    provider: p.provider,
                    cliPath: p.cliPath,
                    authEnv: p.authEnv,
                    setAuthUrl: p.setAuthUrl,
                    log: p.log,
                    linkTarget: p.linkTarget,
                },
                account.id,
                account.dir,
            );
            return persisted ? { kind: "signed-in", email: authed.email } : { kind: "save-failed" };
        }
        case "inapp-success":
        case "terminal-success":
            return account ? { kind: "signed-in", email: null } : { kind: "save-failed" };
        case "terminal-cli-signed-in":
            return { kind: "signed-in", email: null };
        case "inapp-timeout":
            return { kind: "not-detected", stage: "inapp" };
        case "terminal-timeout":
            return { kind: "not-detected", stage: "terminal" };
        case "terminal-unavailable":
            return { kind: "unavailable" };
    }
}
