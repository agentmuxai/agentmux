// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AuthFlowController — the side-effecting partner of the pure
 * `AuthState` reducer.
 *
 * Owns one reducer instance + a poll loop. The launch modal calls
 * `select` / `connect` / `cancel` / `submitCallback` / `submitApiKey`
 * on the controller; the controller dispatches commands into the
 * reducer and runs RPCs in response to the emitted events. The view
 * subscribes to `state()` for reactive reads.
 *
 * Why separate from the modal: the modal is JSX; this is the
 * non-DOM glue between reducer transitions and the backend. Splitting
 * keeps the modal short (renders state) and makes the orchestration
 * testable without a DOM.
 */

import { createSignal, untrack, type Accessor } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

import {
    initialState,
    update,
    type AuthCommand,
    type AuthSessionStatusWire,
    type AuthState,
    type SelectionOutcome,
} from "./auth-state";

/** Backend-facing RPC surface. Allowed to be injected so tests can
 *  swap in a stub. Production passes the `defaultAuthRpc` adapter.
 *
 *  `start()`'s request shape is direct-account only (issue #1624
 *  PR-C Part B) — `AuthFlowController` has exactly one production
 *  consumer (`AgentLaunchModal`, via `PreLaunchAuthPanel`), and that
 *  caller always wants a standalone account, never a bundle. No
 *  `intoBundleId` field: there is no remaining bundle-mode caller of
 *  this interface to keep it for. */
export interface AuthRpc {
    start(req: {
        providerId: string;
        directAccount: true;
        existingAccountId?: string;
        cliPath: string;
        authLoginArgs: string[];
        authCheckArgs: string[];
        authEnv?: Record<string, string>;
        requiresTty?: boolean;
    }): Promise<{ sessionId: string; authUrl?: string }>;
    poll(sessionId: string): Promise<AuthSessionStatusWire>;
    submitCallback(sessionId: string, callbackUrl: string): Promise<void>;
    cancel(sessionId: string): Promise<void>;
    submitApiKey(req: {
        providerId: string;
        intoBundleId?: string;
        apiKey: string;
        accountName: string;
    }): Promise<{ bundleId: string }>;
}

const defaultAuthRpc: AuthRpc = {
    async start(req) {
        return RpcApi.AuthStartCommand(TabRpcClient, req);
    },
    async poll(sessionId) {
        // Backend flattens `providerId` alongside the status fields;
        // the reducer only cares about the status tag + its arms, so
        // drop providerId before handing to the controller.
        const { providerId: _providerId, ...status } = await RpcApi.AuthPollCommand(
            TabRpcClient,
            { sessionId },
        );
        return status as AuthSessionStatusWire;
    },
    async submitCallback(sessionId, callbackUrl) {
        const r = await RpcApi.AuthSubmitCallbackCommand(TabRpcClient, {
            sessionId,
            callbackUrl,
        });
        if (!r.success) {
            throw new Error(r.error ?? "auth.submitcallback rejected");
        }
    },
    async cancel(sessionId) {
        await RpcApi.AuthCancelCommand(TabRpcClient, { sessionId });
    },
    async submitApiKey(req) {
        const r = await RpcApi.AuthSubmitApiKeyCommand(TabRpcClient, req);
        if (!r.success) {
            throw new Error(r.error ?? "auth.submitapikey rejected");
        }
        // Reagent P1 on #850: until PR C of the spec wires the actual
        // bundle row creation on the backend, `bundleId` may be
        // missing on success. Treat that as a failure so the view
        // surfaces it instead of silently transitioning to `ready`
        // with an unusable identity reference.
        if (!r.bundleId) {
            throw new Error(
                "auth.submitapikey accepted the key but backend did not return a bundleId (PR C of the spec wires this).",
            );
        }
        return { bundleId: r.bundleId };
    },
};

/** Resolved CLI metadata the controller passes to `auth.start`. The
 *  view computes this from the provider table + a one-shot
 *  `ResolveCli` call. Keeping it injected (not fetched here) so the
 *  controller stays sync at connect-time and view-tests don't need
 *  to mock the CLI catalog. */
export interface ProviderCliMeta {
    cliPath: string;
    authLoginArgs: string[];
    authCheckArgs: string[];
    authEnv?: Record<string, string>;
    /** Spawn the auth login subprocess under a PTY (instead of plain
     *  piped stdio). Required by providers whose auth subcommand
     *  refuses to run when `isatty()==0` — currently OpenClaw's
     *  `models auth login`. Default false for backwards compat. */
    requiresTty?: boolean;
}

export interface AuthFlowOptions {
    rpc?: AuthRpc;
    /** Poll cadence while `kind === "waiting"`. Defaults to 1000ms.
     *  Tests override to 0 + a manual ticker so they don't depend
     *  on wall-clock time. */
    pollIntervalMs?: number;
    /** Override the timer source (for tests). Default is
     *  `setTimeout`/`clearTimeout`. */
    timers?: {
        setTimeout: (fn: () => void, ms: number) => unknown;
        clearTimeout: (handle: unknown) => void;
    };
    /** Externalize state ownership. When provided, the controller
     *  reads `state()` from `externalGetState` and writes via
     *  `externalDispatch` (which is expected to run the same
     *  pure `update()` reducer the controller would use internally).
     *  Used by AgentLaunchModal to fold auth state into the
     *  launch-flow-state slice (Stage 2d of
     *  SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md). When omitted
     *  (the default — preserves test/standalone use), the controller
     *  uses an internal Solid signal. */
    externalGetState?: () => AuthState;
    externalDispatch?: (cmd: AuthCommand) => void;
}

export class AuthFlowController {
    /** Internal Solid signal — fallback state holder for tests and
     *  any caller that doesn't externalize state via `opts.externalGetState`.
     *  Untouched when externalization is wired. */
    private _state = createSignal<AuthState>(initialState());
    state: Accessor<AuthState>;

    private rpc: AuthRpc;
    private pollIntervalMs: number;
    private timers: NonNullable<AuthFlowOptions["timers"]>;
    private pollHandle: unknown = null;
    /** Monotonically-increasing token incremented on every action that
     *  invalidates an in-flight RPC (selected, cancel, dispose, new
     *  submit). RPC completions check the token at start vs. now —
     *  stale results are ignored. Codex P2 on #850: prevents a slow
     *  `auth.submitapikey` completion from landing after the user
     *  swapped to a different bundle and submitted again. */
    private actionToken = 0;
    private externalDispatch: ((cmd: AuthCommand) => void) | null;
    /** Set only by `cancel()` — an explicit user Cancel click, not any
     *  other reason the state might leave `waiting` (reagent P2 on
     *  #2262: a caller polling via `state().kind !== "waiting"` as its
     *  own cancellation signal can't distinguish "user cancelled" from
     *  "state moved on for some other reason", which would misreport a
     *  non-cancel exit as a plain timeout). Reset at the start of every
     *  fresh `connect()` so a stale cancellation from a PRIOR attempt
     *  never bleeds into a new one. */
    private userCancelled = false;

    constructor(opts: AuthFlowOptions = {}) {
        this.rpc = opts.rpc ?? defaultAuthRpc;
        this.pollIntervalMs = opts.pollIntervalMs ?? 1000;
        this.timers = opts.timers ?? {
            setTimeout: (fn, ms) => setTimeout(fn, ms) as unknown,
            clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
        };
        // State accessor + dispatch path. If the caller injected
        // externalGetState + externalDispatch, route through them so
        // the launch-flow-state slice can be the single source of
        // truth. Otherwise keep the internal signal for back-compat.
        if (opts.externalGetState) {
            this.state = opts.externalGetState;
        } else {
            this.state = this._state[0];
        }
        this.externalDispatch = opts.externalDispatch ?? null;
    }

    /** Dispatch a command into the reducer, project state.
     *  Wrapped in `untrack` so callers running inside a Solid
     *  reactive scope (e.g. a createEffect) don't accidentally
     *  subscribe to `_state` via this read — that subscription
     *  would re-fire the calling effect on every subsequent
     *  dispatch and silently invalidate any session the caller
     *  had just started. */
    dispatch(command: AuthCommand): void {
        if (this.externalDispatch) {
            // External dispatch (e.g. through launch-flow-state) is
            // responsible for running the same pure `update()` reducer.
            // We don't touch `_state` in this path.
            this.externalDispatch(command);
            return;
        }
        untrack(() => {
            const prev = this._state[0]();
            const result = update(prev, command);
            if (result.state !== prev) {
                this._state[1](result.state);
            }
        });
    }

    // ── View-facing actions ───────────────────────────────────────

    selected(providerId: string, bundleId: string, outcome: SelectionOutcome): void {
        // Reagent + Codex P2 on #850 (round 6): cancel any live OAuth
        // session before clearing local state. Without this, switching
        // selection mid-login leaves the backend CLI subprocess
        // running until timeout because `Selected` wipes `sessionId`
        // and dispose()/cancel() can't find it anymore.
        const prev = this.state();
        // #853 also covers `authenticated` and `saving` — same
        // orphan-CLI hazard: OAuth has authenticated→saving→ready,
        // so a selection swap mid-savebundle would leak the backend
        // session. Reagent P1 on #853 round 10 caught the `saving`
        // gap; cancel/dispose already cover it.
        if (
            (prev.kind === "waiting" ||
                prev.kind === "authenticated" ||
                prev.kind === "saving") &&
            prev.sessionId !== ""
        ) {
            void this.rpc.cancel(prev.sessionId).catch(() => {});
        }
        // Bump the action token so any in-flight RPC completions for
        // the previous selection are recognized as stale and dropped.
        this.actionToken += 1;
        this.stopPolling();
        this.dispatch({ type: "Selected", providerId, bundleId, outcome });
    }

    async connect(cli: ProviderCliMeta): Promise<void> {
        const s = this.state();
        // Codex P2 on #854 round 2: bail if disposed. Without this,
        // `startConnect`'s ResolveCli/ensureAuthDir await chain can
        // call connect() after the modal was closed — the reducer
        // drops the resulting dispatches via state.closed, but
        // `rpc.start` still fires and spawns the provider CLI in the
        // background.
        if (s.closed) return;
        if (s.kind !== "unauthenticated" && s.kind !== "expired" && s.kind !== "failed") {
            return;
        }
        // Bump action token + capture so any pre-existing connect()
        // that's still awaiting auth.start can detect it's stale and
        // bail. Codex P1 on #850 (re-iterated): two concurrent
        // connect() calls would race; the older one must not dispatch.
        this.actionToken += 1;
        const myToken = this.actionToken;
        this.userCancelled = false;
        this.dispatch({ type: "ConnectClicked" });
        try {
            const { sessionId, authUrl } = await this.rpc.start({
                providerId: s.providerId,
                directAccount: true,
                // `intoBundleId` (unrenamed — see auth-state.ts's
                // foldPolled comment) carries the account id to
                // reconnect into when the outcome was `expired`/
                // `needs-account` against an already-selected account;
                // empty for a genuinely fresh connect.
                existingAccountId: s.intoBundleId || undefined,
                cliPath: cli.cliPath,
                authLoginArgs: cli.authLoginArgs,
                authCheckArgs: cli.authCheckArgs,
                authEnv: cli.authEnv,
                requiresTty: cli.requiresTty,
            });
            if (this.actionToken !== myToken) {
                // The user did something else (selected/cancelled/
                // started another connect) while auth.start was in
                // flight. Tell the backend to drop the orphan session
                // and bail before dispatching SessionStarted.
                void this.rpc.cancel(sessionId).catch(() => {});
                return;
            }
            this.dispatch({ type: "SessionStarted", sessionId, authUrl });
            // Codex P1 on #850: if the user changed the selection /
            // cancelled / disposed during the `await rpc.start`, the
            // reducer dropped SessionStarted (kind guard). Without
            // this check we'd still `schedulePoll(sessionId)` for a
            // session the user already left, polling — and keeping
            // alive — an orphan backend session. Detect via state
            // and tell the backend to drop the session.
            const after = this.state();
            if (after.kind !== "waiting" || after.sessionId !== sessionId) {
                void this.rpc.cancel(sessionId).catch(() => {});
                return;
            }
            this.schedulePoll(sessionId);
        } catch (e) {
            if (this.actionToken !== myToken) {
                // Stale rejection: another action invalidated this
                // connect attempt while auth.start was failing. Don't
                // clobber the newer state with this old error. Codex
                // P2 on #850 (round 3).
                return;
            }
            // Force the failure transition through the reducer's
            // SessionStarted → Polled(failed) pair so the sessionId
            // gate passes.
            const synthSessionId = "auth-start-failed";
            this.dispatch({ type: "SessionStarted", sessionId: synthSessionId });
            this.dispatch({
                type: "Polled",
                sessionId: synthSessionId,
                status: { status: "failed", error: errMsg(e) },
            });
        }
    }

    async cancel(): Promise<void> {
        const s = this.state();
        // Reagent P1 on #853: also honor `authenticated` — backend
        // session is held alive there too (awaiting auth.savebundle).
        // Clicking Cancel from the SaveBundle panel must fire
        // auth.cancel so the orphan session is released. Mirrors
        // dispose()'s coverage of both kinds.
        //
        // Reagent P1 on #850: bump actionToken even when sessionId is
        // still "" (the startup window between ConnectClicked dispatch
        // and SessionStarted dispatch — auth.start in flight). The bump
        // invalidates the pending connect()'s actionToken gate so its
        // SessionStarted is dropped and the orphan session gets
        // cancelled by connect()'s stale-token path. User intent wins.
        // Reagent P1 on #853 round 9: also cover `saving` — the
        // backend session is still alive during the `auth.savebundle`
        // RPC (cleared only by `BundleSaved`); Cancel clicked there
        // must release it like in `waiting`/`authenticated`.
        if (s.kind !== "waiting" && s.kind !== "authenticated" && s.kind !== "saving") {
            return;
        }
        this.actionToken += 1;
        this.userCancelled = true;
        this.stopPolling();
        this.dispatch({ type: "CancelClicked" });
        const sessionId = s.sessionId;
        if (sessionId === "") return;
        try {
            await this.rpc.cancel(sessionId);
        } catch {
            // Cancel is best-effort. The reducer already moved out
            // of the live-session kinds; a stale backend session
            // times out on its own.
        }
    }

    /** True only after an explicit `cancel()` call for the CURRENT
     *  connect attempt — see `userCancelled`'s own doc comment. Callers
     *  driving their own completion poll (e.g. `runProviderLogin`'s
     *  `isCancelled`) should use this instead of inferring cancellation
     *  from `state().kind !== "waiting"`. */
    wasCancelled(): boolean {
        return this.userCancelled;
    }

    /** Set only while a `requiresLoginTty` provider's login (routed through
     *  `runProviderLogin` in `PreLaunchAuthPanel.tsx`, bypassing `connect()`
     *  entirely) is in flight. reagent P1 on #2262: the Reconnect arm for a
     *  stale (`needs_reauth`/`expired`) account leaves the reducer in
     *  `ready` the whole time — `connect()`'s own guard only accepts
     *  `ConnectClicked` from `unauthenticated`/`expired`/`failed`, so
     *  dispatching it from `ready` is silently dropped and `state().kind`
     *  never changes to `waiting`. That means the state machine has no way
     *  to represent "already in flight" for this specific origin state, so
     *  a second click while the first login is still running looked
     *  identical to the first click and spawned a second, concurrent
     *  terminal-login process against the same account dir. This flag is
     *  the explicit, state-machine-independent guard that gap needs. */
    private ttyLoginInFlight = false;

    /** Returns `false` (and leaves the flag untouched) if a tty-login is
     *  already in flight — the caller should treat that as "ignore this
     *  click," not retry or queue it. Returns `true` (and sets the flag) to
     *  claim the slot; the caller MUST call `endTtyLogin()` when done,
     *  success or failure, typically from a `finally` block. */
    beginTtyLogin(): boolean {
        if (this.ttyLoginInFlight) return false;
        this.ttyLoginInFlight = true;
        return true;
    }

    endTtyLogin(): void {
        this.ttyLoginInFlight = false;
    }

    /** Snapshot the current action generation — capture this before
     *  starting work whose completion should be ignored if the user moves
     *  on (changes selection, cancels, disposes) before it resolves. Same
     *  mechanism `connect()`/`submitCallback()` already use internally
     *  (`actionToken`), exposed for callers outside the class that drive
     *  their own async flow around the controller — specifically
     *  `PreLaunchAuthPanel.tsx`'s `requiresLoginTty` branch (reagent P1 on
     *  #2262): `runProviderLogin` there isn't gated through `connect()`'s
     *  own actionToken check at all, so a `Selected` dispatch mid-flight
     *  (the account/provider dropdown isn't disabled during a tty login)
     *  used to let the ABANDONED login's `Seeded`/`ConnectFailed` outcome
     *  land on top of whatever the user selected next. */
    currentActionToken(): number {
        return this.actionToken;
    }

    /** True if `token` (from an earlier `currentActionToken()`) no longer
     *  matches — i.e. `selected()`/`cancel()`/`dispose()` ran since it was
     *  captured, so whatever produced this result is stale and its outcome
     *  must not be dispatched. */
    isStaleAction(token: number): boolean {
        return token !== this.actionToken;
    }

    async submitCallback(callbackUrl: string): Promise<void> {
        const s = this.state();
        if (s.kind !== "waiting" || s.sessionId === "") return;
        const sessionId = s.sessionId;
        const myToken = this.actionToken;
        this.dispatch({ type: "CallbackSubmitted", callbackUrl });
        try {
            await this.rpc.submitCallback(sessionId, callbackUrl);
        } catch (e) {
            if (this.actionToken !== myToken) {
                // Stale rejection: user has moved on (selected/
                // cancelled/disposed). Don't clobber the newer state
                // with this old failure. Codex P2 on #850.
                return;
            }
            // Failure here means the URL was rejected; transition
            // to failed so the user sees the error.
            this.dispatch({
                type: "Polled",
                sessionId,
                status: { status: "failed", error: errMsg(e) },
            });
            this.stopPolling();
        }
    }

    async submitApiKey(apiKey: string, accountName: string): Promise<void> {
        const s = this.state();
        // Codex P2 on #850 (round 6): mirror the reducer's
        // ApiKeySubmitted gate (unauthenticated|expired|failed) so
        // double-clicks and stale invocations don't double-submit to
        // the backend nor flip an OAuth waiting flow to ready with a
        // bundleId from a wrong-state submit.
        if (s.kind !== "unauthenticated" && s.kind !== "expired" && s.kind !== "failed") {
            return;
        }
        // Bump action token + capture so a late completion can detect
        // that the user already started another submit (codex P2 on #850).
        this.actionToken += 1;
        const myToken = this.actionToken;
        this.dispatch({
            type: "ApiKeySubmitted",
            apiKey,
            accountName,
        });
        try {
            const { bundleId } = await this.rpc.submitApiKey({
                providerId: s.providerId,
                intoBundleId: s.intoBundleId || undefined,
                apiKey,
                accountName,
            });
            if (this.actionToken !== myToken) {
                // Stale completion: user changed selection or started
                // another submit while this was in flight. Codex P2
                // on #850.
                return;
            }
            // API-key flow stays single-phase until backend C-2 lands
            // (see `auth-state.ts` ApiKeyAccepted comment).
            this.dispatch({ type: "ApiKeyAccepted", bundleId });
        } catch (e) {
            if (this.actionToken !== myToken) {
                // Stale rejection: a newer submit already succeeded
                // and moved state to `ready`. Don't clobber it with
                // this old failure. Reagent P1 on #850 round 4.
                return;
            }
            const synthSessionId = "apikey-submit-failed";
            this.dispatch({ type: "SessionStarted", sessionId: synthSessionId });
            this.dispatch({
                type: "Polled",
                sessionId: synthSessionId,
                status: { status: "failed", error: errMsg(e) },
            });
        }
    }

    /** Surface a view-side connect-prep failure (e.g. `ResolveCli`
     *  threw before `auth.start` could fire) as a `failed` state.
     *  Uses the `ConnectFailed` command, which the reducer honors
     *  only from connect-attempt kinds (`unauthenticated`/`expired`/
     *  `failed`/`waiting`). If the user has already moved to
     *  `authenticated`/`saving`/`ready`/`idle`, a stale rejection from
     *  an abandoned connect is dropped instead of clobbering the
     *  newer state — codex P2 on #853 round 7 + codex P2 on #854. */
    failConnect(error: unknown): void {
        const message = error instanceof Error ? error.message : String(error);
        this.dispatch({ type: "ConnectFailed", error: message });
    }

    /** Seed-from-global accepted — the agent's isolated dir now holds the
     *  user's valid global credential, so auth is satisfied WITHOUT in-app
     *  OAuth (the dead end for Claude v2.1.x; SPEC_HOST_CLI_LOGIN_CAPTURE §0).
     *  The seed RPC itself is fired by the view (PreLaunchAuthPanel via the
     *  seed-global-login flow); this records the success in the state machine
     *  so the reducer transitions to `ready` and the Launch button enables.
     *  Single-phase — no session, no poll. */
    markSeeded(bundleId: string): void {
        this.dispatch({ type: "Seeded", bundleId });
    }

    dispose(): void {
        // Fire-and-forget auth.cancel for any in-flight session so we
        // don't leave an orphan CLI subprocess on the backend. Reagent
        // P1 on #853: covers `authenticated` (CLI is done, backend
        // session held alive awaiting `auth.savebundle`) AND `saving`
        // (savebundle RPC in flight, session cleared only by
        // BundleSaved) — unmounting from any of these kinds must
        // still tell the backend to release the session.
        this.actionToken += 1;
        const s = this.state();
        const hasLiveSession =
            (s.kind === "waiting" || s.kind === "authenticated" || s.kind === "saving") &&
            s.sessionId !== "";
        if (hasLiveSession) {
            void this.rpc.cancel(s.sessionId).catch(() => {});
        }
        this.stopPolling();
        this.dispatch({ type: "Disposed" });
    }

    // ── Poll loop ─────────────────────────────────────────────────

    private schedulePoll(sessionId: string): void {
        this.stopPolling();
        this.pollHandle = this.timers.setTimeout(() => {
            void this.pollOnce(sessionId);
        }, this.pollIntervalMs);
    }

    private stopPolling(): void {
        if (this.pollHandle != null) {
            this.timers.clearTimeout(this.pollHandle);
            this.pollHandle = null;
        }
    }

    private async pollOnce(sessionId: string): Promise<void> {
        // The reducer gates stale polls, but we also short-circuit
        // here so we don't fire RPCs for sessions the user already
        // left behind.
        const current = this.state();
        if (current.kind !== "waiting" || current.sessionId !== sessionId) {
            return;
        }
        try {
            const status = await this.rpc.poll(sessionId);
            this.dispatch({ type: "Polled", sessionId, status });
            // Re-check the post-dispatch state — terminal results
            // already cleared `sessionId` and changed `kind` away
            // from `waiting`, so the next branch handles itself.
            const next = this.state();
            if (next.kind === "waiting" && next.sessionId === sessionId && !isTerminal(status)) {
                this.schedulePoll(sessionId);
            }
        } catch (e) {
            // Cancel the backend session BEFORE dispatching the
            // failed transition — the reducer clears `sessionId` on
            // `failed`, after which `dispose()` can no longer reach
            // the auth.cancel path. Reagent P1 on #850 round 5.
            void this.rpc.cancel(sessionId).catch(() => {});
            this.dispatch({
                type: "Polled",
                sessionId,
                status: { status: "failed", error: errMsg(e) },
            });
        }
    }
}

function isTerminal(status: AuthSessionStatusWire): boolean {
    return status.status === "success" || status.status === "failed";
}

function errMsg(e: unknown): string {
    if (e instanceof Error) return e.message;
    return String(e);
}
