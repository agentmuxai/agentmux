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
     * Returns the pane's current `{rows, cols}` (or undefined if not laid out
     * yet). Forwarded to the launch flow to seed the PTY size at spawn.
     */
    getInitialTermSize?: () => { rows: number; cols: number } | undefined;
}

export interface UseAgentControllerStatus {
    authUrl: Accessor<string | null>;
    setAuthUrl: (url: string | null) => void;
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
    const relogin = async () => {
        if (reloginInFlight) return;
        const prov = opts.provider();
        if (!prov) {
            opts.log("auth", "re-login: no active provider", "warn");
            return;
        }
        // The CLI path + auth env are written to block meta at launch; reuse
        // them instead of re-resolving (the agent is already running, so the
        // CLI is installed). If `cmd` is missing the agent never launched —
        // fall back to the full launch flow.
        const cliPath = getBlockMetaKeyAtom(opts.blockId, "cmd")() as string | undefined;
        if (!cliPath) {
            opts.log("auth", "re-login: CLI not resolved yet — running the full launch flow instead", "warn");
            void startLaunchFlow();
            return;
        }
        const envMeta = getBlockMetaKeyAtom(opts.blockId, "cmd:env")();
        const authEnv: Record<string, string> = {};
        if (envMeta && typeof envMeta === "object") {
            for (const [k, v] of Object.entries(envMeta as Record<string, unknown>)) {
                if (typeof v === "string") authEnv[k] = v;
            }
        }
        reloginInFlight = true;
        setLoginWaiting(true);
        try {
            await forceProviderLogin({ provider: prov, cliPath, authEnv, setAuthUrl, log: opts.log });
        } catch (err: any) {
            opts.log("auth", `re-login failed: ${err?.message ?? String(err)}`, "error");
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
            await seedGlobalLogin(prov.id, opts.log, configDir);
        } catch (err: any) {
            opts.log("auth", `use existing login failed: ${err?.message ?? String(err)}`, "error");
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
        const cliPath = getBlockMetaKeyAtom(opts.blockId, "cmd")() as string | undefined;
        if (!cliPath) {
            opts.log("auth", "login via terminal: CLI not resolved — running launch flow instead", "warn");
            void startLaunchFlow();
            return;
        }
        const envMeta = getBlockMetaKeyAtom(opts.blockId, "cmd:env")();
        const authEnv: Record<string, string> = {};
        if (envMeta && typeof envMeta === "object") {
            for (const [k, v] of Object.entries(envMeta as Record<string, unknown>)) {
                if (typeof v === "string") authEnv[k] = v;
            }
        }
        const configDir = prov.authConfigDirEnvVar ? authEnv[prov.authConfigDirEnvVar] : undefined;

        reloginInFlight = true;
        setLoginWaiting(true);
        try {
            await getApi().openLoginTerminal(cliPath, prov.authLoginCommand, authEnv);
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
                opts.log("auth", "Login successful — your next message will use the new token.");
            } else if (!loginCancelled) {
                opts.log("auth", "No login detected after 5 minutes. Complete the login in the terminal, then click 'Use existing login'.", "warn");
            }
        } catch (err: any) {
            opts.log("auth", `terminal login failed: ${err?.message ?? String(err)}`, "error");
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
