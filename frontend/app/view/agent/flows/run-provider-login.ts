// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * runProviderLogin — the SOLE entry point for triggering a provider login
 * anywhere in the app: `/login`, the "Login Again" failure-banner action,
 * and the gated launch flow's auto-login-on-open (which also backs the
 * "Retry Login" button).
 *
 * `getApi().runCliLogin` — the raw host primitive this wraps — must NOT be
 * called directly from anywhere else. It was, once: `launch-flow.ts` had its
 * own independent call + hand-rolled "no URL captured" handling that
 * silently diverged from this file's, and kept a stuck-forever "Working…"
 * spinner alive for the single most common way a user hits this (opening
 * any pane while unauthenticated) even after `/login` and "Login Again"
 * were fixed. See retro-login-three-code-paths-2026-07-20. A test
 * (`run-cli-login-single-caller.test.ts`) pins that raw primitive to exactly
 * one call site — this file's `forceProviderLogin` import — so a new direct
 * caller fails a test instead of silently reintroducing that gap. If you're
 * adding a new UI surface that can trigger a login, call `runProviderLogin`,
 * not `forceProviderLogin` and not `getApi().runCliLogin`.
 *
 * Three tiers, each tried only if the previous one couldn't complete:
 *
 *   1. `forceProviderLogin` — spawn the CLI headless/piped and scrape a
 *      login URL from its output. Fast, and still the right answer for any
 *      provider whose CLI prints one. Claude Code v2.1.x never does: its
 *      OAuth flow opens the browser itself from inside the CLI process, and
 *      a piped/PTY spawn has no attached console for that call to succeed
 *      from — see retro-headless-login-browser-open-2026-07-20 §1.
 *   2. `seedGlobalLogin` + `persistSeededAccount` — Claude only: check
 *      whether the user already has a VALID login in their global
 *      `~/.claude` (common: the CLI installed outside AgentMux, or a prior
 *      AgentMux session). If so, mint a real per-account isolated dir, copy
 *      the credential into it, and persist an IdentityAccount row — not
 *      just a file in the shared default dir
 *      (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7, "single
 *      point, not global": `identity/resolver.rs`'s spawn gate now requires
 *      a real bound account, no ambient exception). No browser, no user
 *      action, completes in well under a second. Skipped for every other
 *      provider — the host command rejects them (`providers.rs`'s
 *      `seed_provider_auth_from_global`), and unlike tier 3 there's no
 *      substitute strategy for tier 2 specifically; they just fall through.
 *   3. `openLoginTerminal` — last resort, same real per-account dir minted
 *      up front for EVERY oauth-class provider (not just Claude). Two
 *      different completion strategies depending on the provider, since
 *      only Claude has a seed-from-global capability to fall back on:
 *        - **Claude**: the dir's env var is stripped so the login lands in
 *          the user's GLOBAL `~/.claude` instead, then polled via
 *          `pollForGlobalLoginSeed` until it copies into the isolated dir.
 *        - **Every other oauth-class provider** (codex, openclaw, gemini,
 *          copilot): the env var is left pointed AT the isolated dir, so
 *          the login writes there directly, then polled via
 *          `pollForCliAuthReady` (asks the CLI's own auth-check command
 *          whether it's authenticated in that dir) since there's no
 *          global-login file to watch for. Persisted on success either way
 *          — same "single point" account. Needs the user to actually
 *          finish the OAuth flow in their browser, so this tier polls for
 *          up to 5 minutes instead of returning immediately.
 *
 * Before this existed, tier 1 failing meant `/login` and "Login Again" both
 * dead-ended on an error message telling the user to go click a *different*
 * button ("Login via terminal") themselves. Tiers 2 and 3 are exactly what
 * that other button already did — this just tries it automatically instead
 * of requiring the user to know which button fixes a URL-less CLI.
 */

import { getApi } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import * as WOS from "@/app/store/wos";
import { forceProviderLogin, type ForceLoginParams } from "./force-login";
import { pollForGlobalLoginSeed, seedGlobalLogin } from "./seed-global-login";
import { ensureAccountDir, persistSeededAccount } from "./register-seeded-account";

export interface RunProviderLoginParams extends ForceLoginParams {
    provider: ForceLoginParams["provider"] & {
        id: string;
        authConfigDirEnvVar: string;
        authCheckCommand: string[];
    };
    /** Polled during the tier-3 wait; return true to abort early (e.g. the user hit Cancel). */
    isCancelled?: () => boolean;
    /** When set, a newly-registered account (tier 2 or 3) is linked to this
     *  agent definition and the block's `cmd:env` is updated to point at
     *  its isolated dir — the pane-level recovery case (an already-running
     *  agent). Omit for a pre-launch flow with no agent yet; that flow's own
     *  launch-time reconcile links the account once one is created. */
    linkTarget?: { blockId: string; agentDefinitionId: string };
    /** Reconnect (not fresh-connect) into this account id, if set — threaded
     *  through to tier 2/3's account-dir minting so the SAME account's
     *  isolated dir is reused/refreshed instead of a new one being minted.
     *  Omit for a genuinely fresh connect. Callers that already know this
     *  agent has a bound account for this provider (e.g. a retry after a
     *  failed login) should always pass it — otherwise every retry mints
     *  and orphans a brand-new account instead of refreshing the one
     *  already in use. */
    existingAccountId?: string;
    /** Fired as soon as tier 2 or 3 registers a real IdentityAccount row —
     *  before `linkTarget`'s own linking (if any). Callers that need to know
     *  the resulting account id/dir for their own purposes (e.g. rebuilding
     *  a local `authEnv` copy to recheck auth status against the NEW
     *  isolated dir, since `finalizeAccount` only updates the persisted
     *  block meta — a caller's own in-memory `authEnv` variable is never
     *  refreshed by this function) should use this instead of threading a
     *  new return shape through `runProviderLogin` — `linkTarget`-driven
     *  pane callers that don't need the value don't have to care it exists. */
    onAccountRegistered?: (accountId: string, dir: string) => void;
    /** Skip tier 1 (headless URL-capture) entirely and go straight to tier 2.
     *  For providers where tier 1 is a documented, unconditional dead end —
     *  e.g. `requiresLoginTty` providers, whose CLI opens its own browser
     *  in-process and needs a real console no piped/PTY spawn has — skipping
     *  avoids a pointless ~15s wait for an attempt that cannot succeed.
     *  Also the right default for a caller whose UI already explicitly says
     *  "login via terminal" (the user has already opted into a real console,
     *  no point trying headless first). Default false — existing callers
     *  are unaffected. */
    skipTier1?: boolean;
}

export type ProviderLoginOutcome =
    | "opened" // tier 1: browser/pane opened with a captured URL
    | "seeded" // tier 2: valid global login copied into a real account, automatically
    | "terminal-success" // tier 3: terminal login completed and was detected
    | "terminal-timeout" // tier 3: terminal opened, but no login within 5 min (or cancelled)
    | "terminal-unavailable"; // tier 3 itself couldn't open (e.g. unsupported platform)

async function finalizeAccount(
    p: RunProviderLoginParams,
    accountId: string,
    dir: string,
): Promise<void> {
    if (!p.linkTarget) return;
    try {
        await RpcApi.LinkAgentIdentityCommand(TabRpcClient, {
            agent_id: p.linkTarget.agentDefinitionId,
            account_id: accountId,
            provider: p.provider.id,
        });
    } catch (e: any) {
        p.log(
            "auth",
            `account created but couldn't be linked to this agent: ${e?.message ?? String(e)}`,
            "warn",
        );
        return;
    }
    try {
        const oref = WOS.makeORef("block", p.linkTarget.blockId);
        await RpcApi.SetMetaCommand(TabRpcClient, {
            oref,
            meta: { "cmd:env": { ...p.authEnv, [p.provider.authConfigDirEnvVar]: dir } },
        });
    } catch {
        // Best-effort — the account is linked either way; the next full
        // resolve (relaunch, or the resolver's own per-turn injection) will
        // still find it even if this specific pane's cached env is stale.
    }
}

/** Poll `CheckCliAuthCommand` against a login that was told to write
 *  DIRECTLY into `authEnv`'s isolated dir, until it reports authenticated,
 *  the deadline passes, or `isCancelled` reports true. The provider-agnostic
 *  sibling of `pollForGlobalLoginSeed` (seed-global-login.ts) — used for
 *  every oauth-class provider OTHER than Claude, which has no seed-from-
 *  global capability at all (`seed_provider_auth_from_global` hard-rejects
 *  every provider but claude — providers.rs) and so must detect completion
 *  by asking the CLI itself whether it's authenticated in the dir the login
 *  was pointed at directly, rather than by watching a global dir get copied. */
async function pollForCliAuthReady(
    cliPath: string,
    authCheckArgs: string[],
    authEnv: Record<string, string>,
    isCancelled: () => boolean,
    opts: { pollMs?: number; timeoutMs?: number } = {},
): Promise<boolean> {
    const pollMs = opts.pollMs ?? 5_000;
    const timeoutMs = opts.timeoutMs ?? 5 * 60 * 1_000;
    const deadline = performance.now() + timeoutMs;
    while (performance.now() < deadline) {
        if (isCancelled()) return false;
        await new Promise<void>((r) => setTimeout(r, pollMs));
        if (isCancelled()) return false;
        try {
            const result = await RpcApi.CheckCliAuthCommand(
                TabRpcClient,
                { cli_path: cliPath, auth_check_args: authCheckArgs, auth_env: authEnv },
                { timeout: 10000 },
            );
            if (result.authenticated) return true;
        } catch {
            // keep polling on transient RPC errors
        }
    }
    return false;
}

export async function runProviderLogin(p: RunProviderLoginParams): Promise<ProviderLoginOutcome> {
    if (!p.skipTier1) {
        const tier1 = await forceProviderLogin(p);
        if (tier1 === "opened") return "opened";
    }

    // Tier 1's login CLI child (piped/PTY, spawned by forceProviderLogin's
    // getApi().runCliLogin) is left running/abandoned when it doesn't
    // produce a URL within its own timeout — cancel it before tier 2/3
    // potentially spawn a second, concurrent login CLI process against the
    // same config dir. cancelCliLogin is idempotent and host-side (safe to
    // call even if nothing is running — see useAgentControllerStatus.ts's
    // and launch-flow.ts's existing best-effort uses of the same call).
    await getApi().cancelCliLogin().catch(() => {});

    // Mint the account dir ONCE, up front, for EVERY oauth-class provider —
    // not just Claude. Account minting itself (ensureAccountDir /
    // persistSeededAccount) has always been provider-agnostic; the backend
    // RPC it calls (identity.ensureaccountdir) already gates on the real
    // oauth-class check (resolver::provider_class), so a non-oauth-class id
    // reaching this function just gets `null` back here, same as before.
    // A prior version of this code additionally gated the CALL on
    // `provider.id === "claude"` — reagent P0: that meant a successful
    // tier-3 terminal login for codex/openclaw never minted or persisted a
    // real IdentityAccount at all, so the agent reported "Login successful"
    // but stayed permanently blocked by the resolver's unconditional
    // oauth-class spawn gate on its very next spawn.
    const minted = await ensureAccountDir(p.provider.id, p.log, p.existingAccountId);

    // Claude-only: seed-from-global. seed_provider_auth_from_global
    // hard-rejects every other provider server-side, so this tier is
    // structurally Claude-specific — not a coverage gap for the others,
    // just a different tier-3 completion strategy for them, below.
    if (minted && p.provider.id === "claude") {
        p.log("auth", "no login URL captured — checking for an existing global Claude login…");
        if (await seedGlobalLogin(p.provider.id, p.log, minted.dir)) {
            // The credential is now valid and sitting in minted.dir — a
            // persist failure here is a bookkeeping problem, not an auth
            // one, and is usually transient (a momentary RPC hiccup). One
            // retry (reagent P2) avoids silently falling through to
            // "opening a terminal window for a fresh login…" for a login
            // that already succeeded, which just confuses the user into
            // thinking they still need to do something.
            let persisted = await persistSeededAccount(p.provider.id, minted.accountId, minted.dir, p.log);
            if (!persisted) {
                p.log("auth", "account registration failed — retrying once…", "warn");
                persisted = await persistSeededAccount(p.provider.id, minted.accountId, minted.dir, p.log);
            }
            if (persisted) {
                p.onAccountRegistered?.(minted.accountId, minted.dir);
                await finalizeAccount(p, minted.accountId, minted.dir);
                return "seeded";
            }
            p.log(
                "auth",
                "your login succeeded, but AgentMux couldn't save the account record — try again in a moment",
                "error",
            );
        }
    }

    p.log("auth", "opening a terminal window for a fresh login…");

    const terminalEnv: Record<string, string> = { ...p.authEnv };
    const isClaude = p.provider.id === "claude";
    if (isClaude) {
        // Claude: strip the isolated dir's env var so the login lands in
        // the user's GLOBAL ~/.claude instead — seed_provider_auth_from_
        // global then copies it into the isolated dir once it lands there
        // (poll below). This is the ONLY provider with that copy-back path.
        delete terminalEnv[p.provider.authConfigDirEnvVar];
    } else if (minted) {
        // Every other oauth-class provider has no seed-from-global
        // capability to fall back on — instead, let the login write
        // DIRECTLY into the isolated dir by keeping the env var pointed at
        // it, and detect completion by asking the CLI itself (below).
        terminalEnv[p.provider.authConfigDirEnvVar] = minted.dir;
    }

    try {
        await getApi().openLoginTerminal(p.cliPath, p.provider.authLoginCommand, terminalEnv);
    } catch (err: any) {
        p.log("auth", `couldn't open a terminal for login: ${err?.message ?? String(err)}`, "error");
        return "terminal-unavailable";
    }
    p.log("auth", "a terminal window opened — complete the login there");

    const isCancelled = p.isCancelled ?? (() => false);
    if (isClaude) {
        const configDir = minted?.dir ?? p.authEnv[p.provider.authConfigDirEnvVar];
        const seeded = await pollForGlobalLoginSeed(p.provider.id, configDir, isCancelled);
        if (!seeded) return "terminal-timeout";
    } else if (minted) {
        const ready = await pollForCliAuthReady(p.cliPath, p.provider.authCheckCommand, terminalEnv, isCancelled);
        if (!ready) return "terminal-timeout";
    } else {
        // Not oauth-class (or the dir mint itself failed) — nothing to poll
        // for and nothing to persist; the terminal opened, but there's no
        // way to detect or register a completed login for this call.
        return "terminal-timeout";
    }

    if (minted) {
        if (await persistSeededAccount(p.provider.id, minted.accountId, minted.dir, p.log)) {
            p.onAccountRegistered?.(minted.accountId, minted.dir);
            await finalizeAccount(p, minted.accountId, minted.dir);
        }
    }
    return "terminal-success";
}
