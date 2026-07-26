// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentControllerStatus — owns all the agent-launch state and the
 * functions that drive it.
 *
 * Step 4 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * State managed:
 *   - authUrl       — the OAuth URL to display when login is needed
 *   - canRetry      — whether the retry button is shown after auth_failed
 *   - flowRunning   — true while the launch flow is actively executing
 *   - agentReady    — true once the controller is registered and ready
 *   - isLoading     — derived: flowRunning OR not agentReady
 *   - loginWaiting  — true during the OAuth poll phase
 *
 * Mutable internal state:
 *   - loginCancelled — flipped by cancelLogin() and by onCleanup
 *
 * Functions exposed:
 *   - startLaunchFlow() — runs the full launch flow once
 *   - cancelLogin()     — flips the cancellation flag and kills the host CLI
 *
 * Dependencies (passed as options):
 *   - blockId   — the block this status belongs to
 *   - provider  — accessor returning the current provider definition
 *   - log       — LogFn from useActivityLog (or any compatible)
 *
 * `setAuthUrl` is exposed alongside the accessor because the slash-command
 * `/login` handler in agent-view.tsx needs to manually set the OAuth URL
 * when the user types the command directly (separate from the launch
 * flow's auto-login phase).
 */

import { createMemo, createSignal, onCleanup, type Accessor } from "solid-js";
import { getApi, getBlockMetaKeyAtom } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import * as WOS from "@/app/store/wos";
import { TabRpcClient } from "@/app/store/rpc-util";
import { runLaunchFlow } from "../flows/launch-flow";
import { persistAndLinkAccount, runProviderLogin } from "../flows/run-provider-login";
import { registerSeededAccount } from "../flows/register-seeded-account";
import { LOGIN_LINK_CAPTURE_LABEL_MS, type LaunchPhase } from "../flows/launch-phase";
import type { ProviderDefinition } from "../providers";

import type { LogFn } from "../types";
export type { LogFn };

export interface UseAgentControllerStatusOptions {
    blockId: string;
    provider: Accessor<ProviderDefinition | undefined>;
    log: LogFn;
    onLoginSuccess?: (email: string | null) => void;
    /** Forwarded to the launch flow — see `LaunchFlowOptions.onNotify`. */
    onNotify?: (text: string, style: "info" | "warning") => void;
    /** Called once when the launch flow completes successfully and the agent is ready to receive messages. */
    onReady?: () => void;
    /**
     * Called when a recovery action (seed-from-global / terminal login) has
     * successfully refreshed the credential. The pane wires this to retry the
     * failed turn, so a single "Use existing login" click both fixes the
     * credential AND drives the agent back to a working state — without it, the
     * seed succeeds silently and the 401 failure row lingers (the "Login Again
     * does nothing" symptom).
     */
    onRecovered?: () => void;
    /**
     * Returns the pane's current `{rows, cols}` (or undefined if not laid out
     * yet). Forwarded to the launch flow to seed the PTY size at spawn.
     */
    getInitialTermSize?: () => { rows: number; cols: number } | undefined;
    /** Forwarded to the launch flow — see `LaunchFlowOptions.onControllerStatus`. */
    onControllerStatus?: (rts: BlockControllerRuntimeStatus) => void;
}

export interface UseAgentControllerStatus {
    authUrl: Accessor<string | null>;
    setAuthUrl: (url: string | null) => void;
    /**
     * User-visible auth-recovery notice, rendered as an error box above the
     * composer (same surface as the auth-URL box). Set when a recovery action
     * fails in a way the user must react to — e.g. "Login Again" captured no
     * OAuth URL so nothing opened (retro-agent-auth-relogin-noop-2026-07-01
     * §5.1: never fail silently). Cleared on the next recovery attempt.
     */
    authNotice: Accessor<string | null>;
    setAuthNotice: (notice: string | null) => void;
    canRetry: Accessor<boolean>;
    flowRunning: Accessor<boolean>;
    agentReady: Accessor<boolean>;
    isLoading: Accessor<boolean>;
    loginWaiting: Accessor<boolean>;
    /** What the flow is doing right now, for a specific footer label instead
     *  of a generic "Working…" — see launch-phase.ts. */
    launchPhase: Accessor<LaunchPhase | null>;
    startLaunchFlow: () => Promise<void>;
    /**
     * Force a provider re-login, bypassing the auth-status check. Wired to the
     * failure-banner / inline-error "Login Again" actions: a 401 means the token
     * is bad even though `CheckCliAuth` still reports it present, so re-running
     * the gated launch flow would trust the lying check and skip the very login
     * the user needs. This always opens the OAuth. See
     * SPEC_REAUTH_FROM_AUTH_ERROR §11.
     */
    relogin: () => Promise<void>;
    /**
     * "Use my existing login" — seed this agent's isolated auth dir from the
     * user's already-valid GLOBAL Claude login instead of a fresh OAuth, the
     * reliable recovery for Claude Code v2.1.x's un-scrapeable login TUI
     * (SPEC_HOST_CLI_LOGIN_CAPTURE §5.5). Like relogin, no restart is needed —
     * the running agent re-reads its credential per request.
     */
    useGlobalLogin: () => Promise<void>;
    /**
     * Open a real terminal window (CREATE_NEW_CONSOLE on Windows) running the
     * provider's login command so the OS can open the browser — the piped/PTY
     * paths that `runCliLogin` uses are headless and block the browser. After
     * spawning, polls every 5s for up to 5 minutes; once credentials appear,
     * seeds a real per-account isolated dir and registers/links the account
     * (not just a file — PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7).
     */
    loginViaTerminal: () => Promise<void>;
    cancelLogin: () => void;
    /**
     * Clear stale auth-recovery UI (the "Retry Login" bar / any lingering
     * `authNotice`) once we have independent proof the controller is
     * healthy. `canRetry`/`authNotice` are set once, from the mount-time
     * gated launch flow or an explicit recovery attempt, and nothing
     * previously reset them if the agent later became healthy through a
     * DIFFERENT path — e.g. a `controllerstatus` event proving the CLI is
     * alive and running turns, which is exactly what
     * `useControllerStatusEvents`'s continuous `onTurnActive` already
     * tracks for `TurnPhase` reconciliation (Agent1/Agent2 stuck-"Working"
     * incidents). Call this from that same signal so "Retry Login" can't
     * outlive the failure it was reporting — a leak reported live: an
     * agent recovered and was answering messages, but the initial
     * auth_failed launch had left the button stuck showing.
     */
    notifyControllerHealthy: () => void;
}

/**
 * Build the auth env vars (CLAUDE_CONFIG_DIR etc.) from a provider
 * definition. Async because it ensures the auth dir exists on disk via
 * the host API. Returns undefined on any failure (non-fatal — falls back
 * to whatever the host's default state is).
 */
async function buildAuthEnv(
    prov: ProviderDefinition | undefined,
): Promise<Record<string, string> | undefined> {
    if (!prov?.authConfigDirEnvVar || !prov?.authDirName) return undefined;
    try {
        const authDir = await getApi().ensureAuthDir(prov.id);
        const env: Record<string, string> = { [prov.authConfigDirEnvVar]: authDir };
        if (prov.authExtraEnv) Object.assign(env, prov.authExtraEnv);
        return env;
    } catch {
        return undefined; // non-fatal — fall back to default auth dir
    }
}

export function useAgentControllerStatus(
    opts: UseAgentControllerStatusOptions,
): UseAgentControllerStatus {
    const [authUrl, setAuthUrl] = createSignal<string | null>(null);
    const [authNotice, setAuthNotice] = createSignal<string | null>(null);
    const [canRetry, setCanRetry] = createSignal(false);
    const [flowRunning, setFlowRunning] = createSignal(false);
    const [agentReady, setAgentReady] = createSignal(false);
    const [loginWaiting, setLoginWaiting] = createSignal(false);
    // What the flow is actually doing right now — see launch-phase.ts. Lets
    // AgentFooter show a specific status (and, for timed phases, a "waiting
    // on X, up to Ys" label) instead of a generic "Working…" for every phase.
    const [launchPhase, setLaunchPhase] = createSignal<LaunchPhase | null>(null);

    // Derived spinner state — caller wires this into the AgentFooter loading prop
    const isLoading = createMemo(() => flowRunning() || !agentReady());

    // Mutable cancellation flag. Flipped by cancelLogin() and by onCleanup.
    // Read inside startLaunchFlow's polling loop via the isCancelled callback.
    let loginCancelled = false;

    const startLaunchFlow = async () => {
        if (flowRunning()) return;
        loginCancelled = false;
        setFlowRunning(true);
        setCanRetry(false);
        // Any fresh launch/retry supersedes a prior recovery attempt — clear a
        // stale authNotice (e.g. "no login URL captured" from an earlier
        // relogin) so it can't linger past a "Retry Login" the user just
        // clicked and mislead them about this attempt's outcome.
        setAuthNotice(null);
        setLaunchPhase(null);
        const prov = opts.provider();
        try {
            const authEnv = await buildAuthEnv(prov);
            const result = await runLaunchFlow({
                blockId: opts.blockId,
                provider: prov,
                log: opts.log,
                setAuthUrl,
                isCancelled: () => loginCancelled,
                setLoginWaiting,
                setLaunchPhase,
                authEnv,
                onLoginSuccess: opts.onLoginSuccess,
                onNotify: opts.onNotify,
                getInitialTermSize: opts.getInitialTermSize,
                onControllerStatus: opts.onControllerStatus,
            });
            if (result === "success") {
                setAgentReady(true);
                opts.onReady?.();
            } else if (result === "auth_failed" && !loginCancelled) {
                setCanRetry(true);
                setAgentReady(true); // clear spinner so retry button is usable
            }
        } catch (err: any) {
            opts.log("error", err?.message ?? String(err), "error");
            setAgentReady(true); // clear spinner on error
        } finally {
            setFlowRunning(false);
            setLaunchPhase(null);
        }
    };

    // Guard against double-firing while a re-login's runCliLogin RPC is in
    // flight (the user double-clicks "Login Again"). Separate from flowRunning
    // — relogin must work even when the gated flow believes it already finished.
    let reloginInFlight = false;

    /**
     * Resolve the provider CLI directly when block meta has no `cmd` yet.
     * The old fallback here ran the GATED launch flow — which trusts
     * `CheckCliAuth`'s expired-but-present false positive and skips login,
     * degrading "Login Again" into exactly the no-op it exists to bypass
     * (retro-agent-auth-relogin-noop-2026-07-01 H2). Resolving the CLI and
     * proceeding with the forced login keeps the user's explicit intent.
     * Returns null (with a visible notice) if resolution fails.
     */
    const resolveCliForRecovery = async (prov: ProviderDefinition, action: string): Promise<string | null> => {
        opts.log("auth", `${action}: CLI path not in block meta — resolving it directly`, "warn");
        try {
            const r = await RpcApi.ResolveCliCommand(TabRpcClient, {
                provider_id: prov.id,
                cli_command: prov.cliCommand,
                npm_package: prov.npmPackage,
                pinned_version: prov.pinnedVersion,
                windows_install_command: prov.windowsInstallCommand,
                unix_install_command: prov.unixInstallCommand,
                block_id: opts.blockId,
            }, { timeout: 300000 });
            // Persist the resolved path back to block meta (mirrors
            // launch-flow.ts) so a subsequent recovery click reuses it instead
            // of re-running the full resolution RPC. Best-effort — a failed
            // write just means the next click re-resolves.
            try {
                const oref = WOS.makeORef("block", opts.blockId);
                await RpcApi.SetMetaCommand(TabRpcClient, { oref, meta: { cmd: r.cli_path } });
            } catch { /* non-fatal: next click re-resolves */ }
            return r.cli_path;
        } catch (err: any) {
            const msg = `Couldn't find the ${prov.cliCommand} CLI to run the login: ${err?.message ?? String(err)}`;
            opts.log("auth", msg, "error");
            setAuthNotice(msg);
            return null;
        }
    };

    /** Look up the account already bound to THIS agent for `providerId`, if
     *  any — pass as `runProviderLogin`'s `existingAccountId` so a recovery
     *  action reuses/refreshes the same account instead of minting and
     *  orphaning a new one on every retry (the same class of gap reagent
     *  caught in launch-flow.ts's Phase 2 — this hook's `relogin`/
     *  `loginViaTerminal` had it too, just never flagged directly since
     *  neither reported "auth_failed" the same visible way Phase 2 did). */
    const existingAccountIdFor = async (providerId: string): Promise<string | undefined> => {
        const agentDefinitionId = getBlockMetaKeyAtom(opts.blockId, "agentId")() as string | undefined;
        if (!agentDefinitionId) return undefined;
        try {
            const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: agentDefinitionId });
            return links.find((l) => l.provider === providerId)?.account_id;
        } catch {
            return undefined;
        }
    };

    /** Auth env for recovery actions: block meta `cmd:env` when present, else rebuilt. */
    const recoveryAuthEnv = async (prov: ProviderDefinition): Promise<Record<string, string>> => {
        const envMeta = getBlockMetaKeyAtom(opts.blockId, "cmd:env")();
        const authEnv: Record<string, string> = {};
        if (envMeta && typeof envMeta === "object") {
            for (const [k, v] of Object.entries(envMeta as Record<string, unknown>)) {
                if (typeof v === "string") authEnv[k] = v;
            }
        }
        if (Object.keys(authEnv).length > 0) return authEnv;
        // Meta env missing (agent never launched) — rebuild the isolated-dir
        // env the launch flow would have used, so the login lands in the same
        // store the agent will read.
        return (await buildAuthEnv(prov)) ?? {};
    };

    const relogin = async () => {
        if (reloginInFlight) return;
        const prov = opts.provider();
        if (!prov) {
            opts.log("auth", "re-login: no active provider", "warn");
            return;
        }
        setAuthNotice(null);
        // Clear any OAuth URL box left by a PRIOR attempt before starting a
        // fresh one — otherwise a subsequent no-progress outcome would stack
        // the error notice below a stale, contradictory URL box (reagent P2).
        setAuthUrl(null);
        loginCancelled = false;
        // The CLI path + auth env are written to block meta at launch; reuse
        // them instead of re-resolving (the agent is already running, so the
        // CLI is installed). If `cmd` is missing, resolve it directly — see
        // resolveCliForRecovery for why this must NOT fall back to the gated
        // launch flow.
        let cliPath = getBlockMetaKeyAtom(opts.blockId, "cmd")() as string | undefined;
        reloginInFlight = true;
        setLoginWaiting(true);
        setLaunchPhase({ kind: "checking-auth" });
        try {
            if (!cliPath) {
                cliPath = (await resolveCliForRecovery(prov, "re-login")) ?? undefined;
                if (!cliPath) return;
            }
            const authEnv = await recoveryAuthEnv(prov);
            // runProviderLogin falls through URL-capture -> global-login-copy ->
            // real-terminal-with-poll before giving up (retro-headless-login-
            // browser-open-2026-07-20) — "Login Again" used to dead-end the
            // instant the CLI produced no URL, which is every time for Claude
            // Code v2.1.x. linkTarget lets a tier-2/3 success register a real
            // Armory account and bind it to THIS agent (single-point
            // enforcement, PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md
            // §7) instead of just seeding a file nothing tracks.
            const agentDefinitionId = getBlockMetaKeyAtom(opts.blockId, "agentId")() as string | undefined;
            // Tier 1 mints the account dir but does NOT persist/link it (it
            // returns "opened" before confirming completion) — captured here
            // so a poll below can call persistAndLinkAccount once IT confirms
            // the login actually finished. reagent P1: without this, a tier-1
            // login that succeeds for any provider whose CLI actually prints
            // a URL (not requiresLoginTty, e.g. codex) via "Login Again" left
            // the minted account unpersisted/unlinked — the resolver's spawn
            // gate then blocks the agent on its very next spawn with no error
            // ever surfaced, since relogin's own "opened" case used to just
            // `break` and report nothing.
            let openedAccountId: string | undefined;
            let openedAccountDir: string | undefined;
            let recheckAuthEnv = authEnv;
            setLaunchPhase(
                prov.headlessLoginUrlUnsupported
                    ? { kind: "opening-login-terminal" }
                    : { kind: "waiting-for-login-link", deadlineMs: Date.now() + LOGIN_LINK_CAPTURE_LABEL_MS },
            );
            const outcome = await runProviderLogin({
                provider: prov,
                cliPath,
                authEnv,
                setAuthUrl,
                log: opts.log,
                isCancelled: () => loginCancelled,
                linkTarget: agentDefinitionId
                    ? { blockId: opts.blockId, agentDefinitionId }
                    : undefined,
                existingAccountId: await existingAccountIdFor(prov.id),
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
                // See launch-flow.ts's identical wiring — without this the
                // phase set just above never updates again for the rest of
                // this call, even though tier 2/3 inside it can run for up
                // to 5 more minutes. reagent P1 on PR #2300.
                onTierChange: (event) => {
                    if (event.tier === "fallback") {
                        setLaunchPhase({ kind: "opening-login-terminal" });
                    } else {
                        setLaunchPhase({ kind: "waiting-for-login-completion", deadlineMs: event.deadlineMs });
                    }
                },
            });
            switch (outcome) {
                case "opened": {
                    // A real OAuth URL was captured and opened — poll until
                    // the user finishes there, cancels, or 5 minutes elapse
                    // (same pattern as launch-flow.ts's own "opened" case).
                    opts.log("auth", "waiting for login to complete...");
                    let authenticated = false;
                    let authedEmail: string | null = null;
                    const deadline = Date.now() + 5 * 60 * 1000;
                    setLaunchPhase({ kind: "waiting-for-login-completion", deadlineMs: deadline });
                    while (!loginCancelled && Date.now() < deadline && !authenticated) {
                        await new Promise<void>((r) => setTimeout(r, 2000));
                        if (loginCancelled) break;
                        try {
                            const recheck = await RpcApi.CheckCliAuthCommand(TabRpcClient, {
                                cli_path: cliPath,
                                auth_check_args: prov.authCheckCommand,
                                auth_env: recheckAuthEnv,
                            }, { timeout: 10000 });
                            if (recheck.authenticated) {
                                authenticated = true;
                                authedEmail = recheck.email ?? null;
                            }
                        } catch {
                            // keep polling on transient RPC errors
                        }
                    }
                    if (authenticated && openedAccountId && openedAccountDir) {
                        await persistAndLinkAccount(
                            {
                                provider: prov,
                                cliPath,
                                authEnv,
                                setAuthUrl,
                                log: opts.log,
                                linkTarget: agentDefinitionId
                                    ? { blockId: opts.blockId, agentDefinitionId }
                                    : undefined,
                            },
                            openedAccountId,
                            openedAccountDir,
                        );
                        opts.log("auth", "Login successful — retrying…");
                        setAuthNotice(null);
                        // Post a visible confirmation into the pane itself — this used to
                        // ONLY happen on the very first auto-login (launch-flow.ts); "Login
                        // Again" retried the failed turn silently, so a user with nothing
                        // queued to retry (or who didn't notice the retry) never saw ANY
                        // acknowledgement that the login actually succeeded.
                        opts.onLoginSuccess?.(authedEmail);
                        opts.onRecovered?.();
                    } else if (!loginCancelled) {
                        setAuthNotice(
                            "Opened a login page, but no login was detected within 5 minutes. " +
                            "Complete the login there, then click “Login Again”.",
                        );
                    }
                    break;
                }
                case "seeded":
                    opts.log("auth", "Signed in from your global login — retrying…");
                    setAuthNotice(null);
                    opts.onLoginSuccess?.(null);
                    opts.onRecovered?.();
                    break;
                case "terminal-success":
                    opts.log("auth", "Login successful — retrying…");
                    setAuthNotice(null);
                    opts.onLoginSuccess?.(null);
                    opts.onRecovered?.();
                    break;
                case "terminal-timeout":
                    // Never fail silently (retro §5.1): tell the user nothing
                    // completed and point at the recovery path that's left.
                    setAuthNotice(
                        "Opened a terminal window for login, but no login was detected within 5 minutes. " +
                        "Complete the login there, then click “Use existing login”.",
                    );
                    break;
                case "terminal-unavailable":
                    setAuthNotice(
                        "Couldn't start a browser login or open a terminal window on this platform." +
                        (prov.id === "claude" ? " Try “Use existing login” if you're already signed in elsewhere." : ""),
                    );
                    break;
            }
        } catch (err: any) {
            const msg = `Re-login failed: ${err?.message ?? String(err)}`;
            opts.log("auth", msg, "error");
            setAuthNotice(msg);
        } finally {
            reloginInFlight = false;
            setLoginWaiting(false);
            setLaunchPhase(null);
        }
    };

    // "Use my existing login" — seed the isolated dir from the global Claude
    // login instead of a fresh OAuth (SPEC_HOST_CLI_LOGIN_CAPTURE §5.5). Guarded
    // against double-fire like relogin; the running agent re-reads its
    // credential per request, so a successful seed needs no restart.
    let seedInFlight = false;
    const useGlobalLogin = async () => {
        if (seedInFlight) return;
        const prov = opts.provider();
        if (!prov) {
            opts.log("auth", "use existing login: no active provider", "warn");
            return;
        }
        setAuthNotice(null);
        seedInFlight = true;
        try {
            // Mint a REAL per-account isolated dir and persist an
            // IdentityAccount row — not just a seed into whatever dir was
            // already resolved. The resolver's spawn gate now requires a
            // real bound account for oauth-class providers, no ambient
            // exception (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md
            // §7), so a bare file-copy into the old shared/resolved dir
            // would leave this agent blocked on its next turn regardless of
            // how valid the credential file itself is.
            const reg = await registerSeededAccount(prov.id, opts.log, await existingAccountIdFor(prov.id));
            if (reg.ok && reg.accountId && reg.dir) {
                const agentDefinitionId = getBlockMetaKeyAtom(opts.blockId, "agentId")() as string | undefined;
                if (agentDefinitionId) {
                    try {
                        await RpcApi.LinkAgentIdentityCommand(TabRpcClient, {
                            agent_id: agentDefinitionId,
                            account_id: reg.accountId,
                            provider: prov.id,
                        });
                        if (prov.authConfigDirEnvVar) {
                            const envMeta = getBlockMetaKeyAtom(opts.blockId, "cmd:env")();
                            const prevEnv: Record<string, string> = {};
                            if (envMeta && typeof envMeta === "object") {
                                for (const [k, v] of Object.entries(envMeta as Record<string, unknown>)) {
                                    if (typeof v === "string") prevEnv[k] = v;
                                }
                            }
                            const oref = WOS.makeORef("block", opts.blockId);
                            await RpcApi.SetMetaCommand(TabRpcClient, {
                                oref,
                                meta: { "cmd:env": { ...prevEnv, [prov.authConfigDirEnvVar]: reg.dir } },
                            });
                        }
                    } catch (e: any) {
                        opts.log(
                            "auth",
                            `account created but couldn't be linked to this agent: ${e?.message ?? String(e)}`,
                            "warn",
                        );
                    }
                }
                // Credential is now valid on disk — but the agent spawns fresh
                // per turn and the failure row only clears on the next turn, so
                // a successful seed looks like it "did nothing". Drive the
                // recovery: retry the failed turn (fresh spawn picks up the new
                // token and TurnStart clears the row).
                opts.log("auth", "Signed in from your global login — retrying…");
                opts.onLoginSuccess?.(null);
                opts.onRecovered?.();
            } else {
                const msg = "Couldn't use your global login — no valid global Claude credential was found. Try “Login via terminal”.";
                opts.log("auth", msg, "warn");
                setAuthNotice(msg);
            }
        } catch (err: any) {
            const msg = `Use existing login failed: ${err?.message ?? String(err)}`;
            opts.log("auth", msg, "error");
            setAuthNotice(msg);
        } finally {
            seedInFlight = false;
        }
    };

    // Open a real visible terminal window so the browser OAuth flow works,
    // then poll for credentials landing (or for the CLI itself to report
    // authenticated, for non-Claude providers). Shares runProviderLogin's
    // tier 2/3 logic (skipTier1 — the user explicitly asked for a terminal
    // login, no point trying headless first) instead of reimplementing it.
    // This used to be a separate, hand-rolled copy of the same logic —
    // reagent caught it reproducing a bug (codex/openclaw could never
    // actually complete a login here: minting was claude-only and the poll
    // called a host command that hard-rejects every other provider) already
    // fixed once in runProviderLogin itself. A second hand-rolled copy would
    // only let that exact class of bug reappear the next time one of the two
    // was fixed and the other wasn't.
    const loginViaTerminal = async () => {
        if (reloginInFlight) return;
        loginCancelled = false;
        const prov = opts.provider();
        if (!prov) {
            opts.log("auth", "login via terminal: no active provider", "warn");
            return;
        }
        setAuthNotice(null);
        // Claim the in-flight guard BEFORE any await. The CLI resolve below
        // can take up to 300 s, and the recovery buttons have no disabled
        // binding — so setting the flag only after the awaits (the previous
        // bug) let a rapid double-click pass the top guard twice and open two
        // terminal windows with overlapping poll loops. Mirror relogin: flag
        // first, reset in finally.
        reloginInFlight = true;
        setLoginWaiting(true);
        setLaunchPhase({ kind: "opening-login-terminal" });
        try {
            let cliPath = getBlockMetaKeyAtom(opts.blockId, "cmd")() as string | undefined;
            if (!cliPath) {
                // Same H2 trap as relogin: the gated launch flow would trust the
                // auth check and skip the login the user explicitly asked for.
                cliPath = (await resolveCliForRecovery(prov, "login via terminal")) ?? undefined;
                if (!cliPath) return;
            }
            const authEnv = await recoveryAuthEnv(prov);
            const agentDefinitionId = getBlockMetaKeyAtom(opts.blockId, "agentId")() as string | undefined;
            const outcome = await runProviderLogin({
                provider: prov,
                cliPath,
                authEnv,
                setAuthUrl,
                log: opts.log,
                isCancelled: () => loginCancelled,
                skipTier1: true,
                linkTarget: agentDefinitionId
                    ? { blockId: opts.blockId, agentDefinitionId }
                    : undefined,
                existingAccountId: await existingAccountIdFor(prov.id),
                // skipTier1 is always true here, so "fallback" never fires —
                // but "polling" still does once the terminal actually opens,
                // giving an accurate deadline instead of leaving the phase
                // on a static "opening terminal" for the whole wait.
                onTierChange: (event) => {
                    if (event.tier === "polling") {
                        setLaunchPhase({ kind: "waiting-for-login-completion", deadlineMs: event.deadlineMs });
                    }
                },
            });
            switch (outcome) {
                case "opened":
                    break;
                case "seeded":
                case "terminal-success":
                    opts.log("auth", "Login successful — retrying…");
                    setAuthNotice(null);
                    opts.onLoginSuccess?.(null);
                    opts.onRecovered?.();
                    break;
                case "terminal-timeout":
                    if (!loginCancelled) {
                        const msg = "No login detected after 5 minutes. Complete the login in the terminal, then click “Use existing login”.";
                        opts.log("auth", msg, "warn");
                        setAuthNotice(msg);
                    }
                    break;
                case "terminal-unavailable":
                    setAuthNotice("Couldn't open a terminal window for login on this platform.");
                    break;
            }
        } catch (err: any) {
            const msg = `Terminal login failed: ${err?.message ?? String(err)}`;
            opts.log("auth", msg, "error");
            setAuthNotice(msg);
        } finally {
            reloginInFlight = false;
            setLoginWaiting(false);
            setLaunchPhase(null);
        }
    };

    const cancelLogin = () => {
        loginCancelled = true;
        getApi().cancelCliLogin().catch(() => {});
        opts.log("auth", "login cancelled", "warn");
        // Immediate UI feedback — the in-flight poll loop notices
        // loginCancelled on its own next tick (up to 2s), but the phase
        // label/cancel button should disappear the instant the user clicks.
        setLaunchPhase(null);
    };

    const notifyControllerHealthy = () => {
        setCanRetry(false);
        setAuthNotice(null);
    };

    // If the pane is closed while login is in progress, cancel and kill
    // the host CLI process. This onCleanup is registered against the
    // SolidJS owner context that called useAgentControllerStatus — the
    // agent presentation view's component scope.
    onCleanup(() => {
        // Cancel unconditionally. The login CLI can already be spawned during
        // the window between runCliLogin() and setLoginWaiting(true) flipping
        // true, so gating on loginWaiting() can leave the child orphaned if the
        // pane closes inside that window. cancelCliLogin is idempotent and
        // swallows errors, so calling it when no login is in flight is safe.
        loginCancelled = true;
        getApi().cancelCliLogin().catch(() => {});
    });

    return {
        authUrl,
        setAuthUrl,
        authNotice,
        setAuthNotice,
        canRetry,
        flowRunning,
        agentReady,
        isLoading,
        loginWaiting,
        launchPhase,
        startLaunchFlow,
        relogin,
        useGlobalLogin,
        loginViaTerminal,
        notifyControllerHealthy,
        cancelLogin,
    };
}
