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

import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import { withLoginWindow } from "../../flows/login-window";
import { logInAndConfirm } from "../../flows/login-and-confirm";
import type { SlashCommand, SlashCommandContext, SlashResult } from "../types";

/**
 * Shared by both success branches below (tier-1 "opened" and tier-3
 * "terminal-success"): restart an already-running controller onto
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
    // The login's window opens inside the Enter that sent /login (its URL
    // comes later), and is closed on every way out if no URL used it
    // (login-window.ts).
    handler: (ctx) => withLoginWindow(ctx.provider(), () => runLogin(ctx)),
};

/** Never report success for a login that didn't complete
 *  (retro-agent-auth-relogin-noop-2026-07-01 §5.1). */
function notDetectedResult(stage: "link" | "inapp" | "terminal"): SlashResult {
    switch (stage) {
        case "link":
            return {
                kind: "error",
                message:
                    "/login: opened a login page, but no login was detected within 5 minutes. " +
                    "Complete the login there, then run /login again.",
            };
        case "terminal":
            return {
                kind: "error",
                message:
                    "/login: opened a terminal window, but no login was detected within 5 minutes. " +
                    "Complete the login there, then run /login again.",
            };
        case "inapp":
            // Another surface's login took the host's single login slot.
            return { kind: "ok" };
    }
}

async function runLogin(ctx: SlashCommandContext): Promise<SlashResult> {
    const prov = ctx.provider();
    const cliPath = ctx.block()?.meta?.["cmd"] ?? "";
    if (!prov || !cliPath) {
        return { kind: "error", message: "/login: provider or CLI path not available" };
    }
    // reagent P1 on PR #2413 (round 3, third pass): mirrors the
    // cancel-flag reset every OTHER login-starting function
    // (relogin()/loginViaTerminal()) already does at
    // its own start. /login never called any of those three, so a flag
    // left `true` by an EARLIER, unrelated cancelled attempt (e.g. the
    // shared AuthUrlBox's Cancel button during a prior relogin()) stayed
    // `true` forever — this fresh attempt's own isCancelled() poll
    // check below would then read as already-cancelled on entry and
    // silently report success without ever checking authentication.
    ctx.resetCancelled();
    ctx.log("auth", "running /login via GUI flow...");
    // Registers this attempt (including the up-to-5-minute poll below)
    // as an in-flight recovery on the SAME shared counter behind
    // loginWaiting() that relogin()/loginViaTerminal()
    // already use — without this, a second message sent while /login is
    // still polling gets held with authWasKnownBadAtQueueTime: false
    // (mid-turn "auth" failures never set canRetry either), so a /login
    // that ultimately fails flushes that held message straight to the
    // still-known-bad controller. Codex P1 on PR #2338 (ninth
    // re-review). Paired with the endRecoveryFlow() in this function's
    // own finally below — every return path (success, error, and the
    // catch) goes through it exactly once.
    // Declare this flow's recovery intent, the same way the failure row's
    // own buttons do — from the pending failure's `turnAttempted`.
    //
    // /login shows the same AuthUrlBox as relogin, so its session offers
    // the same "Use terminal instead". Without this, that escape read the
    // resting `false` and called onReady(), silently dropping a retry that
    // WAS owed when a real auth failure was pending — while clicking the
    // row's own "Login via terminal" in the identical state retried. Same
    // situation, opposite outcome, decided by which entry point the user
    // happened to use. Note this is also a behaviour CHANGE rather than a
    // pre-existing gap: before this PR that handler defaulted to true and
    // the retry did happen. manoz on PR #2951.
    //
    // Defaults false with no pending failure — /login is then a pure
    // credential operation with no turn owed a retry.
    ctx.beginRecoveryFlow(paneSnapshot(ctx.blockId)?.failure?.turnAttempted ?? false);
    try {
        const authEnv: Record<string, string> = {};
        const envMeta = ctx.block()?.meta?.["cmd:env"];
        if (envMeta && typeof envMeta === "object") {
            for (const [k, v] of Object.entries(envMeta)) {
                if (typeof v === "string") authEnv[k] = v;
            }
        }
        // The same login-and-confirm core as the pane's own "Login Again"
        // (flows/login-and-confirm.ts): tier 1 opens the link in the
        // AuthUrlBox, and a tier-1 "opened" is polled and persisted there.
        // linkTarget lets a new account bind to this agent
        // (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7).
        const agentDefinitionId = ctx.block()?.meta?.["agentId"] as string | undefined;
        const result = await logInAndConfirm({
            provider: prov,
            cliPath,
            authEnv,
            setAuthUrl: ctx.setAuthUrl,
            log: ctx.log,
            isCancelled: ctx.isCancelled,
            linkTarget: agentDefinitionId ? { blockId: ctx.blockId, agentDefinitionId } : undefined,
            // Behavior-gate only: no catalog provider sets the flag since
            // SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.2 dropped it for
            // Claude, so /login runs the in-app tier 1 for Claude too.
            skipTier1: prov.headlessLoginUrlUnsupported === true,
        });
        switch (result.kind) {
            case "signed-in":
                ctx.log("auth", "login complete — run /cost to verify");
                // See finalizeLoginSuccess's doc comment for the
                // active-turn / refresh-failure gating.
                return await finalizeLoginSuccess(ctx);
            case "save-failed":
                // The credential landed but no account row backs it: the
                // spawn gate would still block the next turn.
                return {
                    kind: "error",
                    message:
                        "/login: the login succeeded, but AgentMux couldn't save the account record. Try again in a moment.",
                };
            case "not-detected":
                // The user left the flow (Cancel, "Use terminal instead", or
                // closing the pane): nothing wrong to report.
                if (ctx.isCancelled()) return { kind: "ok" };
                return notDetectedResult(result.stage);
            case "unavailable":
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
}
