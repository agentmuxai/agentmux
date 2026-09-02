// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * launch-flow — the full agent launch sequence extracted from agent-view.tsx.
 *
 * Step 2 of docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Phases:
 *   0. Container agents require a container runtime (docker/podman/…).
 *      Skipped for host agents.
 *   1. CLI detection/installation — resolve the provider's cli_command, with
 *      optional npm install if not found and pinnedVersion is set. Subscribes
 *      to `install_progress` events so the caller's log sink sees each npm
 *      line as it happens.
 *   2. Auth check — calls the provider's check-auth command. If unauthenticated,
 *      this does NOT open a browser/terminal on its own — it posts a visible
 *      notification explaining that a login is needed and returns
 *      "auth_failed" immediately. The user must click the resulting "Log in"
 *      affordance (wired to `relogin()`) to actually trigger a login attempt.
 *      A login opening with no prior user action was the exact bug this
 *      changed to fix — see docs/specs/SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26.md.
 *   3. Controller registration — ControllerResync on the tab, read status, set
 *      the resulting `resumed-ready`/`fresh-ready` phase (or, if the resync
 *      itself failed, post an honest warning instead of a false all-clear —
 *      see `resyncFailed` below) depending on whether there's a prior turn.
 *      No transcript notification for the plain resumed/fresh case — that
 *      used to post "Resumed…"/"Ready…" via `notify`, but it narrated
 *      nothing the user couldn't already see and rendered indistinguishably
 *      from the agent's own words; removed per the same "no artificial
 *      messages mixed into transcript content" rule as #2420/6191a1928.
 *
 * The caller provides a log sink and a phase/notify callback. All other
 * state is derived from the block_id + provider definition. `setAuthUrl`/
 * `isCancelled`/`setLoginWaiting` remain on `LaunchFlowOptions` for callers
 * that still pass them, but this function itself no longer drives a login
 * attempt, so it never touches them.
 *
 * Return values:
 *   - "success"     — controller registered, ready for user input
 *   - "auth_failed" — not authenticated; the caller must show a "Log in" affordance
 *                     (wired to `relogin()`) — this function never attempts a login itself
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
import { staticTabId } from "@/app/store/global";
import { lastLinkedAccountId } from "../providers/provider-id-aliases";
import type { LaunchPhase } from "./launch-phase";
import type { ProviderDefinition } from "../providers";

import type { LogFn } from "../types";

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
    /** Posts a permanent, visible line into the pane's conversation — for
     *  anything that isn't a login *success* (that's `onLoginSuccess`) but
     *  still shouldn't be silent: a warning before an automatic relogin
     *  attempt, or the end-of-mount "ready"/"resumed" summary. See
     *  docs/specs/SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26.md. */
    onNotify?: (text: string, style: "info" | "warning") => void;
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
    /**
     * Reports whether Phase 2's auth check actually PROVED the CLI is
     * authenticated (`authResult.authenticated === true`) versus proceeding
     * on an unconfirmed check (the RPC itself threw or timed out — logged
     * as "authentication status unknown — will attempt anyway" and NOT
     * treated as `auth_failed`, since a transient check failure shouldn't
     * block launch). Without this, "success" alone is ambiguous: it means
     * "controller registered, ready for input" (see LaunchFlowResult's own
     * doc comment) and is returned identically whether or not login was
     * ever actually confirmed — a caller that maps every "success" straight
     * to `authStatus: "authenticated"` shows a false green "Logged in" tag
     * for a credential that was never checked (reagent/codex P2 on
     * PR #2318). Not called at all on the `auth_failed`/`fatal` paths —
     * those already have their own explicit outcome.
     */
    onAuthCheckResult?: (confirmed: boolean) => void;
}

export type LaunchFlowResult = "success" | "auth_failed" | "fatal";

export async function runLaunchFlow(opts: LaunchFlowOptions): Promise<LaunchFlowResult> {
    const { blockId, provider, log, authEnv } = opts;
    const setPhase = opts.setLaunchPhase ?? (() => {});
    const notify = opts.onNotify ?? (() => {});

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
    // docs/retro/RETRO_DOCKER_DETECTION_DIVERGENCE_2026_07_04.md.
    const blockData = WOS.getWaveObjectAtom<Block>(oref)();
    const agentMode = blockData?.meta?.agentMode ?? "host";
    const agentDefinitionId = blockData?.meta?.agentId as string | undefined;

    // Resolve this agent's ALREADY-BOUND account dir (if any) up front, and
    // use it in place of `authEnv`'s generic provider-default dir for both
    // the meta env below and the auth check in Phase 2. Without this, the
    // mount-time auth check validates the wrong directory for any agent with
    // a real account binding — `ensureAuthDir` (which built `authEnv`) has no
    // way to know which account this specific agent is bound to, so it
    // always resolves the shared default, which can disagree with the
    // per-account dir the real spawn (`inject_identity_env`) uses. See
    // docs/specs/SPEC_AGENT_PANE_MOUNT_AUTH_CHECK_WRONG_DIR_2026_07_31.md.
    //
    // Gated on `authType === "oauth"`, not `authConfigDirEnvVar` truthy —
    // some api-key providers (e.g. Kimi) also populate that field for an
    // unrelated purpose, and the backend deliberately returns no isolated
    // dir for non-oauth providers (codex P2 on PR #2377: calling the
    // OAuth-directory RPC for those anyway logged a spurious "no isolated
    // config dir" error on every mount despite nothing being wrong).
    //
    // Compares canonicalized provider IDs (codex P1 on PR #2377): a link row
    // persisted under a legacy alias (e.g. "claude-code") must still match
    // `provider.id`'s canonical form, the same way the backend spawn
    // resolver already canonicalizes before matching.
    //
    // Reads the account's own stored OAuth dir via GetIdentityAccountCommand
    // rather than `ensureAccountDir`/`identity.ensureaccountdir` (codex P1 on
    // PR #2377): that RPC deterministically reconstructs a dir path from
    // `account_id` + provider instead of reading the account row, so it can
    // return an empty freshly-created directory for an account whose real
    // credential lives at a different, non-canonical stored path (e.g.
    // carried forward from a bundle-era migration) — silently disagreeing
    // with `inject_identity_env`'s actual spawn-time read of `secret_ref.dir`,
    // reintroducing the exact false-"Log in" bug this fix targets.
    let linkedAccountId: string | undefined;
    let linkLookupDone = false;
    let effectiveAuthEnv = authEnv;
    if (agentDefinitionId && provider.authType === "oauth") {
        linkLookupDone = true;
        try {
            const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, {
                agent_id: agentDefinitionId,
            });
            linkedAccountId = lastLinkedAccountId(links, provider.id);
        } catch {
            // Best-effort — treated as "no linked account" if this lookup fails.
        }
        if (linkedAccountId) {
            try {
                const account = await RpcApi.GetIdentityAccountCommand(TabRpcClient, { id: linkedAccountId });
                if (account?.secret_ref?.backend === "oauth_config_dir" && account.secret_ref.dir) {
                    effectiveAuthEnv = { ...authEnv, [provider.authConfigDirEnvVar]: account.secret_ref.dir };
                }
            } catch {
                // Best-effort — fall back to authEnv's generic dir if this lookup fails.
            }
        }
    }

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
            ...(effectiveAuthEnv && Object.keys(effectiveAuthEnv).length > 0 ? { "cmd:env": effectiveAuthEnv } : {}),
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
            auth_env: effectiveAuthEnv,
        }, { timeout: 30000 });
        if (authResult.authenticated) {
            const emailPart = authResult.email ? ` as ${authResult.email}` : "";
            const methodPart = authResult.auth_method ? ` (${authResult.auth_method})` : "";
            log("auth", `authenticated${emailPart}${methodPart}`);
            opts.onAuthCheckResult?.(true);
        } else {
            needsLogin = true;
        }
    } catch (err: any) {
        log("auth", `check failed: ${err?.message ?? String(err)}`, "warn");
        log("auth", "authentication status unknown — will attempt anyway", "warn");
        opts.onAuthCheckResult?.(false);
    }

    if (needsLogin) {
        // Reuse the account already bound to this agent for this provider,
        // if any — used only to distinguish first-login from auth-expired
        // wording below. A REAL account link can only exist if a prior
        // login completed and got persisted (persistAndLinkAccount/
        // finalizeAccount), unlike blockData?.meta?.["cmd"] (an earlier,
        // broken attempt at this signal — reagent P1 on PR #2304: agent-
        // model.ts's launchAgent() writes `cmd` into meta unconditionally
        // at agent-CREATION time, before any login ever happens, so it was
        // true on every genuine first-ever login too).
        //
        // Already looked up above (`linkLookupDone`) for any oauth-class
        // provider — reuse that result instead of calling
        // ListAgentIdentitiesCommand a second time per mount.
        let existingAccountId: string | undefined = linkedAccountId;
        if (!linkLookupDone && agentDefinitionId) {
            try {
                const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, {
                    agent_id: agentDefinitionId,
                });
                existingAccountId = lastLinkedAccountId(links, provider.id);
            } catch {
                // Best-effort — treated as "no existing account" if this lookup fails.
            }
        }

        // DELIBERATELY DOES NOT open a browser/terminal here. This function
        // used to call runProviderLogin() automatically the instant an
        // unauthenticated agent was opened — a login window appearing with
        // no click, no warning, and no way to decline. Per direct user
        // instruction (2026-07-27, superseding SPEC_AGENT_PANE_AUTH_
        // NOTIFICATIONS_2026_07_26.md §8 Q2's "notify-then-proceed"
        // decision): the mount-time flow now only ever posts a notification
        // and stops — actually starting a login is exclusively a user
        // action, via the "Log in" button (`agent-view.tsx`) wired to
        // `relogin()` in `useAgentControllerStatus.ts`. See
        // docs/specs/SPEC_AGENT_PANE_AUTH_NOTIFICATIONS_2026_07_26.md §8 Q6
        // for why first-login and auth-expired still get different wording.
        if (existingAccountId) {
            setPhase({ kind: "auth-expired" });
            notify(`Your ${provider.displayName} login has expired. Click "Log in" to continue.`, "warning");
        } else {
            setPhase({ kind: "first-login" });
            notify(`${provider.displayName} needs you to sign in before this agent can start. Click "Log in" to continue.`, "info");
        }
        log("auth", "not authenticated — waiting for the user to start a login");
        return "auth_failed";
    }

    // Phase 3: Controller Registration
    setPhase({ kind: "verifying" });
    log("controller", "registering subprocess controller...");
    let resumed = false;
    let resyncFailed = false;
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
        } else if (status === "done" || status === "running") {
            // "running" means a persistent controller resumed while its
            // process was still alive (possibly mid-turn) — if anything a
            // STRONGER resume signal than "done". Missing this case (as an
            // earlier revision of this file did) left that path completely
            // silent — not just unstyled, no log line at all. reagent P1 on
            // PR #2303 (confirmed real via persistent.rs's STATUS_RUNNING
            // and useControllerStatusEvents.test.ts).
            resumed = true;
            // "done" and "running" get distinct wording — "previous turn
            // complete" would contradict "running"'s own meaning (the
            // controller may still be alive/mid-turn, not complete).
            log(
                "agent",
                status === "running"
                    ? "resuming a controller that's still alive — send a message to continue"
                    : "previous turn complete — send a message to continue",
            );
        }
    } catch (err: any) {
        // Don't follow a real resync failure with the generic "ready" message —
        // that previously masked every resync error (including the commit-
        // pressure admission gate's "memory full" refusal) with a misleading
        // all-clear a line later. Surface the actual failure at "error" so it's
        // the last, most visible line in the panel. `resumed` stays false here
        // (unknown, not confirmed) — fresh-ready's wording is the safer default
        // when the resync itself failed to tell us which case this is.
        log("controller", `resync failed: ${err?.message ?? String(err)}`, "error");
        resyncFailed = true;
    }

    // reagent P1 on PR #2304: this used to fall through to the cheerful
    // ready/resumed-ready notification below even when the try block above
    // threw — exactly the misrepresentation the comment above claims to
    // avoid. `resyncFailed` breaks that fallthrough. The function's return
    // value stays "success" (unchanged pre-existing contract for this path —
    // widening it to a new failure variant is a bigger, separate change);
    // this only stops the visible notification from lying about it.
    if (resyncFailed) {
        setPhase({ kind: "fresh-ready" });
        notify("Something went wrong finishing setup — if the agent doesn't respond, try reopening this pane.", "warning");
    } else if (resumed) {
        // No transcript notification here (reagent P1 on #2420 / #6191a1928
        // precedent): "Resumed — continuing where you left off" narrated
        // nothing the user couldn't already see (the prior turn's own
        // history is right there) and rendered with "info" style's empty
        // prefix, indistinguishable from the agent's own words. `setPhase`
        // still fires — `resumed-ready` currently renders no footer label
        // either (formatPhaseLabel returns null for it), but keeping the
        // state transition intact costs nothing and leaves a hook for a
        // future non-transcript indicator if one is wanted later.
        setPhase({ kind: "resumed-ready" });
    } else {
        // Same rationale — "Ready — type a message to start" narrated the
        // empty composer sitting right below it.
        setPhase({ kind: "fresh-ready" });
    }
    return "success";
}
