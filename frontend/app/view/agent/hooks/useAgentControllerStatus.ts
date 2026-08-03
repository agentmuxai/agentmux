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
import { getApi, getBlockMetaKeyAtom, staticTabId } from "@/app/store/global";
import { sleep } from "@/util/util";
import { RpcApi } from "@/app/store/rpc-api";
import { BlockService } from "@/app/store/services";
import * as WOS from "@/app/store/wos";
import { TabRpcClient } from "@/app/store/rpc-util";
import { runLaunchFlow } from "../flows/launch-flow";
import { persistAndLinkAccount, runProviderLogin } from "../flows/run-provider-login";
import { registerSeededAccount } from "../flows/register-seeded-account";
import { LOGIN_LINK_CAPTURE_LABEL_MS, type LaunchPhase } from "../flows/launch-phase";
import { lastLinkedAccountId } from "../providers/provider-id-aliases";
import type { ProviderDefinition } from "../providers";

import type { LogFn } from "../types";
export type { LogFn };

/**
 * Durable logged-in/logged-out signal for the composer strip's status tag —
 * distinct from the transient `launchPhase`/`canRetry` signals, which only
 * describe the in-progress launch/recovery flow and get cleared the instant
 * it finishes. "unknown" only applies before the very first auth check
 * resolves (or after a fatal, non-auth error where the check never ran).
 */
export type AuthStatus = "authenticated" | "unauthenticated" | "unknown";

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
    /**
     * Kill (if running) and respawn this pane's controller process, then
     * refresh its runtime status. Originally internal-only (used by the
     * login-recovery flows to refresh a stale-but-alive process); exposed
     * two ways since:
     *   - to the `unresponsive` failure row's "Restart" action, for a
     *     wedged/`Dead` process (`context: "restart"`, PR #2336);
     *   - to /login's slash-command handler (a fully separate code path
     *     with no access to this hook's closure) so a successful /login on
     *     a pane whose persistent controller was already alive actually
     *     restarts it onto the refreshed credential — `send_message` only
     *     spawns a fresh process when one isn't already running, so
     *     without this the next message would bypass every guard in PR
     *     #2338 (canRetry/loginWaiting/authFailureToPreserve are all
     *     correctly cleared by then) and still reach the stale process,
     *     reproducing the delayed "Not logged in" failure (codex P1,
     *     seventh re-review; defaults to `context: "login"`).
     * See `forceControllerRefresh`'s own doc comment for the full
     * rationale. Best-effort — logs a warning on failure rather than
     * throwing, matching every other call site.
     */
    forceControllerRefresh: (context?: "login" | "restart") => Promise<boolean>;
    /**
     * Mark a recovery attempt as in flight / resolved, feeding the same
     * shared counter behind `loginWaiting()` that `relogin()`/
     * `useGlobalLogin()`/`loginViaTerminal()` already use internally.
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
    beginRecoveryFlow: () => void;
    /** Pairs with `beginRecoveryFlow` — see its doc comment. */
    endRecoveryFlow: () => void;
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
    const [authStatus, setAuthStatus] = createSignal<AuthStatus>("unknown");

    // Derived spinner state — caller wires this into the AgentFooter loading prop
    const isLoading = createMemo(() => flowRunning() || !agentReady());

    // Shared counter behind loginWaiting: relogin()/useGlobalLogin()/
    // loginViaTerminal() are guarded by TWO independent in-flight flags
    // (reloginInFlight covers relogin+loginViaTerminal; seedInFlight covers
    // useGlobalLogin only) — nothing disables the failure-row's OTHER
    // recovery buttons while one is running, so a user can genuinely start
    // both concurrently. A plain boolean loginWaiting, set/cleared
    // independently by each function, lets whichever one finishes first
    // clear the flag while the other is still polling for credentials —
    // reopening the exact "send during an unconfirmed recovery" window this
    // signal exists to close. Codex P2 on PR #2338 (fourth re-review). Each
    // caller must call endRecoveryFlow() EXACTLY once per beginRecoveryFlow()
    // call — see relogin()'s recoveryEnded guard for how a function that
    // clears early (before onRecovered, per the reagent P0 fix) avoids a
    // double-decrement from its own trailing finally.
    let activeRecoveryFlows = 0;
    const beginRecoveryFlow = () => {
        activeRecoveryFlows += 1;
        setLoginWaiting(true);
    };
    const endRecoveryFlow = () => {
        activeRecoveryFlows = Math.max(0, activeRecoveryFlows - 1);
        if (activeRecoveryFlows === 0) setLoginWaiting(false);
    };

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
        // "success" from runLaunchFlow means "controller registered, ready
        // for input" — NOT "login confirmed" (its own doc comment is
        // explicit about this: Phase 2 proceeds on an unconfirmed auth
        // check rather than blocking launch on a transient RPC failure).
        // Without tracking this separately, every "success" got mapped
        // straight to authStatus "authenticated", showing a false green
        // "Logged in" tag for a credential that was never actually checked
        // (reagent/codex P2 on PR #2318).
        let authConfirmed = false;
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
                onAuthCheckResult: (confirmed) => { authConfirmed = confirmed; },
            });
            if (result === "success") {
                setAgentReady(true);
                setAuthStatus(authConfirmed ? "authenticated" : "unknown");
                opts.onReady?.();
            } else if (result === "auth_failed" && !loginCancelled) {
                setCanRetry(true);
                setAgentReady(true); // clear spinner so retry button is usable
                setAuthStatus("unauthenticated");
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

    /** Force the persistent controller to restart (or register, if none
     *  exists yet) after a login recovery actually persisted a new/refreshed
     *  account. `send_message` only spawns a fresh process when one isn't
     *  already running — an agent whose CLI was already alive (spawned
     *  earlier with the old/missing credential) keeps running on that stale
     *  env forever otherwise, so a successful login changes nothing for it
     *  until the pane is manually closed and reopened. `forcerestart: true`
     *  is a no-op when no controller exists yet (the first-ever-login case —
     *  this just performs the Phase-3-equivalent registration launch-flow.ts
     *  itself skipped when it bailed on auth_failed) and kills+recreates one
     *  that does, so the new `cmd:env` (from `finalizeAccount`/`useGlobalLogin`'s
     *  own SetMetaCommand) actually takes effect. Best-effort: a failure here
     *  just means the stale process persists until its own next natural
     *  respawn. See REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md
     *  G4/G5.
     *
     *  Mirrors launch-flow.ts's Phase 3 registration (termsize seed +
     *  onControllerStatus) rather than just the bare resync call — for the
     *  first-ever-login path this IS that pane's only registration, so
     *  omitting either left a freshly spawned CLI's PTY at the default width
     *  until the next manual resize, and callers relying on
     *  `onControllerStatus` never heard about it (reagent P2 on PR #2318).
     *
     *  `context` picks the catch-block's log message — originally hardcoded
     *  for the login-recovery case (the only caller until this point), but
     *  the `unresponsive` failure row's "Restart" action now reuses this
     *  same function for a completely unrelated reason (a wedged process,
     *  nothing to do with signing in), so a restart-triggered RPC failure
     *  must not log a misleading "signed in, but..." message (reagent P2 on
     *  PR #2336). Defaults to "login" — every pre-existing call site stays
     *  unchanged.
     *
     *  Returns whether the resync RPC actually succeeded. /login's
     *  slash-command handler consumes this — it must NOT declare the
     *  controller healthy (notifyControllerHealthy/clearAuthFailure) when
     *  the refresh itself failed, or every fast-fail guard this PR added
     *  gets cleared while the controller is still on the stale credential.
     *  Codex P1 on PR #2338 (tenth re-review). The three original callers
     *  (relogin/useGlobalLogin/loginViaTerminal) still ignore the return
     *  value, unchanged — same best-effort contract they've always had. */
    const forceControllerRefresh = async (context: "login" | "restart" = "login"): Promise<boolean> => {
        try {
            const initialTermSize = opts.getInitialTermSize?.();
            await RpcApi.ControllerResyncCommand(TabRpcClient, {
                tabid: staticTabId(),
                blockid: opts.blockId,
                forcerestart: true,
                ...(initialTermSize ? { rtopts: { termsize: initialTermSize } } : {}),
            });
            const rts = await BlockService.GetControllerStatus(opts.blockId);
            if (rts) opts.onControllerStatus?.(rts);
            return true;
        } catch (e: any) {
            const detail = e?.message ?? String(e);
            if (context === "restart") {
                opts.log("agent", `couldn't restart the unresponsive agent process: ${detail}`, "warn");
            } else {
                opts.log(
                    "auth",
                    `signed in, but couldn't refresh the running agent with the new login — reopen this pane if it still shows as logged out: ${detail}`,
                    "warn",
                );
            }
            return false;
        }
    };

    /** Look up the account already bound to THIS agent for `providerId`, if
     *  any — pass as `runProviderLogin`'s `existingAccountId` so a recovery
     *  action reuses/refreshes the same account instead of minting and
     *  orphaning a new one on every retry (the same class of gap reagent
     *  caught in launch-flow.ts's Phase 2 — this hook's `relogin`/
     *  `loginViaTerminal` had it too, just never flagged directly since
     *  neither reported "auth_failed" the same visible way Phase 2 did).
     *
     *  Uses `lastLinkedAccountId` (codex P1 on PR #2377, second round), not
     *  a raw `.find()`: a strict comparison misses a link stored under a
     *  legacy alias, so `runProviderLogin` would mint and link a NEW
     *  canonical account without replacing the alias row — leaving both
     *  rows present, and since spawn injection processes the canonical row
     *  first and the alias row last, the stale alias directory would
     *  silently overwrite the freshly-authenticated one. Recovery would
     *  report success while the very next spawn still used the expired
     *  credential. */
    const existingAccountIdFor = async (providerId: string): Promise<string | undefined> => {
        const agentDefinitionId = getBlockMetaKeyAtom(opts.blockId, "agentId")() as string | undefined;
        if (!agentDefinitionId) return undefined;
        try {
            const links = await RpcApi.ListAgentIdentitiesCommand(TabRpcClient, { agent_id: agentDefinitionId });
            return lastLinkedAccountId(links, providerId);
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

    const relogin = async (reloginOpts: { retryAfterLogin?: boolean } = {}) => {
        if (reloginInFlight) return;
        const retryAfterLogin = reloginOpts.retryAfterLogin ?? true;
        const prov = opts.provider();
        if (!prov) {
            opts.log("auth", "re-login: no active provider", "warn");
            return;
        }
        // Clears the "Log in" button immediately on click — this is also the
        // action the mount-time launch flow's first-login/auth-expired
        // states hand off to (they never trigger a login themselves; see
        // launch-flow.ts), so a stale canRetry=true must not survive into
        // this attempt's own outcome.
        setCanRetry(false);
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
        beginRecoveryFlow();
        // Guards against double-decrementing activeRecoveryFlows: the
        // success branches below call this early (before onRecovered, so
        // that callback doesn't see a stale loginWaiting — reagent P0), and
        // the trailing finally also calls it unconditionally for the
        // failure paths. Exactly one of those two call sites should ever
        // actually decrement per invocation.
        let recoveryEnded = false;
        const endThisRecoveryFlow = () => {
            if (recoveryEnded) return;
            recoveryEnded = true;
            endRecoveryFlow();
        };
        setLaunchPhase({ kind: "checking-auth" });
        // Tracks whether this attempt actually reached a genuine success
        // branch below — declared outside the try so the `finally` block
        // (which needs to read it) can see it. Used to decide whether to
        // restore the mount-time "Log in" button (reagent/codex P1 on
        // PR #2318: every failure exit — timeout, terminal-unavailable,
        // persistence failure, cancellation, or a thrown exception —
        // previously left `canRetry` stuck at false with no way back into
        // a login attempt short of reopening the pane).
        let succeeded = false;
        try {
            if (!cliPath) {
                // reagent P2 on PR #2300: this step is resolving the CLI path,
                // not checking auth — "checking-auth" here mislabeled a wait
                // that can take up to 5 minutes (resolveCliForRecovery's own
                // install wait) as the wrong, much shorter step.
                setLaunchPhase({ kind: "resolving-cli" });
                cliPath = (await resolveCliForRecovery(prov, "re-login")) ?? undefined;
                if (!cliPath) return;
            }
            setLaunchPhase({ kind: "checking-auth" });
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
                        await sleep(2000);
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
                        // Tier 1's mint-only registration can still fail to
                        // persist (the same DB-write failure mode this PR's
                        // own report documents) — the `seeded`/
                        // `terminal-success` branch below already gates its
                        // success actions on this; this branch previously
                        // discarded the return value and reported success
                        // unconditionally, reproducing the exact "Error: not
                        // logged in" bug this PR set out to fix (reagent/
                        // codex P1 on the re-review of PR #2318).
                        const persisted = await persistAndLinkAccount(
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
                        if (persisted) {
                            opts.log("auth", retryAfterLogin ? "Login successful — retrying…" : "Login successful");
                            setAuthNotice(null);
                            setAuthStatus("authenticated");
                            // Before onRecovered (which may immediately resend the
                            // failed turn) — an already-running stale process must
                            // be refreshed BEFORE anything is sent to it, or the
                            // resend just hits the same stale credential again.
                            await forceControllerRefresh();
                            // Post a visible confirmation into the pane itself — this used to
                            // ONLY happen on the very first auto-login (launch-flow.ts); "Login
                            // Again" retried the failed turn silently, so a user with nothing
                            // queued to retry (or who didn't notice the retry) never saw ANY
                            // acknowledgement that the login actually succeeded.
                            opts.onLoginSuccess?.(authedEmail);
                            succeeded = true;
                            // Clear BEFORE onRecovered — that callback can
                            // synchronously resend the failed turn, and
                            // useAgentCommands.ts's fast-fail guard checks
                            // loginWaiting() before every send. Clearing only
                            // in this function's trailing `finally` (which
                            // runs AFTER onRecovered returns) let that resend
                            // get spuriously rejected as "not logged in yet"
                            // even though recovery just succeeded. reagent P0
                            // on PR #2338 (this is genuinely done at this
                            // point — forceControllerRefresh already
                            // completed above).
                            endThisRecoveryFlow();
                            if (retryAfterLogin) {
                                opts.onRecovered?.();
                            } else {
                                // Mount-time "Log in" success on an agent that never
                                // reached Phase 3 (no turn ever started — see
                                // launch-flow.ts's needsLogin bail). onReadyFn only
                                // ever fires from startLaunchFlow's own success
                                // branch, so without this a first-time login via
                                // this button left the agent running with no
                                // startup sequence ever sent (no instructions,
                                // identity, or context) — reagent/codex P1 on
                                // PR #2318. onReadyFn self-guards on
                                // `agent:sessionid` already being set, so this is a
                                // safe no-op for anything but a genuine first login.
                                opts.onReady?.();
                            }
                        } else {
                            setAuthStatus("unauthenticated");
                            setAuthNotice(
                                "Your login succeeded, but AgentMux couldn't save the account record. Try again in a moment.",
                            );
                        }
                    } else if (!loginCancelled) {
                        setAuthNotice(
                            "Opened a login page, but no login was detected within 5 minutes. " +
                            "Complete the login there, then click “Login Again”.",
                        );
                    }
                    break;
                }
                case "seeded":
                case "terminal-success":
                    // openedAccountId/openedAccountDir are only set by
                    // onAccountRegistered, which run-provider-login.ts fires
                    // ONLY once the account row is actually persisted — a
                    // credential can be validly seeded/typed-in on disk while
                    // that persist call itself fails (was previously silent;
                    // see REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md).
                    // Trusting the outcome string alone reported "Login
                    // successful" for a login the resolver's spawn gate would
                    // then block on the very next turn with no account ever
                    // having existed.
                    if (openedAccountId && openedAccountDir) {
                        opts.log(
                            "auth",
                            outcome === "seeded"
                                ? (retryAfterLogin ? "Signed in from your global login — retrying…" : "Signed in from your global login")
                                : (retryAfterLogin ? "Login successful — retrying…" : "Login successful"),
                        );
                        setAuthNotice(null);
                        setAuthStatus("authenticated");
                        await forceControllerRefresh();
                        opts.onLoginSuccess?.(null);
                        succeeded = true;
                        // See the "opened" branch above — must clear before
                        // onRecovered, not in the trailing finally. reagent
                        // P0 on PR #2338.
                        endThisRecoveryFlow();
                        if (retryAfterLogin) {
                            opts.onRecovered?.();
                        } else {
                            // See the identical "opened" case above — first-time
                            // login via the mount-time "Log in" button never
                            // otherwise triggers the startup sequence.
                            opts.onReady?.();
                        }
                    } else {
                        setAuthStatus("unauthenticated");
                        setAuthNotice(
                            "Your login succeeded, but AgentMux couldn't save the account record. Try again in a moment.",
                        );
                    }
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
            endThisRecoveryFlow();
            setLaunchPhase(null);
            // Restore the mount-time "Log in" button on any unsuccessful
            // outcome — timeout, terminal-unavailable, persistence failure,
            // cancellation, or a thrown exception all fall through to here
            // via `return`/`break`/the catch above. Scoped to
            // `!retryAfterLogin`: that's the only case where THIS call set
            // `canRetry` false in the first place (the "Log in" bar's own
            // click handler); the `retryAfterLogin: true` ("Login Again")
            // call site guards against a stale true from a *different*
            // origin and was never showing that bar to begin with, so it
            // must not start showing it now (reagent/codex on PR #2318).
            if (!succeeded && !retryAfterLogin) {
                setCanRetry(true);
            }
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
        // Unlike relogin()/loginViaTerminal(), this credential-seed work was
        // never reflected in loginWaiting — useAgentCommands.ts's fast-fail
        // guard checks canRetry() || loginWaiting() before letting a send
        // through, so without this a message typed while this async work is
        // still resolving bypassed that guard entirely and reached
        // AgentInputCommand on the same stale, already-known-bad credential
        // the failure banner is showing for. reagent P1 on PR #2338.
        //
        // beginRecoveryFlow/endRecoveryFlow (a shared counter, not a bare
        // boolean): nothing disables the failure row's OTHER recovery
        // buttons while this one is in flight, so a user can genuinely
        // start relogin()/loginViaTerminal() concurrently with this — a
        // bare setLoginWaiting(false) here would clear the flag out from
        // under that still-running flow. Codex P2 on PR #2338 (fourth
        // re-review).
        let recoveryEnded = false;
        const endThisRecoveryFlow = () => {
            if (recoveryEnded) return;
            recoveryEnded = true;
            endRecoveryFlow();
        };
        beginRecoveryFlow();
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
                // Credential is now valid on disk, but a persistent-mode
                // agent whose CLI is already alive won't pick it up on its
                // own — send_message only respawns a controller that isn't
                // already running. Force one before retrying so the resend
                // doesn't just hit the same stale process again (see
                // forceControllerRefresh's doc comment).
                opts.log("auth", "Signed in from your global login — retrying…");
                setAuthStatus("authenticated");
                await forceControllerRefresh();
                opts.onLoginSuccess?.(null);
                // See relogin()'s identical comment — must clear before
                // onRecovered, which can synchronously resend the failed
                // turn straight into useAgentCommands.ts's loginWaiting()
                // guard. reagent P0 on PR #2338.
                endThisRecoveryFlow();
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
            endThisRecoveryFlow();
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
        // Shared counter, not a bare boolean — see beginRecoveryFlow's own
        // doc comment: useGlobalLogin() is guarded by an independent
        // in-flight flag and can genuinely run concurrently with this.
        // Codex P2 on PR #2338 (fourth re-review).
        let recoveryEnded = false;
        const endThisRecoveryFlow = () => {
            if (recoveryEnded) return;
            recoveryEnded = true;
            endRecoveryFlow();
        };
        beginRecoveryFlow();
        setLaunchPhase({ kind: "opening-login-terminal" });
        try {
            let cliPath = getBlockMetaKeyAtom(opts.blockId, "cmd")() as string | undefined;
            if (!cliPath) {
                // Same H2 trap as relogin: the gated launch flow would trust the
                // auth check and skip the login the user explicitly asked for.
                // reagent P2 on PR #2300: label this step "resolving-cli", not
                // "opening-login-terminal" — no terminal opens until after this
                // resolve, which can itself take up to 5 minutes on a fresh install.
                setLaunchPhase({ kind: "resolving-cli" });
                cliPath = (await resolveCliForRecovery(prov, "login via terminal")) ?? undefined;
                if (!cliPath) return;
            }
            setLaunchPhase({ kind: "opening-login-terminal" });
            const authEnv = await recoveryAuthEnv(prov);
            const agentDefinitionId = getBlockMetaKeyAtom(opts.blockId, "agentId")() as string | undefined;
            // Captured so the switch below can gate its "Login successful"
            // messaging on the account having actually been persisted, not
            // just on the outcome string — see relogin()'s matching check
            // and REPORT_LOGIN_PERSIST_FAILURE_AND_STUCK_WORKING_2026_07_27.md.
            let registeredAccountId: string | undefined;
            let registeredAccountDir: string | undefined;
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
                onAccountRegistered: (accountId, dir) => {
                    registeredAccountId = accountId;
                    registeredAccountDir = dir;
                },
                // "fallback" still fires here even though skipTier1 is true —
                // run-provider-login.ts fires it unconditionally right after
                // the (skipped) tier-1 block, not conditioned on skipTier1 —
                // but this flow has nothing displayed for tier 1 to begin
                // with, so there's nothing to update on that event; only
                // "polling" (once the terminal actually opens) needs a phase
                // update here, giving an accurate deadline instead of leaving
                // the phase on a static "opening terminal" for the whole wait.
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
                    if (registeredAccountId && registeredAccountDir) {
                        opts.log("auth", "Login successful — retrying…");
                        setAuthNotice(null);
                        setAuthStatus("authenticated");
                        await forceControllerRefresh();
                        opts.onLoginSuccess?.(null);
                        // See relogin()'s identical comment — reagent P0 on
                        // PR #2338.
                        endThisRecoveryFlow();
                        opts.onRecovered?.();
                    } else {
                        setAuthStatus("unauthenticated");
                        setAuthNotice(
                            "Your login succeeded, but AgentMux couldn't save the account record. Try again in a moment.",
                        );
                    }
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
            endThisRecoveryFlow();
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
        // A live controllerstatus event proves the CLI is running turns right
        // now, independent of whichever recovery path (or none) got it there
        // — the same independent-proof reasoning this function already
        // applies to canRetry/authNotice above.
        setAuthStatus("authenticated");
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
        authStatus,
        startLaunchFlow,
        relogin,
        useGlobalLogin,
        loginViaTerminal,
        notifyControllerHealthy,
        forceControllerRefresh,
        cancelLogin,
        beginRecoveryFlow,
        endRecoveryFlow,
    };
}
