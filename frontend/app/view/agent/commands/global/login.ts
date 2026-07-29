// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * /login — run a GUI OAuth flow via the host API, capture the returned
 * URL, and push it into ctx.setAuthUrl so the auth box appears above
 * the composer. Falls all the way through `runProviderLogin`'s three
 * recovery tiers (retro-headless-login-browser-open-2026-07-20) instead of
 * dead-ending the moment the CLI produces no scrapeable URL — Claude Code
 * v2.1.x never does, in any headless spawn, so that used to mean "does
 * nothing" for the single most common provider.
 *
 * Migrated from useAgentCommands.runLoginCommand (PR #378 era). Unlike
 * pure commands, /login logs progress directly via ctx.log because the
 * auth flow is multi-step and uses an "auth"-level log channel that
 * the dispatcher's single-result formatter doesn't model. This is
 * intentional — centralized result logging is the common case; /login
 * is the exception. See spec §4.4 dispatcher note.
 */

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { persistAndLinkAccount, runProviderLogin } from "../../flows/run-provider-login";
import type { SlashCommand, SlashResult } from "../types";

export const loginCommand: SlashCommand = {
    name: "login",
    category: "auth",
    description: "Authenticate with the active provider (OAuth in browser)",
    arg: { kind: "none" },
    availability: "any-agent",
    handler: async (ctx): Promise<SlashResult> => {
        const prov = ctx.provider();
        const cliPath = ctx.block()?.meta?.["cmd"] ?? "";
        if (!prov || !cliPath) {
            return { kind: "error", message: "/login: provider or CLI path not available" };
        }
        ctx.log("auth", "running /login via GUI flow...");
        try {
            const authEnv: Record<string, string> = {};
            const envMeta = ctx.block()?.meta?.["cmd:env"];
            if (envMeta && typeof envMeta === "object") {
                for (const [k, v] of Object.entries(envMeta)) {
                    if (typeof v === "string") authEnv[k] = v;
                }
            }
            // Shared with the failure-banner / inline-error "Login Again" action
            // (useAgentControllerStatus.relogin) — opens the OAuth in an in-app
            // browser pane, or falls through to the global-login copy / real-terminal
            // tiers when the CLI produces no scrapeable URL. See run-provider-login.ts.
            // linkTarget lets a tier-2/3 success register a real Armory account
            // bound to this agent (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7).
            const agentDefinitionId = ctx.block()?.meta?.["agentId"] as string | undefined;
            const linkTarget = agentDefinitionId
                ? { blockId: ctx.blockId, agentDefinitionId }
                : undefined;
            // Tier 1 mints the account dir but does NOT persist/link it (it
            // returns "opened" before confirming completion) — captured here
            // so the poll below can call persistAndLinkAccount once IT
            // confirms the login actually finished. reagent P1: without
            // this, a tier-1 login that succeeds for any provider whose CLI
            // actually prints a URL (not requiresLoginTty, e.g. codex) via
            // /login left the minted account unpersisted/unlinked — the
            // resolver's spawn gate then blocks the agent on its very next
            // spawn even though this handler just reported "run /cost to
            // verify" as if the login were already usable.
            let openedAccountId: string | undefined;
            let openedAccountDir: string | undefined;
            let recheckAuthEnv = authEnv;
            const outcome = await runProviderLogin({
                provider: prov,
                cliPath,
                authEnv,
                setAuthUrl: ctx.setAuthUrl,
                log: ctx.log,
                linkTarget,
                onAccountRegistered: (accountId, dir) => {
                    openedAccountId = accountId;
                    openedAccountDir = dir;
                    if (prov.authConfigDirEnvVar) {
                        recheckAuthEnv = { ...authEnv, [prov.authConfigDirEnvVar]: dir };
                    }
                },
                // See catalog.ts's DEAD END note — skip tier 1's ~15s
                // URL-capture wait for providers that can never produce one.
                skipTier1: prov.headlessLoginUrlUnsupported === true,
            });
            switch (outcome) {
                case "opened": {
                    ctx.log("auth", "waiting for login to complete...");
                    let authenticated = false;
                    const deadline = Date.now() + 5 * 60 * 1000;
                    while (Date.now() < deadline && !authenticated) {
                        await new Promise<void>((r) => setTimeout(r, 2000));
                        try {
                            const recheck = await RpcApi.CheckCliAuthCommand(TabRpcClient, {
                                cli_path: cliPath,
                                auth_check_args: prov.authCheckCommand,
                                auth_env: recheckAuthEnv,
                            }, { timeout: 10000 });
                            if (recheck.authenticated) authenticated = true;
                        } catch {
                            // keep polling on transient RPC errors
                        }
                    }
                    if (authenticated && openedAccountId && openedAccountDir) {
                        // reagent P1 (re-review of PR #2318): must check the
                        // return value — the exact same persist-failure gap
                        // found and fixed in useAgentControllerStatus.ts's
                        // relogin() "opened" branch. Without this, a DB-write
                        // failure here still reported "login complete" while
                        // leaving no real account behind for the resolver's
                        // spawn gate to find on the very next turn.
                        const persisted = await persistAndLinkAccount(
                            { provider: prov, cliPath, authEnv, setAuthUrl: ctx.setAuthUrl, log: ctx.log, linkTarget },
                            openedAccountId,
                            openedAccountDir,
                        );
                        if (!persisted) {
                            return {
                                kind: "error",
                                message:
                                    "/login: the login succeeded, but AgentMux couldn't save the account record. Try again in a moment.",
                            };
                        }
                        // Restart an already-running persistent controller so
                        // this refreshed credential actually takes effect —
                        // send_message only spawns a fresh process when one
                        // isn't already running. Must happen BEFORE declaring
                        // success below, matching relogin()/useGlobalLogin()/
                        // loginViaTerminal()'s own ordering. Codex P1 on
                        // PR #2338 (seventh re-review).
                        await ctx.forceControllerRefresh();
                        ctx.log("auth", "login complete — run /cost to verify");
                        // A pane that already showed the mount-time "Log in"
                        // bar (canRetry() true) before the user typed /login
                        // directly, bypassing that button, would otherwise
                        // have every subsequent message fast-failed forever —
                        // /login never went through relogin(), the only other
                        // place that manages canRetry. Codex P1 on PR #2338.
                        ctx.notifyControllerHealthy();
                        // A stale pre-existing "auth" failure row must also
                        // be cleared — otherwise the caller's NEXT normal
                        // send re-captures it as authFailureToPreserve and
                        // both fast-fails the message and re-shows this now-
                        // stale banner, even though the credential is fine.
                        // reagent P1 on PR #2338 (re-review).
                        ctx.clearAuthFailure();
                        return { kind: "ok" };
                    }
                    return {
                        kind: "error",
                        message:
                            "/login: opened a login page, but no login was detected within 5 minutes. " +
                            "Complete the login there, then run /login again.",
                    };
                }
                case "seeded":
                case "terminal-success":
                    // openedAccountId/openedAccountDir are only set once
                    // onAccountRegistered fires — run-provider-login.ts only
                    // calls it once the account row is actually persisted, so
                    // this also catches the case where a credential seeded/
                    // typed in successfully but the DB write itself failed.
                    // See REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md.
                    if (openedAccountId && openedAccountDir) {
                        // See the "opened" branch's identical call above.
                        await ctx.forceControllerRefresh();
                        if (outcome === "terminal-success") {
                            ctx.log("auth", "login complete — run /cost to verify");
                        }
                        // See the "opened" branch's identical calls above.
                        ctx.notifyControllerHealthy();
                        ctx.clearAuthFailure();
                        return { kind: "ok" };
                    }
                    return {
                        kind: "error",
                        message:
                            "/login: the login succeeded, but AgentMux couldn't save the account record. Try again in a moment.",
                    };
                case "terminal-timeout":
                    // Never report success for a login that didn't complete
                    // (retro-agent-auth-relogin-noop-2026-07-01 §5.1).
                    return {
                        kind: "error",
                        message:
                            "/login: opened a terminal window, but no login was detected within 5 minutes. " +
                            "Complete the login there, then run /login again.",
                    };
                case "terminal-unavailable":
                    return {
                        kind: "error",
                        message: "/login: the CLI produced no login URL, and a terminal window couldn't be opened on this platform.",
                    };
            }
        } catch (err: any) {
            return { kind: "error", message: `/login failed: ${err?.message ?? String(err)}` };
        }
    },
};
