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
import { forceProviderLogin } from "../flows/force-login";
import { seedGlobalLogin } from "../flows/seed-global-login";
import type { ProviderDefinition } from "../providers";

import type { LogFn } from "../types";
export type { LogFn };

export interface UseAgentControllerStatusOptions {
    blockId: string;
    provider: Accessor<ProviderDefinition | undefined>;
    log: LogFn;
    onLoginSuccess?: (email: string | null) => void;
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
     * spawning, polls `seedGlobalLogin` every 5s for up to 5 minutes; seeds
     * the isolated dir as soon as credentials appear.
     */
    loginViaTerminal: () => Promise<void>;
    cancelLogin: () => void;
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
                authEnv,
                onLoginSuccess: opts.onLoginSuccess,
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
        // fresh one — otherwise a subsequent "no-url" outcome would stack the
        // error notice below a stale, contradictory URL box (reagent P2).
        setAuthUrl(null);
        // The CLI path + auth env are written to block meta at launch; reuse
        // them instead of re-resolving (the agent is already running, so the
        // CLI is installed). If `cmd` is missing, resolve it directly — see
        // resolveCliForRecovery for why this must NOT fall back to the gated
        // launch flow.
        let cliPath = getBlockMetaKeyAtom(opts.blockId, "cmd")() as string | undefined;
        reloginInFlight = true;
        setLoginWaiting(true);
        try {
            if (!cliPath) {
                cliPath = (await resolveCliForRecovery(prov, "re-login")) ?? undefined;
                if (!cliPath) return;
            }
            const authEnv = await recoveryAuthEnv(prov);
            const outcome = await forceProviderLogin({ provider: prov, cliPath, authEnv, setAuthUrl, log: opts.log });
            if (outcome === "no-url") {
                // Never fail silently (retro §5.1): tell the user nothing
                // opened and point at the recovery paths that do work.
                setAuthNotice(
                    "Couldn't start a browser login — the CLI didn't produce a login URL, so nothing was opened. " +
                    "Use “Login via terminal” to complete the login in a real terminal window" +
                    (prov.id === "claude" ? ", or “Use existing login” to copy your global Claude login into this agent." : "."),
                );
            }
        } catch (err: any) {
            const msg = `Re-login failed: ${err?.message ?? String(err)}`;
            opts.log("auth", msg, "error");
            setAuthNotice(msg);
        } finally {
            reloginInFlight = false;
            setLoginWaiting(false);
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
            // Seed into the agent's RESOLVED auth dir (from cmd:env), not a
            // guessed one; the host guards it to ~/.agentmux so the seed never
            // writes the user's ~/.claude (SPEC_PROVIDER_ISOLATION §4.5).
            const envMeta = getBlockMetaKeyAtom(opts.blockId, "cmd:env")();
            let configDir: string | undefined;
            if (prov.authConfigDirEnvVar && envMeta && typeof envMeta === "object") {
                const v = (envMeta as Record<string, unknown>)[prov.authConfigDirEnvVar];
                if (typeof v === "string") configDir = v;
            }
            const seeded = await seedGlobalLogin(prov.id, opts.log, configDir);
            if (seeded) {
                // Credential is now valid on disk — but the agent spawns fresh
                // per turn and the failure row only clears on the next turn, so
                // a successful seed looks like it "did nothing". Drive the
                // recovery: retry the failed turn (fresh spawn picks up the new
                // token and TurnStart clears the row).
                opts.log("auth", "Signed in from your global login — retrying…");
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
    // then poll for credentials seeding into the isolated dir.
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
        try {
            let cliPath = getBlockMetaKeyAtom(opts.blockId, "cmd")() as string | undefined;
            if (!cliPath) {
                // Same H2 trap as relogin: the gated launch flow would trust the
                // auth check and skip the login the user explicitly asked for.
                cliPath = (await resolveCliForRecovery(prov, "login via terminal")) ?? undefined;
                if (!cliPath) return;
            }
            const authEnv = await recoveryAuthEnv(prov);
            const configDir = prov.authConfigDirEnvVar ? authEnv[prov.authConfigDirEnvVar] : undefined;

            // Strip CLAUDE_CONFIG_DIR (and equivalents) from the terminal env so the
            // login writes to the user's global ~/.claude instead of the isolated dir.
            // seedGlobalLogin polls the global dir and copies on success — if we kept
            // the isolated dir key the poll would look in global but the creds would
            // land in isolated, and a terminal-fresh login would never be detected.
            const terminalEnv: Record<string, string> = { ...authEnv };
            if (prov.authConfigDirEnvVar) delete terminalEnv[prov.authConfigDirEnvVar];

            await getApi().openLoginTerminal(cliPath, prov.authLoginCommand, terminalEnv);
            opts.log("auth", "A terminal window opened — complete the login there, then come back.");

            // Poll silently every 5s for up to 5 minutes; seed on first hit.
            const POLL_MS = 5_000;
            const TIMEOUT_MS = 5 * 60 * 1_000;
            const deadline = performance.now() + TIMEOUT_MS;
            const silentLog: typeof opts.log = () => {};
            let seeded = false;
            while (!seeded && performance.now() < deadline && !loginCancelled) {
                await new Promise<void>((r) => setTimeout(r, POLL_MS));
                if (loginCancelled) break;
                seeded = await seedGlobalLogin(prov.id, silentLog, configDir);
            }
            if (seeded) {
                opts.log("auth", "Login successful — retrying…");
                setAuthNotice(null);
                opts.onRecovered?.();
            } else if (!loginCancelled) {
                const msg = "No login detected after 5 minutes. Complete the login in the terminal, then click “Use existing login”.";
                opts.log("auth", msg, "warn");
                setAuthNotice(msg);
            }
        } catch (err: any) {
            const msg = `Terminal login failed: ${err?.message ?? String(err)}`;
            opts.log("auth", msg, "error");
            setAuthNotice(msg);
        } finally {
            reloginInFlight = false;
            setLoginWaiting(false);
        }
    };

    const cancelLogin = () => {
        loginCancelled = true;
        getApi().cancelCliLogin().catch(() => {});
        opts.log("auth", "login cancelled", "warn");
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
        startLaunchFlow,
        relogin,
        useGlobalLogin,
        loginViaTerminal,
        cancelLogin,
    };
}
