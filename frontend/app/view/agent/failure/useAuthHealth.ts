// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Split out of agent-view.tsx (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §3.5 step 9).
// Two hooks, called where their code sat in agent-view.tsx so effect order is
// unchanged: the synthetic row before the account-binding hook, the rest after
// it (it needs refreshLinkedAccountId).

import type { AgentPaneModel } from "@/app/store/agent-pane-registration";
import { snapshot as paneSnapshot } from "@/app/store/agent-pane-state-store";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import { muxEventSubscribe } from "@/app/store/mps";
import { sleep } from "@/util/util";
import { createEffect, onCleanup, untrack, type Accessor } from "solid-js";
import { retryRecheckAfterBind } from "./recheck-after-bind";
import { decideSyntheticRow } from "./synthetic-row";

type PaneStateAndDispatch = Pick<AgentPaneModel, "state" | "dispatchPane">;

export function useSyntheticAuthRow(opts: { canRetry: Accessor<boolean>; paneModel: PaneStateAndDispatch }): void {
    const { paneModel } = opts;
    // Pre-launch auth failure → the SAME failure row every other auth failure
    // uses, instead of the separate blue "Log in" bar this replaced
    // (docs/specs/PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md).
    //
    // The DECISION lives in decideSyntheticRow (failure/synthetic-row.ts) and
    // is unit-tested there; this is wiring only. It was extracted after this
    // effect produced several P1s across PR #2951 — it sits inline in the pane
    // component and no existing harness can reach it, so the logic was
    // unassertable while it lived here.
    //
    // Tracks BOTH canRetry and the failure signal. Tracking the failure is
    // what lets a dismissed REAL failure fall back to this row while the agent
    // is still unauthenticated (reagent P1) — without it the pane kept no login
    // affordance at all, strictly worse than the undismissable bar it replaced.
    // Dismissing THIS row still sticks; decideSyntheticRow tells the two apart
    // from the previous value, which is why that state is threaded here.
    let prevFailure: PaneFailure | null = null;
    let syntheticDismissed = false;
    createEffect(() => {
        const decision = decideSyntheticRow({
            canRetry: opts.canRetry(),
            current: paneModel.state.failure,
            previous: prevFailure,
            syntheticDismissed,
        });
        prevFailure = untrack(() => paneModel.state.failure);
        syntheticDismissed = decision.syntheticDismissed;
        if (decision.action === "raise") {
            paneModel.dispatchPane(
                {
                    type: "FailureObserved",
                    at: Date.now(),
                    turnAttempted: false,
                    failure: {
                        code: "auth",
                        title: "Not signed in",
                        detail:
                            "This agent hasn't been signed in to its provider yet, so it never started. " +
                            "Sign in to launch it — nothing has run, so there's no turn to retry.",
                        retryable: true,
                    },
                },
                "system"
            );
            // Keep prevFailure in step with what we just dispatched, so the
            // re-run this write triggers sees "our row is showing" rather than
            // "a row just appeared from nowhere".
            prevFailure = untrack(() => paneModel.state.failure);
        } else if (decision.action === "retract") {
            paneModel.dispatchPane({ type: "FailureCleared" }, "system");
            prevFailure = null;
        }
    });
}

/** What useAuthHealth reads outside its options; the real ones by default. */
export interface AuthHealthDeps {
    /** The live failure's code for this pane (from the pane-state store). */
    failureCode: () => string | undefined;
    subscribe: (sub: { eventType: string; handler: () => void }) => () => void;
    sleep: (ms: number) => Promise<void>;
}

export function useAuthHealth(opts: {
    blockId: string;
    agentDefinitionId: Accessor<string | undefined>;
    paneModel: PaneStateAndDispatch;
    status: {
        canRetry: Accessor<boolean>;
        notifyControllerHealthy: () => void;
        recheckAuthAfterBind: () => Promise<boolean>;
    };
    refreshLinkedAccountId: () => Promise<void>;
    deps?: Partial<AuthHealthDeps>;
}): { declareAuthHealthy: () => void } {
    const { paneModel } = opts;
    const deps: AuthHealthDeps = {
        failureCode: () => paneSnapshot(opts.blockId)?.failure?.data.code,
        subscribe: (sub) => muxEventSubscribe(sub),
        sleep,
        ...opts.deps,
    };

    // Declare the auth-blocking state resolved: clears canRetry/authNotice
    // (notifyControllerHealthy) and, ONLY when the live failure is actually
    // an auth failure, clears it too — never unconditionally, so an
    // unrelated concurrent failure (rate_limited, context_exceeded, …) that
    // happens to be showing isn't silently wiped. Shared by two independent
    // proofs of health: a live controllerstatus event showing an active turn
    // (below), and a verified auth re-check after an external bind (below).
    const declareAuthHealthy = () => {
        opts.status.notifyControllerHealthy();
        if (deps.failureCode() === "auth") {
            paneModel.dispatchPane({ type: "FailureCleared" }, "system");
        }
    };

    // Bounded retry around recheckAuthAfterBind — NOT a stylistic choice, a
    // correctness fix. `agentidentities:changed` is published by the
    // backend SYNCHRONOUSLY inside the `LinkAgentIdentityCommand` handler,
    // before it even responds to the RPC (the `COMMAND_LINK_AGENT_IDENTITY`
    // handler in agent_handlers/identity.rs);
    // RPC responses and WS events share one in-order connection, so this
    // pane's subscription below fires before `bindAccountToAgent`'s own
    // `SetMetaCommand` — which only runs AFTER that same Link RPC resolves
    // client-side — has refreshed `cmd:env` to the newly-bound account's
    // dir. The very first recheck therefore reads STALE env and fails on
    // essentially every bind, not as an edge case but as the common case —
    // reagentx P1 on PR #2969. Retry ladder lives in recheck-after-bind.ts,
    // unit-tested there (dependency-injected, no DOM/RPC mocking needed) —
    // kept out of this file for the same reason
    // PLAN_LOGIN_CTA_SURFACE_CONSOLIDATION_2026_09_02.md's retrospective
    // extracted decideSyntheticRow out of here after several P1s: inline
    // logic in this component is unassertable by any existing harness.
    const recheckAuthAfterBindWithRetry = () =>
        retryRecheckAfterBind({
            recheck: opts.status.recheckAuthAfterBind,
            stillBlocked: () => opts.status.canRetry() || deps.failureCode() === "auth",
            sleep: deps.sleep,
            onHealthy: declareAuthHealthy,
        });

    // Auto-unblock: a bind can happen from ANYWHERE (the Armory's
    // Bind-to-Agent menu, the per-agent Identity tab, or this pane's own
    // "Bind account" above) — this pane must notice regardless of source.
    // `agentidentities:changed:<agentId>` already fires on every one of
    // those; nothing previously listened for it here.
    //
    // Re-verifies via CheckCliAuth before declaring healthy (a bind event is
    // not itself proof the new credential works) and NEVER auto-retries a
    // turn — see recheckAuthAfterBind's and declareAuthHealthy's own doc
    // comments. SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md §2.
    createEffect(() => {
        const agentDefinitionId = opts.agentDefinitionId();
        if (!agentDefinitionId) return;
        const unsub = deps.subscribe({
            eventType: `agentidentities:changed:${agentDefinitionId}`,
            handler: () => {
                void opts.refreshLinkedAccountId();
                const blocked = opts.status.canRetry() || deps.failureCode() === "auth";
                if (!blocked) return;
                void recheckAuthAfterBindWithRetry();
            },
        });
        onCleanup(unsub);
    });

    return { declareAuthHealthy };
}
