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
 *   2. `seedGlobalLogin` + `persistSeededAccount` — if no URL was captured,
 *      check whether the user already has a VALID login in their global
 *      `~/.claude` (common:
 *      the CLI installed outside AgentMux, or a prior AgentMux session). If
 *      so, mint a real per-account isolated dir, copy the credential into
 *      it, and persist an IdentityAccount row — not just a file in the
 *      shared default dir (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md
 *      §7, "single point, not global": `identity/resolver.rs`'s spawn gate
 *      now requires a real bound account, no ambient exception). No
 *      browser, no user action, completes in well under a second.
 *      Claude-only (the host command rejects other providers —
 *      `providers.rs`'s `seed_provider_auth_from_global`), so this tier is
 *      skipped for everything else.
 *   3. `openLoginTerminal` — last resort: mint the same kind of per-account
 *      dir, spawn the login command in a REAL visible console window (which
 *      gives the CLI's own browser-open call something to attach to) with
 *      that dir's env var stripped so the login lands in the user's GLOBAL
 *      `~/.claude`, then poll for it to land — same "single point" account
 *      persisted on success. Needs the user to actually finish the OAuth
 *      flow in their browser, so this tier polls for up to 5 minutes
 *      instead of returning immediately.
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
    provider: ForceLoginParams["provider"] & { id: string; authConfigDirEnvVar: string };
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
     *  Omit for a genuinely fresh connect. */
    existingAccountId?: string;
    /** Fired as soon as tier 2 or 3 registers a real IdentityAccount row —
     *  before `linkTarget`'s own linking (if any). Callers that need to know
     *  the resulting account id for their own purposes (e.g. the New Agent
     *  modal selecting the newly-created account in its dropdown) should use
     *  this instead of threading a new return shape through `runProviderLogin`
     *  — `linkTarget`-driven pane callers don't need it and shouldn't have to
     *  care that it exists. */
    onAccountRegistered?: (accountId: string, dir: string) => void;
    /** Skip tier 1 (headless URL-capture) entirely and go straight to tier 2.
     *  For providers where tier 1 is a documented, unconditional dead end —
     *  e.g. `requiresLoginTty` providers, whose CLI opens its own browser
     *  in-process and needs a real console no piped/PTY spawn has — skipping
     *  avoids a pointless ~15s wait for an attempt that cannot succeed, and
     *  (for callers that need a completion signal, which tier 1 alone doesn't
     *  provide) keeps the "opened, now what?" case structurally unreachable
     *  rather than something every caller has to defend against. Default
     *  false — existing callers are unaffected. */
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

    // Mint the account dir ONCE, up front (Claude only; other providers
    // have no seed-from-global detection path at all) so tier 2 and tier 3
    // share the SAME account instead of each minting its own — minting
    // twice left an orphaned, unpersisted account dir behind on disk
    // whenever tier 2's seed step failed partway through (ensureAccountDir
    // succeeded, seedGlobalLogin didn't). existingAccountId threads through
    // so a Reconnect (not a fresh Connect) refreshes the SAME account's dir.
    const minted = p.provider.id === "claude" ? await ensureAccountDir(p.provider.id, p.log, p.existingAccountId) : null;

    if (minted && p.provider.id === "claude") {
        p.log("auth", "no login URL captured — checking for an existing global Claude login…");
        if (await seedGlobalLogin(p.provider.id, p.log, minted.dir)) {
            if (await persistSeededAccount(p.provider.id, minted.accountId, minted.dir, p.log)) {
                p.onAccountRegistered?.(minted.accountId, minted.dir);
                await finalizeAccount(p, minted.accountId, minted.dir);
                return "seeded";
            }
        }
    }

    p.log("auth", "opening a terminal window for a fresh login…");
    const configDir = minted?.dir ?? p.authEnv[p.provider.authConfigDirEnvVar];

    const terminalEnv: Record<string, string> = { ...p.authEnv };
    delete terminalEnv[p.provider.authConfigDirEnvVar];
    try {
        await getApi().openLoginTerminal(p.cliPath, p.provider.authLoginCommand, terminalEnv);
    } catch (err: any) {
        p.log("auth", `couldn't open a terminal for login: ${err?.message ?? String(err)}`, "error");
        return "terminal-unavailable";
    }
    p.log("auth", "a terminal window opened — complete the login there");

    const isCancelled = p.isCancelled ?? (() => false);
    const seeded = await pollForGlobalLoginSeed(p.provider.id, configDir, isCancelled);
    if (!seeded) return "terminal-timeout";

    if (minted) {
        if (await persistSeededAccount(p.provider.id, minted.accountId, minted.dir, p.log)) {
            p.onAccountRegistered?.(minted.accountId, minted.dir);
            await finalizeAccount(p, minted.accountId, minted.dir);
        }
    }
    return "terminal-success";
}
