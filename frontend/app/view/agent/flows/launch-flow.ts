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
import { persistAndLinkAccount, runProviderLogin } from "./run-provider-login";
import { LOGIN_LINK_CAPTURE_LABEL_MS, type LaunchPhase } from "./launch-phase";
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
    /** Reports which phase of the flow is currently running, so the caller
     *  can show a specific status (and, for phases carrying a deadline, a
     *  "waiting on X, up to Ys" label) instead of a generic spinner. See
     *  launch-phase.ts. Optional so existing callers/tests are unaffected. */
    setLaunchPhase?: (phase: LaunchPhase | null) => void;
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
    const setPhase = opts.setLaunchPhase ?? (() => {});

    if (!provider) {
        log("error", "no provider definition — cannot resolve CLI", "error");
        setPhase({ kind: "failed", reason: "no provider definition" });
        return "fatal";
    }

    setPhase({ kind: "resolving-cli" });
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
            setPhase({ kind: "failed", reason: "container runtime not found" });
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
        // recent entry — reads better in the shell terminal with the
        // error message last, right before the user's eye.
        if (t.retry) log("cli", t.retry, "warn");
        log("cli", `${t.title}: ${t.message}`, "error");
        setPhase({ kind: "failed", reason: t.message });
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
    setPhase({ kind: "checking-auth" });
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
        setLoginWaiting(true);
        try {
            // Shared with `/login` and the "Login Again" failure-banner action
            // (retro-headless-login-browser-open-2026-07-20 / retro-login-
            // three-code-paths-2026-07-20) — this used to be a hand-rolled
            // `runCliLogin` call with no fallback when the CLI produced no
            // scrapeable URL, which is every time for Claude Code v2.1.x. That
            // left this specific call site — the one "Retry Login" actually
            // triggers — stuck polling CheckCliAuth against a dir nothing was
            // writing to, for the full 5-minute deadline, every single click.
            // linkTarget lets a tier-2/3 success register a real Armory
            // account bound to THIS agent instead of just seeding a file
            // nothing tracks (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md
            // §7 — required now that the resolver's spawn gate has no
            // ambient exception).
            const agentDefinitionId = blockData?.meta?.agentId as string | undefined;

            // Reuse the account already bound to this agent for this
            // provider, if any — reagent P1: without this, every retry
            // through this flow minted a brand-new account+dir instead of
            // refreshing the one already in use, orphaning an unlinked
            // IdentityAccount row/dir on every failed "Retry Login" click.
            let existingAccountId: string | undefined;
            if (agentDefinitionId) {
                try {
                    const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, {
                        agent_id: agentDefinitionId,
                    });
                    existingAccountId = links.find((l) => l.provider === provider.id)?.account_id;
                } catch {
                    // Best-effort — a fresh account still gets minted below if this lookup fails.
                }
            }

            // Tier 2/3 mint (or reuse) an isolated account dir that's
            // DIFFERENT from the pre-login `authEnv` closure above — this
            // local copy is what the post-login recheck below actually
            // queries, so it must be refreshed. reagent P0: without this,
            // the "seeded"/"terminal-success" recheck queried the OLD
            // directory (nothing had ever been written there) and reported
            // authenticated: false even though the login just succeeded,
            // defeating the entire "Retry Login" flow this file exists for.
            let recheckAuthEnv = authEnv ?? {};
            // Captured for the "opened" case specifically — tier 1 mints
            // and reports the account but does NOT persist/link it (it
            // returns before confirming completion); once THIS function's
            // own poll below confirms authenticated: true, it must call
            // persistAndLinkAccount itself using these. reagent P0 on
            // #2263: without this, a tier-1 login that succeeds for any
            // provider whose CLI actually prints a URL (not
            // requiresLoginTty — e.g. gemini/copilot) never gets a real
            // IdentityAccount, and the resolver's spawn gate blocks the
            // agent on its very next spawn regardless of what this poll
            // reports now.
            let openedAccountId: string | undefined;
            let openedAccountDir: string | undefined;
            // See catalog.ts's DEAD END note — providers with
            // headlessLoginUrlUnsupported skip straight past tier 1's
            // URL-capture wait, so there's no login-link timer to label here.
            setPhase(
                provider.headlessLoginUrlUnsupported
                    ? { kind: "opening-login-terminal" }
                    : { kind: "waiting-for-login-link", deadlineMs: Date.now() + LOGIN_LINK_CAPTURE_LABEL_MS },
            );
            const outcome = await runProviderLogin({
                provider,
                cliPath: cliResult.cli_path,
                authEnv: authEnv ?? {},
                setAuthUrl,
                log,
                isCancelled,
                linkTarget: agentDefinitionId ? { blockId, agentDefinitionId } : undefined,
                existingAccountId,
                onAccountRegistered: (accountId, dir) => {
                    openedAccountId = accountId;
                    openedAccountDir = dir;
                    if (provider.authConfigDirEnvVar) {
                        recheckAuthEnv = { ...(authEnv ?? {}), [provider.authConfigDirEnvVar]: dir };
                    }
                },
                // See catalog.ts's DEAD END note — for these providers tier 1
                // cannot ever succeed, so skip its ~15s capture wait instead
                // of running (and always losing) it on every login.
                skipTier1: provider.headlessLoginUrlUnsupported === true,
                // Without this, the phase set just above (a URL-capture
                // countdown, or a static "opening terminal" for skipTier1)
                // never updates again for the rest of this single await —
                // tier 2/3 inside runProviderLogin can run for up to 5 more
                // minutes with the footer frozen on whatever was true when
                // this call started. reagent P1 on PR #2300.
                onTierChange: (event) => {
                    if (event.tier === "fallback") {
                        setPhase({ kind: "opening-login-terminal" });
                    } else {
                        setPhase({ kind: "waiting-for-login-completion", deadlineMs: event.deadlineMs });
                    }
                },
            });

            let authenticated = false;
            let authedEmail: string | null = null;

            if (outcome === "opened") {
                // A real OAuth URL was captured and opened — poll until the
                // user finishes there, cancels, or 5 minutes elapse.
                log("auth", "waiting for login to complete...");
                const deadline = Date.now() + 5 * 60 * 1000;
                setPhase({ kind: "waiting-for-login-completion", deadlineMs: deadline });
                while (!isCancelled() && Date.now() < deadline && !authenticated) {
                    await new Promise<void>((r) => setTimeout(r, 2000));
                    if (isCancelled()) break;
                    try {
                        const recheck = await RpcApi.CheckCliAuthCommand(TabRpcClient, {
                            cli_path: cliResult.cli_path,
                            auth_check_args: provider.authCheckCommand,
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
                // Tier 1 minted the account but deliberately didn't persist
                // it (see run-provider-login.ts's persistAndLinkAccount doc
                // comment) — now that THIS poll has confirmed the login
                // actually completed, persist and link it for real.
                if (authenticated && openedAccountId && openedAccountDir) {
                    await persistAndLinkAccount(
                        {
                            provider,
                            cliPath: cliResult.cli_path,
                            authEnv: authEnv ?? {},
                            setAuthUrl,
                            log,
                            linkTarget: agentDefinitionId ? { blockId, agentDefinitionId } : undefined,
                        },
                        openedAccountId,
                        openedAccountDir,
                    );
                }
            } else if (outcome === "seeded" || outcome === "terminal-success") {
                // A credential already landed on disk (copied from a valid
                // global login, or completed in the terminal-fallback tier,
                // which already polled internally) — one-shot confirm instead
                // of polling again.
                setPhase({ kind: "verifying" });
                try {
                    const recheck = await RpcApi.CheckCliAuthCommand(TabRpcClient, {
                        cli_path: cliResult.cli_path,
                        auth_check_args: provider.authCheckCommand,
                        auth_env: recheckAuthEnv,
                    }, { timeout: 10000 });
                    authenticated = recheck.authenticated;
                    authedEmail = recheck.email ?? null;
                } catch {
                    // The credential is on disk even if this recheck RPC
                    // itself failed transiently — don't block success on it.
                    authenticated = true;
                }
            }
            // "terminal-timeout" / "terminal-unavailable" leave authenticated
            // false and fall through to the AMX-AUTH-002 error below.

            setLoginWaiting(false);
            // Reap the login CLI now the attempt has concluded (success,
            // timeout, or cancel). On manual-paste success the child self-exits;
            // if creds appeared without a paste it would otherwise linger at the
            // prompt. cancelCliLogin is idempotent and host-side.
            getApi().cancelCliLogin().catch(() => {});
            setAuthUrl(null);

            if (isCancelled()) {
                setPhase({ kind: "failed", reason: "cancelled" });
                return "auth_failed";
            }

            if (authenticated) {
                const emailPart = authedEmail ? ` as ${authedEmail}` : "";
                log("auth", `authenticated${emailPart}`);
                opts.onLoginSuccess?.(authedEmail);
            } else {
                // Synthesize an AMX-AUTH-002 wire payload so the log
                // entry renders with the catalog's friendly title +
                // retry hint, matching the typed-error pattern used
                // elsewhere. Same retry-first / error-last ordering
                // as above.
                const t = translateError({
                    code: "AMX-AUTH-002",
                    details: { provider: provider.id, seconds: 300 },
                });
                log("auth", `retry: ${t.retry ?? `run '${provider.cliCommand} ${provider.authLoginCommand.join(" ")}' manually`}`, "warn");
                log("auth", `${t.title}: ${t.message}`, "error");
                setPhase({ kind: "failed", reason: t.message });
                return "auth_failed";
            }
        } catch (err: any) {
            setLoginWaiting(false);
            getApi().cancelCliLogin().catch(() => {});
            setAuthUrl(null);
            log("auth", `login failed: ${err?.message ?? String(err)}`, "error");
            log("auth", `run: ${provider.cliCommand} ${provider.authLoginCommand.join(" ")}`, "warn");
            setPhase({ kind: "failed", reason: err?.message ?? String(err) });
            return "auth_failed";
        }
    }

    // Phase 3: Controller Registration
    setPhase({ kind: "verifying" });
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

    setPhase({ kind: "ready" });
    return "success";
}
