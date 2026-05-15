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
 *  swap in a stub. Production passes the `defaultAuthRpc` adapter. */
export interface AuthRpc {
    start(req: {
        providerId: string;
        intoBundleId?: string;
        cliPath: string;
        authLoginArgs: string[];
        authCheckArgs: string[];
        authEnv?: Record<string, string>;
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

export const defaultAuthRpc: AuthRpc = {
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
}

export class AuthFlowController {
    private _state = createSignal<AuthState>(initialState());
    state: Accessor<AuthState> = this._state[0];

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

    constructor(opts: AuthFlowOptions = {}) {
        this.rpc = opts.rpc ?? defaultAuthRpc;
        this.pollIntervalMs = opts.pollIntervalMs ?? 1000;
        this.timers = opts.timers ?? {
            setTimeout: (fn, ms) => setTimeout(fn, ms) as unknown,
            clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
        };
    }

    /** Dispatch a command into the reducer, project state.
     *  Wrapped in `untrack` so callers running inside a Solid
     *  reactive scope (e.g. a createEffect) don't accidentally
     *  subscribe to `_state` via this read — that subscription
     *  would re-fire the calling effect on every subsequent
     *  dispatch and silently invalidate any session the caller
     *  had just started. */
    dispatch(command: AuthCommand): void {
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
        // Selecting clears any prior session; if there was an
        // in-flight poll, stop it. Bump the action token so any
        // in-flight RPC completions for the previous selection are
        // recognized as stale and dropped.
        this.actionToken += 1;
        this.stopPolling();
        this.dispatch({ type: "Selected", providerId, bundleId, outcome });
    }

    async connect(cli: ProviderCliMeta): Promise<void> {
        const s = this.state();
        if (s.kind !== "unauthenticated" && s.kind !== "expired" && s.kind !== "failed") {
            return;
        }
        // Bump action token + capture so any pre-existing connect()
        // that's still awaiting auth.start can detect it's stale and
        // bail. Codex P1 on #850 (re-iterated): two concurrent
        // connect() calls would race; the older one must not dispatch.
        this.actionToken += 1;
        const myToken = this.actionToken;
        this.dispatch({ type: "ConnectClicked" });
        try {
            const { sessionId, authUrl } = await this.rpc.start({
                providerId: s.providerId,
                intoBundleId: s.bundleId || undefined,
                cliPath: cli.cliPath,
                authLoginArgs: cli.authLoginArgs,
                authCheckArgs: cli.authCheckArgs,
                authEnv: cli.authEnv,
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
        if (s.kind !== "waiting" || s.sessionId === "") return;
        const sessionId = s.sessionId;
        this.actionToken += 1;
        this.stopPolling();
        this.dispatch({ type: "CancelClicked" });
        try {
            await this.rpc.cancel(sessionId);
        } catch {
            // Cancel is best-effort. The reducer already moved out
            // of `waiting`; a stale backend session times out on
            // its own.
        }
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
                intoBundleId: s.bundleId || undefined,
                apiKey,
                accountName,
            });
            if (this.actionToken !== myToken) {
                // Stale completion: user changed selection or started
                // another submit while this was in flight. Drop result.
                return;
            }
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

    dispose(): void {
        // Fire-and-forget auth.cancel for any in-flight session so we
        // don't leave an orphan CLI subprocess on the backend.
        this.actionToken += 1;
        const s = this.state();
        if (s.kind === "waiting" && s.sessionId !== "") {
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
