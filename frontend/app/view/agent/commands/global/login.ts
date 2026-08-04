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
import { sleep } from "@/util/util";
import { TabRpcClient } from "@/app/store/rpc-util";
import { persistAndLinkAccount, runProviderLogin } from "../../flows/run-provider-login";
import type { SlashCommand, SlashCommandContext, SlashResult } from "../types";

/**
 * Shared by both success branches below (tier-1 "opened" and tier-2/3
 * "seeded"/"terminal-success"): restart an already-running controller onto
 * the refreshed credential — unless a turn is actively streaming on it.
 * `agentmux-srv`'s `resync_controller` with `force: true` unconditionally
 * stops the existing controller process before respawning it (see
 * `blockcontroller/mod.rs`'s `needs_replace` check), so forcing a restart
 * mid-turn would kill in-progress agent work.
 *
 * If a turn IS active, the restart (and declaring the pane healthy) is
 * DEFERRED until that turn ends, not skipped outright — persistent
 * providers keep the controller alive across many turns, not just this
 * one, so skipping-and-declaring-healthy would leave the controller on the
 * stale credential indefinitely with every fast-fail guard already
 * cleared. Codex P1 on PR #2338 (thirteenth re-review); superseded the
 * tenth re-review's skip-and-declare-healthy approach, which reintroduced
 * exactly the bug forceControllerRefresh was added to /login to fix, just
 * delayed by one turn.
 *
 * If the refresh RPC itself fails (and wasn't deferred), the controller may
 * still be on the stale credential — declaring the pane healthy anyway
 * would clear every fast-fail guard this PR added and let the next message
 * reach that stale process regardless. Codex P1 on PR #2338 (tenth
 * re-review).
 */
async function finalizeLoginSuccess(ctx: SlashCommandContext): Promise<SlashResult> {
    if (ctx.isTurnActive()) {
        ctx.deferControllerRefreshUntilIdle();
        ctx.log("auth", "login saved — the running agent will pick it up once the current turn finishes", "warn");
        return { kind: "ok" };
    }
    const refreshed = await ctx.forceControllerRefresh();
    if (!refreshed) {
        return {
            kind: "error",
            message:
                "/login: signed in, but couldn't refresh the running agent with the new login. " +
                "Reopen this pane if it still shows as logged out.",
        };
    }
    // A pane that already showed the mount-time "Log in" bar (canRetry()
    // true) before the user typed /login directly, bypassing that button,
    // would otherwise have every subsequent message fast-failed forever —
    // /login never went through relogin(), the only other place that
    // manages canRetry. Codex P1 on PR #2338.
    ctx.notifyControllerHealthy();
    // A stale pre-existing "auth" failure row must also be cleared —
    // otherwise the caller's NEXT normal send re-captures it as
    // authFailureToPreserve and both fast-fails the message and re-shows
    // this now-stale banner, even though the credential is fine. reagent
    // P1 on PR #2338 (re-review).
    ctx.clearAuthFailure();
    return { kind: "ok" };
}

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
        // Registers this attempt (including the up-to-5-minute poll below)
        // as an in-flight recovery on the SAME shared counter behind
        // loginWaiting() that relogin()/useGlobalLogin()/loginViaTerminal()
        // already use — without this, a second message sent while /login is
        // still polling gets held with authWasKnownBadAtQueueTime: false
        // (mid-turn "auth" failures never set canRetry either), so a /login
        // that ultimately fails flushes that held message straight to the
        // still-known-bad controller. Codex P1 on PR #2338 (ninth
        // re-review). Paired with the endRecoveryFlow() in this function's
        // own finally below — every return path (success, error, and the
        // catch) goes through it exactly once.
        ctx.beginRecoveryFlow();
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
                // Behavior-gate only: skip tier 1's ~15s URL-capture wait for
                // providers whose CLI is documented to never print one. Since
                // SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.2 dropped the
                // flag for Claude (2.1.198+ prints the authorize URL under our
                // PTY spawn), no catalog provider sets it — so /login now runs
                // the in-app tier 1 for Claude too: the AuthUrlBox above the
                // composer shows the URL + paste box, and the "opened" branch
                // below polls for completion and persists the account.
                skipTier1: prov.headlessLoginUrlUnsupported === true,
            });
            switch (outcome) {
                case "opened": {
                    ctx.log("auth", "waiting for login to complete...");
                    let authenticated = false;
                    const deadline = Date.now() + 5 * 60 * 1000;
                    // reagent P1 on PR #2413 (round 3, second pass): the
                    // AuthUrlBox Cancel / "Use terminal instead" buttons
                    // call useAgentControllerStatus's cancelLogin()/
                    // useTerminalInstead() directly — this poll had no way
                    // to learn that happened and kept running for up to its
                    // own 5-minute deadline regardless, long past
                    // useTerminalInstead()'s 20s backstop (which then
                    // reported a bogus "taking longer than expected"
                    // instead of ever actually opening a terminal). Checked
                    // in the loop condition AND right after the sleep,
                    // mirroring relogin()'s identical "opened" poll.
                    while (!ctx.isCancelled() && Date.now() < deadline && !authenticated) {
                        await sleep(2000);
                        if (ctx.isCancelled()) break;
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
                    if (ctx.isCancelled()) {
                        // The user explicitly switched away (e.g. "Use
                        // terminal instead", already opening its own
                        // terminal login) — silent, not an error: never
                        // fail silently (retro §5.1) still holds, but there
                        // is nothing wrong to report here, just a flow the
                        // user chose to leave.
                        return { kind: "ok" };
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
                        ctx.log("auth", "login complete — run /cost to verify");
                        // See finalizeLoginSuccess's doc comment for the
                        // active-turn / refresh-failure gating.
                        return await finalizeLoginSuccess(ctx);
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
                        if (outcome === "terminal-success") {
                            ctx.log("auth", "login complete — run /cost to verify");
                        }
                        // See finalizeLoginSuccess's doc comment for the
                        // active-turn / refresh-failure gating.
                        return await finalizeLoginSuccess(ctx);
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
        } finally {
            ctx.endRecoveryFlow();
        }
    },
};
