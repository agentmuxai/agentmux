// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * launch-flow — the full agent launch sequence extracted from agent-view.tsx.
 *
 * Step 2 of specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Phases:
 *   0. Container agents require a container runtime (docker/podman/…).
 *      Skipped for host agents.
 *   1. CLI detection/installation — resolve the provider's cli_command, with
 *      optional npm install if not found and pinnedVersion is set. Subscribes
 *      to `install_progress` events so the caller's log sink sees each npm
 *      line as it happens.
 *   2. Auth check — calls the provider's check-auth command. If unauthenticated,
 *      spawns the login command via the CEF host (so the browser opens
 *      correctly on Windows), then polls with 2s cadence until authenticated,
 *      cancelled, or 5 minutes elapse.
 *   3. Controller registration — ControllerResync on the tab, read status, log
 *      "ready" or "done" depending on whether there's a prior turn.
 *
 * The caller provides a log sink, a cancellation accessor, and callbacks for
 * two pieces of external state (`authUrl`, `loginWaiting`). All other state
 * is derived from the block_id + provider definition.
 *
 * Return values:
 *   - "success"     — controller registered, ready for user input
 *   - "auth_failed" — login timed out, cancelled, or erred (retry makes sense)
 *   - "fatal"       — CLI missing, docker missing, provider unknown (retry won't help)
 */

import { translateError } from "@/app/errors/translate";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ensureCapability, getCapability } from "@/app/store/toolchain-capabilities";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import * as WOS from "@/app/store/wos";
import { BlockService } from "@/app/store/services";
import { getApi, staticTabId } from "@/app/store/global";
import { openOAuthBrowserPane } from "./open-oauth-pane";
import type { ProviderDefinition } from "../providers";

import type { LogFn } from "../types";
export type { LogFn };

export interface LaunchFlowOptions {
    blockId: string;
    provider: ProviderDefinition | undefined;
    log: LogFn;
    setAuthUrl: (url: string | null) => void;
    isCancelled: () => boolean;
    setLoginWaiting: (v: boolean) => void;
    authEnv?: Record<string, string>;
    /** Called once when login is confirmed — append a success message to the chat. */
    onLoginSuccess?: (email: string | null) => void;
    /**
     * Returns the pane's current `{rows, cols}` (or undefined if not laid out
     * yet). Used to seed the PTY size on the Phase-3 resync so the agent CLI is
     * born at the right width, avoiding the post-spawn resize race. See
     * docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.
     */
    getInitialTermSize?: () => { rows: number; cols: number } | undefined;
    /**
     * Called once Phase 3's `GetControllerStatus` resolves, with the raw
     * runtime status — previously this result was only ever logged. Used
     * to seed `TurnPhase` from `rts.turn_active` at mount instead of the
     * hardcoded `Idle` default. See
     * docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md
     * Finding 1.
     */
    onControllerStatus?: (rts: BlockControllerRuntimeStatus) => void;
}

export type LaunchFlowResult = "success" | "auth_failed" | "fatal";

export async function runLaunchFlow(opts: LaunchFlowOptions): Promise<LaunchFlowResult> {
    const { blockId, provider, log, setAuthUrl, isCancelled, setLoginWaiting, authEnv } = opts;

    if (!provider) {
        log("error", "no provider definition — cannot resolve CLI", "error");
        return "fatal";
    }

    const oref = WOS.makeORef("block", blockId);

    // Phase 0: Container agents require a container runtime. Reads the
    // shared toolchain-capabilities store (forced fresh, since staleness
    // right before an actual launch is exactly the failure mode being
    // guarded against) rather than its own CLI-on-PATH check — previously
    // this used ResolveCliCommand directly, which only confirms the `docker`
    // binary is on PATH and can't tell the daemon is stopped, so an agent
    // could pass this gate and still fail deeper in container spawn. See
    // docs/retros/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    const blockData = WOS.getWaveObjectAtom<Block>(oref)();
    const agentMode = blockData?.meta?.agentMode ?? "host";
    if (agentMode === "container") {
        log("docker", "container agent — checking for container runtime...");
        await ensureCapability("docker", { force: true });
        const docker = getCapability("docker");
        if (docker.status !== "available") {
            log("docker", "Container runtime not found", "error");
            log("docker", "Container agents require a compatible container runtime (e.g. Docker) to run.", "error");
            return "fatal";
        }
        log("docker", "container runtime available");
    }

    // Phase 1: CLI Detection / Installation
    log("cli", `checking for ${provider.cliCommand}...`);
    if (provider.pinnedVersion) {
        log("cli", `if not found locally, will install ${provider.npmPackage}@${provider.pinnedVersion} via npm (this may take 1-2 minutes)...`);
    }

    // Subscribe to install progress events — backend streams npm/installer output line-by-line
    const installScope = WOS.makeORef("block", blockId);
    const unsubInstall = waveEventSubscribe({
        eventType: WpsEvent.InstallProgress,
        scope: installScope,
        handler: (event: any) => {
            const msg: string = event?.data?.message ?? "";
            if (msg) log("install", msg);
        },
    });

    let cliResult: ResolveCliResult;
    try {
        cliResult = await RpcApi.ResolveCliCommand(TabRpcClient, {
            provider_id: provider.id,
            cli_command: provider.cliCommand,
            npm_package: provider.npmPackage,
            pinned_version: provider.pinnedVersion,
            windows_install_command: provider.windowsInstallCommand,
            unix_install_command: provider.unixInstallCommand,
            block_id: blockId,
        }, { timeout: 300000 });
    } catch (err: any) {
        unsubInstall();
        // Route through translateError so typed wire-format errors
        // from ResolveCli render as readable text in the activity
        // log instead of raw JSON like `{"code":"AMX-CLI-001",...}`.
        // Legacy free-text errors pass through unchanged.
        const t = translateError(err);
        // Log the retry hint FIRST so the error line is the most
        // recent entry — `ActivityLogPanel` derives the panel's
        // failed-state styling from the most recent log entry's
        // level (`agent-activity-log--has-error`).
        if (t.retry) log("cli", t.retry, "warn");
        log("cli", `${t.title}: ${t.message}`, "error");
        return "fatal";
    }
    unsubInstall();

    if (cliResult.source === "installed") {
        log("cli", `installed ${provider.npmPackage} (${cliResult.version})`);
    } else if (cliResult.source === "local_install") {
        log("cli", `found: ${cliResult.cli_path} (${cliResult.version}) [local install]`);
    } else {
        log("cli", `found: ${cliResult.cli_path} (${cliResult.version})`);
    }

    // Update block meta with resolved CLI path and auth env vars.
    // cmd:env is read by AgentInputCommand when spawning the subprocess —
    // it must include CLAUDE_CONFIG_DIR (or equivalent) so the subprocess
    // uses the same isolated auth dir that was validated in Phase 2.
    // Without this, the subprocess runs without the env var and falls back
    // to the global ~/.claude/ dir, failing auth silently after login.
    await RpcApi.SetMetaCommand(TabRpcClient, {
        oref,
        meta: {
            cmd: cliResult.cli_path,
            ...(authEnv && Object.keys(authEnv).length > 0 ? { "cmd:env": authEnv } : {}),
        },
    });

    // Phase 2: Auth Check → auto-login if not authenticated
    log("auth", `checking ${provider.cliCommand} authentication...`);
    let needsLogin = false;
    try {
        const authResult = await RpcApi.CheckCliAuthCommand(TabRpcClient, {
            cli_path: cliResult.cli_path,
            auth_check_args: provider.authCheckCommand,
            auth_env: authEnv,
        }, { timeout: 30000 });
        if (authResult.authenticated) {
            const emailPart = authResult.email ? ` as ${authResult.email}` : "";
            const methodPart = authResult.auth_method ? ` (${authResult.auth_method})` : "";
            log("auth", `authenticated${emailPart}${methodPart}`);
        } else {
            needsLogin = true;
        }
    } catch (err: any) {
        log("auth", `check failed: ${err?.message ?? String(err)}`, "warn");
        log("auth", "authentication status unknown — will attempt anyway", "warn");
    }

    if (needsLogin) {
        log("auth", "not authenticated — starting login flow...");
        try {
            // Run from the CEF host (GUI process) so the browser opens correctly on Windows.
            // Returns immediately after spawning — browser opens, frontend polls for completion.
            //
            // The host API may return an OAuth URL string captured from the
            // CLI's stdout (Claude Code does this reliably; other providers
            // may not). When available, push it into setAuthUrl so the
            // auth-url box renders above the composer with a Copy button —
            // the user can paste the URL into their browser manually if the
            // auto-open didn't fire or got blocked. See
            // SPEC_AGENT_PANE_FOLLOWUPS item #3.
            const loginUrl = await getApi().runCliLogin(
                cliResult.cli_path,
                provider.authLoginCommand,
                authEnv ?? {},
                provider.requiresLoginTty ?? false,
            );
            if (loginUrl) {
                setAuthUrl(loginUrl);
                log("auth", "opening browser...");
                // Open the OAuth URL in the system browser — it already has the
                // user's session/cookies, so login there is more likely to
                // auto-complete; falls back to an in-app browser pane if the
                // system browser can't be opened. The auth-url box above the
                // composer is the URL backup either way.
                const opened = await openOAuthBrowserPane(loginUrl);
                if (opened === "pane") {
                    log("auth", "opened login in an in-app browser pane — complete login there");
                } else if (opened === "external") {
                    log("auth", "opened login in your system browser — complete login there");
                } else {
                    log("auth", "could not open a browser; copy the URL from the box above and open it manually", "warn");
                }
            } else {
                log("auth", "attempting to open browser for login...");
                log("auth", "if no browser opened, run the login command manually", "warn");
            }

            // Poll until authenticated, cancelled, or timed out (5 minutes)
            log("auth", "waiting for login to complete...");
            setLoginWaiting(true);
            const deadline = Date.now() + 5 * 60 * 1000;
            let authenticated = false;
            while (!isCancelled() && Date.now() < deadline) {
                await new Promise<void>((r) => setTimeout(r, 2000));
                if (isCancelled()) break;
                try {
                    const recheckResult = await RpcApi.CheckCliAuthCommand(TabRpcClient, {
                        cli_path: cliResult.cli_path,
                        auth_check_args: provider.authCheckCommand,
                        auth_env: authEnv,
                    }, { timeout: 10000 });
                    if (recheckResult.authenticated) {
                        const emailPart = recheckResult.email ? ` as ${recheckResult.email}` : "";
                        log("auth", `authenticated${emailPart}`);
                        opts.onLoginSuccess?.(recheckResult.email ?? null);
                        authenticated = true;
                        break;
                    }
                } catch {
                    // keep polling on transient RPC errors
                }
            }
            setLoginWaiting(false);

            // Reap the login CLI now the attempt has concluded (success,
            // timeout, or cancel). On manual-paste success the child self-exits;
            // if creds appeared without a paste it would otherwise linger at the
            // prompt. cancelCliLogin is idempotent and host-side.
            getApi().cancelCliLogin().catch(() => {});

            // Always clear auth URL after the login attempt
            setAuthUrl(null);

            if (isCancelled()) {
                return "auth_failed";
            }

            if (!authenticated) {
                // Synthesize an AMX-AUTH-002 wire payload so the log
                // entry renders with the catalog's friendly title +
                // retry hint, matching the typed-error pattern used
                // elsewhere. Same retry-first / error-last ordering
                // so ActivityLogPanel keeps failed-state styling.
                const t = translateError({
                    code: "AMX-AUTH-002",
                    details: { provider: provider.id, seconds: 300 },
                });
                log("auth", `retry: ${t.retry ?? `run '${provider.cliCommand} ${provider.authLoginCommand.join(" ")}' manually`}`, "warn");
                log("auth", `${t.title}: ${t.message}`, "error");
                return "auth_failed";
            }
        } catch (err: any) {
            setLoginWaiting(false);
            getApi().cancelCliLogin().catch(() => {});
            setAuthUrl(null);
            log("auth", `login failed: ${err?.message ?? String(err)}`, "error");
            log("auth", `run: ${provider.cliCommand} ${provider.authLoginCommand.join(" ")}`, "warn");
            return "auth_failed";
        }
    }

    // Phase 3: Controller Registration
    log("controller", "registering subprocess controller...");
    try {
        // Seed the PTY at the pane's current width so the agent CLI wraps
        // correctly from its first byte — the backend opens the PTY at this
        // size (shell.rs `pty_size_from_rt_opts`), avoiding the post-spawn
        // resize race. Omitted when the pane isn't laid out yet; the live
        // ResizeObserver in usePtyWidth corrects any later change.
        const initialTermSize = opts.getInitialTermSize?.();
        await RpcApi.ControllerResyncCommand(TabRpcClient, {
            tabid: staticTabId(),
            blockid: blockId,
            forcerestart: false,
            ...(initialTermSize ? { rtopts: { termsize: initialTermSize } } : {}),
        });
        const rts = await BlockService.GetControllerStatus(blockId);
        const status = rts?.shellprocstatus ?? "init";
        log("controller", `status: ${status}`);
        if (rts) opts.onControllerStatus?.(rts);
        if (status === "init") {
            log("agent", "ready — type a message below to start");
        } else if (status === "done") {
            log("agent", "previous turn complete — send a message to continue");
        }
    } catch (err: any) {
        // Don't follow a real resync failure with the generic "ready" message —
        // that previously masked every resync error (including the commit-
        // pressure admission gate's "memory full" refusal) with a misleading
        // all-clear a line later. Surface the actual failure at "error" so it's
        // the last, most visible line in the panel.
        log("controller", `resync failed: ${err?.message ?? String(err)}`, "error");
    }

    return "success";
}
