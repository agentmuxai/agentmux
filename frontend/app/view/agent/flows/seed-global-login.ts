// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * seedGlobalLogin — "Use my existing Claude login": copy the user's already-
 * valid GLOBAL Claude credential into this agent's ISOLATED auth dir, instead
 * of driving a fresh OAuth.
 *
 * The reliable recovery for Claude Code v2.1.x, whose self-driving login TUI
 * the host can't scrape a URL from (SPEC_HOST_CLI_LOGIN_CAPTURE §5.5). The host
 * command `seed_provider_auth_from_global` validates the global credential's
 * expiry and copies it verbatim (incl. its `refreshToken`, so the isolated
 * session keeps refreshing) into `~/.agentmux/shared/providers/claude/`.
 *
 * Like force-login, no controller restart is needed: the running persistent
 * agent re-reads its credential per request, so the next message uses the
 * seeded token and clears the failure row.
 *
 * Returns true when a credential was seeded; false (with a logged reason) when
 * there's no global login or it's also expired — the caller should then fall
 * back to "Login Again".
 */

import { getApi } from "@/app/store/global";
import { sleep } from "@/util/async";
import type { LogFn } from "../types";

export async function seedGlobalLogin(
    providerId: string,
    log: LogFn,
    configDir?: string,
): Promise<boolean> {
    log("auth", "Use existing login — copying your global Claude login into this agent…");
    // `configDir` is the agent's resolved auth dir (from cmd:env). The host
    // seeds into it only if it's under ~/.agentmux; a stale ~/.claude is
    // rejected there and falls back to the shared dir — the seed never writes
    // the user's own login (SPEC_PROVIDER_ISOLATION §4.5 / INV-R).
    //
    // reagent P0: seed_provider_auth_from_global (providers.rs) hard-rejects
    // (throws) for every provider except claude — this function is
    // documented as Claude-specific in its own name, but nothing previously
    // stopped a caller from invoking it for another provider, and when that
    // happened the throw propagated as an unhandled rejection through every
    // caller (pollForGlobalLoginSeed's loop below included), instead of the
    // graceful "not seeded" this function's own return-false convention
    // implies. Callers should route non-claude providers elsewhere entirely
    // (see run-provider-login.ts's per-provider tier-3 strategy), but this
    // function must never throw regardless — it's a shared utility, and a
    // host command rejecting an entire class of input is a landmine for any
    // future caller, not just the ones already found.
    try {
        const res = await getApi().seedProviderAuthFromGlobal(providerId, configDir);
        if (res?.seeded) {
            log("auth", "copied your existing login — just send your message again");
            return true;
        }
        if (res?.status === "expired") {
            log("auth", "your global Claude login is also expired — use “Login Again” to re-authenticate", "warn");
        } else {
            log("auth", "no valid global Claude login found — use “Login Again” to authenticate", "warn");
        }
        return false;
    } catch (e: any) {
        log("auth", `seed-from-global failed: ${e?.message ?? String(e)}`, "warn");
        return false;
    }
}

/**
 * Poll `seedGlobalLogin` on an interval until it succeeds, the deadline
 * passes, or `isCancelled` reports true. Shared by "Login via terminal"
 * (useAgentControllerStatus) and the terminal-fallback tier of
 * `runProviderLogin` (retro-headless-login-browser-open-2026-07-20) — both
 * open a real terminal window for the user to complete an OAuth flow in,
 * then need to notice the credential landing on disk without the user
 * having to click anything else.
 */
export async function pollForGlobalLoginSeed(
    providerId: string,
    configDir: string | undefined,
    isCancelled: () => boolean,
    opts: { pollMs?: number; timeoutMs?: number } = {},
): Promise<boolean> {
    const pollMs = opts.pollMs ?? 5_000;
    const timeoutMs = opts.timeoutMs ?? 5 * 60 * 1_000;
    const deadline = performance.now() + timeoutMs;
    const silentLog: LogFn = () => {};
    while (performance.now() < deadline) {
        if (isCancelled()) return false;
        await sleep(pollMs);
        if (isCancelled()) return false;
        if (await seedGlobalLogin(providerId, silentLog, configDir)) return true;
    }
    return false;
}
