// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * forceProviderLogin — run a provider OAuth login UNCONDITIONALLY, bypassing
 * the `CheckCliAuth` status check.
 *
 * Why bypass the check: `claude auth status` (and equivalents) only confirm a
 * credential is PRESENT, not that it's still VALID. An expired/revoked token
 * still reports "authenticated", so the agent 401s on every real API call while
 * the status check insists everything is fine (the false-positive that made
 * "Login Again" do nothing — SPEC_REAUTH_FROM_AUTH_ERROR §11). When the user
 * explicitly asks to re-login (the failure banner / inline-error "Login Again",
 * or the `/login` command) we KNOW the token is bad, so we must force a fresh
 * OAuth instead of trusting the check.
 *
 * Opens the OAuth in the system browser (with an in-app browser pane as a
 * fallback) and surfaces the URL via `setAuthUrl` so the auth box appears above
 * the composer with the URL + paste-the-code input. The running persistent
 * agent re-reads its credential per request, so it picks up the fresh token on
 * the next message — no controller restart required.
 *
 * Deliberately does NOT run the `CheckCliAuth` success-poll the gated launch
 * flow uses: with a present-but-expired token the poll would "succeed" on the
 * first tick and reap the in-flight login CLI before the user finishes.
 */

import { getApi } from "@/app/store/global";
import { openOAuthBrowserPane } from "./open-oauth-pane";
import type { ProviderDefinition } from "../providers";
import type { LogFn } from "../types";

export interface ForceLoginParams {
    provider: Pick<ProviderDefinition, "authLoginCommand" | "requiresLoginTty">;
    /** Resolved CLI path (from block meta `cmd`, set at launch). */
    cliPath: string;
    /** Auth env (e.g. CLAUDE_CONFIG_DIR) — from block meta `cmd:env`. */
    authEnv: Record<string, string>;
    setAuthUrl: (url: string | null) => void;
    log: LogFn;
}

/**
 * Outcome of a forced login attempt. "no-url" means the CLI produced no
 * scrapeable OAuth URL — nothing was opened, and the CALLER must surface a
 * user-visible error pointing at the reliable recovery paths (the silent
 * warn-only branch here is how "Login Again" became a dead button —
 * retro-agent-auth-relogin-noop-2026-07-01 §5.1).
 */
export type ForceLoginOutcome = "opened" | "no-url";

export async function forceProviderLogin(p: ForceLoginParams): Promise<ForceLoginOutcome> {
    const { provider, cliPath, authEnv, setAuthUrl, log } = p;
    log("auth", "re-login: forcing a fresh OAuth (bypassing the auth-status check)…");

    const url = await getApi().runCliLogin(
        cliPath,
        provider.authLoginCommand,
        authEnv,
        provider.requiresLoginTty ?? false,
    );

    if (url) {
        setAuthUrl(url);
        const opened = await openOAuthBrowserPane(url);
        if (opened === "pane") {
            log("auth", "opened login in an in-app browser pane — complete login there");
        } else if (opened === "external") {
            log("auth", "opened login in your system browser — complete login there");
        } else {
            log("auth", "could not open a browser; copy the URL from the box above and open it manually", "warn");
        }
        log("auth", "after you finish, just send your message again — the agent will use the new token");
        return "opened";
    }
    // No URL captured — the CLI either crashed at spawn or runs a login TUI
    // that prints no parseable URL (e.g. Claude Code ≤2.1.183; the pinned
    // 2.1.198+ DOES print one — SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md
    // §2 — so hitting this for Claude today means an older/odd CLI, and the
    // behavior-gate falls through to runProviderLogin's tiers 2/3). Nothing
    // opened.
    log("auth", "no login URL captured from the CLI — nothing was opened", "warn");
    return "no-url";
}
