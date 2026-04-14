// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * /login — run a GUI OAuth flow via the host API, capture the returned
 * URL, and push it into ctx.setAuthUrl so the auth box appears above
 * the composer.
 *
 * Migrated from useAgentCommands.runLoginCommand (PR #378 era). Unlike
 * pure commands, /login logs progress directly via ctx.log because the
 * auth flow is multi-step and uses an "auth"-level log channel that
 * the dispatcher's single-result formatter doesn't model. This is
 * intentional — centralized result logging is the common case; /login
 * is the exception. See spec §4.4 dispatcher note.
 */

import { getApi } from "@/app/store/global";
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
            const url = await getApi().runCliLogin(cliPath, prov.authLoginCommand, authEnv);
            if (url) {
                ctx.setAuthUrl(url);
                ctx.log("auth", "OAuth URL captured — browser should open automatically");
                ctx.log("auth", "if it didn't, copy the URL from the box above");
            } else {
                ctx.log("auth", "a browser window should have opened — complete login there");
            }
            ctx.log("auth", "run /cost to verify authentication once logged in");
            return { kind: "ok" };
        } catch (err: any) {
            return { kind: "error", message: `/login failed: ${err?.message ?? String(err)}` };
        }
    },
};
