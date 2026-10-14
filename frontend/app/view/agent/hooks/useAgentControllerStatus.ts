// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentControllerStatus — runs an agent pane's launch and login flows.
 *
 * Step 4 of docs/specs/SPEC_AGENT_VIEW_MODULARIZATION_2026_04_13.md.
 *
 * Every piece of state here (the footer phase, `canRetry`, `authStatus`,
 * the auth URL and notice, `loginWaiting`, the in-flight login, the cancel
 * flag and the recovery intent) lives in one per-pane reducer,
 * `store/launch-phase-state`. This hook does the I/O and reports each step
 * to it as a command. The provider steps (resolve the CLI, check auth, log
 * in and confirm, restart the controller) come from
 * flows/login-and-confirm.ts, shared with the mount-time launch flow and
 * `/login`.
 *
 * Functions exposed:
 *   - startLaunchFlow() — runs the mount-time launch flow once
 *   - relogin() / loginViaTerminal() — the pane's login actions
 *   - cancelLogin() — cancels the login in flight and kills the host CLI
 *
 * `setAuthUrl` is exposed alongside the accessor because the slash-command
 * `/login` handler in agent-view.tsx needs to manually set the OAuth URL
 * when the user types the command directly (separate from the launch
 * flow's auto-login phase).
 */

import { createMemo, onCleanup, type Accessor } from "solid-js";
import { withLoginWindow } from "../flows/login-window";
import { loginBackend } from "../flows/login-backend";
import { getBlockMetaKeyAtom } from "@/app/store/global";
import { sleep } from "@/util/util";
import { RpcApi } from "@/app/store/rpc-api";
import * as MOS from "@/app/store/mos";
import { TabRpcClient } from "@/app/store/rpc-util";
import { runLaunchFlow } from "../flows/launch-flow";
import {
    checkCliAuth,
    linkedAccountFor,
    logInAndConfirm,
    resolveProviderCli,
} from "../flows/login-and-confirm";
import { registerController } from "../flows/register-controller";
import type { LaunchPhase } from "../flows/launch-phase";
import type { ProviderDefinition } from "../providers";
import { bindAccountToAgent, type BindCandidate } from "@/app/view/identity/bind-to-agent-menu";
import type { Account } from "@/app/view/identity/identity-model";
import {
    createLaunchPhaseStore,
    type AuthStatus,
    type LoginVia,
    type MountResult,
} from "@/app/store/launch-phase-state";
import type { LoginStep } from "@/app/store/launch-phase-state/types";

import type { LogFn } from "../types";
import { ensureProviderAuthDir } from "../agent-launch-env";
import type { BlockControllerRuntimeStatus } from "@/types/rpc/BlockControllerRuntimeStatus";
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
     * Called when a recovery action (relogin / terminal login) has
     * successfully refreshed the credential. The pane wires this to retry the
     * failed turn, so a single recovery click both fixes the credential AND
     * drives the agent back to a working state — without it, the login
     * succeeds silently and the 401 failure row lingers (the "Login Again does
     * nothing" symptom).
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
    /** Durable logged-in/logged-out state for the composer strip's status
     *  tag — see the `AuthStatus` doc comment above. */
    authStatus: Accessor<AuthStatus>;
    startLaunchFlow: () => Promise<void>;
    /**
     * Force a provider re-login, bypassing the auth-status check. Wired to
     * TWO different call sites with different retry semantics:
     *   - The failure-banner "Login Again" action (a real turn failed on a
     *     401): `retryAfterLogin` defaults to true so the failed turn
     *     resends automatically once the credential is fixed.
     *   - The mount-time "Log in" button (agent-view.tsx, shown on
     *     `canRetry`/auth_failed — no turn was ever attempted): callers MUST
     *     pass `{ retryAfterLogin: false }`, or a successful login on an
     *     agent with prior history silently resends its LAST OLD MESSAGE as
     *     a brand-new turn the instant login completes — surprising on its
     *     own, and it also buries the "Login successful" notification under
     *     the new turn's immediate stream of output (reported: "no
     *     indication" after a successful mount-time login).
     * Always opens the OAuth, bypassing CheckCliAuth. See
     * SPEC_REAUTH_FROM_AUTH_ERROR §11.
     */
    relogin: (opts?: { retryAfterLogin?: boolean }) => Promise<void>;
    /**
     * Open a real terminal window (CREATE_NEW_CONSOLE on Windows) running the
     * provider's login command so the OS can open the browser — the piped/PTY
     * paths that `runCliLogin` uses are headless and block the browser. After
     * spawning, polls every 5s for up to 5 minutes; once credentials appear,
     * seeds a real per-account isolated dir and registers/links the account
     * (not just a file — PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7).
     */
    /**
     * Open a real terminal for the browser OAuth.
     *
     * `retryAfterLogin` mirrors {@link relogin}'s identical argument and must
     * be decided by the CALLER, at click time, from the failure the button
     * belongs to — NOT re-derived after the fact. Defaults to `true` (a turn
     * ran and failed on auth; re-run it). Pass `false` for the pre-launch
     * case: no turn ever ran, so there is nothing to retry and the agent needs
     * its startup sequence instead.
     *
     * Deriving this after login succeeded is what produced the cross-hook race
     * reagent and manoz both caught on PR #2951 — the success path's own
     * `setCanRetry(false)` synchronously flushes an effect that clears the
     * failure this decision was being read from.
     */
    loginViaTerminal: (opts?: { retryAfterLogin?: boolean }) => Promise<void>;
    /**
     * AuthUrlBox's "Use terminal instead" secondary action
     * (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.3 surface 2): cancels
     * whichever in-app login session is currently in flight and switches to
     * loginViaTerminal() once it's actually torn down (loginViaTerminal()
     * alone would no-op — it guards on the same in-flight flag).
     */
    useTerminalInstead: () => Promise<void>;
    cancelLogin: () => void;
    /**
     * True once `cancelLogin()` has fired for the CURRENTLY in-flight login
     * attempt (any of relogin()/loginViaTerminal(), OR
     * /login via `resetCancelled()` below — reset false at the start of
     * each). Exposed so a caller with its own long-running poll against the
     * SAME shared AuthUrlBox UI — /login (commands/global/login.ts) is the
     * one case, since it drives the identical `setAuthUrl`/Cancel/"Use
     * terminal instead" surface but isn't itself one of the functions
     * above — can notice an explicit cancel and stop polling instead of
     * running to its own multi-minute timeout. reagent P1 on PR #2413
     * (round 3, second pass).
     */
    isCancelled: () => boolean;
    /**
     * Reset the cancellation flag `isCancelled()` reads, mirroring the
     * reset every OTHER login-starting function
     * (relogin()/loginViaTerminal()) already does at its
     * own start. /login (commands/global/login.ts) is the one login-
     * starting flow that lives outside this hook and never called any of
     * those three, so without this a PRIOR attempt's cancel (via the same
     * shared AuthUrlBox's Cancel button) left `isCancelled()` reading
     * `true` forever — every SUBSEQUENT, otherwise-unrelated /login
     * invocation in the pane then hit its cancellation check as already
     * true and silently reported success without ever polling for
     * authentication. reagent P1 on PR #2413 (round 3, third pass).
     */
    resetCancelled: () => void;
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
    /**
     * Kill (if running) and respawn this pane's controller process, then
     * refresh its runtime status. Originally internal-only (used by the
     * login-recovery flows to refresh a stale-but-alive process); also
     * exposed to /login's slash-command handler (a fully separate code path
     * with no access to this hook's closure) so a successful /login on a
     * pane whose persistent controller was already alive actually restarts
     * it onto the refreshed credential — `send_message` only spawns a
     * fresh process when one isn't already running, so without this the
     * next message would bypass every guard in PR #2338
     * (canRetry/loginWaiting/authFailureToPreserve are all correctly
     * cleared by then) and still reach the stale process, reproducing the
     * delayed "Not logged in" failure (codex P1, seventh re-review).
     * See `forceControllerRefresh`'s own doc comment for the full
     * rationale. Best-effort — logs a warning on failure rather than
     * throwing, matching every other call site.
     */
    forceControllerRefresh: (context?: "login") => Promise<boolean>;
    /**
     * Mark a recovery attempt as in flight / resolved, feeding the same
     * shared counter behind `loginWaiting()` that `relogin()`/
     * `loginViaTerminal()` already use internally.
     * Exposed so /login's slash-command handler (a fully separate code
     * path — see `forceControllerRefresh`'s doc comment) can register its
     * own up-to-5-minute poll as an in-flight recovery too. Without this,
     * `loginWaiting()` reads `false` for the entire duration of a /login
     * attempt: a second message the user sends while it's still polling
     * gets held with `authWasKnownBadAtQueueTime: false` (neither
     * `canRetry()` nor `loginWaiting()` is true — mid-turn "auth" failures
     * don't set `canRetry`, and /login never touched `loginWaiting` at
     * all), so if /login then fails, that held message flushes straight to
     * the still-known-bad controller once the phase resets to Idle. Codex
     * P1 on PR #2338 (ninth re-review). Caller MUST call `endRecoveryFlow`
     * exactly once per `beginRecoveryFlow` call (a `finally` block), same
     * contract as the counter's other three callers.
     */
    /**
     * Register a recovery flow. Pass `retryAfterLogin` when the caller drives
     * its own login (i.e. `/login`) — see the implementation's comment.
     */
    beginRecoveryFlow: (retryAfterLogin?: boolean) => void;
    /** Pairs with `beginRecoveryFlow` — see its doc comment. */
    endRecoveryFlow: () => void;
    /**
     * Re-run the CLI auth check against this pane's CURRENT `cmd`/`cmd:env`
     * block meta, without opening any login flow. Used after an external
     * bind event (a different pane, Connectors' Bind-to-Agent menu, or
     * this pane's own `bindExistingAccount` below) to verify the newly
     * bound credential actually works before declaring the pane unblocked.
     * On success, clears `canRetry`/`authNotice` via `notifyControllerHealthy()`
     * and returns `true`; the CALLER is responsible for also clearing a
     * live `state.failure` row if one is showing (reducer-owned, outside
     * this hook — mirror the exact `paneSnapshot(...).failure?.data.code
     * === "auth"` guard `agent-view.tsx`'s `onActiveTurnConfirmed` already
     * uses, so an unrelated concurrent non-auth failure is never silently
     * wiped). See docs/specs/SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md §2.2.
     */
    recheckAuthAfterBind: () => Promise<boolean>;
    /**
     * Bind `account` to this pane's agent (link upsert + best-effort live
     * `cmd:env`/forced-resync apply) — identical sequence to Connectors'
     * Bind-to-Agent context menu (`bindAccountToAgent`, reused directly, not
     * reimplemented). Deliberately does NOT retry the failed turn or call
     * `recheckAuthAfterBind` itself: binding fires the same
     * `agentidentities:changed:<agentId>` event any other bind source does,
     * and the pane's own subscription to that event (agent-view.tsx) is
     * what re-verifies and unblocks — one code path for "how does a pane
     * recover after a bind," regardless of where the bind came from. See
     * spec §3.4.
     */
    bindExistingAccount: (account: Account) => Promise<void>;
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
        const authDir = await ensureProviderAuthDir(prov.id);
        const env: Record<string, string> = { [prov.authConfigDirEnvVar]: authDir };
        if (prov.authExtraEnv) Object.assign(env, prov.authExtraEnv);
        return env;
    } catch {
        return undefined; // non-fatal — fall back to default auth dir
    }
}

/** The string entries of this block's `cmd:env` meta. */
function blockAuthEnv(blockId: string): Record<string, string> {
    const envMeta = getBlockMetaKeyAtom(blockId, "cmd:env")();
    const authEnv: Record<string, string> = {};
    if (envMeta && typeof envMeta === "object") {
        for (const [k, v] of Object.entries(envMeta as Record<string, unknown>)) {
            if (typeof v === "string") authEnv[k] = v;
        }
    }
    return authEnv;
}

export function useAgentControllerStatus(
    opts: UseAgentControllerStatusOptions,
): UseAgentControllerStatus {
    const store = createLaunchPhaseStore(opts.blockId);
    const { dispatch } = store;

    // Derived spinner state — caller wires this into the AgentFooter loading prop
    const isLoading = createMemo(() => store.mountRunning() || !store.agentReady());

    const setAuthUrl = (url: string | null) => {
        dispatch({ type: "AuthUrlSet", url });
    };
    const setAuthNotice = (notice: string | null) => {
        dispatch({ type: "AuthNoticeSet", notice });
    };
    const isCancelled = () => store.snapshot().cancelled;

    // /login and binds join the same counter behind loginWaiting() as the
    // pane's own logins: a plain boolean let whichever flow finished first
    // clear it while another was still waiting on a credential. Each begin
    // needs exactly one end.
    const beginRecoveryFlow = (retryAfterLogin?: boolean) => {
        dispatch({ type: "RecoveryBegan", retryAfterLogin });
    };
    const endRecoveryFlow = () => {
        dispatch({ type: "RecoveryEnded" });
    };

    const startLaunchFlow = async () => {
        const started = dispatch({ type: "MountStarted" }).some((e) => e.type === "mount-started");
        if (!started) return;
        const prov = opts.provider();
        // "success" means the controller registered, not that the login was
        // confirmed: the auth check may have failed transiently and been
        // skipped. Only a confirmed check shows the "Logged in" tag.
        let authConfirmed = false;
        let result: MountResult;
        try {
            const authEnv = await buildAuthEnv(prov);
            result = await runLaunchFlow({
                blockId: opts.blockId,
                provider: prov,
                log: opts.log,
                setLaunchPhase: (phase) => {
                    dispatch({ type: "MountPhaseReported", phase });
                },
                authEnv,
                onNotify: opts.onNotify,
                getInitialTermSize: opts.getInitialTermSize,
                onControllerStatus: opts.onControllerStatus,
                onAuthCheckResult: (confirmed) => { authConfirmed = confirmed; },
            });
        } catch (err: any) {
            opts.log("error", err?.message ?? String(err), "error");
            result = "error";
        }
        try {
            dispatch({ type: "MountFinished", result, authConfirmed });
            if (result === "success") opts.onReady?.();
        } catch (err: any) {
            opts.log("error", err?.message ?? String(err), "error");
        } finally {
            dispatch({ type: "MountEnded" });
        }
    };

    /**
     * Resolve the provider CLI directly when block meta has no `cmd` yet.
     * Falling back to the gated launch flow would trust `CheckCliAuth`'s
     * expired-but-present false positive and skip the login the user asked
     * for (retro-agent-auth-relogin-noop-2026-07-01 H2).
     */
    const resolveCliForRecovery = async (
        prov: ProviderDefinition,
        action: string,
    ): Promise<{ cliPath: string } | { error: string }> => {
        opts.log("auth", `${action}: CLI path not in block meta — resolving it directly`, "warn");
        try {
            const r = await resolveProviderCli(prov, opts.blockId);
            // Persist it so the next click reuses it. Best-effort.
            try {
                const oref = MOS.makeORef("block", opts.blockId);
                await RpcApi.SetMetaCommand(TabRpcClient, { oref, meta: { cmd: r.cli_path } });
            } catch { /* non-fatal: next click re-resolves */ }
            return { cliPath: r.cli_path };
        } catch (err: any) {
            const msg = `Couldn't find the ${prov.cliCommand} CLI to run the login: ${err?.message ?? String(err)}`;
            opts.log("auth", msg, "error");
            return { error: msg };
        }
    };

    /** Force the persistent controller to restart (or register, if none
     *  exists yet) after a login recovery actually persisted a new/refreshed
     *  account. `send_message` only spawns a fresh process when one isn't
     *  already running — an agent whose CLI was already alive (spawned
     *  earlier with the old/missing credential) keeps running on that stale
     *  env forever otherwise. `forcerestart: true` is a no-op when no
     *  controller exists yet (the first-ever-login case — this is that
     *  pane's only registration, since launch-flow.ts bailed on auth_failed
     *  before its own). See
     *  REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md G4/G5.
     *
     *  Returns whether the restart succeeded: /login must not declare the
     *  pane healthy when it failed. The pane's own logins ignore it
     *  (best-effort). */
    const forceControllerRefresh = async (_context: "login" = "login"): Promise<boolean> => {
        try {
            const rts = await registerController(opts.blockId, {
                forceRestart: true,
                termSize: opts.getInitialTermSize?.(),
            });
            if (rts) opts.onControllerStatus?.(rts);
            return true;
        } catch (e: any) {
            const detail = e?.message ?? String(e);
            opts.log(
                "auth",
                `signed in, but couldn't refresh the running agent with the new login — reopen this pane if it still shows as logged out: ${detail}`,
                "warn",
            );
            return false;
        }
    };

    /** Auth env for recovery actions: block meta `cmd:env` when present, else
     *  the isolated-dir env the launch flow would have used (agent never
     *  launched), so the login lands where the agent will read it. */
    const recoveryAuthEnv = async (prov: ProviderDefinition): Promise<Record<string, string>> => {
        const authEnv = blockAuthEnv(opts.blockId);
        if (Object.keys(authEnv).length > 0) return authEnv;
        return (await buildAuthEnv(prov)) ?? {};
    };

    /** The pane's own login: "Log in" / "Login Again" (relogin) or "Login via
     *  terminal". One at a time; the reducer drops a second request. */
    const runPaneLogin = async (via: LoginVia, retryAfterLogin: boolean) => {
        if (store.snapshot().login) return;
        const prov = opts.provider();
        if (!prov) {
            opts.log("auth", `${via === "relogin" ? "re-login" : "login via terminal"}: no active provider`, "warn");
            return;
        }
        const startedEvent = dispatch({ type: "LoginRequested", via, retryAfterLogin }, "user")
            .find((e) => e.type === "login-started");
        if (!startedEvent || startedEvent.type !== "login-started") return;
        const id = startedEvent.id;
        const step = (s: LoginStep) => {
            dispatch({ type: "LoginStepReported", id, ...s });
        };
        try {
            // The CLI path and auth env are in block meta once the agent has
            // launched; reuse them.
            let cliPath = getBlockMetaKeyAtom(opts.blockId, "cmd")() as string | undefined;
            if (!cliPath) {
                step({ step: "resolving-cli" });
                const resolved = await resolveCliForRecovery(prov, via === "relogin" ? "re-login" : "login via terminal");
                if ("error" in resolved) {
                    dispatch({ type: "LoginFailed", id, failure: { kind: "error", message: resolved.error } });
                    return;
                }
                cliPath = resolved.cliPath;
            }
            step({ step: "cli-ready" });
            const authEnv = await recoveryAuthEnv(prov);
            const agentDefinitionId = getBlockMetaKeyAtom(opts.blockId, "agentId")() as string | undefined;
            // A terminal login skips the link capture; so does a provider
            // whose CLI never prints a link.
            const skipTier1 = via === "terminal" || prov.headlessLoginUrlUnsupported === true;
            step({ step: "provider-login", linkCapture: !skipTier1 });
            const result = await logInAndConfirm({
                provider: prov,
                cliPath,
                authEnv,
                setAuthUrl,
                log: opts.log,
                isCancelled,
                // Links a new account to THIS agent
                // (PLAN_LOGIN_SINGLE_PATH_CONSOLIDATION_2026_07_20.md §7).
                linkTarget: agentDefinitionId ? { blockId: opts.blockId, agentDefinitionId } : undefined,
                // Refresh the account already bound here instead of minting
                // and orphaning a new one on every retry.
                existingAccountId: agentDefinitionId ? await linkedAccountFor(agentDefinitionId, prov.id) : undefined,
                skipTier1,
                // Relogin keeps the in-app session open in runProviderLogin
                // (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.1).
                ...(via === "relogin" ? { awaitTier1Completion: true } : {}),
                onTierChange: (event) => {
                    step(event.tier === "fallback" ? { step: "fallback" } : { step: "awaiting-completion", deadlineMs: event.deadlineMs });
                },
            });
            if (result.kind === "signed-in") {
                opts.log("auth", retryAfterLogin ? "Login successful — retrying…" : "Login successful");
                dispatch({ type: "LoginSucceeded", id });
                // Restart a stale process BEFORE anything is resent to it.
                await forceControllerRefresh();
                opts.onLoginSuccess?.(result.email);
                // loginWaiting() must read false before the resend below:
                // useAgentCommands' fast-fail guard checks it on every send.
                dispatch({ type: "LoginRecoveryReleased", id });
                if (retryAfterLogin) {
                    opts.onRecovered?.();
                } else {
                    // No turn ever ran: send the startup sequence instead.
                    // onReady self-guards on `agent:sessionid`.
                    opts.onReady?.();
                }
            } else {
                const failed = dispatch({ type: "LoginFailed", id, failure: result }).find((e) => e.type === "login-failed");
                if (via === "terminal" && result.kind === "not-detected" && failed?.type === "login-failed" && failed.notice) {
                    opts.log("auth", failed.notice, "warn");
                }
            }
        } catch (err: any) {
            const msg = `${via === "relogin" ? "Re-login failed" : "Terminal login failed"}: ${err?.message ?? String(err)}`;
            opts.log("auth", msg, "error");
            dispatch({ type: "LoginFailed", id, failure: { kind: "error", message: msg } });
        } finally {
            dispatch({ type: "LoginEnded", id });
        }
    };

    // The login's window opens inside the click (its URL comes later), and is
    // closed on every way out of the flow if no URL used it (login-window.ts).
    const relogin = (reloginOpts: { retryAfterLogin?: boolean } = {}) =>
        store.snapshot().login
            ? Promise.resolve()
            : withLoginWindow(opts.provider(), () => runPaneLogin("relogin", reloginOpts.retryAfterLogin ?? true));

    // A real visible terminal, so the browser OAuth works: runProviderLogin's
    // tier 3, with tier 1 skipped since the user asked for a terminal.
    const loginViaTerminal = (terminalOpts: { retryAfterLogin?: boolean } = {}) =>
        runPaneLogin("terminal", terminalOpts.retryAfterLogin ?? true);

    const cancelLogin = () => {
        dispatch({ type: "CancelRequested" }, "user");
        loginBackend().cancel().catch(() => {});
        opts.log("auth", "login cancelled", "warn");
    };

    // "Use terminal instead" — AuthUrlBox's secondary action
    // (SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.3 surface 2). Shown for
    // relogin's sessions and /login's, so it waits on the shared recovery
    // counter (which both feed) rather than the pane's own login, then starts
    // the terminal login once the cancelled one has torn down. The backstop
    // is generous: a poll tick can sit inside a 10 s auth-check RPC.
    const useTerminalInstead = async () => {
        // Read before cancelling: teardown resets it. For a /login session
        // that declared nothing this is false, so onReady rather than a retry.
        const intent = store.snapshot().retryIntent;
        cancelLogin();
        const waiting = () => store.snapshot().recoveryFlows > 0;
        const deadline = Date.now() + 20_000;
        while (waiting() && Date.now() < deadline) {
            await sleep(200);
        }
        if (waiting()) {
            // Never fail silently (retro-agent-auth-relogin-noop-2026-07-01 §5.1).
            setAuthNotice(
                "The previous login attempt is taking longer than expected to stop. " +
                "Please wait a moment and try “Login via terminal” again.",
            );
            return;
        }
        // The user switched flows; the cancelled one's notice doesn't apply.
        setAuthNotice(null);
        // A switch of mechanism, not a new decision: keep the same intent.
        await loginViaTerminal({ retryAfterLogin: intent });
    };

    const notifyControllerHealthy = () => {
        dispatch({ type: "ControllerHealthy" });
    };

    const recheckAuthAfterBind = async (): Promise<boolean> => {
        const prov = opts.provider();
        if (!prov) return false;
        // Check the env this pane will actually spawn with (after a bind,
        // the newly-bound account's dir), never a freshly derived one.
        const cliPath = getBlockMetaKeyAtom(opts.blockId, "cmd")() as string | undefined;
        if (!cliPath) return false;
        try {
            const result = await checkCliAuth(cliPath, prov.authCheckCommand, blockAuthEnv(opts.blockId), 10000);
            if (!result.authenticated) return false;
            notifyControllerHealthy();
            return true;
        } catch {
            // Transient RPC error: still blocked. The next bind event or a
            // manual retry gets another chance.
            return false;
        }
    };

    const bindExistingAccount = async (account: Account): Promise<void> => {
        const agentDefinitionId = getBlockMetaKeyAtom(opts.blockId, "agentId")() as string | undefined;
        if (!agentDefinitionId) return;
        // Never retries a turn, so it declares no intent, but it is still an
        // in-flight recovery for loginWaiting().
        beginRecoveryFlow();
        setAuthNotice(null);
        try {
            const candidate: BindCandidate = {
                agentId: agentDefinitionId,
                agentName: "",
                runningBlockId: opts.blockId,
                boundHere: false,
                boundElsewhereName: null,
            };
            await bindAccountToAgent(account, candidate);
        } catch (e: any) {
            // A failed link upsert surfaces as a notice, like every other
            // failed recovery here (callers don't attach their own catch).
            const msg = `Couldn't bind "${account.name}": ${e?.message ?? String(e)}`;
            opts.log("auth", msg, "error");
            setAuthNotice(msg);
        } finally {
            endRecoveryFlow();
        }
    };

    // Pane closed: cancel unconditionally. The login CLI can already be
    // spawned before loginWaiting() turns true, and the host cancel is
    // idempotent, so gating on it could orphan the child.
    onCleanup(() => {
        dispatch({ type: "Disposed" });
        loginBackend().cancel().catch(() => {});
    });

    return {
        authUrl: store.authUrl,
        setAuthUrl,
        authNotice: store.authNotice,
        setAuthNotice,
        canRetry: store.canRetry,
        flowRunning: store.mountRunning,
        agentReady: store.agentReady,
        isLoading,
        loginWaiting: store.loginWaiting,
        launchPhase: store.phase,
        authStatus: store.authStatus,
        startLaunchFlow,
        relogin,
        loginViaTerminal,
        useTerminalInstead,
        notifyControllerHealthy,
        forceControllerRefresh,
        cancelLogin,
        isCancelled,
        resetCancelled: () => {
            dispatch({ type: "CancelReset" });
        },
        beginRecoveryFlow,
        endRecoveryFlow,
        recheckAuthAfterBind,
        bindExistingAccount,
    };
}
