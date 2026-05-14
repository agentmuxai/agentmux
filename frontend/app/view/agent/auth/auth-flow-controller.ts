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

import { createSignal, type Accessor } from "solid-js";

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

    constructor(opts: AuthFlowOptions = {}) {
        this.rpc = opts.rpc ?? defaultAuthRpc;
        this.pollIntervalMs = opts.pollIntervalMs ?? 1000;
        this.timers = opts.timers ?? {
            setTimeout: (fn, ms) => setTimeout(fn, ms) as unknown,
            clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
        };
    }

    /** Dispatch a command into the reducer, project state. */
    dispatch(command: AuthCommand): void {
        const prev = this._state[0]();
        const result = update(prev, command);
        if (result.state !== prev) {
            this._state[1](result.state);
        }
    }

    // ── View-facing actions ───────────────────────────────────────

    selected(providerId: string, bundleId: string, outcome: SelectionOutcome): void {
        // Selecting clears any prior session; if there was an
        // in-flight poll, stop it.
        this.stopPolling();
        this.dispatch({ type: "Selected", providerId, bundleId, outcome });
    }

    async connect(cli: ProviderCliMeta): Promise<void> {
        const s = this.state();
        if (s.kind !== "unauthenticated" && s.kind !== "expired" && s.kind !== "failed") {
            return;
        }
        this.dispatch({ type: "ConnectClicked" });
        try {
            // Reagent P1 on #850: backend's auth.start returns
            // `{ sessionId, authUrl? }` (not `{ sessionId, status }`).
            // The initial status is "pending" until the CLI prints
            // something; the poll loop picks it up on the first tick.
            const { sessionId, authUrl } = await this.rpc.start({
                providerId: s.providerId,
                intoBundleId: s.bundleId || undefined,
                cliPath: cli.cliPath,
                authLoginArgs: cli.authLoginArgs,
                authCheckArgs: cli.authCheckArgs,
                authEnv: cli.authEnv,
            });
            this.dispatch({ type: "SessionStarted", sessionId, authUrl });
            this.schedulePoll(sessionId);
        } catch (e) {
            // Force the failure transition through the reducer's
            // SessionStarted → Polled(failed) pair so the sessionId
            // gate passes. Reagent P2 on #850: previously this also
            // dispatched a `Polled { sessionId: "" }` that was always
            // dropped by the gate — removed as dead code.
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
        this.dispatch({ type: "CallbackSubmitted", callbackUrl });
        try {
            await this.rpc.submitCallback(sessionId, callbackUrl);
        } catch (e) {
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
            this.dispatch({ type: "ApiKeyAccepted", bundleId });
        } catch (e) {
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
        // don't leave an orphan CLI subprocess on the backend. Codex
        // P2 on #850: previously dispose() only cleared the local
        // timer + marked closed.
        const s = this.state();
        if (s.kind === "waiting" && s.sessionId !== "") {
            void this.rpc.cancel(s.sessionId).catch(() => {
                // Best-effort: the session times out backend-side too.
            });
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
        // The reducer gates stale results, but we also short-circuit
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
