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

import { runProviderLogin } from "../../flows/run-provider-login";
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
            const outcome = await runProviderLogin({
                provider: prov,
                cliPath,
                authEnv,
                setAuthUrl: ctx.setAuthUrl,
                log: ctx.log,
                linkTarget: agentDefinitionId
                    ? { blockId: ctx.blockId, agentDefinitionId }
                    : undefined,
            });
            switch (outcome) {
                case "opened":
                    ctx.log("auth", "run /cost to verify authentication once logged in");
                    return { kind: "ok" };
                case "seeded":
                    return { kind: "ok" };
                case "terminal-success":
                    ctx.log("auth", "login complete — run /cost to verify");
                    return { kind: "ok" };
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
