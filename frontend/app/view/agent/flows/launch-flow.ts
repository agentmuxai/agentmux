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

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import * as WOS from "@/app/store/wos";
import { BlockService } from "@/app/store/services";
import { getApi, staticTabId } from "@/app/store/global";
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
}

export type LaunchFlowResult = "success" | "auth_failed" | "fatal";

export async function runLaunchFlow(opts: LaunchFlowOptions): Promise<LaunchFlowResult> {
    const { blockId, provider, log, setAuthUrl, isCancelled, setLoginWaiting, authEnv } = opts;

    if (!provider) {
        log("error", "no provider definition — cannot resolve CLI", "error");
        return "fatal";
    }

    const oref = WOS.makeORef("block", blockId);

    // Phase 0: Container agents require a container runtime
    const blockData = WOS.getWaveObjectAtom<Block>(oref)();
    const agentMode = blockData?.meta?.agentMode ?? "host";
    if (agentMode === "container") {
        log("docker", "container agent — checking for container runtime...");
        try {
            const dockerResult = await RpcApi.ResolveCliCommand(TabRpcClient, {
                provider_id: "docker",
                cli_command: "docker",
                npm_package: "",
                pinned_version: "",
                windows_install_command: "",
                unix_install_command: "",
            }, { timeout: 10000 });
            log("docker", `found: ${dockerResult.cli_path} (${dockerResult.version})`);
        } catch {
            log("docker", "Container runtime not found", "error");
            log("docker", "Container agents require a compatible container runtime (e.g. Docker) to run.", "error");
            return "fatal";
        }
    }

    // Phase 1: CLI Detection / Installation
    log("cli", `checking for ${provider.cliCommand}...`);
    if (provider.pinnedVersion) {
        log("cli", `if not found locally, will install ${provider.npmPackage}@${provider.pinnedVersion} via npm (this may take 1-2 minutes)...`);
    }

    // Subscribe to install progress events — backend streams npm/installer output line-by-line
    const installScope = WOS.makeORef("block", blockId);
    const unsubInstall = waveEventSubscribe({
        eventType: "install_progress",
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
        const msg = err?.message ?? String(err);
        log("cli", msg, "error");
        log("error", `${provider.cliCommand} not available — install manually or check your internet connection`, "error");
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
            );
            if (loginUrl) {
                setAuthUrl(loginUrl);
                log("auth", "opening browser...");
                try {
                    getApi().openExternal(loginUrl);
                    log("auth", "browser opened — complete login there");
                } catch (err: any) {
                    log("auth", `browser did not open: ${err?.message ?? String(err)}`, "warn");
                    log("auth", "copy the URL from the box above and open it manually", "warn");
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

            // Always clear auth URL after the login attempt
            setAuthUrl(null);

            if (isCancelled()) {
                return "auth_failed";
            }

            if (!authenticated) {
                log("auth", "login timed out after 5 minutes", "error");
                log("auth", `retry: click the button below, or run '${provider.cliCommand} ${provider.authLoginCommand.join(" ")}' manually`, "warn");
                return "auth_failed";
            }
        } catch (err: any) {
            setLoginWaiting(false);
            setAuthUrl(null);
            log("auth", `login failed: ${err?.message ?? String(err)}`, "error");
            log("auth", `run: ${provider.cliCommand} ${provider.authLoginCommand.join(" ")}`, "warn");
            return "auth_failed";
        }
    }

    // Phase 3: Controller Registration
    log("controller", "registering subprocess controller...");
    try {
        await RpcApi.ControllerResyncCommand(TabRpcClient, {
            tabid: staticTabId(),
            blockid: blockId,
            forcerestart: false,
        });
        const rts = await BlockService.GetControllerStatus(blockId);
        const status = rts?.shellprocstatus ?? "init";
        log("controller", `status: ${status}`);
        if (status === "init") {
            log("agent", "ready — type a message below to start");
        } else if (status === "done") {
            log("agent", "previous turn complete — send a message to continue");
        }
    } catch (err: any) {
        log("controller", `resync failed: ${err?.message ?? String(err)}`, "warn");
        log("agent", "ready — type a message below to start");
    }

    return "success";
}
